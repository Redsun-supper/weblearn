package models

import (
	"database/sql/driver"
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

// 这里原先还有 User / DataItem 两个模型（表 users / data_items）：
// 它们是早期占位代码留下的空表，与账号服务 auth.db 里的 users 表同名不同源，
// 留着必被误用，已随 /api/user/*、/api/data/* 占位接口一起删除。
// 用户与本机会话数据只存在于账号服务，本服务不再自己存一份。

// WordSense 一条释义：一个词性一块（名词一块、动词一块），可以带自己的例句与中文翻译
type WordSense struct {
	Pos         string `json:"pos"`         // 词性标签，如 "n."、"v."、"adj."
	Meaning     string `json:"meaning"`     // 该词性下的释义正文
	Example     string `json:"example"`     // 该词性专属例句，可空（空了就沿用词条本身的例句）
	Translation string `json:"translation"` // 该例句的中文翻译，可空
}

// WordSenses 多条释义。
//
// 存储方式：在 words 表里存成**一列 JSON 文本**（words.senses），不单独开子表。
// 取舍依据：释义永远跟着词条一起读写、从不单独查询。存 JSON 省掉一次 join，
// 传输上也更小——子表方案每条释义还得额外带 id / word_id。
// 代价是不能用 SQL 直接查某条释义（本项目没有这种需求）。
type WordSenses []WordSense

// Value 写入数据库：空列表统一存 "[]"，
// 避免 NULL 与空串两种「没有释义」的状态混着来
func (s WordSenses) Value() (driver.Value, error) {
	if len(s) == 0 {
		return "[]", nil
	}
	b, err := json.Marshal([]WordSense(s))
	if err != nil {
		return nil, err
	}
	return string(b), nil
}

// Scan 从数据库读取。
//
// ⚠️ 无论读到什么都先把接收者重置为**非 nil 的空切片**：只有这样序列化成 JSON 时才是
// `[]` 而不是 `null`，而前端 Rust 引擎按数组解析，遇到 null 会直接报错（"items": null 已踩过一次）。
func (s *WordSenses) Scan(src interface{}) error {
	*s = WordSenses{}
	if src == nil {
		return nil
	}
	var raw string
	switch v := src.(type) {
	case string:
		raw = v
	case []byte:
		raw = string(v)
	default:
		return fmt.Errorf("senses 列类型不支持: %T", src)
	}
	raw = strings.TrimSpace(raw)
	if raw == "" || raw == "null" {
		return nil
	}
	var list []WordSense
	if err := json.Unmarshal([]byte(raw), &list); err != nil {
		// 读时宽容：手工把某一行的 JSON 改坏，不应该让整个复习页打不开。
		// 这一行按「没有多释义」处理，界面退回单词条展示；写入口是严格校验的。
		return nil
	}
	if list != nil {
		*s = list
	}
	return nil
}

// MarshalJSON 保证 nil 也输出成 `[]`，前端拿到的永远是数组
func (s WordSenses) MarshalJSON() ([]byte, error) {
	if s == nil {
		return []byte("[]"), nil
	}
	return json.Marshal([]WordSense(s))
}

// Word 单词词条（纯词条数据；记忆状态另存 WordReview，两者分开）
type Word struct {
	ID       uint   `json:"id" gorm:"primaryKey"`
	Word     string `json:"word" gorm:"uniqueIndex;size:100"`
	Phonetic string `json:"phonetic" gorm:"size:200"`
	Meaning  string `json:"meaning" gorm:"type:text"`
	Example  string `json:"example" gorm:"type:text"`
	// ExampleTranslation 例句的中文翻译（可空）
	ExampleTranslation string `json:"example_translation" gorm:"type:text"`
	// Senses 多释义（JSON 文本，可空）。为空时前端按 Meaning 里的词性标签自动拆分展示
	Senses    WordSenses `json:"senses" gorm:"type:text"`
	Subject   string     `json:"subject" gorm:"size:30;default:english;index"`
	Book      string     `json:"book" gorm:"size:60;index"` // 词书/册（如「必修一」），后台分组用，可空
	Unit      string     `json:"unit" gorm:"size:60;index"` // 单元（如「Unit 1」），后台分组用，可空
	CreatedAt time.Time  `json:"created_at"`
	UpdatedAt time.Time  `json:"updated_at"`
}

// TableName 指定数据库表名
func (Word) TableName() string {
	return "words"
}

// WordReview 单个用户对某个单词的 FSRS 记忆状态（与前端 Rust 引擎 CardState 字段一一对应）
// 说明：由前端 Rust 引擎计算出的记忆状态持久化到这里；due_at <= 当前时间即可复习
//
// ⚠️ 唯一键是 **(user_id, word_id)**，不是 word_id 单独唯一：**词库共享、进度私有**，
// 同一个单词每个人各有一行进度。P0-1 之前这里是 `gorm:"uniqueIndex"`（一个词全局一行），
// 第二个用户一复习就会覆盖第一个人的进度——旧索引 idx_word_reviews_word_id 必须由
// cmd/migrate 删掉（GORM 的 AutoMigrate 只补新索引，不会删旧索引）。
type WordReview struct {
	ID uint `json:"id" gorm:"primaryKey"`
	// UserID 这条进度属于谁（账号服务的 users.id，来自 gx_access 令牌的 sub）
	UserID           uint       `json:"user_id" gorm:"not null;default:0;uniqueIndex:idx_word_reviews_user_word;index:idx_word_reviews_user_due,priority:1"`
	WordID           uint       `json:"word_id" gorm:"not null;uniqueIndex:idx_word_reviews_user_word;index:idx_word_reviews_word"`
	Stability        float64    `json:"stability"`                                                // 记忆稳定度（天）
	Difficulty       float64    `json:"difficulty"`                                               // 记忆难度 1~10
	DesiredRetention float64    `json:"desired_retention" gorm:"default:0.9"`                     // 期望记忆保持率
	DueAt            *time.Time `json:"due_at" gorm:"index:idx_word_reviews_user_due,priority:2"` // 下次到期时间
	LastReviewAt     *time.Time `json:"last_review_at"`
	Reps             uint       `json:"reps" gorm:"default:0"`   // 复习次数
	Lapses           uint       `json:"lapses" gorm:"default:0"` // 遗忘（Again）次数
	UpdatedAt        time.Time  `json:"updated_at"`
}

// TableName 指定数据库表名
func (WordReview) TableName() string {
	return "word_reviews"
}

// ReviewLog 复习记录日志
// 说明：每次提交复习写一条，便于后续做 FSRS 参数优化（compute_parameters）
type ReviewLog struct {
	ID uint `json:"id" gorm:"primaryKey"`
	// UserID 谁复习的；日志的「今日新学 / 连续天数 / 记忆保持率」都按它统计
	UserID          uint      `json:"user_id" gorm:"not null;default:0;index:idx_review_logs_user_time,priority:1"`
	WordID          uint      `json:"word_id" gorm:"index"` // 删词条时的级联统计仍按 word_id 查
	Rating          uint8     `json:"rating"`               // 1=Again 2=Hard 3=Good 4=Easy
	StabilityBefore float64   `json:"stability_before"`     // 复习前的稳定度
	StabilityAfter  float64   `json:"stability_after"`      // 复习后的稳定度
	DifficultyAfter float64   `json:"difficulty_after"`
	IntervalDays    float64   `json:"interval_days"` // 引擎算出的下次间隔（天）
	ReviewedAt      time.Time `json:"reviewed_at" gorm:"index:idx_review_logs_user_time,priority:2"`
	// IsProbe 是否为「每日抽查」：抽查卡按新卡重算记忆状态，
	// 其 stability_after 会明显低于 before（相当于把间隔压缩了）。
	// 日后做 FSRS 参数优化（compute_parameters）时应当排除这批记录，否则参数会被带偏。
	IsProbe bool `json:"is_probe"`
	// IsReset 是否为「重置重学」：单一循环池里「今日 5 个」可能命中已经学过的词，
	// 引擎按新卡口径重算（docs/review-pool-plan.md 的 C11）。
	// 这类记录的 stability_before 会被强制记成 0，使「今日新学」只统计真正的第一次学；
	// 做 FSRS 参数优化时同样必须排除。
	IsReset bool `json:"is_reset"`
}

// TableName 指定数据库表名
func (ReviewLog) TableName() string {
	return "review_logs"
}
