package database

// P0-1「进度按人隔离」的迁移与自检。
//
// 背景：P0-1 之前 word_reviews 上有一条 UNIQUE(word_id) 索引（GORM 按 `uniqueIndex` 标签
// 自动建的 idx_word_reviews_word_id），含义是**一个单词全局只有一行进度**。第二个用户一复习
// 同一个词，就会覆盖第一个人的进度；改成 (user_id, word_id) 唯一之后，两个人各有一行。
//
// ⚠️ 坑在这里：**GORM 的 AutoMigrate 只补新列/新索引，不会删旧索引**。旧的唯一索引若留着，
// 第二个用户第一次复习同一个词就会撞 `UNIQUE constraint failed`——而且是运行时才炸。
// 所以旧索引必须显式删掉，由 `cmd/migrate` 执行（这是用户选定的「显式迁移」方案：
// 刻意不做启动自愈，只做启动自检，见 docs/launch-plan.md 的 P0-1 决策表）。

import (
	"fmt"
	"io"

	"gorm.io/gorm"

	"backend-go/models"
)

const (
	// LegacyReviewUniqueIndex 是 P0-1 之前 word_reviews 上的 UNIQUE(word_id) 索引（GORM 默认命名）
	LegacyReviewUniqueIndex = "idx_word_reviews_word_id"
	// LegacyReviewDueIndex 是被 (user_id, due_at) 复合索引取代的单列索引；
	// 它不会造成错误，但每次写入都要维护，且已没有任何查询会用到它。
	LegacyReviewDueIndex = "idx_word_reviews_due_at"
	// ReviewUserWordIndex / ReviewLogUserTimeIndex 是迁移后应当存在的两个新索引
	ReviewUserWordIndex    = "idx_word_reviews_user_word"
	ReviewLogUserTimeIndex = "idx_review_logs_user_time"
)

// IndexPresent 只读检查某个索引是否存在。
func IndexPresent(db *gorm.DB, name string) (bool, error) {
	var n int64
	if err := db.Raw(
		`SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = ?`, name,
	).Scan(&n).Error; err != nil {
		return false, err
	}
	return n > 0, nil
}

// LegacyReviewIndexPresent 判断这个库是不是还是 P0-1 之前的结构。
func LegacyReviewIndexPresent(db *gorm.DB) (bool, error) {
	return IndexPresent(db, LegacyReviewUniqueIndex)
}

// HasColumn 只读检查某张表是否已有某一列（`cmd/migrate` 的 dry-run 要在
// AutoMigrate **之前**报告影响面，那时 user_id 列在旧库上还不存在）。
func HasColumn(db *gorm.DB, table, column string) (bool, error) {
	var rows []struct {
		Name string
	}
	// PRAGMA 不支持参数绑定，这里的表名是代码里的常量，不是外部输入
	if err := db.Raw("PRAGMA table_info('" + table + "')").Scan(&rows).Error; err != nil {
		return false, err
	}
	for _, r := range rows {
		if r.Name == column {
			return true, nil
		}
	}
	return false, nil
}

// OrphanCounts 数一下还没有归属（user_id = 0）的历史进度行。
// ⚠️ 要求 user_id 列已经存在（旧库上要先 AutoMigrate 补列）。
func OrphanCounts(db *gorm.DB) (reviews int64, logs int64, err error) {
	if err = db.Model(&models.WordReview{}).Where("user_id = 0").Count(&reviews).Error; err != nil {
		return 0, 0, err
	}
	if err = db.Model(&models.ReviewLog{}).Where("user_id = 0").Count(&logs).Error; err != nil {
		return 0, 0, err
	}
	return reviews, logs, nil
}

// LegacyOrphanEstimate 估计「这次迁移会清掉多少行」，供 dry-run 报告用。
// 老库还没有 user_id 列时，现存的每一行都将变成 user_id = 0 的行，所以直接数总行数。
func LegacyOrphanEstimate(db *gorm.DB) (reviews int64, logs int64, err error) {
	hasReviewUser, err := HasColumn(db, "word_reviews", "user_id")
	if err != nil {
		return 0, 0, err
	}
	hasLogUser, err := HasColumn(db, "review_logs", "user_id")
	if err != nil {
		return 0, 0, err
	}
	if hasReviewUser && hasLogUser {
		return OrphanCounts(db)
	}
	if err := db.Model(&models.WordReview{}).Count(&reviews).Error; err != nil {
		return 0, 0, err
	}
	if err := db.Model(&models.ReviewLog{}).Count(&logs).Error; err != nil {
		return 0, 0, err
	}
	return reviews, logs, nil
}

// EnsureReviewUserIsolation 是**启动自检**（只读，不改结构）：
// 旧结构还在就返回错误，由调用方拒绝启动并提示该跑哪条命令。
func EnsureReviewUserIsolation(db *gorm.DB) error {
	legacy, err := LegacyReviewIndexPresent(db)
	if err != nil {
		return fmt.Errorf("检查复习进度表结构失败: %w", err)
	}
	if !legacy {
		return nil
	}
	return fmt.Errorf(`这个库还是 P0-1 之前的旧结构（索引 %s 仍在）：直接启动会让第二个用户的复习进度互相覆盖。
请先执行迁移（迁移完成前本服务拒绝启动）：
    go run ./cmd/migrate          # 先看它打算做什么，不改动任何数据
    go run ./cmd/migrate -apply   # 确认后执行，会先自动快照到 backups/<时间戳>/`, LegacyReviewUniqueIndex)
}

// ApplyReviewUserIsolation 执行迁移（幂等，重复执行只会报告「没有可清理的行」）：
//
//  1. 删掉 user_id = 0 的历史行（P0-1 决策：旧数据清理，不归属给任何人）；
//  2. 删掉旧的 UNIQUE(word_id) 索引与已被复合索引取代的单列 due_at 索引。
//
// 调用前必须先 AutoMigrate（user_id 列与新索引由它补齐），见 database.Migrate。
func ApplyReviewUserIsolation(db *gorm.DB, out io.Writer) error {
	// DROP INDEX 不支持参数绑定；这两个名字是本文件的常量，不是外部输入。
	dropLegacy := "DROP INDEX IF EXISTS " + LegacyReviewUniqueIndex
	dropDue := "DROP INDEX IF EXISTS " + LegacyReviewDueIndex

	return db.Transaction(func(tx *gorm.DB) error {
		reviews, logs, err := OrphanCounts(tx)
		if err != nil {
			return err
		}
		if reviews > 0 {
			if err := tx.Where("user_id = 0").Delete(&models.WordReview{}).Error; err != nil {
				return err
			}
		}
		if logs > 0 {
			if err := tx.Where("user_id = 0").Delete(&models.ReviewLog{}).Error; err != nil {
				return err
			}
		}
		if err := tx.Exec(dropLegacy).Error; err != nil {
			return err
		}
		if err := tx.Exec(dropDue).Error; err != nil {
			return err
		}

		fmt.Fprintf(out, "  清理无归属的历史进度：word_reviews %d 行、review_logs %d 行\n", reviews, logs)
		fmt.Fprintf(out, "  删除旧索引：%s、%s\n", LegacyReviewUniqueIndex, LegacyReviewDueIndex)
		return nil
	})
}
