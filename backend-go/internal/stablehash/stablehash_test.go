// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package stablehash

// 这一组测试的价值在于**把 SQLite 的溢出陷阱钉死**。
//
// 背景（实测，见包注释）：SQLite 在整数溢出时会把结果提升成 REAL，
// `45522878347300299 * 1103515245` 是 `real` 类型的 `5.02e+25`，
// 再 `& 2147483647` 会因为超出 int64 而恒定返回 2147483647 —— 所有词哈希相同、排序静默失效。
//
// 所以 TestSQLHashMatchesGo 必须用**真 SQLite** 跑，而不是只测 Go 侧：
// 它一次性锁住「Go 与 SQL 逐位一致」和「没有踩溢出陷阱」两件事。

import (
	"database/sql"
	"fmt"
	"testing"

	_ "modernc.org/sqlite"
)

func TestHashIsDeterministic(t *testing.T) {
	a := Hash(1, 777, 20261002)
	b := Hash(1, 777, 20261002)
	if a != b {
		t.Fatalf("同一输入两次计算出不同结果：%d vs %d", a, b)
	}
	if a < 0 || a >= 1<<21 {
		t.Fatalf("哈希必须落在 [0, 2^21)，实际 %d", a)
	}
}

func TestHashDiffersByUserAndDay(t *testing.T) {
	// 不同用户在同一词上必须给出不同位置（否则每个人的池子顺序一样，就失去了「按用户随机」）
	if Hash(1, 42, 20261002) == Hash(2, 42, 20261002) {
		t.Error("不同用户的哈希相同，池子顺序会完全一致")
	}
	// 跨天必须换一批（否则「每天抽 5 个」永远是同一批）
	if Hash(1, 42, 20261002) == Hash(1, 42, 20261003) {
		t.Error("跨天的哈希相同，每天抽的 5 个不会变化")
	}
}

func TestHashDistributesEvenly(t *testing.T) {
	// 3500 个词（用户拍板的未来规模）分 10 桶，每桶理想 350，允许 ±15%
	const n, buckets = 3500, 10
	var counts [buckets]int
	for wid := int64(1); wid <= n; wid++ {
		counts[Hash(1, wid, 20261002)*buckets/(1<<21)]++
	}
	lo, hi := n/buckets*85/100, n/buckets*115/100
	for i, c := range counts {
		if c < lo || c > hi {
			t.Errorf("第 %d 桶有 %d 个，超出允许范围 [%d, %d]（分布不均会让某些词永远排不到前面）", i, c, lo, hi)
		}
	}
}

// TestTop5IsStableAndRotates 锁住「今日 5 个」的两个性质：当天固定、跨天轮换。
func TestTop5IsStableAndRotates(t *testing.T) {
	top5 := func(day int64) map[int64]bool {
		type kv struct {
			id int64
			h  int64
		}
		all := make([]kv, 0, 3500)
		for wid := int64(1); wid <= 3500; wid++ {
			all = append(all, kv{wid, Hash(1, wid, day)})
		}
		for i := 0; i < 5; i++ {
			mi := i
			for j := i + 1; j < len(all); j++ {
				if all[j].h < all[mi].h {
					mi = j
				}
			}
			all[i], all[mi] = all[mi], all[i]
		}
		out := map[int64]bool{}
		for i := 0; i < 5; i++ {
			out[all[i].id] = true
		}
		return out
	}

	today, tomorrow := top5(20261002), top5(20261003)
	if len(today) != 5 {
		t.Fatalf("今日集合应有 5 个词，实际 %d", len(today))
	}
	// 同一天重复计算必须完全一致（翻页/刷新依赖这一条）
	again := top5(20261002)
	for id := range today {
		if !again[id] {
			t.Fatalf("同一天两次计算得到不同的集合：%d 只在其中一次出现", id)
		}
	}
	// 跨天应当基本换批（允许偶然重复，但不能一模一样）
	same := 0
	for id := range today {
		if tomorrow[id] {
			same++
		}
	}
	if same == 5 {
		t.Error("今天与明天的「前 5 个」完全相同，每天抽 5 个失去意义")
	}
}

// TestSQLHashMatchesGo 用真 SQLite 验证 SQL 表达式与 Go 实现逐位一致。
//
// ⚠️ 这是本包最重要的一条测试：手写 SQL 表达式时括号、取模顺序、位运算优先级
// 任何一处出错都会静默改变排序（甚至全表哈希相同），而在浏览器里只表现为「顺序怪怪的」。
func TestSQLHashMatchesGo(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatalf("打开内存 SQLite 失败：%v", err)
	}
	defer db.Close()

	if _, err := db.Exec("CREATE TABLE words(id INTEGER PRIMARY KEY)"); err != nil {
		t.Fatalf("建表失败：%v", err)
	}
	tx, err := db.Begin()
	if err != nil {
		t.Fatal(err)
	}
	stmt, err := tx.Prepare("INSERT INTO words(id) VALUES(?)")
	if err != nil {
		t.Fatal(err)
	}
	for i := 1; i <= 3500; i++ {
		if _, err := stmt.Exec(i); err != nil {
			t.Fatal(err)
		}
	}
	stmt.Close()
	if err := tx.Commit(); err != nil {
		t.Fatal(err)
	}

	const userID, day = int64(1), int64(20261002)
	expr := SQLHashExpr("words.id", userID, day)

	rows, err := db.Query("SELECT words.id, " + expr + " AS h FROM words ORDER BY words.id")
	if err != nil {
		t.Fatalf("SQL 表达式执行失败（括号或运算符优先级有问题？）：%v", err)
	}
	defer rows.Close()

	seen := map[int64]bool{}
	n := 0
	for rows.Next() {
		var id, got int64
		if err := rows.Scan(&id, &got); err != nil {
			t.Fatal(err)
		}
		want := Hash(userID, id, day)
		if got != want {
			t.Fatalf("word=%d SQL 算出 %d，Go 算出 %d —— 两侧不再等价", id, got, want)
		}
		if got < 0 || got >= 1<<21 {
			t.Fatalf("word=%d 的哈希 %d 越界", id, got)
		}
		seen[got] = true
		n++
	}
	if n != 3500 {
		t.Fatalf("应扫到 3500 行，实际 %d", n)
	}

	// 溢出陷阱的回归判据：如果 SQL 里发生了「提升为 REAL 再 & 2147483647」，
	// 所有行会退化成同一个值（2147483647 或 2097151），这里就会暴露。
	if len(seen) < 3400 {
		t.Fatalf("3500 个词只算出 %d 个不同哈希，分布已经退化（疑似踩到 SQLite 溢出提升为 REAL 的陷阱）", len(seen))
	}
}

// TestSQLHashExprIsPureInteger 直接对「溢出会提升为 REAL」这一行为做防守。
//
// 判据：整条表达式的类型必须是 integer。只要中间某一步溢出成 REAL，
// SQLite 的 typeof() 就会给出 real。
func TestSQLHashExprIsPureInteger(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	// 取一个会让「先乘后模」写法溢出的输入组合，确认我们的写法仍然全是整数
	for _, tc := range []struct{ user, word, day int64 }{
		{1, 1, 20261002},
		{999999, 3500, 20261002},
		{1, 3500, 29991231},
	} {
		// 子查询里只有一列 id，没有 words 这张表，所以哈希的限定名要用 `id`
		sqlText := fmt.Sprintf("SELECT typeof(%s) FROM (SELECT %d AS id)", SQLHashExpr("id", tc.user, tc.day), tc.word)
		var typ string
		if err := db.QueryRow(sqlText).Scan(&typ); err != nil {
			t.Fatalf("查询类型失败：%v", err)
		}
		if typ != "integer" {
			t.Errorf("user=%d word=%d day=%d 的表达式类型是 %s（应为 integer）——中间结果溢出成 REAL 了",
				tc.user, tc.word, tc.day, typ)
		}
	}
}


