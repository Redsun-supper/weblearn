// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package database

// P0-1 迁移的回归测试：这段逻辑只能在**老结构**上验证，所以这里手工造一份 P0-1 之前的表结构
// （就是 GORM 当年按 `uniqueIndex` 标签生成的样子），再跑真实的迁移函数。
//
// 为什么值得单独测：迁移一旦出错，表现是「第二个用户复习同一个词时运行时炸 UNIQUE 约束」——
// 很难在生产上立刻联想到是漏跑/跑错迁移，所以这里把每条断言都钉死。

import (
	"fmt"
	"io"
	"sync/atomic"
	"testing"

	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/models"
)

var migrateTestDBCounter atomic.Int64

// legacySchemaDDL 是 P0-1 之前的表结构：word_reviews 上是 UNIQUE(word_id)（一个词全局一行），
// review_logs 没有 user_id。
const legacySchemaDDL = `
CREATE TABLE word_reviews (
	id integer PRIMARY KEY AUTOINCREMENT,
	word_id integer,
	stability real,
	difficulty real,
	desired_retention real,
	due_at datetime,
	last_review_at datetime,
	reps integer,
	lapses integer,
	updated_at datetime
);
CREATE UNIQUE INDEX idx_word_reviews_word_id ON word_reviews(word_id);
CREATE INDEX idx_word_reviews_due_at ON word_reviews(due_at);
CREATE TABLE review_logs (
	id integer PRIMARY KEY AUTOINCREMENT,
	word_id integer,
	rating integer,
	stability_before real,
	stability_after real,
	difficulty_after real,
	interval_days real,
	reviewed_at datetime,
	is_probe numeric
);
CREATE INDEX idx_review_logs_word_id ON review_logs(word_id);
`

// openLegacyDB 起一个内存库并把它造成 P0-1 之前的样子。
func openLegacyDB(t *testing.T) *gorm.DB {
	t.Helper()
	name := fmt.Sprintf("gx_migrate_test_%d", migrateTestDBCounter.Add(1))
	db, err := Open("file:" + name + "?mode=memory&cache=shared")
	if err != nil {
		t.Fatalf("打开内存库失败: %v", err)
	}
	db.Logger = logger.Default.LogMode(logger.Silent)

	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("取出底层 sql.DB 失败: %v", err)
	}
	sqlDB.SetMaxOpenConns(1)
	sqlDB.SetMaxIdleConns(1)
	t.Cleanup(func() { _ = sqlDB.Close() })

	if err := db.Exec(legacySchemaDDL).Error; err != nil {
		t.Fatalf("造旧表结构失败: %v", err)
	}
	if err := db.Exec(`INSERT INTO word_reviews (word_id, stability, difficulty, desired_retention, reps, lapses)
		VALUES (1, 2.0, 5.0, 0.9, 1, 0)`).Error; err != nil {
		t.Fatalf("造历史进度失败: %v", err)
	}
	if err := db.Exec(`INSERT INTO review_logs (word_id, rating, stability_before, stability_after, difficulty_after, interval_days, reviewed_at, is_probe)
		VALUES (1, 3, 0, 2.0, 5.0, 2.0, datetime('now'), 0)`).Error; err != nil {
		t.Fatalf("造历史日志失败: %v", err)
	}
	return db
}

func TestMigrateDropsLegacyUniqueIndexAndOrphans(t *testing.T) {
	db := openLegacyDB(t)

	// 迁移前：旧结构在，且还没有 user_id 列
	legacy, err := LegacyReviewIndexPresent(db)
	if err != nil {
		t.Fatalf("检测旧索引失败: %v", err)
	}
	if !legacy {
		t.Fatal("刚造好的旧结构应当报「还是 P0-1 之前」")
	}
	hasUserID, err := HasColumn(db, "word_reviews", "user_id")
	if err != nil {
		t.Fatalf("检测列失败: %v", err)
	}
	if hasUserID {
		t.Fatal("旧结构不该有 user_id 列（这正是本次迁移要补的）")
	}

	// dry-run 的估算：还没有 user_id 列时，现存每一行都将变成无归属行
	reviews, logs, err := LegacyOrphanEstimate(db)
	if err != nil {
		t.Fatalf("估算影响面失败: %v", err)
	}
	if reviews != 1 || logs != 1 {
		t.Fatalf("预估待清理行数应为 1/1，实际 %d/%d", reviews, logs)
	}

	// 启动自检必须先拦住（拒绝启动），迁移前不能放行
	if err := EnsureReviewUserIsolation(db); err == nil {
		t.Fatal("迁移前启动自检应当报错")
	}

	// 真正的迁移：AutoMigrate 补列与新索引，再清孤儿行、删旧索引
	if err := Migrate(db); err != nil {
		t.Fatalf("AutoMigrate 失败: %v", err)
	}
	if err := ApplyReviewUserIsolation(db, io.Discard); err != nil {
		t.Fatalf("迁移失败: %v", err)
	}

	// 1) 旧索引没了、新索引在
	legacy, err = LegacyReviewIndexPresent(db)
	if err != nil {
		t.Fatalf("复检旧索引失败: %v", err)
	}
	if legacy {
		t.Fatalf("迁移后旧索引 %s 仍存在", LegacyReviewUniqueIndex)
	}
	for _, name := range []string{ReviewUserWordIndex, ReviewLogUserTimeIndex} {
		present, err := IndexPresent(db, name)
		if err != nil {
			t.Fatalf("查询索引 %s 失败: %v", name, err)
		}
		if !present {
			t.Fatalf("迁移后缺少索引 %s", name)
		}
	}

	// 2) 无归属的历史行被清掉
	reviews, logs, err = OrphanCounts(db)
	if err != nil {
		t.Fatalf("统计孤儿行失败: %v", err)
	}
	if reviews != 0 || logs != 0 {
		t.Fatalf("迁移后不该还有 user_id = 0 的行，实际 %d/%d", reviews, logs)
	}

	// 3) 迁移后启动自检必须放行
	if err := EnsureReviewUserIsolation(db); err != nil {
		t.Fatalf("迁移后启动自检仍报错: %v", err)
	}

	// 4) 复合唯一键真的生效：两个用户可以各写一行，同一个用户再写第二行应当被挡住
	mk := func(userID uint) *models.WordReview {
		return &models.WordReview{UserID: userID, WordID: 1, Stability: 1, Difficulty: 5}
	}
	if err := db.Create(mk(1)).Error; err != nil {
		t.Fatalf("用户 1 写进度失败: %v", err)
	}
	if err := db.Create(mk(2)).Error; err != nil {
		t.Fatalf("用户 2 写同一个词的进度失败（唯一键没改成复合键）: %v", err)
	}
	if err := db.Create(mk(1)).Error; err == nil {
		t.Fatal("同一个用户对同一个词写第二行应当被唯一约束挡住")
	}

	// 5) 幂等：再跑一次不报错、也不改变结果
	if err := ApplyReviewUserIsolation(db, io.Discard); err != nil {
		t.Fatalf("重复执行迁移应当无害，实际: %v", err)
	}
	var n int64
	if err := db.Model(&models.WordReview{}).Count(&n).Error; err != nil {
		t.Fatalf("统计进度行失败: %v", err)
	}
	if n != 2 {
		t.Fatalf("重复执行迁移后进度行数应仍为 2，实际 %d", n)
	}
}
