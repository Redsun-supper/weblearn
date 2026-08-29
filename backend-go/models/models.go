package models

// User 用户数据模型
// 用途：定义用户信息的结构，用于数据库操作和API响应
type User struct {
	ID       uint   `json:"id" gorm:"primaryKey"`
	Username string `json:"username" gorm:"uniqueIndex;size:50"`
	Email    string `json:"email" gorm:"uniqueIndex;size:100"`
	Password string `json:"-" gorm:"size:255"` // json:"-" 表示不序列化到JSON响应中
}

// TableName 指定数据库表名
// 用途：GORM框架使用此方法确定模型对应的数据库表名
func (User) TableName() string {
	return "users"
}

// DataItem 数据项模型
// 用途：定义通用数据项的结构
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