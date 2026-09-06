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

// ---------- 请求/响应结构 ----------

type addWordItem struct {
	Word     string `json:"word"`
	Phonetic string `json:"phonetic"`
	Meaning  string `json:"meaning"`
	Example  string `json:"example"`
	Subject  string `json:"subject"`
}

type addWordsRequest struct {
	Words []addWordItem `json:"words"`
}

type submitReviewRequest struct {
	WordID        uint    `json:"word_id"`
	Rating        uint8   `json:"rating"`         // 1=Again 2=Hard 3=Good 4=Easy
	Stability     float64 `json:"stability"`      // 引擎算出的新记忆状态
	Difficulty    float64 `json:"difficulty"`     // 引擎算出的新记忆状态
	IntervalDays  float64 `json:"interval_days"`  // 引擎算出的下次间隔（天）
	DesiredRetain float64 `json:"desired_retention"`
}

// 到期复习卡（单词 + FSRS 记忆状态）
type dueCard struct {
	ID           uint       `json:"id" gorm:"column:word_id"`
	Word         string     `json:"word" gorm:"column:word"`
	Phonetic     string     `json:"phonetic" gorm:"column:phonetic"`
	Meaning      string     `json:"meaning" gorm:"column:meaning"`
	Example      string     `json:"example" gorm:"column:example"`
	Subject      string     `json:"subject" gorm:"column:subject"`
	Stability    float64    `json:"stability" gorm:"column:stability"`
	Difficulty   float64    `json:"difficulty" gorm:"column:difficulty"`
	DueAt        *time.Time `json:"due_at" gorm:"column:due_at"`
	LastReviewAt *time.Time `json:"last_review_at" gorm:"column:last_review_at"`
	Reps         uint       `json:"reps" gorm:"column:reps"`
}

// ---------- 接口实现 ----------

// ListWords 获取单词列表
// GET /api/words?limit=20&offset=0&subject=english
func (h *ReviewHandler) ListWords(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 500 {
		limit = 20
	}
	offset, _ := strconv.Atoi(c.DefaultQuery("offset", "0"))
	if offset < 0 {
		offset = 0
	}
	subject := c.Query("subject")

	var words []models.Word
	q := h.db.Model(&models.Word{}).Order("id ASC").Limit(limit).Offset(offset)
	if subject != "" {
		q = q.Where("subject = ?", subject)
	}
	if err := q.Find(&words).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items":  words,
		"limit":  limit,
		"offset": offset,
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

	created := 0
	skipped := 0
	var err error
	for _, item := range req.Words {
		word := strings.TrimSpace(item.Word)
		if word == "" {
			skipped++
			continue
		}
		record := models.Word{
			Word:     word,
			Phonetic: item.Phonetic,
			Meaning:  item.Meaning,
			Example:  item.Example,
			Subject:  item.Subject,
		}
		if record.Subject == "" {
			record.Subject = "english"
		}
		// 单词重复则跳过（不报错）
		var existing models.Word
		findErr := h.db.Where("word = ?", record.Word).First(&existing).Error
		if findErr == nil {
			skipped++
			continue
		}
		if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			err = findErr
			break
		}
		if createErr := h.db.Create(&record).Error; createErr != nil {
			err = createErr
			break
		}
		created++
	}

	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "写入失败:" + err.Error()})
		return
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
	now := time.Now()
	if nowMs, err := strconv.ParseInt(c.Query("now"), 10, 64); err == nil && nowMs > 0 {
		now = time.UnixMilli(nowMs)
	}

	var cards []dueCard
	err := h.db.Model(&models.Word{}).
		Select(`
			words.id AS word_id, words.word, words.phonetic, words.meaning, words.example, words.subject,
			word_reviews.stability, word_reviews.difficulty, word_reviews.due_at,
			word_reviews.last_review_at, word_reviews.reps
		`).
		Joins("JOIN word_reviews ON word_reviews.word_id = words.id").
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
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 || limit > 200 {
		limit = 20
	}

	var words []models.Word
	err := h.db.Model(&models.Word{}).
		Joins("LEFT JOIN word_reviews ON word_reviews.word_id = words.id").
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

// SubmitReview 提交一次复习结果（引擎已算好新状态，这里只做持久化）
// POST /api/reviews/submit
// 请求体：{ word_id, rating(1-4), stability, difficulty, interval_days, desired_retention? }
// 逻辑：更新/创建 word_reviews（due_at = now + max(interval_days*86400, 600) 秒），并记一条 review_logs
func (h *ReviewHandler) SubmitReview(c *gin.Context) {
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
	// 间隔最短 10 分钟，防止 Again 后马上再次到期
	dueSeconds := math.Max(req.IntervalDays*86400, 600)
	dueAt := now.Add(time.Duration(dueSeconds * float64(time.Second)))

	desiredRetention := req.DesiredRetain
	if desiredRetention <= 0 || desiredRetention >= 1 {
		desiredRetention = 0.9
	}

	var review models.WordReview
	err := h.db.Where("word_id = ?", req.WordID).First(&review).Error
	isNew := errors.Is(err, gorm.ErrRecordNotFound)
	if err != nil && !isNew {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	// 复习前的稳定度（用于日志）
	stabilityBefore := review.Stability
	if isNew {
		review = models.WordReview{WordID: req.WordID}
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
			WordID:          req.WordID,
			Rating:          req.Rating,
			StabilityBefore: stabilityBefore,
			StabilityAfter:  req.Stability,
			DifficultyAfter: req.Difficulty,
			IntervalDays:    req.IntervalDays,
			ReviewedAt:      now,
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
func (h *ReviewHandler) ReviewStats(c *gin.Context) {
	now := time.Now()

	var totalWords int64
	h.db.Model(&models.Word{}).Count(&totalWords)

	var newWords int64
	h.db.Model(&models.Word{}).
		Joins("LEFT JOIN word_reviews ON word_reviews.word_id = words.id").
		Where("word_reviews.id IS NULL").
		Count(&newWords)

	var dueCount int64
	h.db.Model(&models.WordReview{}).
		Where("due_at IS NOT NULL AND due_at <= ?", now).
		Count(&dueCount)

	var reviewedCount int64
	h.db.Model(&models.WordReview{}).Count(&reviewedCount)

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"total_words":    totalWords,
		"new_words":      newWords,
		"due_cards":      dueCount,
		"reviewed_words": reviewedCount,
	}})
}
