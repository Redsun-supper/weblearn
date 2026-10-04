// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package main

// 复习进度清空工具（单一循环池重构的一次性配套命令，见 docs/review-pool-plan.md 的 E20）。
//
// 背景：P0-1 之前「每天 5 个新词」的配额记在浏览器 localStorage 里，导致 100 个词的词库里
// 只有 5 个词进过复习队列（数据库实证：words=100 而 word_reviews 只有 5 行）。
// 新模型（单一循环池）不再区分「已学 / 未学」——上线的第一步就是把旧的进度账**清空重来**。
//
// ⚠️ 这是**不可逆**操作。因此本命令有两道闸：
//  1. 不带 `-yes` 时只做「演练」：报告将要删多少行，一行都不动；
//  2. 带 `-yes` 时也会**先自动跑一次 VACUUM INTO 快照**，快照失败就中止，不删。
//
// 只清空两张复习表，**绝不动 words 词表**（词表是资产，3500 词的导入成果都在里面）。
//
// 用法（在 backend-go/ 目录下）：
//
//	go run ./cmd/discard-progress              # 演练：只报告
//	go run ./cmd/discard-progress -yes         # 先快照，再清空
//	go run ./cmd/discard-progress -yes -db ../x.db -out ../backups
//
// 退出码：0 = 成功；1 = 失败（含快照失败、结构自检不通过）。

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"gorm.io/gorm"

	"backend-go/database"
	"backend-go/models"
	"backend-go/snapshot"
)

func main() {
	var (
		dsn     = flag.String("db", "guangxue.db", "要清空的库文件路径")
		outRoot = flag.String("out", filepath.Join("..", "backups"), "快照输出根目录（清空前必留一份）")
		keep    = flag.Int("keep", 0, "只保留最近 N 份快照（0 = 不清理）")
		yes     = flag.Bool("yes", false, "确认执行清空（不加则只做演练，不修改任何数据）")
	)
	flag.Parse()

	db, err := database.Open(*dsn)
	if err != nil {
		fmt.Fprintf(os.Stderr, "❌ %v\n", err)
		os.Exit(1)
	}

	// ⚠️ 自检必须在动数据之前：旧结构的库（P0-1 之前的 UNIQUE(word_id)）说明
	// 这个库根本没跑过按人隔离的版本，清空它没有意义、还可能删掉别人正在用的数据。
	if err := database.EnsureReviewUserIsolation(db); err != nil {
		fmt.Fprintf(os.Stderr, "❌ 拒绝操作：%v\n", err)
		os.Exit(1)
	}

	var reviews, logs, words int64
	db.Model(&models.WordReview{}).Count(&reviews)
	db.Model(&models.ReviewLog{}).Count(&logs)
	db.Model(&models.Word{}).Count(&words)

	fmt.Printf("库：%s\n", *dsn)
	fmt.Printf("  words        保留    %d 行（词表，一行都不会动）\n", words)
	fmt.Printf("  word_reviews 待清空  %d 行\n", reviews)
	fmt.Printf("  review_logs  待清空  %d 行\n", logs)

	if reviews == 0 && logs == 0 {
		fmt.Println("\n✓ 两张复习表已经是空的，无需清理。")
		return
	}

	if !*yes {
		fmt.Println("\n（演练模式：未做任何修改。确认无误后加 -yes 执行，执行前会自动留一份快照。）")
		return
	}

	// ---- 第一道闸之后的保险：先快照 ----
	absRoot, err := snapshot.SafeRoot(*outRoot)
	if err != nil {
		fmt.Fprintf(os.Stderr, "❌ %v\n", err)
		os.Exit(1)
	}
	target, err := snapshot.NewDir(absRoot)
	if err != nil {
		fmt.Fprintf(os.Stderr, "❌ 创建快照目录失败：%v\n", err)
		os.Exit(1)
	}
	if err := snapshot.Snapshot(*dsn, target); err != nil {
		fmt.Fprintf(os.Stderr, "❌ 快照失败，已中止清空（你的数据没有被改动）：%v\n", err)
		os.Exit(1)
	}
	fmt.Printf("\n✓ 快照已保存：%s\n", target)

	// ---- 清空（一个事务里删两张表）----
	err = db.Transaction(func(tx *gorm.DB) error {
		if err := tx.Session(&gorm.Session{AllowGlobalUpdate: true}).Delete(&models.ReviewLog{}).Error; err != nil {
			return fmt.Errorf("清空 review_logs 失败：%w", err)
		}
		if err := tx.Session(&gorm.Session{AllowGlobalUpdate: true}).Delete(&models.WordReview{}).Error; err != nil {
			return fmt.Errorf("清空 word_reviews 失败：%w", err)
		}
		return nil
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "\n❌ %v\n", err)
		fmt.Fprintf(os.Stderr, "   快照仍在 %s，可用它恢复\n", target)
		os.Exit(1)
	}

	var afterReviews, afterLogs, afterWords int64
	db.Model(&models.WordReview{}).Count(&afterReviews)
	db.Model(&models.ReviewLog{}).Count(&afterLogs)
	db.Model(&models.Word{}).Count(&afterWords)

	fmt.Printf("\n清空后：\n")
	fmt.Printf("  words        %d 行（应与清空前相同：%d）\n", afterWords, words)
	fmt.Printf("  word_reviews %d 行\n", afterReviews)
	fmt.Printf("  review_logs  %d 行\n", afterLogs)

	if afterWords != words {
		fmt.Fprintln(os.Stderr, "⚠️ 词表行数发生变化，这不正常，请用上面的快照核对！")
		os.Exit(1)
	}
	if afterReviews != 0 || afterLogs != 0 {
		fmt.Fprintln(os.Stderr, "⚠️ 仍有残留行，请检查。")
		os.Exit(1)
	}

	if *keep > 0 {
		snapshot.Prune(absRoot, *keep)
	}
	fmt.Printf("\n✓ 已清空 %d 行进度 + %d 行日志，词表 %d 行未动。\n", reviews, logs, words)
	fmt.Println("  重启 Go 后端后，所有用户都会从「空池子」开始（池内总数 = 词表行数）。")
}
