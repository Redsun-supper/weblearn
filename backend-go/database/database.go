package database

import (
	"log"
	"os"
	"time"

	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/models"
)

// DB 全局数据库连接（由 Init 初始化）
var DB *gorm.DB

// Init 打开 SQLite 并自动迁移表结构（只创建缺失的表/列，不删除已有数据）。
// dsn 为数据库文件路径，例如 "guangxue.db"
func Init(dsn string) *gorm.DB {
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{
		Logger: logger.New(
			log.New(os.Stdout, "\r\n", log.LstdFlags),
			logger.Config{
				SlowThreshold: 200 * time.Millisecond,
				LogLevel:      logger.Warn,
				// ErrRecordNotFound 在本项目中是正常控制流（判断「新词 / 未加入复习的卡」，
				// 以及导入词表时判断单词是否已存在），不作为错误打印，避免刷日志
				IgnoreRecordNotFoundError: true,
				Colorful:                  true,
			},
		),
	})
	if err != nil {
		log.Fatalf("打开数据库失败 (%s): %v", dsn, err)
	}

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
