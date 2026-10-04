// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package handlers

import (
	"net/http"
	"testing"
	"time"

	"gorm.io/gorm"

	"backend-go/internal/stablehash"
	"backend-go/models"
)

// 本文件覆盖**单一循环池**（docs/review-pool-plan.md）的取数口径。
//
// 与旧版的根本区别：池子 = `words` 全表，不再有「已学 / 未学」的身份差别。
// 排序由服务端一次定死，四个优先级桶（用户决策 C10）：
//
//	0 = 今日随机抽出的 5 个（置顶，全天固定 —— 稳定哈希）
//	1 = 已过期（due_at <= now，越久越靠前）
//	2 = 从未复习过（没有进度行）
//	3 = 未到期（due_at ASC）
//
// 桶内键：桶 1/3 用 due_at；桶 0/2 用按 (user, 当天, word) 的稳定哈希。
//
// ⚠️ 这里用的是真 HTTP + 真 SQLite（setupTestRouter），所以也顺带覆盖了
// 「哈希表达式在 SQL 里能不能跑」——SQLite 的整数溢出陷阱就藏在这一层。

// poolTestDay 固定「今天」的种子，避免测试跨零点抖动。
// 全部池用例都显式带 `?today=<这个值>`，与服务端的 Asia/Shanghai 回退解耦。
const poolTestDay = "20260101"

// cardWordIDs 取出卡片列表里的 word_id（响应里的字段名是 id，见 dueCard 结构）
func cardWordIDs(t *testing.T, data map[string]interface{}) []uint {
	t.Helper()
	list := items(t, data)
	out := make([]uint, 0, len(list))
	for _, item := range list {
		raw, ok := item["id"]
		if !ok {
			t.Fatalf("卡片缺少 id 字段，实际字段=%v", mapKeys(item))
		}
		out = append(out, uint(toInt64(t, raw)))
	}
	return out
}

// assertIDOrder 断言 id 序列与期望完全一致（顺序也要求一致）
func assertIDOrder(t *testing.T, got, want []uint, label string) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("%s 长度期望 %d（%v），实际 %d（%v）", label, len(want), want, len(got), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("%s 第 %d 位期望 %d，实际 %d（完整序列 %v）", label, i+1, want[i], got[i], got)
		}
	}
}

// modelsWord 造一个字段齐全的词条（cardFields 用例用），避免测试里出现超长字面量
func modelsWord() models.Word {
	return models.Word{
		Word:               "kernel",
		Phonetic:           "/ˈkɜː.nəl/",
		Meaning:            "n. 核心；内核",
		Example:            "The kernel handles interrupts.",
		ExampleTranslation: "内核负责处理中断。",
		Book:               "必修一",
		Unit:               "Unit 1",
	}
}

// withDay 给路径拼上固定的 today 参数
func withDay(path string) string {
	if path == "" {
		return path
	}
	if contains(path, "?") {
		return path + "&today=" + poolTestDay
	}
	return path + "?today=" + poolTestDay
}

func contains(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}

// wantedDailyIDs 用与 stablehash 相同的口径离线算出「今日 5 个」，用于断言。
// 独立实现一遍（不是调服务端代码），这样服务端哪天换了排序键，测试就会红。
func wantedDailyIDs(wordIDs []uint, userID uint, seed int64) []uint {
	type kv struct {
		id uint
		h  int64
	}
	all := make([]kv, 0, len(wordIDs))
	for _, id := range wordIDs {
		all = append(all, kv{id, stablehash.Hash(int64(userID), int64(id), seed)})
	}
	n := dailyWordCount
	if len(all) < n {
		n = len(all)
	}
	for i := 0; i < n; i++ {
		mi := i
		for j := i + 1; j < len(all); j++ {
			if all[j].h < all[mi].h {
				mi = j
			}
		}
		all[i], all[mi] = all[mi], all[i]
	}
	out := make([]uint, 0, n)
	for i := 0; i < n; i++ {
		out = append(out, all[i].id)
	}
	return out
}

func mustSeed(t *testing.T, seed string) int64 {
	t.Helper()
	var v int64
	for i := 0; i < len(seed); i++ {
		if seed[i] < '0' || seed[i] > '9' {
			t.Fatalf("poolTestDay 必须是 8 位数字，实际 %q", seed)
		}
		v = v*10 + int64(seed[i]-'0')
	}
	return v
}

// queueCard 把响应里的一张卡读成好用的结构
type queueCard struct {
	ID        uint
	Bucket    int
	Daily     bool
	HasReview bool
	DueAt     *int64
}

func readQueueCards(t *testing.T, data map[string]interface{}) []queueCard {
	t.Helper()
	list := items(t, data)
	out := make([]queueCard, 0, len(list))
	for _, item := range list {
		c := queueCard{
			ID:        uint(toInt64(t, item["id"])),
			Bucket:    toInt(t, item["bucket"]),
			Daily:     item["daily"] == true,
			HasReview: item["has_review"] == true,
		}
		if raw, ok := item["due_at"]; ok && raw != nil {
			if s, ok := raw.(string); ok {
				if ts, err := time.Parse(time.RFC3339, s); err == nil {
					ms := ts.UnixMilli()
					c.DueAt = &ms
				}
			}
		}
		out = append(out, c)
	}
	return out
}

func cardIDs(cards []queueCard) []uint {
	out := make([]uint, 0, len(cards))
	for _, c := range cards {
		out = append(out, c.ID)
	}
	return out
}

func findCard(t *testing.T, cards []queueCard, id uint) queueCard {
	t.Helper()
	for _, c := range cards {
		if c.ID == id {
			return c
		}
	}
	t.Fatalf("池子里找不到词 %d（池子=%v）", id, cardIDs(cards))
	return queueCard{}
}

// assertBucketOrder 断言四个桶按 0→1→2→3 的次序出现（桶内不做断言）
func assertBucketOrder(t *testing.T, cards []queueCard, label string) {
	t.Helper()
	last := -1
	for i, c := range cards {
		if c.Bucket < last {
			t.Fatalf("%s 的桶次序错了：第 %d 张 bucket=%d，但前面已经出现过 bucket=%d", label, i+1, c.Bucket, last)
		}
		last = c.Bucket
	}
}

// assertPageNoOverlap 断言两页没有重复的词（分页必须稳定，否则用户会重复看到同一张卡）
func assertPageNoOverlap(t *testing.T, a, b []queueCard) {
	t.Helper()
	seen := map[uint]bool{}
	for _, c := range a {
		seen[c.ID] = true
	}
	for _, c := range b {
		if seen[c.ID] {
			t.Fatalf("第 1 页与第 2 页出现重复的词 %d（第 1 页=%v，第 2 页=%v）",
				c.ID, cardIDs(a), cardIDs(b))
		}
	}
}

// ---------- 池子的基本形状 ----------

// TestPoolContainsEveryWord 池子是全表：一个新用户也应该看到**所有**词，
// 而不是旧模型那样「只有已学词」。
func TestPoolContainsEveryWord(t *testing.T) {
	router, db := setupTestRouter(t)
	ids := []uint{
		seedWord(t, db, "alpha"),
		seedWord(t, db, "bravo"),
		seedWord(t, db, "charlie"),
	}
	// 只给其中一个建进度行，池子仍应是三个
	due := time.Now().Add(-time.Hour)
	seedReview(t, db, ids[0], 3, 5, &due, 1, 0)

	data, code := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue"), nil))
	if code != http.StatusOK {
		t.Fatalf("queue 的 code 期望 200，实际 %d", code)
	}
	assertInt(t, data, "total", 3)
	if got := cardIDs(readQueueCards(t, data)); len(got) != 3 {
		t.Fatalf("池子应包含全部 3 个词，实际 %v", got)
	}
}

// TestPoolBucketOrderAndOverdueFirst 校验四个桶的次序与桶内规则：
// 今日 5 个最前 → 已过期（越久越前）→ 从未复习 → 未到期（越近越前）。
//
// ⚠️ 两个必须遵守的约束（都实测踩过）：
//  1. 池子里的词必须**多于 5 个**，否则「今日 5 个」把全池盖住，四个桶退化成只剩桶 0；
//  2. 断言桶内次序时只能看**非置顶**卡 —— 置顶卡一律进桶 0，哪怕它已经过期
//     （今天恰好抽到一张过期的卡时，它的 bucket 就是 0 而不是 1）。
func TestPoolBucketOrderAndOverdueFirst(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()
	overdue := seedWord(t, db, "overdue-one")
	futureFar := seedWord(t, db, "future-far")
	futureNear := seedWord(t, db, "future-near")
	never := seedWord(t, db, "never-studied")
	filler := make([]uint, 0, 8)
	for i := 0; i < 8; i++ {
		id := seedWord(t, db, "filler"+itoa(int64(i)))
		due := now.Add(time.Duration(10+i) * time.Hour)
		seedReview(t, db, id, 1, 5, &due, 1, 0)
		filler = append(filler, id)
	}

	overdueAt := now.Add(-48 * time.Hour)
	far := now.Add(72 * time.Hour)
	near := now.Add(2 * time.Hour)
	seedReview(t, db, overdue, 1, 5, &overdueAt, 1, 0)
	seedReview(t, db, futureFar, 1, 5, &far, 1, 0)
	seedReview(t, db, futureNear, 1, 5, &near, 1, 0)

	all := append([]uint{overdue, futureFar, futureNear, never}, filler...)
	daily := wantedDailyIDs(all, 1, mustSeed(t, poolTestDay))

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?limit=100"), nil))
	cards := readQueueCards(t, data)
	if len(cards) != len(all) {
		t.Fatalf("池子应返回全部 %d 个词，实际 %d（%v）", len(all), len(cards), cardIDs(cards))
	}
	assertBucketOrder(t, cards, "池子")

	for _, id := range daily {
		c := findCard(t, cards, id)
		if c.Bucket != 0 || !c.Daily {
			t.Fatalf("今日置顶词 %d 应当是 bucket=0 且 daily=true，实际 bucket=%d daily=%v", id, c.Bucket, c.Daily)
		}
	}
	assertInt(t, data, "daily", int64(dailyWordCount))

	// 桶 1 = 已过期且**不在今日置顶**的词。本夹具里只有 overdue 一个，
	// 所以「桶 1 恰好是它」同时钉住了两件事：过期进桶 1、置顶优先级高于过期。
	inBucket1 := []uint{}
	for _, c := range cards {
		if c.Bucket == 1 {
			inBucket1 = append(inBucket1, c.ID)
		}
	}
	assertIDOrder(t, inBucket1, []uint{overdue}, "桶 1（已过期）")
	if c := findCard(t, cards, overdue); c.Daily {
		t.Fatalf("本夹具故意让 %d 不在今日置顶里，但它被算成了置顶", overdue)
	}

	// 确定没有置顶的 8 个未来卡：按 due_at 升序（越近越前）
	posFar, posNear := -1, -1
	futureBucketOrder := []uint{}
	for i, c := range cards {
		if c.Bucket != 3 || c.Daily {
			continue
		}
		futureBucketOrder = append(futureBucketOrder, c.ID)
		if c.ID == futureFar {
			posFar = i
		}
		if c.ID == futureNear {
			posNear = i
		}
	}
	if len(futureBucketOrder) == 0 {
		t.Fatalf("桶 3 不应为空，实际顺序=%v", cardIDs(cards))
	}
	// 桶 3 内部按 due_at 升序：futureNear(+2h) 之后才是 futureFar(+72h)
	if posFar >= 0 && posNear >= 0 && posNear > posFar {
		t.Fatalf("未到期卡应按 due_at 升序（越近越前），实际顺序=%v", cardIDs(cards))
	}

	// 从没复习过的词带 has_review=false；有进度行的带 true
	if c := findCard(t, cards, never); c.HasReview {
		t.Fatalf("从未复习的词 %d 的 has_review 应为 false", never)
	}
	if c := findCard(t, cards, overdue); !c.HasReview {
		t.Fatalf("有进度行的词 %d 的 has_review 应为 true", overdue)
	}
}

// TestPoolOrderIsStableWithinDay 「今日 5 个」必须全天固定：
// 同一用户同一天多次请求（刷新 / 翻页）不能换批，否则第 1 页和第 2 页会重叠或漏卡。
func TestPoolOrderIsStableWithinDay(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()
	ids := make([]uint, 0, 12)
	for i := 0; i < 12; i++ {
		id := seedWord(t, db, "stable"+itoa(int64(i)))
		due := now.Add(time.Duration(i) * time.Hour)
		seedReview(t, db, id, 1, 5, &due, 1, 0)
		ids = append(ids, id)
	}

	first, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue"), nil))
	second, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue"), nil))
	gotFirst, gotSecond := cardIDs(readQueueCards(t, first)), cardIDs(readQueueCards(t, second))

	assertIDOrder(t, gotFirst, gotSecond, "同一天两次请求的池子顺序")
	// 而且要与离线按哈希算出的「今日 5 个」一致
	wantDaily := wantedDailyIDs(ids, 1, mustSeed(t, poolTestDay))
	gotDaily := []uint{}
	for _, c := range readQueueCards(t, first) {
		if c.Daily {
			gotDaily = append(gotDaily, c.ID)
		}
	}
	if len(gotDaily) != len(wantDaily) {
		t.Fatalf("今日置顶数期望 %d，实际 %d（%v）", len(wantDaily), len(gotDaily), gotDaily)
	}
	for _, id := range wantDaily {
		found := false
		for _, g := range gotDaily {
			if g == id {
				found = true
			}
		}
		if !found {
			t.Fatalf("今日置顶词 %d 不在结果里（实际 %v）", id, gotDaily)
		}
	}
}

// TestPoolOrderChangesNextDay 跨天应当换一批置顶词（否则「每天抽 5 个」没意义）。
func TestPoolOrderChangesNextDay(t *testing.T) {
	router, db := setupTestRouter(t)
	for i := 0; i < 20; i++ {
		seedWord(t, db, "rota"+itoa(int64(i)))
	}

	day1, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?today=20260101", nil))
	day2, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?today=20260102", nil))

	daily1 := map[uint]bool{}
	for _, c := range readQueueCards(t, day1) {
		if c.Daily {
			daily1[c.ID] = true
		}
	}
	same := 0
	for _, c := range readQueueCards(t, day2) {
		if c.Daily && daily1[c.ID] {
			same++
		}
	}
	if same == dailyWordCount {
		t.Fatalf("跨天后的置顶词完全没变（%d 个全相同），稳定哈希的日期维度没生效", same)
	}
}

// TestPoolPaginationHasNoOverlap 分页不重叠：两页之间不能出现同一个词。
func TestPoolPaginationHasNoOverlap(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()
	for i := 0; i < 15; i++ {
		id := seedWord(t, db, "page"+itoa(int64(i)))
		due := now.Add(time.Duration(i) * time.Hour)
		seedReview(t, db, id, 1, 5, &due, 1, 0)
	}

	page1, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?limit=7&offset=0"), nil))
	page2, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?limit=7&offset=7"), nil))
	page3, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?limit=7&offset=14"), nil))

	c1, c2, c3 := readQueueCards(t, page1), readQueueCards(t, page2), readQueueCards(t, page3)
	assertPageNoOverlap(t, c1, c2)
	assertPageNoOverlap(t, c2, c3)
	assertInt(t, page1, "total", 15)
	assertInt(t, page2, "total", 15) // total 不随翻页变化
	if len(c1) != 7 || len(c2) != 7 || len(c3) != 1 {
		t.Fatalf("分页长度期望 7/7/1，实际 %d/%d/%d", len(c1), len(c2), len(c3))
	}

	// 拼起来应当正好是整池（顺序一致、不重不漏）
	merged := append(append(append([]uint{}, cardIDs(c1)...), cardIDs(c2)...), cardIDs(c3)...)
	full, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?limit=100"), nil))
	assertIDOrder(t, merged, cardIDs(readQueueCards(t, full)), "分页拼接结果与整页结果")
}

// TestPoolStillServesWhenEverythingIsFarFuture 用户决策 B9：
// 整池都被推到未来时，队列**仍然不空**（想学就能一直往下翻）。
func TestPoolStillServesWhenEverythingIsFarFuture(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()
	for i := 0; i < 6; i++ {
		id := seedWord(t, db, "later"+itoa(int64(i)))
		due := now.Add(time.Duration(200+i) * 24 * time.Hour)
		seedReview(t, db, id, 30, 5, &due, 5, 0)
	}

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue"), nil))
	cards := readQueueCards(t, data)
	if len(cards) != 6 {
		t.Fatalf("整池都在未来时仍应返回全部 6 张，实际 %d", len(cards))
	}
	for _, c := range cards {
		// 桶 0（今日置顶）优先于一切，所以命中置顶的卡不算违反「全部未到期 → 桶 3」
		if c.Bucket == 0 {
			continue
		}
		if c.Bucket != 3 {
			t.Fatalf("全部未到期时非置顶卡都应是 bucket=3，实际词 %d bucket=%d", c.ID, c.Bucket)
		}
	}
	// due 池仍只给「真正已过期」的
	dueData, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/due"), nil))
	assertIDOrder(t, cardIDs(readQueueCards(t, dueData)), nil, "全未到期时的 due 池")
}

// TestPoolNeverReviewedSortedByHash 从未复习的词之间按稳定哈希排队（不是按 id），
// 而且同一天内稳定、翻页不重。
func TestPoolNeverReviewedSortedByHash(t *testing.T) {
	router, db := setupTestRouter(t)
	ids := make([]uint, 0, 10)
	for i := 0; i < 10; i++ {
		ids = append(ids, seedWord(t, db, "fresh"+itoa(int64(i))))
	}

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?limit=100"), nil))
	cards := readQueueCards(t, data)
	if len(cards) != 10 {
		t.Fatalf("期望 10 张，实际 %d", len(cards))
	}

	// 桶 0（今日 5 个）之外的桶 2 词，必须按哈希升序
	seed := mustSeed(t, poolTestDay)
	lastHash := int64(-1)
	counted := 0
	for _, c := range cards {
		if c.Bucket != 2 {
			continue
		}
		h := stablehash.Hash(1, int64(c.ID), seed)
		if h < lastHash {
			t.Fatalf("桶 2 的词没按哈希升序：词 %d 的哈希 %d 小于前一个 %d", c.ID, h, lastHash)
		}
		lastHash = h
		counted++
	}
	if counted != 5 {
		t.Fatalf("10 个未复习词里应有 5 个落在桶 2（另外 5 个是今日置顶），实际 %d", counted)
	}
}

// ---------- 已废弃 / 只读诊断的接口 ----------

// TestReviewsNewIsDeprecated 旧「新词池」接口在新模型下不再返回候选（池子即全表）。
func TestReviewsNewIsDeprecated(t *testing.T) {
	router, db := setupTestRouter(t)
	seedWord(t, db, "legacy")
	seedWord(t, db, "legacy2")

	data, code := decodeData(t, doJSON(t, router, "GET", "/api/reviews/new?limit=20", nil))
	if code != http.StatusOK {
		t.Fatalf("new 的 code 期望 200（保留路由避免旧页面 404），实际 %d", code)
	}
	if list := items(t, data); len(list) != 0 {
		t.Fatalf("已废弃的 new 接口应返回空列表，实际 %d 条", len(list))
	}
	if data["legacy"] != true {
		t.Fatalf("new 接口应带 legacy=true 提示调用方改用 queue，实际字段=%v", mapKeys(data))
	}
}

// TestReviewsProbesIsPoolTail probes 变成只读诊断：只给「有进度行且到期时间最远」的词。
func TestReviewsProbesIsPoolTail(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()
	never := seedWord(t, db, "no-progress")
	near := seedWord(t, db, "due-near")
	far := seedWord(t, db, "due-far")
	tNear := now.Add(time.Hour)
	tFar := now.Add(30 * 24 * time.Hour)
	seedReview(t, db, near, 1, 5, &tNear, 1, 0)
	seedReview(t, db, far, 1, 5, &tFar, 1, 0)

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/probes"), nil))
	got := cardIDs(readQueueCards(t, data))
	assertIDOrder(t, got, []uint{far, near}, "probes 池尾")
	for _, id := range got {
		if id == never {
			t.Fatalf("没有进度行的词不应出现在 probes（它没有 due_at）")
		}
	}
}

// ---------- 卡片字段 ----------

// TestReviewsCardFields 三个取数接口复用同一套卡片字段。
// ⚠️ 单一循环池之后 stability / difficulty 不再回给客户端（用户决策 E18），
// 取而代之的是 has_review / daily / bucket 三个计算列。
func TestReviewsCardFields(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWordWith(t, db, modelsWord())
	now := time.Now().Add(-time.Minute)
	seedReview(t, db, wordID, 4.5, 6.5, &now, 7, 1)

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue"), nil))
	list := items(t, data)
	if len(list) != 1 {
		t.Fatalf("queue 期望 1 张卡，实际 %d 张", len(list))
	}
	card := list[0]
	wantKeys := []string{
		"id", "word", "phonetic", "meaning", "example", "example_translation",
		"senses", "subject", "due_at", "last_review_at",
		"has_review", "daily", "bucket",
	}
	for _, key := range wantKeys {
		if _, ok := card[key]; !ok {
			t.Fatalf("卡片缺少字段 %q，实际字段=%v", key, mapKeys(card))
		}
	}
	// 用户决策 E18：引擎一律按新卡口径重算，所以这两个字段不再下发
	for _, key := range []string{"stability", "difficulty"} {
		if _, ok := card[key]; ok {
			t.Fatalf("字段 %q 不应再出现在响应里（用户决策 E18）", key)
		}
	}
	if card["word"] != "kernel" {
		t.Fatalf("卡片 word 期望 kernel，实际 %v", card["word"])
	}
	if card["has_review"] != true {
		t.Fatalf("有进度行的卡 has_review 应为 true，实际 %v", card["has_review"])
	}
	// senses 为空时必须序列化成 []（不是 null）——前端 Rust 引擎按数组解析，遇 null 会报错
	senses, ok := card["senses"].([]interface{})
	if !ok {
		t.Fatalf("senses 期望数组（空也必须是 []），实际 %T（值=%v）", card["senses"], card["senses"])
	}
	if len(senses) != 0 {
		t.Fatalf("senses 期望空数组，实际 %v", senses)
	}
}

// TestReviewsCardWithoutReviewHasNoState 没有进度行的卡不得下发状态字段，
// 否则引擎会把「没学过」当成「稳定度 0」——这正是改用指针要解决的坑。
func TestReviewsCardWithoutReviewHasNoState(t *testing.T) {
	router, db := setupTestRouter(t)
	seedWord(t, db, "virgin")

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue"), nil))
	card := items(t, data)[0]
	for _, key := range []string{"stability", "difficulty", "reps", "last_review_at"} {
		if v, ok := card[key]; ok && v != nil {
			t.Fatalf("没复习过的卡不应带 %q（实际 %v）", key, v)
		}
	}
	if card["has_review"] != false {
		t.Fatalf("没复习过的卡 has_review 应为 false，实际 %v", card["has_review"])
	}
	if card["due_at"] != nil {
		t.Fatalf("没复习过的卡 due_at 应为 null，实际 %v", card["due_at"])
	}
}

// ---------- 边界与钳制 ----------

// TestReviewsLimitClamping limit 的边界钳制：0 / 负数 / 超大值都退回默认。
func TestReviewsLimitClamping(t *testing.T) {
	cases := []struct {
		name      string
		path      string
		limitQ    string
		wantLimit int64
	}{
		{name: "queue 默认", path: "/api/reviews/queue", limitQ: "", wantLimit: 100},
		{name: "queue limit=0 退回默认", path: "/api/reviews/queue", limitQ: "0", wantLimit: 100},
		{name: "queue 负数退回默认", path: "/api/reviews/queue", limitQ: "-5", wantLimit: 100},
		{name: "queue 非数字退回默认", path: "/api/reviews/queue", limitQ: "abc", wantLimit: 100},
		{name: "queue 超上限退回默认", path: "/api/reviews/queue", limitQ: "501", wantLimit: 100},
		{name: "queue 上限内保留", path: "/api/reviews/queue", limitQ: "50", wantLimit: 50},
		{name: "queue 正好上限保留", path: "/api/reviews/queue", limitQ: "500", wantLimit: 500},
		{name: "due 默认", path: "/api/reviews/due", limitQ: "", wantLimit: 0}, // due 不回 limit 字段
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			seedWord(t, db, "yonder")

			path := tc.path
			if tc.limitQ != "" {
				path += "?limit=" + tc.limitQ
			}
			data, code := decodeData(t, doJSON(t, router, "GET", withDay(path), nil))
			if code != http.StatusOK {
				t.Fatalf("%s 的 code 期望 200，实际 %d", path, code)
			}
			if tc.wantLimit > 0 {
				assertInt(t, data, "limit", tc.wantLimit)
			}
		})
	}
}

// TestReviewsQueueOffsetNegative offset 传负数按 0 处理（不能变成 SQL 的负偏移）
func TestReviewsQueueOffsetNegative(t *testing.T) {
	router, db := setupTestRouter(t)
	first := seedWord(t, db, "zealot")
	second := seedWord(t, db, "abbey")
	now := time.Now()
	seedReview(t, db, first, 1, 5, &now, 1, 0)
	seedReview(t, db, second, 2, 5, &now, 2, 0)

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/queue?offset=-3"), nil))
	assertInt(t, data, "offset", 0)
	if n := len(cardWordIDs(t, data)); n != 2 {
		t.Fatalf("负数 offset 应按 0 处理并返回全部 2 条，实际 %d 条", n)
	}
}

// TestReviewsQueueTodayParamRejected 脏的 today 参数不能把种子带偏：
// 非法值一律回落到服务端时区算出的当天。
func TestReviewsQueueTodayParamRejected(t *testing.T) {
	router, db := setupTestRouter(t)
	seedWord(t, db, "sanity")

	for _, bad := range []string{"abc", "2026", "19990101", "99999999"} {
		data, code := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?today="+bad, nil))
		if code != http.StatusOK {
			t.Fatalf("today=%s 的 code 期望 200，实际 %d", bad, code)
		}
		assertInt(t, data, "total", 1)
	}
}

// TestReviewsDueOnlyExpired due 池只放已到期的卡，按 due_at 升序（最旧的排最前）
func TestReviewsDueOnlyExpired(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()
	oldest := seedWord(t, db, "expired-old")
	recent := seedWord(t, db, "expired-recent")
	future := seedWord(t, db, "future-one")
	never := seedWord(t, db, "never-one")

	t2 := now.Add(-3 * time.Hour)
	t1 := now.Add(-10 * time.Minute)
	tf := now.Add(time.Hour)
	seedReview(t, db, oldest, 1, 5, &t2, 1, 0)
	seedReview(t, db, recent, 1, 5, &t1, 1, 0)
	seedReview(t, db, future, 1, 5, &tf, 1, 0)

	data, code := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/due"), nil))
	if code != http.StatusOK {
		t.Fatalf("due 的 code 期望 200，实际 %d", code)
	}
	assertIDOrder(t, cardWordIDs(t, data), []uint{oldest, recent}, "due 池")
	for _, id := range cardWordIDs(t, data) {
		if id == future || id == never {
			t.Fatalf("未到期 / 没学过的词 %d 不应出现在 due 池", id)
		}
	}
	if _, ok := data["now"]; !ok {
		t.Fatalf("due 响应缺少 now 字段，实际字段=%v", mapKeys(data))
	}
}

// TestReviewsDueNowOverride due 的 now 参数可覆盖「现在」：
// 传一个更早的时刻，原本已到期的卡就不算到期了（客户端与服务端时间对齐用的口子）。
func TestReviewsDueNowOverride(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "wander")
	due := time.Now().Add(30 * time.Minute)
	seedReview(t, db, wordID, 2, 5, &due, 1, 0)

	data, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/due"), nil))
	assertIDOrder(t, cardWordIDs(t, data), nil, "未到期时的 due 池")

	futureMs := time.Now().Add(time.Hour).UnixMilli()
	data2, _ := decodeData(t, doJSON(t, router, "GET", withDay("/api/reviews/due?now="+itoa(futureMs)), nil))
	assertIDOrder(t, cardWordIDs(t, data2), []uint{wordID}, "now 覆盖后的 due 池")
	assertInt(t, data2, "now", futureMs)
}

// TestSeedReviewHelperStillWorks 防止本文件重写后误删了其它测试依赖的夹具语义。
func TestSeedReviewHelperStillWorks(t *testing.T) {
	_, db := setupTestRouter(t)
	id := seedWord(t, db, "helper")
	now := time.Now()
	seedReview(t, db, id, 2.5, 4.5, &now, 3, 1)

	var row models.WordReview
	if err := db.Where("word_id = ?", id).First(&row).Error; err != nil {
		t.Fatalf("夹具没有写入进度行：%v", err)
	}
	if row.Stability != 2.5 || row.Difficulty != 4.5 || row.Reps != 3 || row.Lapses != 1 {
		t.Fatalf("夹具写入的状态不对：%+v", row)
	}
}

var _ = gorm.ErrRecordNotFound




