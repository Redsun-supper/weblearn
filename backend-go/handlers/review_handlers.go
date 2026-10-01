package handlers

import (
	"errors"
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"

	"backend-go/middleware"
	"backend-go/models"
)

// ReviewHandler 词汇间隔复习（FSRS）相关处理器
type ReviewHandler struct {
	db *gorm.DB
}

// NewReviewHandler 创建处理器实例
func NewReviewHandler(db *gorm.DB) *ReviewHandler {
	return &ReviewHandler{db: db}
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
	// ⚠️ 日志里的 stability_before 仍是数据库里的真实旧值，所以「今日新学」统计不会被它污染；
	// 记这个标记是为了日后做 FSRS 参数优化时能排除这批「间隔被大幅压缩」的记录。
	IsProbe bool `json:"is_probe"`
}

// 到期复习卡（单词 + FSRS 记忆状态）
type dueCard struct {
	ID                 uint              `json:"id" gorm:"column:word_id"`
	Word               string            `json:"word" gorm:"column:word"`
	Phonetic           string            `json:"phonetic" gorm:"column:phonetic"`
	Meaning            string            `json:"meaning" gorm:"column:meaning"`
	Example            string            `json:"example" gorm:"column:example"`
	ExampleTranslation string            `json:"example_translation" gorm:"column:example_translation"`
	Senses             models.WordSenses `json:"senses" gorm:"column:senses"`
	Subject            string            `json:"subject" gorm:"column:subject"`
	Stability          float64           `json:"stability" gorm:"column:stability"`
	Difficulty         float64           `json:"difficulty" gorm:"column:difficulty"`
	DueAt              *time.Time        `json:"due_at" gorm:"column:due_at"`
	LastReviewAt       *time.Time        `json:"last_review_at" gorm:"column:last_review_at"`
	Reps               uint              `json:"reps" gorm:"column:reps"`
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

	now := time.Now()
	if nowMs, err := strconv.ParseInt(c.Query("now"), 10, 64); err == nil && nowMs > 0 {
		now = time.UnixMilli(nowMs)
	}

	var cards []dueCard
	// ⚠️ 必须按 user_id 过滤：词库共享，但每个人的到期时间各不相同
	err := h.db.Model(&models.Word{}).
		Select(`
			words.id AS word_id, words.word, words.phonetic, words.meaning, words.example,
			words.example_translation, words.senses, words.subject,
			word_reviews.stability, word_reviews.difficulty, word_reviews.due_at,
			word_reviews.last_review_at, word_reviews.reps
		`).
		Joins("JOIN word_reviews ON word_reviews.word_id = words.id").
		Where("word_reviews.user_id = ?", userID).
		Where("word_reviews.due_at IS NOT NULL AND word_reviews.due_at <= ?", now).
		Order("word_reviews.due_at ASC").
		Limit(limit).
		Scan(&cards).Error
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items": cards,
		"now":   now.UnixMilli(),
	}})
}

// NewWords 获取尚未加入复习的新单词（供前端随机器随机抽取今日新词）
// GET /api/reviews/new?limit=20
func (h *ReviewHandler) NewWords(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 200 {
		limit = 20
	}

	var words []models.Word
	// ⚠️ user_id 必须写在 **join 条件里**，不能写成 WHERE：
	// LEFT JOIN 之后，当前用户没学过的词那一侧全是 NULL；把 user_id 放进 WHERE 会把这些 NULL 行
	// 直接过滤掉，`word_reviews.id IS NULL` 就永远不成立——新词列表会直接变空。
	err := h.db.Model(&models.Word{}).
		Joins("LEFT JOIN word_reviews ON word_reviews.word_id = words.id AND word_reviews.user_id = ?", userID).
		Where("word_reviews.id IS NULL").
		Order("words.id ASC").
		Limit(limit).
		Find(&words).Error
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{"items": words}})
}

// learnedQuery 构造「当前用户已学词 + 记忆状态」的查询，供复习队列与抽查候选共用。
//
// 只取当前用户已经加入复习的词（有该用户的 word_reviews 行、due_at 非空），字段与到期卡一致
// （复用 dueCard 结构，客户端引擎的解析代码不用改）。
func (h *ReviewHandler) learnedQuery(userID uint, order string) *gorm.DB {
	return h.db.Model(&models.Word{}).
		Select(`
			words.id AS word_id, words.word, words.phonetic, words.meaning, words.example,
			words.example_translation, words.senses, words.subject,
			word_reviews.stability, word_reviews.difficulty, word_reviews.due_at,
			word_reviews.last_review_at, word_reviews.reps
		`).
		Joins("JOIN word_reviews ON word_reviews.word_id = words.id").
		Where("word_reviews.user_id = ?", userID).
		Where("word_reviews.due_at IS NOT NULL").
		Order(order)
}

// QueueReviews 复习队列：**所有已学词**按紧迫度（due_at 升序）排列，含尚未到期的，想多学就能一直往下翻。
// GET /api/reviews/queue?limit=100&offset=0
//
// 与 /api/reviews/due 的区别：due 只给「已经到期」的，queue 给整库并把「离到期还有多久」一起排好序。
// 到期与否只影响**顺序**、不影响能否出现（已过期的最旧优先；未到期的每 10 个一块、块内打乱，
// 这两件事由客户端引擎负责，见 modules/english/engine/src/session.rs 的 plan_day）。
//
// 分页：`total` 是已学词总数，客户端翻到底就说明整库过了一遍。
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

	now := time.Now()
	if nowMs, err := strconv.ParseInt(c.Query("now"), 10, 64); err == nil && nowMs > 0 {
		now = time.UnixMilli(nowMs)
	}

	var total int64
	// 总数必须与下面的分页结果同一口径（都只统计当前用户），否则前端按 total 翻页会算错
	if err := h.db.Model(&models.WordReview{}).
		Where("user_id = ? AND due_at IS NOT NULL", userID).
		Count(&total).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "统计失败:" + err.Error()})
		return
	}

	var cards []dueCard
	if err := h.learnedQuery(userID, "word_reviews.due_at ASC").Limit(limit).Offset(offset).Scan(&cards).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items":  cards,
		"total":  total,
		"limit":  limit,
		"offset": offset,
		"now":    now.UnixMilli(),
	}})
}

// ProbeCandidates 每日抽查候选：**到期时间最远**的已学词（due_at 倒序）。
// GET /api/reviews/probes?limit=20
//
// 用途：每天固定抽几个「最轮不到复习」的词提前确认记忆强度，避免「总是快过期的天天出现、
// 而间隔已经拉到几十天的词永远不出现」。
// 多给候选的原因：客户端还会按 localStorage 里的「最近抽查过的词」过滤，多取几条让它有得挑。
func (h *ReviewHandler) ProbeCandidates(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 200 {
		limit = 20
	}

	var cards []dueCard
	if err := h.learnedQuery(userID, "word_reviews.due_at DESC").Limit(limit).Scan(&cards).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{"items": cards}})
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
	dueSeconds := math.Max(req.IntervalDays*86400, 600)
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

	// 复习前的稳定度（用于日志）：⚠️ 必须取库里的真实旧值，不要改成引擎的输入状态，
	// 否则抽查卡（引擎按新卡重算）会被 stats 统计成「今日新学」
	stabilityBefore := review.Stability
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
			IntervalDays:    req.IntervalDays,
			ReviewedAt:      now,
			IsProbe:         req.IsProbe,
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
	}})
}

// ReviewStats 复习统计
// GET /api/reviews/stats
// 返回：词库总览（总词数/未学/到期/已学）+ 今日进度（今日已复习）+ 习惯指标（连续天数、记忆保持率）
func (h *ReviewHandler) ReviewStats(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	now := time.Now()
	// 今日零点（服务器本地时区），用于统计「今日已复习」
	startOfToday := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())

	// 词库共享：总词数看全库；「还没学的词」要扣掉**当前用户**已经学过的那部分
	var totalWords int64
	h.db.Model(&models.Word{}).Count(&totalWords)

	var newWords int64
	h.db.Model(&models.Word{}).
		Joins("LEFT JOIN word_reviews ON word_reviews.word_id = words.id AND word_reviews.user_id = ?", userID).
		Where("word_reviews.id IS NULL").
		Count(&newWords)

	// 以下每一项都是**个人**数据（到期数、已学词数、今日进度、连续天数、保持率），全部按 user_id 收口
	var dueCount int64
	h.db.Model(&models.WordReview{}).
		Where("user_id = ? AND due_at IS NOT NULL AND due_at <= ?", userID, now).
		Count(&dueCount)

	var reviewedCount int64
	h.db.Model(&models.WordReview{}).Where("user_id = ?", userID).Count(&reviewedCount)

	// ---- 复习日志统计（今日进度 / 连续天数 / 记忆保持率）----
	var totalReviews int64
	h.db.Model(&models.ReviewLog{}).Where("user_id = ?", userID).Count(&totalReviews)

	var todayReviewed int64
	h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ?", userID, startOfToday).
		Count(&todayReviewed)

	// 今日新学 / 今日复习：首次复习的日志里 stability_before 为 0（当时还是新卡），
	// 之后的复习都带上前一次的稳定度。据此把今日的复习拆成两类，供界面顶部展示。
	var todayNew int64
	h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ? AND stability_before = 0", userID, startOfToday).
		Count(&todayNew)

	var todayReview int64
	h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ? AND stability_before > 0", userID, startOfToday).
		Count(&todayReview)

	var againTotal int64
	h.db.Model(&models.ReviewLog{}).Where("user_id = ? AND rating = ?", userID, 1).Count(&againTotal)

	// 记忆保持率 = 非「忘记」评分所占比例（0~1，保留三位小数）
	retentionRate := 0.0
	if totalReviews > 0 {
		retentionRate = math.Round(float64(totalReviews-againTotal)/float64(totalReviews)*1000) / 1000
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"total_words":    totalWords,
		"new_words":      newWords,
		"due_cards":      dueCount,
		"reviewed_words": reviewedCount,
		"total_reviews":  totalReviews,
		"today_reviewed": todayReviewed,
		"today_new":      todayNew,
		"today_review":   todayReview,
		"streak_days":    h.calcStreakDays(userID, startOfToday),
		"retention_rate": retentionRate,
	}})
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
