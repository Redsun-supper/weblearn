// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package database

import (
	"fmt"
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

// Open 打开 SQLite 连接。不做结构检查、不做迁移——`cmd/migrate` 需要自己控制
// 「先看结构、再决定要不要动」的顺序，所以这里把它和 Init 拆开。
func Open(dsn string) (*gorm.DB, error) {
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
		return nil, fmt.Errorf("打开数据库失败 (%s): %w", dsn, err)
	}
	return db, nil
}

// Migrate 自动迁移本服务真正使用的表（只创建缺失的表/列/索引，不删除已有数据）。
//
// ⚠️ 它**不会**删掉 P0-1 之前那条 UNIQUE(word_id) 索引——GORM 只补新索引、不删旧索引。
// 那条索引由 `cmd/migrate` 显式删除，见 database/user_isolation.go。
func Migrate(db *gorm.DB) error {
	// 只迁移本服务真正使用的表。原先还有 &models.User{} / &models.DataItem{}
	// 两张与账号服务同名不同源的空表，已随占位接口一并删除（见 routes/routes.go 的说明）。
	return db.AutoMigrate(
		&models.Word{},
		&models.WordReview{},
		&models.ReviewLog{},
	)
}

// Init 打开数据库、完成启动自检与自动迁移，并设置全局 DB（业务代码走这个入口）。
// dsn 为数据库文件路径，例如 "guangxue.db"
func Init(dsn string) *gorm.DB {
	db, err := Open(dsn)
	if err != nil {
		log.Fatal(err)
	}

	// ⚠️ 自检必须在 AutoMigrate **之前**：旧库要在被动过之前就拦下来（拒绝启动），
	// 否则会出现「服务跑起来了，但第二个用户一复习就撞 UNIQUE 约束」这种运行时才暴露的问题。
	if err := EnsureReviewUserIsolation(db); err != nil {
		log.Fatalf("❌ %v", err)
	}

	if err := Migrate(db); err != nil {
		log.Fatalf("数据库迁移失败: %v", err)
	}

	DB = db
	return db
}
