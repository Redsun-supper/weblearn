package models

import "time"

// User 用户数据模型
// 用途：定义用户信息的结构，用于数据库操作和API响应
type User struct {
	ID       uint   `json:"id" gorm:"primaryKey"`
	Username string `json:"username" gorm:"uniqueIndex;size:50"`
	Email    string `json:"email" gorm:"uniqueIndex;size:100"`
	Password string `json:"-" gorm:"size:255"` // json:"-" 表示不序列化到JSON响应中
}

// TableName 指定数据库表名
func (User) TableName() string {
	return "users"
}

// DataItem 数据项模型
type DataItem struct {
	ID        uint   `json:"id" gorm:"primaryKey"`
	Title     string `json:"title" gorm:"size:200"`
	Content   string `json:"content" gorm:"type:text"`
	CreatedAt string `json:"created_at"`
	UpdatedAt string `json:"updated_at"`
}

// TableName 指定数据库表名
func (DataItem) TableName() string {
	return "data_items"
}

// Word 单词词条（词汇库）
// 说明：纯词条数据，与复习记忆状态分离存储
type Word struct {
	ID        uint      `json:"id" gorm:"primaryKey"`
	Word      string    `json:"word" gorm:"uniqueIndex;size:100"`
	Phonetic  string    `json:"phonetic" gorm:"size:200"`
	Meaning   string    `json:"meaning" gorm:"type:text"`
	Example   string    `json:"example" gorm:"type:text"`
	Subject   string    `json:"subject" gorm:"size:30;default:english;index"`
	Book      string    `json:"book" gorm:"size:60;index"` // 词书/册（如「必修一」），后台分组用，可空
	Unit      string    `json:"unit" gorm:"size:60;index"` // 单元（如「Unit 1」），后台分组用，可空
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// TableName 指定数据库表名
func (Word) TableName() string {
	return "words"
}

// WordReview 单词的 FSRS 记忆状态（与前端 Rust 引擎 CardState 字段一一对应）
// 说明：由前端 Rust 引擎计算出的记忆状态持久化到这里；due_at <= 当前时间即可复习
type WordReview struct {
	ID               uint       `json:"id" gorm:"primaryKey"`
	WordID           uint       `json:"word_id" gorm:"uniqueIndex"`
	Stability        float64    `json:"stability"`        // 记忆稳定度（天）
	Difficulty       float64    `json:"difficulty"`       // 记忆难度 1~10
	DesiredRetention float64    `json:"desired_retention" gorm:"default:0.9"` // 期望记忆保持率
	DueAt            *time.Time `json:"due_at" gorm:"index"`                  // 下次到期时间
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
	ID              uint      `json:"id" gorm:"primaryKey"`
	WordID          uint      `json:"word_id" gorm:"index"`
	Rating          uint8     `json:"rating"`           // 1=Again 2=Hard 3=Good 4=Easy
	StabilityBefore float64   `json:"stability_before"` // 复习前的稳定度
	StabilityAfter  float64   `json:"stability_after"`  // 复习后的稳定度
	DifficultyAfter float64   `json:"difficulty_after"`
	IntervalDays    float64   `json:"interval_days"` // 引擎算出的下次间隔（天）
	ReviewedAt      time.Time `json:"reviewed_at" gorm:"index"`
}

// TableName 指定数据库表名
func (ReviewLog) TableName() string {
	return "review_logs"
}
