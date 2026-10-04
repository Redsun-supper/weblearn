// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// 命令 seed：把 JSON 词表导入 SQLite 的 words 表，供单词间隔复习使用。
//
// 用法（在 backend-go 目录下执行）：
//
//	go run ./cmd/seed
//	go run ./cmd/seed -file seed/words_english.json -db guangxue.db
//
// 词表 JSON 格式：
//
//	{"words":[{"word":"apple","phonetic":"/ˈæp.əl/","meaning":"n. 苹果","example":"I eat an apple.",
//	           "example_translation":"我吃一个苹果。","senses":[{"pos":"n.","meaning":"苹果"}]}]}
//
// 说明：words.word 是唯一索引，已存在的单词会跳过，因此本命令可以重复执行。
// example_translation 与 senses 都是可选的；senses 留空时，展示层会按 meaning 里的
// 词性标签自动分块（「n. 好处；益处 v. 有益于」拆成名词、动词两块）。
// 换成自己的词表（如中考/高考/四六级词表）时，只要保持同样的 JSON 结构即可。
package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"strings"

	"gorm.io/gorm"

	"backend-go/config"
	"backend-go/database"
	"backend-go/models"
)

// wordFile 词表文件结构
type wordFile struct {
	Words []wordItem `json:"words"`
}

// wordItem 单条词目；example_translation 与 senses 可选
type wordItem struct {
	Word               string            `json:"word"`
	Phonetic           string            `json:"phonetic"`
	Meaning            string            `json:"meaning"`
	Example            string            `json:"example"`
	ExampleTranslation string            `json:"example_translation"`
	Senses             models.WordSenses `json:"senses"`
	Subject            string            `json:"subject"`
}

func main() {
	cfg := config.LoadConfig()
	file := flag.String("file", "seed/words_english.json", "词表 JSON 文件路径")
	dsn := flag.String("db", cfg.DBPath, "SQLite 数据库文件路径")
	flag.Parse()

	raw, err := os.ReadFile(*file)
	if err != nil {
		fmt.Fprintf(os.Stderr, "读取词表失败: %v\n", err)
		os.Exit(1)
	}
	var data wordFile
	if err := json.Unmarshal(raw, &data); err != nil {
		fmt.Fprintf(os.Stderr, "解析词表失败: %v\n", err)
		os.Exit(1)
	}
	if len(data.Words) == 0 {
		fmt.Fprintln(os.Stderr, "词表为空，未导入任何单词")
		os.Exit(1)
	}

	// Init 会自动建表/迁移，首次执行即可直接导入
	db := database.Init(*dsn)

	created, skipped, failed := 0, 0, 0
	for _, item := range data.Words {
		word := strings.TrimSpace(item.Word)
		if word == "" {
			skipped++
			continue
		}
		// 已存在则跳过（保持幂等）
		var existing models.Word
		findErr := db.Where("word = ?", word).First(&existing).Error
		if findErr == nil {
			skipped++
			continue
		}
		if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			failed++
			fmt.Fprintf(os.Stderr, "查询失败 (%s): %v\n", word, findErr)
			continue
		}

		subject := strings.TrimSpace(item.Subject)
		if subject == "" {
			subject = "english"
		}
		record := models.Word{
			Word:               word,
			Phonetic:           item.Phonetic,
			Meaning:            item.Meaning,
			Example:            item.Example,
			ExampleTranslation: strings.TrimSpace(item.ExampleTranslation),
			Senses:             item.Senses,
			Subject:            subject,
		}
		if err := db.Create(&record).Error; err != nil {
			failed++
			fmt.Fprintf(os.Stderr, "写入失败 (%s): %v\n", word, err)
			continue
		}
		created++
	}

	fmt.Printf("词表导入完成：新增 %d，跳过 %d，失败 %d（数据库：%s）\n", created, skipped, failed, *dsn)
}
