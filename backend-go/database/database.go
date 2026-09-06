package database

import (
	"log"

	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/models"
)

// DB 全局数据库连接（由 Init 初始化）
var DB *gorm.DB

// Init 打开 SQLite 数据库并自动迁移表结构
// dsn 为数据库文件路径，例如 "guangxue.db"
func Init(dsn string) *gorm.DB {
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{
		Logger: logger.Default.LogMode(logger.Warn),
	})
	if err != nil {
		log.Fatalf("打开数据库失败 (%s): %v", dsn, err)
	}

	// 自动建表 / 迁移（仅创建缺失的表与列，不删除数据）
	if err := db.AutoMigrate(
		&models.User{},
		&models.DataItem{},
		&models.Word{},
		&models.WordReview{},
		&models.ReviewLog{},
	); err != nil {
		log.Fatalf("数据库迁移失败: %v", err)
	}

	DB = db
	return db
}
