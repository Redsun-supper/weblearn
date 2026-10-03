package handlers

import (
	"errors"
	"fmt"
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"

	"backend-go/internal/stablehash"
	"backend-go/middleware"
	"backend-go/models"
)

// ReviewHandler 词汇间隔复习（FSRS）相关处理器
type ReviewHandler struct {
	db *gorm.DB
}

// localDayKey 返回「按当地自然日」的哈希种子（例如 2026-10-02 → 20261002）。
//
// ⚠️ 浏览器与服务端必须用同一套「今天」，否则跨零点前后会出现两套「今日 5 个」。
// 契约：客户端传 `?today=YYYYMMDD`（它才知道用户的本地日期）；不传则回落 `Asia/Shanghai`。
// 这里刻意**不解析 tz=+08:00 这类偏移**：固定偏移在夏令时地区会在切换当天错一小时，
// 而这件事正是「哪一天」要避免的误差。
func localDayKey(c *gin.Context, now time.Time) int64 {
	if v := strings.TrimSpace(c.Query("today")); len(v) == 8 {
		if n, err := strconv.ParseInt(v, 10, 64); err == nil {
			// 合理性检查：2000-01-01 ~ 2100-12-31，防止脏参数把种子打乱
			if n >= 20000101 && n <= 21001231 {
				return n
			}
		}
	}
	return daySeed(now)
}

// NewReviewHandler 创建处理器实例
func NewReviewHandler(db *gorm.DB) *ReviewHandler {
	return &ReviewHandler{db: db}
}

// ---------- 单一循环池的公共口径（docs/review-pool-plan.md） ----------

// DST 判定用的时区名。**不要**改成 "Local"：LoadLocation("Local") 依赖宿主机配置，
// 本地测试与线上容器可能不一致，会让「今天」的含义随部署环境漂移。
const poolTZName = "Asia/Shanghai"

// dailyWordCount 每天置顶的随机词数（用户决策 A2：从整个池子里抽 5 个放最前面）
const dailyWordCount = 5

// maxIntervalDays 下次到期时间的**上限**（用户决策 B8：封顶一年）。
//
// 为什么必须有：FSRS 在连续评 Easy 时会把间隔推到几个月甚至几年，
// 于是「记得牢的词」会在池子里消失很久 —— 这正是用户说的「无法无限学下去」的根源。
// 封顶后每张卡一年内必定回到池子里一次。
const maxIntervalDays = 365

// poolTZ 返回统计与哈希种子所用的时区；加载失败时回落到固定 +8 时区（绝不静默用 UTC）。
func poolTZ() *time.Location {
	if loc, err := time.LoadLocation(poolTZName); err == nil {
		return loc
	}
	return time.FixedZone("UTC+8", 8*3600)
}

// daySeed 把「当地自然日」折成哈希种子（例如 2026-10-02 → 20260210）。
//
// ⚠️ 必须与 stablehash.DayKey 的口径一致，否则「今日 5 个」在前端与服务端会算出不同的批次。
// 这里复用 stablehash.DayKey，避免两处各写一遍。
func daySeed(t time.Time) int64 {
	y, m, d := t.In(poolTZ()).Date()
	return stablehash.DayKey(y, int(m), d)
}

// dailyInSQL 把「今日 5 个」的 word_id 列表拼成可直接接 `IN` 的括号串。
//
// 为什么由调用方先把 id 取出来、而不是在 SQL 里写子查询（曾经那样写过）：
//   - 子查询里的词表有别名（`w2`），哈希表达式必须跟着写成 `w2.id`，很容易与主查询的
//     `words.id` 写混 —— 实测报 `no such column`；
//   - 同一条语句里会出现 3~4 个一模一样的子查询，白算几遍。
//
// 列表为空时返回 `(NULL)`：`x IN (NULL)` 恒为 NULL（既非真也非假），
// 语义上等价于「今天没有置顶卡」，不会误判。
func dailyInSQL(ids []uint) string {
	if len(ids) == 0 {
		return "(NULL)"
	}
	parts := make([]string, 0, len(ids))
	for _, id := range ids {
		parts = append(parts, strconv.FormatUint(uint64(id), 10))
	}
	return "(" + strings.Join(parts, ",") + ")"
}

// poolBucketSQL 返回优先级桶表达式：
//
//	0 = 今日 5 个（置顶）  1 = 已过期  2 = 从未复习  3 = 未到期
//
// 用户决策 C10 定的顺序：今日 5 个 → 已过期（越久越前）→ 未复习过 → 未到期。
func poolBucketSQL(daily []uint, now time.Time) string {
	return fmt.Sprintf(`(CASE
		WHEN words.id IN %s THEN 0
		WHEN wr.due_at IS NOT NULL AND wr.due_at <= '%s' THEN 1
		WHEN wr.id IS NULL THEN 2
		ELSE 3 END)`, dailyInSQL(daily), now.Format("2006-01-02 15:04:05"))
}

// poolOrderSQL 返回池子的稳定排序：先按优先级桶，再按桶内键。
//
// 桶 0 / 2（没有到期时间可比）用**稳定哈希**排；桶 1 / 3 用 due_at 排。
// 哈希负责「按用户随机但同一天固定」，due_at 负责「越久没过期越靠前」。
func poolOrderSQL(userID, seed int64, daily []uint, now time.Time) string {
	return fmt.Sprintf(`%s, %s, wr.due_at ASC, words.id ASC`,
		poolBucketSQL(daily, now), stablehash.SQLHashExpr("words.id", userID, seed))
}

// poolSelectSQL 是池查询的公共 SELECT 列表。
//
// has_review / daily / bucket 都是**计算列**：
//   - has_review：LEFT JOIN 之后判断这一侧到底有没有进度行（NULL 说明从没复习过）
//   - daily：这张卡是不是「今日 5 个」之一
//   - bucket：优先级桶序号，供前端分组展示
//
// ⚠️ stability / difficulty 目前**不回给客户端**（用户决策 E18：库里留着、响应里删掉），
// 因为新模型下引擎一律按「新卡」口径重算。将来要恢复 FSRS 累积，把这两列加回 SELECT 即可。
func poolSelectSQL(userID, seed int64, daily []uint, now time.Time) string {
	// ⚠️ 进度列必须写 join 的别名 `wr`，不能写 `word_reviews.due_at`：
	// 一旦语句里的表只有一个别名，原表名就不再可引用（实测报 no such column）。
	return fmt.Sprintf(`
		words.id AS word_id, words.word, words.phonetic, words.meaning, words.example,
		words.example_translation, words.senses, words.subject,
		wr.due_at, wr.last_review_at,
		(wr.id IS NOT NULL) AS has_review,
		(words.id IN %s) AS daily,
		%s AS bucket
	`, dailyInSQL(daily), poolBucketSQL(daily, now))
}

// currentUser 取本次请求的用户 id；取不到就按未登录回 401 并返回 false。
//
// 正常路径上 RequireUser 中间件已经拦掉未登录请求，这里是**兜底**：
// 万一以后有人把某个复习接口挂到没鉴权的路由组上，宁可 401，也不能让 user_id = 0
// 的记录进库——那种行对任何用户都不可见，等于静默丢数据（词库共享、进度私有，
// 没有「公共进度」这种东西，见 models.WordReview 的注释）。
func currentUser(c *gin.Context) (uint, bool) {
	id, ok := middleware.CurrentUserID(c)
	if !ok {
		c.JSON(http.StatusUnauthorized, gin.H{"code": 401, "message": "请先登录", "error": "unauthenticated"})
		return 0, false
	}
	return id, true
}

// ---------- 请求/响应结构 ----------

// addWordItem 批量添加（POST /api/words）里的单条词目。
// example_translation / senses / book / unit 均可选；senses 留空时展示层按 meaning 里的词性标签自动分块。
type addWordItem struct {
	Word               string            `json:"word"`
	Phonetic           string            `json:"phonetic"`
	Meaning            string            `json:"meaning"`
	Example            string            `json:"example"`
	ExampleTranslation string            `json:"example_translation"`
	Senses             models.WordSenses `json:"senses"`
	Subject            string            `json:"subject"`
	Book               string            `json:"book"`
	Unit               string            `json:"unit"`
}

// updateWordRequest 更新词条请求
// 语义为全量更新：后台表单会把所有字段一起提交，未填的字段即视为清空
type updateWordRequest struct {
	Word               string            `json:"word"`
	Phonetic           string            `json:"phonetic"`
	Meaning            string            `json:"meaning"`
	Example            string            `json:"example"`
	ExampleTranslation string            `json:"example_translation"`
	Senses             models.WordSenses `json:"senses"`
	Subject            string            `json:"subject"`
	Book               string            `json:"book"`
	Unit               string            `json:"unit"`
}

type addWordsRequest struct {
	Words []addWordItem `json:"words"`
}

// submitReviewRequest 提交一次复习（POST /api/reviews/submit）的请求体。
// stability / difficulty / interval_days 都是引擎算好的新状态，后端只负责落库，不重算。
type submitReviewRequest struct {
	WordID        uint    `json:"word_id"`
	Rating        uint8   `json:"rating"` // 1=Again 2=Hard 3=Good 4=Easy
	Stability     float64 `json:"stability"`
	Difficulty    float64 `json:"difficulty"`
	IntervalDays  float64 `json:"interval_days"` // 引擎算出的下次间隔（天）
	DesiredRetain float64 `json:"desired_retention"`
	// IsProbe 标记「每日抽查」卡：这类卡评分时引擎按**新卡**重算记忆状态（相当于重新体检）。
	// 记这个标记是为了日后做 FSRS 参数优化时能排除这批「间隔被大幅压缩」的记录。
	IsProbe bool `json:"is_probe"`
	// IsReset 标记「重置重学」：单一循环池里，今日 5 个可能命中**已经学过**的词，
	// 引擎按新卡口径重算（见 docs/review-pool-plan.md 的 C11）。
	//
	// ⚠️ 它对统计有直接影响：日志里的 stability_before 会被强制写成 0，
	// 于是「今日新学」不会把这种循环重学算成新学（用户决策：统计只认真正的第一次学）。
	// 旧模型只有抽查会重置，所以当时靠「stability_before 取库里的真实旧值」就够了；
	// 新模型里重置是常态，必须有显式标记。
	IsReset bool `json:"is_reset"`
}

// 复习卡（单词 + FSRS 记忆状态）。
//
// ⚠️ 单一循环池（docs/review-pool-plan.md）之后，**池子是 `words` 全表**，
// 所以这里会出现「没有进度行」的卡（从没复习过的词）：
//   - stability / difficulty / due_at / last_review_at / reps 全是**指针**，来自 LEFT JOIN 的 NULL
//     会被序列化成 JSON null；引擎把 null 当作「新卡」（state = None、天数按 0）处理。
//   - 早先这几个字段是**非指针 float64**，LEFT JOIN 的 NULL 会被扫成 0 —— 于是「没学过」和
//     「学过但稳定度算出来是 0」无法区分。改成指针后这个坑一并解决（用户决策 E18）。
//
// daily / bucket 是单一循环池新增的排序信息：
//   - daily：这张卡属于「今日随机抽出的 5 个」之一（置顶）。引擎对这类卡按新卡口径重算，
//     提交时带 is_reset，使统计不把它记成「今日新学」。
//   - bucket：0 = 今日 5 个，1 = 已过期，2 = 从未复习，3 = 未到期。界面可据此分组。
type dueCard struct {
	ID                 uint              `json:"id" gorm:"column:word_id"`
	Word               string            `json:"word" gorm:"column:word"`
	Phonetic           string            `json:"phonetic" gorm:"column:phonetic"`
	Meaning            string            `json:"meaning" gorm:"column:meaning"`
	Example            string            `json:"example" gorm:"column:example"`
	ExampleTranslation string            `json:"example_translation" gorm:"column:example_translation"`
	Senses             models.WordSenses `json:"senses" gorm:"column:senses"`
	Subject            string            `json:"subject" gorm:"column:subject"`
	Stability          *float64          `json:"stability,omitempty" gorm:"column:stability"`
	Difficulty         *float64          `json:"difficulty,omitempty" gorm:"column:difficulty"`
	DueAt              *time.Time        `json:"due_at" gorm:"column:due_at"`
	LastReviewAt       *time.Time        `json:"last_review_at" gorm:"column:last_review_at"`
	Reps               *uint             `json:"reps,omitempty" gorm:"column:reps"`
	HasReview          bool              `json:"has_review" gorm:"column:has_review"`
	Daily              bool              `json:"daily" gorm:"column:daily"`
	Bucket             int               `json:"bucket" gorm:"column:bucket"`
}

// normalizeSenses 清洗多释义：去空白、丢掉整条为空的项
// 后台表单里「加了一行又没填」不应该在库里留下一条空释义
func normalizeSenses(list models.WordSenses) models.WordSenses {
	out := make(models.WordSenses, 0, len(list))
	for _, s := range list {
		item := models.WordSense{
			Pos:         strings.TrimSpace(s.Pos),
			Meaning:     strings.TrimSpace(s.Meaning),
			Example:     strings.TrimSpace(s.Example),
			Translation: strings.TrimSpace(s.Translation),
		}
		// 词性与释义都空：这一行没填（只填了例句也算），丢弃
		if item.Pos == "" && item.Meaning == "" {
			continue
		}
		out = append(out, item)
	}
	return out
}

// ---------- 接口实现 ----------

// wordQuery 依据查询参数构造词条过滤条件
// 列表与总数共用同一个条件，保证分页信息与实际结果口径一致
// 支持参数：subject / book / unit / search（关键词匹配单词或释义）
func (h *ReviewHandler) wordQuery(c *gin.Context) *gorm.DB {
	q := h.db.Model(&models.Word{})
	if subject := strings.TrimSpace(c.Query("subject")); subject != "" {
		q = q.Where("subject = ?", subject)
	}
	if book := strings.TrimSpace(c.Query("book")); book != "" {
		q = q.Where("book = ?", book)
	}
	if unit := strings.TrimSpace(c.Query("unit")); unit != "" {
		q = q.Where("unit = ?", unit)
	}
	if search := strings.TrimSpace(c.Query("search")); search != "" {
		like := "%" + search + "%"
		q = q.Where("word LIKE ? OR meaning LIKE ?", like, like)
	}
	return q
}

// parseIDParam 解析路径参数 :id，返回 0 表示不合法
func parseIDParam(c *gin.Context) uint {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		return 0
	}
	return uint(id)
}

// ListWords 获取单词列表（后台表格用）
// GET /api/words?limit=20&offset=0&subject=english&book=必修一&unit=Unit 1&search=apple
func (h *ReviewHandler) ListWords(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 500 {
		limit = 20
	}
	offset, _ := strconv.Atoi(c.DefaultQuery("offset", "0"))
	if offset < 0 {
		offset = 0
	}

	var total int64
	if err := h.wordQuery(c).Count(&total).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "统计失败:" + err.Error()})
		return
	}

	var words []models.Word
	if err := h.wordQuery(c).Order("id ASC").Limit(limit).Offset(offset).Find(&words).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items":  words,
		"total":  total,
		"limit":  limit,
		"offset": offset,
	}})
}

// GetWord 获取单个词条
// GET /api/words/:id
func (h *ReviewHandler) GetWord(c *gin.Context) {
	id := parseIDParam(c)
	if id == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "id 不合法"})
		return
	}
	var word models.Word
	if err := h.db.First(&word, id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			c.JSON(http.StatusNotFound, gin.H{"code": 404, "message": "词条不存在"})
			return
		}
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": word})
}

// WordOptions 获取词条中已使用的词书 / 单元去重列表（后台筛选下拉用）
// GET /api/word-options
// 注：放在 /api/word-options 而不是 /api/words/options，是为了避免与 /api/words/:id 的通配路由冲突
func (h *ReviewHandler) WordOptions(c *gin.Context) {
	// 显式初始化成空切片：Go 的 nil 切片会被序列化成 null，前端要额外兜底
	books := []string{}
	units := []string{}
	if err := h.db.Model(&models.Word{}).Where("book <> ''").
		Distinct().Order("book ASC").Pluck("book", &books).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	if err := h.db.Model(&models.Word{}).Where("unit <> ''").
		Distinct().Order("unit ASC").Pluck("unit", &units).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"books": books,
		"units": units,
	}})
}

// UpdateWord 更新词条内容
// PUT /api/words/:id
// 说明：全量更新；改名时会先检查是否与其它词条重名（words.word 是唯一索引），
// 冲突返回 409 而不是让数据库报错，便于后台给出友好提示。
func (h *ReviewHandler) UpdateWord(c *gin.Context) {
	id := parseIDParam(c)
	if id == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "id 不合法"})
		return
	}
	var req updateWordRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "请求体格式错误: " + err.Error()})
		return
	}
	word := strings.TrimSpace(req.Word)
	if word == "" {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "word 不能为空"})
		return
	}

	var record models.Word
	if err := h.db.First(&record, id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			c.JSON(http.StatusNotFound, gin.H{"code": 404, "message": "词条不存在"})
			return
		}
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	if word != record.Word {
		var dup models.Word
		err := h.db.Where("word = ?", word).First(&dup).Error
		if err == nil {
			c.JSON(http.StatusConflict, gin.H{"code": 409, "message": "已存在同名单词：" + word})
			return
		}
		if !errors.Is(err, gorm.ErrRecordNotFound) {
			c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
			return
		}
	}

	record.Word = word
	record.Phonetic = strings.TrimSpace(req.Phonetic)
	record.Meaning = strings.TrimSpace(req.Meaning)
	record.Example = strings.TrimSpace(req.Example)
	record.ExampleTranslation = strings.TrimSpace(req.ExampleTranslation)
	record.Senses = normalizeSenses(req.Senses)
	record.Book = strings.TrimSpace(req.Book)
	record.Unit = strings.TrimSpace(req.Unit)
	if subject := strings.TrimSpace(req.Subject); subject != "" {
		record.Subject = subject
	}

	if err := h.db.Save(&record).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "保存失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "更新成功", "data": record})
}

// DeleteWord 删除词条
// DELETE /api/words/:id
//
// ⚠️ 会连同该词的复习状态（word_reviews）与复习日志（review_logs）一并删除。
// 日志若不删，会变成指向不存在词条的脏数据，导致 /api/reviews/stats 的
// 累计复习次数与记忆保持率虚高。
// 只是想改错别字的话请用 PUT 更新，不要删了重建（否则记忆进度会丢失）。
func (h *ReviewHandler) DeleteWord(c *gin.Context) {
	id := parseIDParam(c)
	if id == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "id 不合法"})
		return
	}

	var record models.Word
	if err := h.db.First(&record, id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			c.JSON(http.StatusNotFound, gin.H{"code": 404, "message": "词条不存在"})
			return
		}
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	// ⚠️ 删词条是**跨用户**操作：词条本身是共享的，删掉它就等于删掉所有人对这个词的进度。
	// 计数与删除都故意不按 user_id 过滤（后台要看到「这一下删了多少人的多少条记录」）。
	// 「软删除 / 只下架内容、保留进度」的方案本期不考虑（见 docs/launch-plan.md 的 P0-1 决策表）。
	var reviewCount int64
	h.db.Model(&models.WordReview{}).Where("word_id = ?", id).Count(&reviewCount)
	var logCount int64
	h.db.Model(&models.ReviewLog{}).Where("word_id = ?", id).Count(&logCount)

	err := h.db.Transaction(func(tx *gorm.DB) error {
		if err := tx.Where("word_id = ?", id).Delete(&models.WordReview{}).Error; err != nil {
			return err
		}
		if err := tx.Where("word_id = ?", id).Delete(&models.ReviewLog{}).Error; err != nil {
			return err
		}
		return tx.Delete(&models.Word{}, id).Error
	})
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "删除失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "删除成功", "data": gin.H{
		"deleted_word":    record.Word,
		"removed_reviews": reviewCount,
		"removed_logs":    logCount,
	}})
}

// AddWords 批量添加单词
// POST /api/words
func (h *ReviewHandler) AddWords(c *gin.Context) {
	var req addWordsRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "请求体格式错误: " + err.Error()})
		return
	}
	if len(req.Words) == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "words 不能为空"})
		return
	}

	// 一次性取出已存在的单词建索引，避免逐条 SELECT（导入几千词时差别很大：原来是 N 次查询 + N 次插入）
	var existingWords []string
	if err := h.db.Model(&models.Word{}).Pluck("word", &existingWords).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	// 用「小写」做键：把 Abandon / abandon 视为同一个词，避免词库里出现仅大小写不同的重复项
	seen := make(map[string]bool, len(existingWords))
	for _, w := range existingWords {
		seen[strings.ToLower(w)] = true
	}

	skipped := 0
	records := make([]models.Word, 0, len(req.Words))
	for _, item := range req.Words {
		word := strings.TrimSpace(item.Word)
		if word == "" {
			skipped++
			continue
		}
		key := strings.ToLower(word)
		if seen[key] {
			skipped++
			continue
		}
		seen[key] = true // 同一批内部也去重

		subject := strings.TrimSpace(item.Subject)
		if subject == "" {
			subject = "english"
		}
		records = append(records, models.Word{
			Word:               word,
			Phonetic:           strings.TrimSpace(item.Phonetic),
			Meaning:            strings.TrimSpace(item.Meaning),
			Example:            strings.TrimSpace(item.Example),
			ExampleTranslation: strings.TrimSpace(item.ExampleTranslation),
			Senses:             normalizeSenses(item.Senses),
			Subject:            subject,
			Book:               strings.TrimSpace(item.Book),
			Unit:               strings.TrimSpace(item.Unit),
		})
	}

	// 分批插入，避免单条 SQL 的参数过多
	created := 0
	const batchSize = 200
	for i := 0; i < len(records); i += batchSize {
		end := i + batchSize
		if end > len(records) {
			end = len(records)
		}
		if err := h.db.Create(records[i:end]).Error; err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "写入失败:" + err.Error()})
			return
		}
		created += end - i
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "添加成功", "data": gin.H{
		"created": created,
		"skipped": skipped,
	}})
}

// DueReviews 获取到期复习卡列表
// GET /api/reviews/due?limit=20&now=1710000000000
// now 可选（Unix 毫秒），默认服务器当前时间；到期 = review.due_at <= now
func (h *ReviewHandler) DueReviews(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 200 {
		limit = 20
	}
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	nowParsed := time.Now()
	if nowMs, err := strconv.ParseInt(c.Query("now"), 10, 64); err == nil && nowMs > 0 {
		nowParsed = time.UnixMilli(nowMs)
	}

	seed := daySeed(nowParsed)
	daily := h.dailyWordIDs(userID, seed)
	var cards []dueCard
	// ⚠️ 必须按 user_id 过滤：词库共享，但每个人的到期时间各不相同。
	// 单一循环池之后本接口只给「已过期」这一类（桶 1）——引擎不再调用它，
	// 保留是为了兼容与排障（客户端用的是 /api/reviews/queue）。
	err := h.db.Model(&models.Word{}).
		Select(poolSelectSQL(int64(userID), seed, daily, nowParsed)).
		Joins("LEFT JOIN word_reviews wr ON wr.word_id = words.id AND wr.user_id = ?", userID).
		Where("wr.due_at IS NOT NULL AND wr.due_at <= ?", nowParsed).
		Order("wr.due_at ASC").
		Limit(limit).
		Scan(&cards).Error
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items": cards,
		"now":   nowParsed.UnixMilli(),
	}})
}

// NewWords 在新模型（单一循环池）下**已废弃**：池子 = words 全表，不再有「未加入复习的词列表」。
// GET /api/reviews/new?limit=20
//
// 保留路由与字段形状（`{items: []}`）只为两条：① 旧前端缓存页面不会拿到 404；
// ② 排障时能一眼看出「这个接口已经不再参与编排」。真正取卡请用 /api/reviews/queue。
func (h *ReviewHandler) NewWords(c *gin.Context) {
	if _, ok := currentUser(c); !ok {
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items":  []dueCard{},
		"total":  0,
		"legacy": true,
		"note":   "单一循环池模型下已废弃：池子就是整个词库，请改用 /api/reviews/queue",
	}})
}

// QueueReviews 复习队列 = **整个循环池**（`words` 全表 + 该用户的到期状态）。
// GET /api/reviews/queue?limit=100&offset=0&now=<ms>&today=<YYYYMMDD>
//
// 排序由服务端一次定死（用户决策 C10），四个优先级桶：
//
//	0 = 今日随机抽出的 5 个（置顶，每次刷新都是同一批 —— 见 stablehash 的稳定哈希）
//	1 = 已过期（due_at <= now，越久越靠前）
//	2 = 从未复习过（没有进度行）
//	3 = 未到期（due_at ASC）
//
// 桶内键：桶 1/3 用 due_at，桶 0/2 用「按 (user, 当天, word) 的稳定哈希」——
// 于是池子对每个用户是**随机但稳定**的排列，翻页不会重叠或漏卡。
//
// 分页：`total` 是**池子总词数**（= 词表行数），不再是「已学词数」。
// 想一直学下去就一路往下翻：桶 3 的卡即使没到期也会照常返回（用户决策 B9）。
func (h *ReviewHandler) QueueReviews(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "100"))
	if limit <= 0 || limit > 500 {
		limit = 100
	}
	offset, _ := strconv.Atoi(c.DefaultQuery("offset", "0"))
	if offset < 0 {
		offset = 0
	}

	nowParsed := time.Now()
	if nowMs, err := strconv.ParseInt(c.Query("now"), 10, 64); err == nil && nowMs > 0 {
		nowParsed = time.UnixMilli(nowMs)
	}
	seed := localDayKey(c, nowParsed)
	daily := h.dailyWordIDs(userID, seed)

	var total int64
	// ⚠️ total 与下面的 items 必须同一口径（都是整个池子），否则前端按 total 翻页会算错
	if err := h.db.Model(&models.Word{}).Count(&total).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "统计失败:" + err.Error()})
		return
	}

	var cards []dueCard
	err := h.db.Model(&models.Word{}).
		Select(poolSelectSQL(int64(userID), seed, daily, nowParsed)).
		Joins("LEFT JOIN word_reviews wr ON wr.word_id = words.id AND wr.user_id = ?", userID).
		Order(poolOrderSQL(int64(userID), seed, daily, nowParsed)).
		Limit(limit).Offset(offset).
		Scan(&cards).Error
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items":  cards,
		"total":  total,
		"limit":  limit,
		"offset": offset,
		"now":    nowParsed.UnixMilli(),
		"daily":  dailyWordCount,
	}})
}

// ProbeCandidates 在单一循环池模型下只作**只读诊断**用。
// GET /api/reviews/probes?limit=20
//
// 旧模型里它是「每天抽查」的候选来源（到期时间最远的已学词）。
// 新模型把「让冷门词露面」的职责交给了整个循环池（用户决策 C12：抽查降级为排序权重），
// 因此引擎不再调用它；这里保留「到期时间最远的前 N 个」这个视图，便于排障时看池尾。
func (h *ReviewHandler) ProbeCandidates(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 200 {
		limit = 20
	}

	nowParsed := time.Now()
	seed := localDayKey(c, nowParsed)
	daily := h.dailyWordIDs(userID, seed)

	var cards []dueCard
	err := h.db.Model(&models.Word{}).
		Select(poolSelectSQL(int64(userID), seed, daily, nowParsed)).
		Joins("LEFT JOIN word_reviews wr ON wr.word_id = words.id AND wr.user_id = ?", userID).
		Where("wr.due_at IS NOT NULL").
		Order("wr.due_at DESC").
		Limit(limit).
		Scan(&cards).Error
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items":  cards,
		"legacy": true,
		"note":   "新模型下引擎不再调用本接口，仅供排障查看池尾（到期最远的卡）",
	}})
}

// SubmitReview 提交一次复习结果（引擎已算好新状态，这里只做持久化）
// POST /api/reviews/submit
// 请求体：{ word_id, rating(1-4), stability, difficulty, interval_days, desired_retention? }
// 逻辑：更新/创建 word_reviews（due_at = now + max(interval_days*86400, 600) 秒），并记一条 review_logs
func (h *ReviewHandler) SubmitReview(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	var req submitReviewRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "请求体格式错误: " + err.Error()})
		return
	}
	if req.WordID == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "word_id 不能为空"})
		return
	}
	if req.Rating < 1 || req.Rating > 4 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "rating 必须为 1~4"})
		return
	}
	validState := !math.IsInf(req.Stability, 0) && !math.IsNaN(req.Stability) &&
		!math.IsInf(req.Difficulty, 0) && !math.IsNaN(req.Difficulty) &&
		req.Stability >= 0 && req.Difficulty >= 0 &&
		req.IntervalDays >= 0 && req.IntervalDays < 3650
	if !validState {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "stability/difficulty/interval_days 数值不合法"})
		return
	}
	var word models.Word
	if err := h.db.First(&word, req.WordID).Error; err != nil {
		c.JSON(http.StatusNotFound, gin.H{"code": 404, "message": "单词不存在"})
		return
	}

	now := time.Now()
	// 间隔最短 10 分钟（600 秒），防止 Again 后马上再次到期。
	// ⚠️ 下限必须与引擎侧一致（session.rs 的 due_ms = now + max(interval_days*86400000, 600000)），
	// 否则卡插回池子后的排序位置会与服务端实际的 due_at 有偏差。
	//
	// 上限见 maxIntervalDays（用户决策 B8：封顶一年）——FSRS 满分时会把间隔推到几年，
	// 那会让「记得牢的词」在池子里消失很久，是「无法无限学下去」的根源。
	// 这里与引擎侧（session.rs 的 MAX_INTERVAL_MS）用同一个 365 天口径，两侧必须同步改。
	intervalDays := math.Min(math.Max(req.IntervalDays, 0), maxIntervalDays)
	dueSeconds := math.Max(intervalDays*86400, 600)
	dueAt := now.Add(time.Duration(dueSeconds * float64(time.Second)))

	desiredRetention := req.DesiredRetain
	if desiredRetention <= 0 || desiredRetention >= 1 {
		desiredRetention = 0.9
	}

	var review models.WordReview
	// ⚠️ 只找**当前用户**对这一行的进度：词库共享，同一个人一行
	err := h.db.Where("user_id = ? AND word_id = ?", userID, req.WordID).First(&review).Error
	isNew := errors.Is(err, gorm.ErrRecordNotFound)
	if err != nil && !isNew {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	// 复习前的稳定度（用于日志）：取库里的真实旧值。
	// ⚠️ 新模型下「重置重学」是常态（今日 5 个可能命中已学词），必须靠 is_reset 把这类记录
	// 的 stability_before 记成 0，否则「今日新学」会被当天的循环重学冲高。
	stabilityBefore := review.Stability
	if req.IsReset {
		stabilityBefore = 0
	}
	if isNew {
		review = models.WordReview{UserID: userID, WordID: req.WordID}
	}

	review.Stability = req.Stability
	review.Difficulty = req.Difficulty
	review.DesiredRetention = desiredRetention
	review.DueAt = &dueAt
	review.LastReviewAt = &now
	review.Reps++
	if req.Rating == 1 { // Again 记一次遗忘
		review.Lapses++
	}

	err = h.db.Transaction(func(tx *gorm.DB) error {
		var saveErr error
		if isNew {
			saveErr = tx.Create(&review).Error
		} else {
			saveErr = tx.Save(&review).Error
		}
		if saveErr != nil {
			return saveErr
		}
		log := models.ReviewLog{
			UserID:          userID,
			WordID:          req.WordID,
			Rating:          req.Rating,
			StabilityBefore: stabilityBefore,
			StabilityAfter:  req.Stability,
			DifficultyAfter: req.Difficulty,
			IntervalDays:    intervalDays,
			ReviewedAt:      now,
			IsProbe:         req.IsProbe,
			IsReset:         req.IsReset,
		}
		return tx.Create(&log).Error
	})
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "保存失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "提交成功", "data": gin.H{
		"word_id": req.WordID,
		"due_at":  dueAt.UnixMilli(),
		"reps":    review.Reps,
		// interval_days 回传**截断后**的值：调用方能据此发现「服务端封顶生效了」，
		// 而不是只看到一个比预期早很多的 due_at（用户决策 B8，上限 365 天）
		"interval_days": intervalDays,
		"capped":        req.IntervalDays > maxIntervalDays,
	}})
}

// ReviewStats 复习统计
// GET /api/reviews/stats?today=YYYYMMDD
//
// 单一循环池模型下的口径（用户决策 D17）：
//
//	pool_size      池子总词数（= 整个词表；不再有「未学词」这个概念）
//	pool_due       池内**现在到期**的卡数（引擎用来决定「今天有没有活干」）
//	reviewed_words 你已经复习过的词数（进度行数，纯信息）
//	today_new      今日**真正第一次学**的词数（stability_before = 0 且不是重置重学）
//	today_reviewed 今日提交的总复习数（含重置重学）
//	streak_days / retention_rate 习惯指标（口径不变）
//
// ⚠️ 与旧版的区别：`new_words`（未学词数）已无意义并被移除；`today_new` 不再会被
// 「重置重学」冲高（靠 review_logs.is_reset 区分）。
func (h *ReviewHandler) ReviewStats(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	now := time.Now()
	// 今日零点（当地时区），用于统计「今日已复习」。
	// 客户端可传 ?today=YYYYMMDD 指定它自己的自然日（见 localDayKey）。
	startOfToday := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	if todayMs, err := strconv.ParseInt(c.Query("today_ms"), 10, 64); err == nil && todayMs > 0 {
		// 客户端直接给出「它当地今天零点」的时间戳时优先采用（跨时区用户也准）
		startOfToday = time.UnixMilli(todayMs)
		now = now.In(startOfToday.Location())
	} else {
		startOfToday = time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, poolTZ())
	}

	// 池子里总共多少词 = 整个词表（不再区分「学过 / 没学过」）
	var poolSize int64
	h.db.Model(&models.Word{}).Count(&poolSize)

	// 池内现在到期：有进度行、且 due_at <= 现在
	var dueCount int64
	h.db.Model(&models.WordReview{}).
		Where("user_id = ? AND due_at IS NOT NULL AND due_at <= ?", userID, now).
		Count(&dueCount)

	// 已经复习过的词数（纯信息：池子里有多少词已经有了进度行）
	var reviewedCount int64
	h.db.Model(&models.WordReview{}).Where("user_id = ?", userID).Count(&reviewedCount)

	// ---- 复习日志统计（今日进度 / 连续天数 / 记忆保持率）----
	var totalReviews int64
	h.db.Model(&models.ReviewLog{}).Where("user_id = ?", userID).Count(&totalReviews)

	var todayReviewed int64
	h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ?", userID, startOfToday).
		Count(&todayReviewed)

	// 今日新学：首次复习的日志里 stability_before 为 0，**且不是重置重学**。
	// 旧模型靠「抽查卡记库里的真实旧值」就能区分；新模型里重置是常态，必须看 is_reset。
	var todayNew int64
	h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ? AND stability_before = 0 AND is_reset = ?", userID, startOfToday, false).
		Count(&todayNew)

	todayReview := todayReviewed - todayNew
	if todayReview < 0 {
		todayReview = 0
	}

	// 今日完成「每日 5 个」的进度：今日 5 个里已经评过几个（不分新学 / 重置）
	dailyDone := h.countTodayDaily(c, userID, startOfToday, now)

	var againTotal int64
	h.db.Model(&models.ReviewLog{}).Where("user_id = ? AND rating = ?", userID, 1).Count(&againTotal)

	// 记忆保持率 = 非「忘记」评分所占比例（0~1，保留三位小数）
	retentionRate := 0.0
	if totalReviews > 0 {
		retentionRate = math.Round(float64(totalReviews-againTotal)/float64(totalReviews)*1000) / 1000
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"pool_size":      poolSize,
		"pool_due":       dueCount,
		"reviewed_words": reviewedCount,
		"total_reviews":  totalReviews,
		"today_reviewed": todayReviewed,
		"today_new":      todayNew,
		"today_review":   todayReview,
		"daily_target":   dailyWordCount,
		"daily_done":     dailyDone,
		"streak_days":    h.calcStreakDays(userID, startOfToday),
		"retention_rate": retentionRate,
		// 兼容字段：旧前端读 total_words / new_words / due_cards。
		// 新模型下池子就是全部词表，所以 total_words = pool_size；
		// 「未学词」这个概念已不存在，new_words 保留为「还没复习过的词数」（纯信息）。
		"total_words": poolSize,
		"new_words":   poolSize - reviewedCount,
		"due_cards":   dueCount,
	}})
}

// countTodayDaily 统计「今日 5 个」里已经评过几个。
//
// 判定方式：先按稳定哈希取出今日那 5 个 word_id，再看今天这些词有没有提交记录。
// 不用「进度行是否已存在」来判断——用户可能在别的日子就学过它（循环池的正常情况）。
func (h *ReviewHandler) countTodayDaily(c *gin.Context, userID uint, startOfToday, now time.Time) int64 {
	ids := h.dailyWordIDs(userID, localDayKey(c, now))
	if len(ids) == 0 {
		return 0
	}
	var done int64
	h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND word_id IN ? AND reviewed_at >= ?", userID, ids, startOfToday).
		Distinct("word_id").
		Count(&done)
	return done
}

// dailyWordIDs 取出「今日 5 个」的 word_id 列表（按稳定哈希排序取前 5）。
//
// 与队列里的 daily 桶用的是同一套哈希与同一个种子，所以列表必然一致 ——
// 两处口径若不同步，界面上的「今日 3/5」就会与实际置顶的卡对不上。
func (h *ReviewHandler) dailyWordIDs(userID uint, seed int64) []uint {
	var ids []uint
	h.db.Model(&models.Word{}).
		Select("words.id").
		Order(stablehash.SQLHashExpr("words.id", int64(userID), seed)).
		Limit(dailyWordCount).
		Pluck("words.id", &ids)
	return ids
}

// calcStreakDays 计算**某个用户**的连续复习天数
// 说明：以「本地自然日」为单位向前累计连续有复习记录的天数；
// 若今天还没有复习记录，则从昨天起算（这样当天刚开始时不会立刻显示断签）。
// 为避免依赖 SQLite 的日期函数与时区差异，这里只取回原始时间点，在 Go 侧换算自然日。
func (h *ReviewHandler) calcStreakDays(userID uint, startOfToday time.Time) int {
	// 最多回溯 400 天，足够覆盖任意真实连续记录
	var reviewedAt []time.Time
	if err := h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ?", userID, startOfToday.AddDate(0, 0, -400)).
		Pluck("reviewed_at", &reviewedAt).Error; err != nil {
		return 0
	}
	if len(reviewedAt) == 0 {
		return 0
	}

	days := make(map[string]bool, len(reviewedAt))
	for _, t := range reviewedAt {
		days[t.In(startOfToday.Location()).Format("2006-01-02")] = true
	}

	cursor := startOfToday
	if !days[cursor.Format("2006-01-02")] {
		cursor = cursor.AddDate(0, 0, -1)
	}
	streak := 0
	for days[cursor.Format("2006-01-02")] {
		streak++
		cursor = cursor.AddDate(0, 0, -1)
	}
	return streak
}

