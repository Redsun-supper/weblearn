package main

// P0-1 迁移工具：把复习进度从「一个单词全局一行」改成「一人一行」。
//
// 为什么需要它：P0-1 之前 word_reviews 上有 UNIQUE(word_id)（索引 idx_word_reviews_word_id），
// 第二个用户复习同一个词会覆盖第一个人的进度；改造后唯一键是 (user_id, word_id)。
// 而 **GORM 的 AutoMigrate 只补新列/新索引、不会删旧索引**，旧索引必须显式删掉，
// 否则第二个用户一复习就撞 `UNIQUE constraint failed`——运行时才炸，很难联想到是漏跑迁移。
//
// 本工具做四件事（幂等，已迁移过只会报告「无需迁移」）：
//
//  1. 只读检查：旧索引还在不在、有多少行没有归属；
//  2. （-apply 时）先做一份安全快照（VACUUM INTO，见 snapshot 包）；
//  3. AutoMigrate 补上 user_id 列与新索引；
//  4. 清掉 user_id = 0 的历史行（P0-1 决策：旧数据清理，不归属给任何人），
//     再删掉旧的唯一索引与已被取代的单列 due_at 索引，最后复验一遍。
//
// 用法（在 backend-go/ 目录下）：
//
//	go run ./cmd/migrate                          # 默认 dry-run：只报告，不改动任何东西
//	go run ./cmd/migrate -apply                   # 执行（先自动快照到 <仓库>/backups/<时间戳>/）
//	go run ./cmd/migrate -db D:\path\guangxue.db -apply       # 指定库
//	go run ./cmd/migrate -out D:\gx-backup -apply             # 指定快照根目录
//
// 退出码：0 = 成功或无需迁移；1 = 失败。

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"backend-go/database"
	"backend-go/snapshot"
)

func main() {
	var (
		dsn     = flag.String("db", "guangxue.db", "数据库文件路径")
		apply   = flag.Bool("apply", false, "真的执行迁移（默认只体检、不改动）")
		outRoot = flag.String("out", filepath.Join("..", "backups"), "迁移前快照的输出根目录")
	)
	flag.Parse()

	db, err := database.Open(*dsn)
	if err != nil {
		fail(err)
	}

	legacy, err := database.LegacyReviewIndexPresent(db)
	if err != nil {
		fail(err)
	}
	if !legacy {
		fmt.Printf("✓ %s 已经是 P0-1 之后的结构（旧索引 %s 不存在），无需迁移。\n",
			filepath.ToSlash(*dsn), database.LegacyReviewUniqueIndex)
		return
	}

	// 注意：这一步在 AutoMigrate **之前**跑，旧库上 user_id 列可能还不存在，
	// 所以用 LegacyOrphanEstimate（没有列时按「现存行全都会变成无归属」来估算）。
	reviews, logs, err := database.LegacyOrphanEstimate(db)
	if err != nil {
		fail(err)
	}
	hasUserID, err := database.HasColumn(db, "word_reviews", "user_id")
	if err != nil {
		fail(err)
	}

	fmt.Printf("发现旧结构：%s 仍是 UNIQUE(word_id)，意味着一个单词全局只有一行进度。\n", database.LegacyReviewUniqueIndex)
	fmt.Printf("本次迁移会：\n")
	if hasUserID {
		fmt.Printf("  1. 清理没有归属（user_id = 0）的历史进度：word_reviews %d 行、review_logs %d 行\n", reviews, logs)
	} else {
		fmt.Printf("  1. 补上 user_id 列（旧库还没有），现有 %d 行进度 / %d 行日志都会变成 user_id = 0，随即被清理\n", reviews, logs)
	}
	fmt.Printf("  2. 补上两个新索引：%s、%s\n",
		database.ReviewUserWordIndex, database.ReviewLogUserTimeIndex)
	fmt.Printf("  3. 删除旧索引：%s、%s\n", database.LegacyReviewUniqueIndex, database.LegacyReviewDueIndex)

	if !*apply {
		fmt.Printf("\n（dry-run：以上改动都**没有**执行。确认后加 -apply，届时会先自动快照。）\n")
		return
	}

	absRoot, err := snapshot.SafeRoot(*outRoot)
	if err != nil {
		fail(err)
	}
	dir, err := snapshot.NewDir(absRoot)
	if err != nil {
		fail(fmt.Errorf("创建快照目录失败: %w", err))
	}
	fmt.Printf("\n先留一份底：\n")
	if err := snapshot.Snapshot(*dsn, dir); err != nil {
		fail(fmt.Errorf("迁移前的快照失败，已中止（不带着没备份的库做迁移）: %w", err))
	}

	if err := database.Migrate(db); err != nil {
		fail(fmt.Errorf("AutoMigrate 失败: %w", err))
	}
	if err := database.ApplyReviewUserIsolation(db, os.Stdout); err != nil {
		fail(fmt.Errorf("迁移失败: %w", err))
	}

	// 复验：旧索引必须消失、新索引必须存在，否则下次启动仍然会被自检拦下来。
	stillLegacy, err := database.LegacyReviewIndexPresent(db)
	if err != nil {
		fail(err)
	}
	hasNew, err := database.IndexPresent(db, database.ReviewUserWordIndex)
	if err != nil {
		fail(err)
	}
	if stillLegacy || !hasNew {
		fail(fmt.Errorf("复验未通过（旧索引仍在=%v，新索引存在=%v），请检查上面的输出", stillLegacy, hasNew))
	}
	fmt.Printf("\n✓ 迁移完成，快照在 %s；现在可以正常启动服务了。\n", dir)
}

func fail(err error) {
	fmt.Fprintf(os.Stderr, "✗ %v\n", err)
	os.Exit(1)
}
