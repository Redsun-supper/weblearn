// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 临时工具：直接读取 SQLite 实际存储内容（用后端同款 glebarez/sqlite 驱动，纯 Go 无 CGO）
package main

import (
	"fmt"
	"os"

	"backend-go/config"
	"backend-go/database"
	"backend-go/models"
)

func main() {
	cfg := config.LoadConfig()
	db := database.Init(cfg.DBPath)

	fmt.Println("数据库文件:", cfg.DBPath)
	if fi, err := os.Stat(cfg.DBPath); err == nil {
		fmt.Printf("文件大小: %d 字节\n", fi.Size())
	}

	fmt.Println("\n===== 表清单 =====")
	var tables []string
	db.Raw("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").Scan(&tables)
	for _, t := range tables {
		var n int64
		db.Raw("SELECT COUNT(*) FROM " + t).Scan(&n)
		fmt.Printf("  %-16s %d 行\n", t, n)
	}

	fmt.Println("\n===== words 表结构（单词存在哪） =====")
	type col struct {
		Cid       int
		Name      string
		Type      string
		NotNull   int
		DfltValue *string
		Pk        int
	}
	var cols []col
	db.Raw("PRAGMA table_info(words)").Scan(&cols)
	for _, c := range cols {
		def := ""
		if c.DfltValue != nil {
			def = " default=" + *c.DfltValue
		}
		fmt.Printf("  %-12s %-10s notnull=%d pk=%d%s\n", c.Name, c.Type, c.NotNull, c.Pk, def)
	}

	fmt.Println("\n===== words 索引 =====")
	type idx struct {
		Seq  int
		Name string
		Uniq int
	}
	var idxs []idx
	db.Raw("PRAGMA index_list(words)").Scan(&idxs)
	for _, i := range idxs {
		fmt.Printf("  %s unique=%d\n", i.Name, i.Uniq)
	}

	fmt.Println("\n===== 实际存储的 3 条样本 =====")
	var words []models.Word
	db.Model(&models.Word{}).Order("id ASC").Limit(3).Find(&words)
	for _, w := range words {
		fmt.Printf("  id=%d word=%q phonetic=%q meaning=%q subject=%q example=%q\n",
			w.ID, w.Word, w.Phonetic, w.Meaning, w.Subject, w.Example)
	}

	fmt.Println("\n===== 记忆状态 word_reviews =====")
	var reviews []models.WordReview
	// 进度是「按人一行」的（P0-1）：同一个词会有多行，所以先按 user_id 再按 word_id 排，
	// 打印时也带上 user_id，否则排查「某个用户为什么没有进度」时看不出是谁的行。
	db.Model(&models.WordReview{}).Order("user_id ASC, word_id ASC").Find(&reviews)
	for _, r := range reviews {
		var w models.Word
		db.First(&w, r.WordID)
		fmt.Printf("  user_id=%d word=%q stability=%.4f difficulty=%.4f reps=%d lapses=%d due_at=%v\n",
			r.UserID, w.Word, r.Stability, r.Difficulty, r.Reps, r.Lapses, r.DueAt)
	}

	fmt.Println("\n===== 复习日志 review_logs =====")
	var logs []models.ReviewLog
	db.Model(&models.ReviewLog{}).Order("id ASC").Find(&logs)
	for _, l := range logs {
		flag := ""
		if l.IsProbe {
			flag = "  [抽查]"
		}
		fmt.Printf("  user_id=%d word_id=%d rating=%d %.4f→%.4f interval=%.4f天 at=%s%s\n",
			l.UserID, l.WordID, l.Rating, l.StabilityBefore, l.StabilityAfter, l.IntervalDays,
			l.ReviewedAt.Format("2006-01-02 15:04:05"), flag)
	}
}
