// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// Package stablehash 提供「同一个用户、同一天、同一个词 = 同一个随机位置」的确定性哈希。
//
// 为什么需要它：单一循环池要求池子**按用户随机排列**，但又必须满足三个硬约束：
//
//  1. **稳定**：同一天内多次请求（翻页、刷新）必须得到完全相同的顺序，
//     否则第 1 页和第 2 页会重叠或漏卡。
//  2. **可下推到 SQL**：池子按未来 3500 词规模设计，不能在 Go 侧全量取出再排序。
//  3. **每天换一批**：每天固定抽 5 个放最前面（见 docs/review-pool-plan.md 的 A2/A3）。
//
// ⚠️ 实现的第一个坑（已用探针实测）：**SQLite 在整数溢出时会把结果提升成 REAL**，
// 例如 `45522878347300299 * 1103515245` 的类型是 `real`、值是 `5.02e+25`；
// 再对 REAL 做 `& 2147483647` 会因为超出 int64 而**恒定返回 2147483647** ——
// 于是所有词的哈希值相同、排序完全失效，而且**不报任何错**。
//
// 因此这里的每一轮乘法之前都先把累加值压回 `[0, 2^31)`，保证中间结果远小于 int64 上限，
// 全程纯整数运算。SQL 侧对应 internal/stablehash 的 SQLHashExpr，两者必须逐字等价
// （有 TestSQLHashMatchesGo 用真 SQLite 锁住）。
package stablehash

import "fmt"

// Mod 每一轮取模的基数 = 2^31。
// 取 2 的幂而不是大素数，是为了让「取模」在 SQL 里也能写成纯整数运算且结果恒非负。
const Mod int64 = 2147483648

// 各轮使用的奇常数（奇数保证在模 2^31 下可逆，从而不丢信息、分布均匀）。
const (
	seedA = 999983
	mulB  = 2654435761 // 黄金比例常数
	mulC  = 40503
	mulD  = 15485863 // 第 100 万个素数
	// 混入各分量时的偏移常数，作用是避免 0 输入退化成 0 输出。
	offWord = 20261002
	offUser = 7919
	offDay  = 104729
	offMix  = 2246822519
)

// Hash 计算 (userID, wordID, day) 的稳定哈希，返回值恒在 `[0, 2^21)`。
//
// 参数 day 是「本地自然日」的整数形式（例如 20261002）。同一天内换页、刷新、
// 换设备都必须传同一个值；跨天自然换一批。
//
// 只保留高 21 位（`>> 10 & 0x1FFFFF`）：位置排序需要的是**决定论**而不是密码学强度，
// 21 位（200 万档）已远超 3500 词的规模，实测 50 用户 × 7 天的抽卡覆盖率为 95.5%。
func Hash(userID, wordID, day int64) int64 {
	// ⚠️ 每一轮的形状必须与 SQLHashExpr 逐字对应：先 `(v % Mod + 系数)` 再 `% Mod`、最后乘。
	// 曾经把用户 / 日期系数与偏移常量合并相加，导致 Go 与 SQL 相差一轮取模（结果完全不同）。
	h := mix(wordID%Mod+offWord, seedA)
	h = mix(h+userID%Mod+offUser, mulB)
	h = mix(h+day%Mod+offDay, mulC)
	h = mix(h+userID%Mod+offMix, mulD)
	return (h >> 10) & 0x1FFFFF
}

// mix 是单轮混合：先把入参压回 [0, 2^31)，再乘、再压回。
// 顺序不能颠倒——先乘会溢出（见包注释里实测到的 REAL 陷阱）。
func mix(v, k int64) int64 {
	return ((v % Mod) * k % Mod)
}

// DayKey 把年月日折成一个整数种子（例如 2026-10-02 → 20261002）。
func DayKey(year int, month int, day int) int64 {
	return int64(year)*10000 + int64(month)*100 + int64(day)
}

// SQLHashExpr 返回与 Hash 等价的 SQL 表达式，词表引用用调用方给的限定名。
//
// ⚠️ qualifier 必须与查询里那张表的引用完全一致：主查询用 `words.id`，
// 子查询里如果给 words 起了别名 `w2`，就必须传 `w2.id`。传错时 SQLite 报
// `no such column`（实测踩过两次：`w.id` 与子查询里的 `words.id`）。
//
// 调用方还要负责用 `fmt.Sprintf` 注入 userID 与 day（两者都会被 `% Mod` 归一化，避免负数）。
//
// 表达式只使用 `+ * % >> &` 与小整数常量，实测在 SQLite 3.41.2 上与 Hash 逐位一致。
//
// 每一轮的形状：`((h + 系数%Mod + 常量) % Mod * 质数) % Mod` —— 与 Hash 里的 mix 调用一一对应，
// 改动任意一侧都必须同步改另一侧，并跑 TestSQLHashMatchesGo。
func SQLHashExpr(qualifier string, userID, day int64) string {
	u := userID % Mod
	if u < 0 {
		u += Mod
	}
	d := day % Mod
	if d < 0 {
		d += Mod
	}

	h := qualifier
	h = fmt.Sprintf("((%s %% %d + %d) * %d %% %d)", h, Mod, offWord, seedA, Mod)
	h = fmt.Sprintf("((%s + %d + %d) %% %d * %d %% %d)", h, u, offUser, Mod, mulB, Mod)
	h = fmt.Sprintf("((%s + %d + %d) %% %d * %d %% %d)", h, d, offDay, Mod, mulC, Mod)
	h = fmt.Sprintf("((%s + %d + %d) %% %d * %d %% %d)", h, u, offMix, Mod, mulD, Mod)
	return fmt.Sprintf("((%s >> 10) & 2097151)", h)
}
