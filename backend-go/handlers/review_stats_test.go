package handlers

import (
	"net/http"
	"testing"
	"time"

	"backend-go/models"
)

// 本文件覆盖 GET /api/reviews/stats 的各项口径，重点在「今日」的边界：
//   - today_new / today_reviewed / today_review 都只统计**服务器本地时区当天零点起**的日志；
//   - today_new  = 当天 stability_before = 0 的日志（首次复习的卡）
//   - today_review = 当天 stability_before > 0 的日志（抽查也走这条，因为它的 before 是库里真实旧值）
//   - streak_days 以自然日为单位向前累计，今天没复习时从昨天起算
//   - retention_rate = 非「忘记」评分占比（全量日志，不限于今天），三位小数
//   - total_* / new_words / due_cards / reviewed_words 是「当前状态」类指标，与今日无关
//
// 跨天数据全部用 seedLog 摆到「今天 / 昨天 / 前天」的真实时间点上，不 mock 系统时钟——
// 这样跨天边界比较的是真实时间，而不是被替换过的假时钟。

// TestStatsTodayBoundary 今日口径的边界：昨天与今天的日志必须被分开统计。
func TestStatsTodayBoundary(t *testing.T) {
	router, db := setupTestRouter(t)
	w1 := seedWord(t, db, "apple")
	w2 := seedWord(t, db, "balance")
	w3 := seedWord(t, db, "cabin")

	// 昨天：3 条日志（2 条新学 + 1 条复习）
	seedLog(t, db, w1, 3, 0, 2.0, 1.5, daysAgoAt(1, 10, 0), false)
	seedLog(t, db, w1, 3, 0, 2.5, 2.0, daysAgoAt(1, 11, 0), false)
	seedLog(t, db, w1, 1, 2.0, 0.5, 0, daysAgoAt(1, 12, 0), false)

	// 今天：2 条日志（1 条新学 + 1 条复习）
	seedLog(t, db, w2, 3, 0, 3.0, 3.0, daysAgoAt(0, 0, 0), false)
	seedLog(t, db, w3, 4, 4.0, 12.0, 12.0, daysAgoAt(0, 8, 30), false)

	stats := getStats(t, router)

	// today_reviewed：只数今天这 2 条
	assertInt(t, stats, "today_reviewed", 2)
	// today_new：今天只有 w2 的日志 stability_before = 0
	assertInt(t, stats, "today_new", 1)
	// today_review：今天 stability_before > 0 的只有 w3 那一条
	assertInt(t, stats, "today_review", 1)
	// 三者自洽：today_reviewed = today_new + today_review
	assertInt(t, stats, "today_reviewed",
		toInt64(t, stats["today_new"])+toInt64(t, stats["today_review"]))
	// total_reviews：跨天累计，5 条都在
	assertInt(t, stats, "total_reviews", 5)
}

// TestStatsTodayNewCountsProbesAsReview 抽查（is_probe=true）不会被算成「今日新学」：
// 它的 stability_before 取的是库里那一行的真实旧值（非 0），所以进 today_review 而不是 today_new。
// 这是 README 里明确写过的口径，也是最容易被实现改坏的一处。
func TestStatsTodayNewCountsProbesAsReview(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "drizzle")

	// 昨天正常学了一次（库里有稳定度 5.0）
	seedLog(t, db, wordID, 3, 0, 5.0, 5.0, daysAgoAt(1, 9, 0), false)
	past := daysAgoAt(1, 9, 0)
	seedReview(t, db, wordID, 5.0, 5.0, &past, 1, 0)

	// 今天走真实 HTTP 打一张抽查卡
	probeReq := newReviewRequest(wordID, 3, 1.0, 5.0, 1.0)
	probeReq.IsProbe = true
	submitReview(t, router, probeReq)

	stats := getStats(t, router)
	assertInt(t, stats, "today_reviewed", 1)
	assertInt(t, stats, "today_new", 0) // 抽查不污染「今日新学」
	assertInt(t, stats, "today_review", 1)
	assertInt(t, stats, "total_reviews", 2)

	// 日志里的标记与 stability_before 都要对得上
	logs := logsOf(t, db, wordID)
	if !logs[1].IsProbe {
		t.Fatalf("第二条日志的 is_probe 期望 true")
	}
	if logs[1].StabilityBefore != 5.0 {
		t.Fatalf("抽查日志的 stability_before 期望库里的旧值 5.0，实际 %v", logs[1].StabilityBefore)
	}
}

// TestStatsTodayCountsHTTPSubmits 今日口径与真实提交链路打通：
// 通过 /api/reviews/submit 提交的日志会立刻计入今日统计（reviewed_at 就是提交时刻），
// 并且「今日新学」与「今日复习」按**提交那一刻库里的真实旧值**区分：
//   - first 昨天已经学过（库里有稳定性 1.5 的记忆状态），今天的提交 stability_before=1.5 → 计入今日复习；
//   - second 是全新的卡（库里没有记忆状态），stability_before=0 → 计入今日新学。
//
// 这条同时钉住了 SubmitReview 里那句「stability_before 取库里的真实旧值，而不是引擎传来的新状态」：
// 若哪天改成用请求里的 stability，两次提交都会被算成今日新学，today_new 会变成 2。
func TestStatsTodayCountsHTTPSubmits(t *testing.T) {
	router, db := setupTestRouter(t)
	first := seedWord(t, db, "eager")
	second := seedWord(t, db, "fabric")

	// 昨天留下的历史数据：一条旧日志 + first 已有的记忆状态（故意不新建卡）
	seedLog(t, db, first, 3, 0, 1.0, 1.0, daysAgoAt(1, 20, 0), false)
	yesterday := daysAgoAt(1, 20, 0)
	seedReview(t, db, first, 1.5, 5, &yesterday, 1, 0)

	// 今天两次提交：first 是复习（库里已有 1.5），second 是新学（库里没有记录）
	submitReview(t, router, newReviewRequest(first, 3, 2.0, 5.0, 2))
	submitReview(t, router, newReviewRequest(second, 3, 4.0, 5.0, 4))

	stats := getStats(t, router)
	assertInt(t, stats, "today_reviewed", 2)
	assertInt(t, stats, "today_new", 1) // 只有 second 的首次复习是「新学」
	assertInt(t, stats, "today_review", 1)
	assertInt(t, stats, "total_reviews", 3) // 含昨天那条

	// 落库侧核对：first 的第二次日志必须记着库里的旧值 1.5，而不是请求里的 2.0
	logs := logsOf(t, db, first)
	if len(logs) != 2 {
		t.Fatalf("first 的日志期望 2 条（昨天 + 今天），实际 %d 条", len(logs))
	}
	if logs[1].StabilityBefore != 1.5 {
		t.Fatalf("今天这条日志的 stability_before 期望库里的旧值 1.5，实际 %v", logs[1].StabilityBefore)
	}
	if logs[1].StabilityAfter != 2.0 {
		t.Fatalf("今天这条日志的 stability_after 期望本次提交的 2.0，实际 %v", logs[1].StabilityAfter)
	}
}

// TestStatsStreakDays 连续复习天数的四种局面：
// 今天+昨天+前天 → 3；今天没复习但昨天有 → 从昨天起算；中间断一天 → 只数到断点；完全没记录 → 0。
func TestStatsStreakDays(t *testing.T) {
	cases := []struct {
		name     string
		daysAgo  []int
		wantDays int64
	}{
		{name: "无任何记录", daysAgo: nil, wantDays: 0},
		{name: "只有今天", daysAgo: []int{0}, wantDays: 1},
		{name: "今天与昨天", daysAgo: []int{0, 1}, wantDays: 2},
		{name: "今天昨天前天", daysAgo: []int{0, 1, 2}, wantDays: 3},
		{name: "今天没学从昨天起算", daysAgo: []int{1, 2}, wantDays: 2},
		{name: "今天没学昨天也没学", daysAgo: []int{2, 3}, wantDays: 0},
		{name: "中间断一天只数到断点", daysAgo: []int{0, 1, 3, 4}, wantDays: 2},
		{name: "只有前天", daysAgo: []int{2}, wantDays: 0},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "graceful")

			for i, d := range tc.daysAgo {
				// 同一天多条也无所谓（按自然日去重），这里每天只放一条，时刻错开避免时间戳完全相同
				seedLog(t, db, wordID, 3, 0, float64(i+1), float64(i+1), daysAgoAt(d, 9, i%60), false)
			}

			stats := getStats(t, router)
			assertInt(t, stats, "streak_days", tc.wantDays)
		})
	}
}

// TestStatsRetentionRate 记忆保持率 = 非 Again 评分占比，保留三位小数；无日志时为 0。
func TestStatsRetentionRate(t *testing.T) {
	cases := []struct {
		name         string
		ratings      []uint8
		wantRetained float64
	}{
		{name: "无日志", ratings: nil, wantRetained: 0},
		{name: "全部忘记", ratings: []uint8{1, 1}, wantRetained: 0},
		{name: "全部记得", ratings: []uint8{3, 2, 4}, wantRetained: 1},
		{name: "三分之一忘记", ratings: []uint8{1, 3, 4}, wantRetained: 0.667},
		{name: "三分之二忘记", ratings: []uint8{1, 1, 4}, wantRetained: 0.333},
		{name: "一半忘记", ratings: []uint8{1, 3}, wantRetained: 0.5},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "humble")

			// 刻意混入「昨天」的记录：保持率是全量口径，不能被今日边界截断
			for i, rating := range tc.ratings {
				seedLog(t, db, wordID, rating, 0, 1.0, 1.0, daysAgoAt(i%2, 10, i), false)
			}

			stats := getStats(t, router)
			assertFloat(t, stats, "retention_rate", tc.wantRetained)
			assertInt(t, stats, "total_reviews", int64(len(tc.ratings)))
		})
	}
}

// TestStatsRetentionRateCountsAllLogs 保持率按全量日志算（不只今天），
// 且「忘记」只认 rating=1（Hard=2 不算忘记）。
func TestStatsRetentionRateCountsAllLogs(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "intricate")

	seedLog(t, db, wordID, 1, 0, 0.5, 0, daysAgoAt(3, 8, 0), false) // 前天以前的忘记
	seedLog(t, db, wordID, 2, 0.5, 1.0, 1, daysAgoAt(1, 8, 0), false)
	seedLog(t, db, wordID, 3, 1.0, 2.0, 2, daysAgoAt(0, 8, 0), false)

	stats := getStats(t, router)
	// 3 条日志里 1 条 Again → (3-1)/3 = 0.667
	assertFloat(t, stats, "retention_rate", 0.667)
	assertInt(t, stats, "total_reviews", 3)
}

// TestStatsWordCounters 当前状态类指标：
// total_words / new_words（没有复习行的词）/ reviewed_words（有复习行的词，不论是否到期）/ due_cards（到期数）。
func TestStatsWordCounters(t *testing.T) {
	router, db := setupTestRouter(t)
	learnedDue := seedWord(t, db, "jolly")   // 已学且已到期
	learnedFuture := seedWord(t, db, "keen") // 已学但未到期
	learnedNoDue := seedWord(t, db, "lucid") // 已学但 due_at 为空（脏数据）
	_ = seedWord(t, db, "mellow")            // 完全没学
	_ = seedWord(t, db, "nimble")            // 完全没学

	past := time.Now().Add(-2 * time.Hour)
	future := time.Now().Add(48 * time.Hour)
	seedReview(t, db, learnedDue, 3, 5, &past, 2, 0)
	seedReview(t, db, learnedFuture, 6, 5, &future, 3, 0)
	// due_at 为空的行：生产里不会出现（提交时必写），用来确认「到期数」不会把它算进去
	seedReview(t, db, learnedNoDue, 1, 5, nil, 1, 0)

	stats := getStats(t, router)
	assertInt(t, stats, "total_words", 5)
	assertInt(t, stats, "new_words", 2)
	assertInt(t, stats, "reviewed_words", 3) // 含 due_at 为空的那张
	assertInt(t, stats, "due_cards", 1)      // 只有已到期的那张
	assertInt(t, stats, "total_reviews", 0)
	assertFloat(t, stats, "retention_rate", 0)
	assertInt(t, stats, "streak_days", 0)
}

// TestStatsDueCardsBoundary due_cards 的边界是「due_at <= now」：
// 恰好过去的算到期，恰好未来的不算。用 ±1 小时把边界夹住，避免依赖毫秒级临界。
func TestStatsDueCardsBoundary(t *testing.T) {
	cases := []struct {
		name       string
		offset     time.Duration
		wantDueNow int64
	}{
		{name: "刚过去一小时算到期", offset: -time.Hour, wantDueNow: 1},
		{name: "还有一小时不算到期", offset: time.Hour, wantDueNow: 0},
		{name: "刚好一分钟前算到期", offset: -time.Minute, wantDueNow: 1},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "opaque")
			due := time.Now().Add(tc.offset)
			seedReview(t, db, wordID, 2, 5, &due, 1, 0)

			stats := getStats(t, router)
			assertInt(t, stats, "due_cards", tc.wantDueNow)
			assertInt(t, stats, "reviewed_words", 1)
			assertInt(t, stats, "new_words", 0)
			assertInt(t, stats, "total_words", 1)
		})
	}
}

// TestStatsEmptyDB 空库必须返回一套完整的零值，而不是缺字段或报错
// （前端会用 total_words 做除法 / 展示，缺字段会直接崩）。
func TestStatsEmptyDB(t *testing.T) {
	router, _ := setupTestRouter(t)

	stats := getStats(t, router)
	wantKeys := []string{
		"total_words", "new_words", "due_cards", "reviewed_words",
		"total_reviews", "today_reviewed", "today_new", "today_review",
		"streak_days", "retention_rate",
	}
	for _, key := range wantKeys {
		if _, ok := stats[key]; !ok {
			t.Fatalf("空库的 stats 缺少字段 %q，实际字段=%v", key, mapKeys(stats))
		}
	}
	for _, key := range []string{"total_words", "new_words", "due_cards", "reviewed_words", "total_reviews", "today_reviewed", "today_new", "today_review", "streak_days"} {
		assertInt(t, stats, key, 0)
	}
	assertFloat(t, stats, "retention_rate", 0)
}

// TestStatsAfterDeleteWord 删除词条后统计口径不虚高：
// DELETE /api/words/:id 会连带删掉该词的复习状态与日志（README 明确写过），
// 否则 total_reviews 与 retention_rate 会被指向不存在词条的脏日志撑高。
func TestStatsAfterDeleteWord(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "placid")

	// 先制造 1 条 Again 日志，让保持率低于 1
	submitReview(t, router, newReviewRequest(wordID, 1, 0.4, 9, 0))
	before := getStats(t, router)
	assertInt(t, before, "total_reviews", 1)
	assertFloat(t, before, "retention_rate", 0)

	resp := doJSON(t, router, http.MethodDelete, "/api/words/1", nil)
	if resp.Status != http.StatusOK {
		t.Fatalf("删除词条期望 HTTP 200，实际 %d，响应体=%s", resp.Status, string(resp.Body))
	}

	after := getStats(t, router)
	assertInt(t, after, "total_reviews", 0)
	assertInt(t, after, "total_words", 0)
	assertInt(t, after, "reviewed_words", 0)
	assertInt(t, after, "new_words", 0)
	// 删掉唯一一条 Again 日志后，保持率回到「无数据」的 0（不是 1）
	assertFloat(t, after, "retention_rate", 0)

	if n := countRows(t, db, &models.ReviewLog{}); n != 0 {
		t.Fatalf("删除词条后 review_logs 期望 0 行，实际 %d 行", n)
	}
	if n := countRows(t, db, &models.WordReview{}); n != 0 {
		t.Fatalf("删除词条后 word_reviews 期望 0 行，实际 %d 行", n)
	}
}
