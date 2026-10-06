// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
package handlers

import (
	"encoding/json"
	"math"
	"net/http"
	"testing"
	"time"

	"gorm.io/gorm"

	"backend-go/models"
)

// 本文件覆盖 POST /api/reviews/submit 的三块口径：
//  1. 到期计算：due_at = 提交时刻 + max(interval_days*86400, 600) 秒（下限 600 秒与引擎侧对齐）；
//  2. 状态落库：stability / difficulty / desired_retention / reps / lapses / due_at / last_review_at；
//  3. 日志落库：stability_before（取库里的真实旧值）/ stability_after / interval_days / is_probe。
//
// 时间断言的做法：接口内部用 time.Now()，测试不去 mock 时钟，而是在提交前后各取一次
// 真实时间（before / after）夹住预期值；再借「日志 reviewed_at 与 due_at 同源于同一个 now」
// 这一点，用日志回溯出的提交时刻反推 due_at，从而把 600 秒下限、秒级换算这些口径钉死。

// submitWindow 一次提交前后的真实时间窗
type submitWindow struct {
	before time.Time
	after  time.Time
}

// newSubmitWindow 记录提交前的时刻；配合 finish 使用
func newSubmitWindow() *submitWindow {
	return &submitWindow{before: time.Now()}
}

// finish 记录提交后的时刻并返回时间窗
func (w *submitWindow) finish() submitWindow {
	w.after = time.Now()
	return *w
}

// assertDueAtWindow 断言绝对时刻落在「提交时刻 + 预期间隔」的 ±tolerance 秒窗口内
func assertDueAtWindow(t *testing.T, got, want time.Time, tolerance time.Duration, label string) {
	t.Helper()
	diff := got.Sub(want)
	if diff < 0 {
		diff = -diff
	}
	if diff > tolerance {
		t.Fatalf("%s 偏差过大：实际 %s，预期约 %s（相差 %s，容差 %s）",
			label, got.Format(time.RFC3339Nano), want.Format(time.RFC3339Nano), diff, tolerance)
	}
}

// windowFor 由「提交前后的时间窗 + 间隔秒数」算出预期到期时刻的允许范围
func windowFor(w submitWindow, dueSeconds float64) (earliest, latest time.Time) {
	d := durationFromSeconds(dueSeconds)
	return w.before.Add(d), w.after.Add(d)
}

// assertDueAtInWindow 断言到期时刻落在 [提交前+间隔, 提交后+间隔] 内
func assertDueAtInWindow(t *testing.T, got time.Time, w submitWindow, dueSeconds float64, label string) {
	t.Helper()
	earliest, latest := windowFor(w, dueSeconds)
	if got.Before(earliest) || got.After(latest) {
		t.Fatalf("%s 不在预期区间：实际 %s，预期落在 [%s, %s]（间隔 %.0f 秒）",
			label, got.Format(time.RFC3339Nano),
			earliest.Format(time.RFC3339Nano), latest.Format(time.RFC3339Nano), dueSeconds)
	}
}

// durationFromSeconds 复刻生产里 float64 秒 → time.Duration 的换算，避免测试自己重算时口径不一致
func durationFromSeconds(seconds float64) time.Duration {
	return time.Duration(seconds * float64(time.Second))
}

// assertDueAtMatchesLog 用日志里的 reviewed_at 反推到期时刻。
// 生产代码里 due_at 与日志的 reviewed_at 取自**同一个 time.Now()**（`now := time.Now()`），
// 所以「落库 due_at - 日志 reviewed_at」应当精确等于间隔秒数；留 1 秒容差是防纳秒截断与
// 驱动写入时的精度差异（SQLite 时间列的存储精度）。
func assertDueAtMatchesLog(t *testing.T, db *gorm.DB, wordID uint, wantIntervalDays float64) time.Time {
	t.Helper()
	logs := logsOf(t, db, wordID)
	if len(logs) == 0 {
		t.Fatalf("word_id=%d 没有任何复习日志", wordID)
	}
	last := logs[len(logs)-1]
	row := reviewRowOf(t, db, wordID)
	if row.DueAt == nil {
		t.Fatalf("word_id=%d 的 due_at 落库为 NULL", wordID)
	}
	wantSeconds := math.Max(wantIntervalDays*86400, 600)
	want := last.ReviewedAt.Add(durationFromSeconds(wantSeconds))
	assertDueAtWindow(t, *row.DueAt, want, time.Second, "落库 due_at 相对日志 reviewed_at")

	if row.LastReviewAt == nil {
		t.Fatalf("word_id=%d 的 last_review_at 落库为 NULL", wordID)
	}
	assertDueAtWindow(t, *row.LastReviewAt, last.ReviewedAt, time.Second, "落库 last_review_at 相对日志 reviewed_at")
	return *row.DueAt
}

// newReviewRequest 造一个合法的提交请求
func newReviewRequest(wordID uint, rating uint8, stability, difficulty, intervalDays float64) submitReviewRequest {
	return submitReviewRequest{
		WordID:        wordID,
		Rating:        rating,
		Stability:     stability,
		Difficulty:    difficulty,
		IntervalDays:  intervalDays,
		DesiredRetain: 0.9,
	}
}

// ---------- rating 四个分支 ----------

// TestSubmitReviewRatingBranches 四个评分分支都要落到正确字段：
// Again(1) 累加 lapses 并记一条遗忘日志，Hard/Good/Easy 只更新状态；
// 后端不重算 stability——引擎给什么就存什么（这是接口契约，不是实现细节）。
func TestSubmitReviewRatingBranches(t *testing.T) {
	cases := []struct {
		name              string
		rating            uint8
		stability         float64
		difficulty        float64
		intervalDays      float64
		wantLapses        uint
		wantIntervalInLog float64
	}{
		{name: "Again", rating: 1, stability: 0.4, difficulty: 8.2, intervalDays: 0, wantLapses: 1, wantIntervalInLog: 0},
		{name: "Hard", rating: 2, stability: 2.5, difficulty: 7.1, intervalDays: 2.5, wantLapses: 0, wantIntervalInLog: 2.5},
		{name: "Good", rating: 3, stability: 6.5, difficulty: 5.4, intervalDays: 6.5, wantLapses: 0, wantIntervalInLog: 6.5},
		{name: "Easy", rating: 4, stability: 15.25, difficulty: 3.3, intervalDays: 15.25, wantLapses: 0, wantIntervalInLog: 15.25},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "apple")

			win := newSubmitWindow()
			data := submitReview(t, router, newReviewRequest(wordID, tc.rating, tc.stability, tc.difficulty, tc.intervalDays))
			window := win.finish()

			// 响应回执：word_id / due_at（毫秒）/ reps
			assertInt(t, data, "word_id", int64(wordID))
			assertInt(t, data, "reps", 1)
			dueAtMs := toInt64(t, data["due_at"])

			// 状态落库
			row := reviewRowOf(t, db, wordID)
			assertFloat(t, map[string]interface{}{"stability": row.Stability}, "stability", tc.stability)
			assertFloat(t, map[string]interface{}{"difficulty": row.Difficulty}, "difficulty", tc.difficulty)
			if row.Reps != 1 {
				t.Fatalf("reps 期望 1，实际 %d", row.Reps)
			}
			if row.Lapses != tc.wantLapses {
				t.Fatalf("rating=%d 时 lapses 期望 %d，实际 %d", tc.rating, tc.wantLapses, row.Lapses)
			}
			if row.DueAt == nil {
				t.Fatalf("due_at 落库为 NULL")
			}
			if dueAtMs != row.DueAt.UnixMilli() {
				t.Fatalf("响应 due_at(%d) 与落库 due_at(%d) 不一致", dueAtMs, row.DueAt.UnixMilli())
			}

			// 到期计算：间隔下限 600 秒，其余按 interval_days*86400
			wantSeconds := math.Max(tc.intervalDays*86400, 600)
			assertDueAtInWindow(t, *row.DueAt, window, wantSeconds, "due_at")
			assertDueAtMatchesLog(t, db, wordID, tc.intervalDays)

			// 日志落库
			logs := logsOf(t, db, wordID)
			if len(logs) != 1 {
				t.Fatalf("复习日志期望 1 条，实际 %d 条", len(logs))
			}
			got := logs[0]
			if got.WordID != wordID {
				t.Fatalf("日志 word_id 期望 %d，实际 %d", wordID, got.WordID)
			}
			if got.Rating != tc.rating {
				t.Fatalf("日志 rating 期望 %d，实际 %d", tc.rating, got.Rating)
			}
			if got.StabilityBefore != 0 {
				t.Fatalf("首次复习的 stability_before 期望 0，实际 %v", got.StabilityBefore)
			}
			if got.StabilityAfter != tc.stability {
				t.Fatalf("日志 stability_after 期望 %v，实际 %v", tc.stability, got.StabilityAfter)
			}
			if got.DifficultyAfter != tc.difficulty {
				t.Fatalf("日志 difficulty_after 期望 %v，实际 %v", tc.difficulty, got.DifficultyAfter)
			}
			if got.IntervalDays != tc.wantIntervalInLog {
				t.Fatalf("日志 interval_days 期望 %v，实际 %v", tc.wantIntervalInLog, got.IntervalDays)
			}
			if got.IsProbe {
				t.Fatalf("未传 is_probe 时日志的 is_probe 期望 false，实际 true")
			}
		})
	}
}

// TestSubmitReviewAgainWithZeroIntervalFloor 单独钉住 Again 分支的到期下限：
// 引擎给 interval_days=0 时 due_at 必须是 now+600 秒，而不是「立刻到期」。
// 这条下限必须与引擎侧 session.rs 的 due_ms = now + max(interval_days*86400000, 600000) 一致，
// 否则卡片插回池子后的排序位置会和服务端实际的 due_at 对不上。
func TestSubmitReviewAgainWithZeroIntervalFloor(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "abandon")

	win := newSubmitWindow()
	data := submitReview(t, router, newReviewRequest(wordID, 1, 0.3, 9, 0))
	window := win.finish()

	const floorSeconds = 600
	row := reviewRowOf(t, db, wordID)
	assertDueAtInWindow(t, *row.DueAt, window, floorSeconds, "Again 的 due_at")

	// 与 600 秒下限的偏差应当很小（远小于 1 秒以上的漂移）
	dueAtMs := toInt64(t, data["due_at"])
	assertDueAtWindow(t, time.UnixMilli(dueAtMs), window.before.Add(floorSeconds*time.Second),
		10*time.Second, "响应 due_at 相对提交前时刻")
}

// TestSubmitReviewDueIntervalScale 到期换算的时间尺度：
// 分钟级（0.5 天）与「毫秒精度会不会被 int 截断」的边界都要落在窗口内。
// 0.25 天 = 21600 秒用于确认换算走的是 float64 秒而不是被取整成天。
func TestSubmitReviewDueIntervalScale(t *testing.T) {
	cases := []struct {
		name         string
		intervalDays float64
		wantSeconds  float64
	}{
		{name: "半小时", intervalDays: 0.5, wantSeconds: 43200},
		{name: "四分之一天", intervalDays: 0.25, wantSeconds: 21600},
		{name: "一天", intervalDays: 1, wantSeconds: 86400},
		{name: "十天", intervalDays: 10, wantSeconds: 864000},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "benefit")

			win := newSubmitWindow()
			submitReview(t, router, newReviewRequest(wordID, 3, 5, 5, tc.intervalDays))
			window := win.finish()

			row := reviewRowOf(t, db, wordID)
			assertDueAtInWindow(t, *row.DueAt, window, tc.wantSeconds, "due_at")
			assertDueAtMatchesLog(t, db, wordID, tc.intervalDays)
		})
	}
}

// TestSubmitReviewFirstTimeCreatesState 首次复习（库里没有 word_reviews 行）要新建一行，
// 而不是走到「更新已有行」的分支——CLAUDE.md 红线 7「word_reviews 行懒创建」的落点。
func TestSubmitReviewFirstTimeCreatesState(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "capable")

	data := submitReview(t, router, newReviewRequest(wordID, 3, 4.2, 5.5, 4.2))
	assertInt(t, data, "reps", 1)

	if n := countRows(t, db, &models.WordReview{}); n != 1 {
		t.Fatalf("首次提交后 word_reviews 期望 1 行，实际 %d 行", n)
	}
	row := reviewRowOf(t, db, wordID)
	if row.ID == 0 || row.WordID != wordID {
		t.Fatalf("新建的复习行字段不对: %+v", row)
	}
	if row.Lapses != 0 {
		t.Fatalf("首次 Good 评分 lapses 期望 0，实际 %d", row.Lapses)
	}
	// desired_retention 传了 0.9，应当原样落库
	if row.DesiredRetention != 0.9 {
		t.Fatalf("desired_retention 期望 0.9，实际 %v", row.DesiredRetention)
	}
	// 首次复习没有旧稳定度，日志里 stability_before 必须是 0（stats 的「今日新学」据此判定）
	logs := logsOf(t, db, wordID)
	if logs[0].StabilityBefore != 0 {
		t.Fatalf("首次复习 stability_before 期望 0，实际 %v", logs[0].StabilityBefore)
	}
}

// TestSubmitReviewUpdatesExistingState 已有复习记录的第二次提交：
// reps 递增、状态被新值覆盖、日志的 stability_before 取的是**库里的真实旧值**而不是引擎输入。
func TestSubmitReviewUpdatesExistingState(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "diligent")

	// 先造一行历史状态：稳定度 3.0（这就是第二次提交时日志该记下的 stability_before）
	past := time.Now().Add(-48 * time.Hour)
	seedReview(t, db, wordID, 3.0, 6.0, &past, 2, 0)

	data := submitReview(t, router, newReviewRequest(wordID, 4, 12.5, 3.1, 12.5))
	assertInt(t, data, "reps", 3) // 历史 2 次 + 本次 1 次

	row := reviewRowOf(t, db, wordID)
	if row.Stability != 12.5 || row.Difficulty != 3.1 {
		t.Fatalf("状态未被新值覆盖: stability=%v difficulty=%v", row.Stability, row.Difficulty)
	}
	if row.Reps != 3 {
		t.Fatalf("reps 期望 3，实际 %d", row.Reps)
	}
	if row.Lapses != 0 {
		t.Fatalf("非 Again 评分不应累加 lapses，实际 %d", row.Lapses)
	}
	if n := countRows(t, db, &models.WordReview{}); n != 1 {
		t.Fatalf("第二次提交不应新建行，word_reviews 期望 1 行，实际 %d 行", n)
	}

	logs := logsOf(t, db, wordID)
	if len(logs) != 1 {
		t.Fatalf("本次提交的日志期望 1 条，实际 %d 条", len(logs))
	}
	if logs[0].StabilityBefore != 3.0 {
		t.Fatalf("stability_before 期望取库里的旧值 3.0，实际 %v", logs[0].StabilityBefore)
	}
	if logs[0].StabilityAfter != 12.5 {
		t.Fatalf("stability_after 期望 12.5，实际 %v", logs[0].StabilityAfter)
	}
	// 旧值非 0 → 这条日志属于「今日复习」而不是「今日新学」
	if logs[0].StabilityBefore == 0 {
		t.Fatalf("已有记录的复习不应产生 stability_before=0 的日志")
	}
}

// TestSubmitReviewRepeatedSameDay 同一天重复提交：
// 每次提交都是一次独立复习（reps 递增、各写一条日志），不会因为「同一天」被合并或丢弃。
func TestSubmitReviewRepeatedSameDay(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "elaborate")

	first := submitReview(t, router, newReviewRequest(wordID, 3, 2.0, 6.0, 2))
	assertInt(t, first, "reps", 1)

	second := submitReview(t, router, newReviewRequest(wordID, 2, 3.5, 6.4, 3.5))
	assertInt(t, second, "reps", 2)

	third := submitReview(t, router, newReviewRequest(wordID, 3, 5.0, 6.0, 5))
	assertInt(t, third, "reps", 3)

	logs := logsOf(t, db, wordID)
	if len(logs) != 3 {
		t.Fatalf("同一天 3 次提交应产生 3 条日志，实际 %d 条", len(logs))
	}
	// 三条日志的 stability_before 依次是上一次的 after：0 → 2.0 → 3.5
	wantBefores := []float64{0, 2.0, 3.5}
	wantAfters := []float64{2.0, 3.5, 5.0}
	for i, lg := range logs {
		if lg.StabilityBefore != wantBefores[i] {
			t.Fatalf("第 %d 条日志 stability_before 期望 %v，实际 %v", i+1, wantBefores[i], lg.StabilityBefore)
		}
		if lg.StabilityAfter != wantAfters[i] {
			t.Fatalf("第 %d 条日志 stability_after 期望 %v，实际 %v", i+1, wantAfters[i], lg.StabilityAfter)
		}
	}

	// 同一天的三条日志都会被统计进「今日已复习」
	stats := getStats(t, router)
	assertInt(t, stats, "today_reviewed", 3)
}

// TestSubmitReviewProbeFlag 抽查卡的 is_probe 标记：
// 显式传 true 记 true，不传记 false；抽查同样不改变 stability_before 的取值口径。
func TestSubmitReviewProbeFlag(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "fragile")

	// 先正常学一次，让库里有一个非 0 的稳定度
	submitReview(t, router, newReviewRequest(wordID, 3, 8.0, 5.0, 8))

	// 再提交一次抽查：引擎按新卡重算（稳定度明显变小），但日志的 stability_before 仍是库里的旧值
	probeReq := newReviewRequest(wordID, 3, 1.2, 5.0, 1.2)
	probeReq.IsProbe = true
	submitReview(t, router, probeReq)

	logs := logsOf(t, db, wordID)
	if len(logs) != 2 {
		t.Fatalf("期望 2 条日志，实际 %d 条", len(logs))
	}
	if logs[0].IsProbe {
		t.Fatalf("第一次普通复习的 is_probe 期望 false，实际 true")
	}
	if !logs[1].IsProbe {
		t.Fatalf("抽查提交的 is_probe 期望 true，实际 false")
	}
	// 抽查按新卡重算 → stability_after 明显小于 before，这正是「不能被算成今日新学」的原因
	if logs[1].StabilityAfter >= logs[0].StabilityAfter {
		t.Fatalf("抽查的 stability_after(%v) 应低于上一次的 after(%v)",
			logs[1].StabilityAfter, logs[0].StabilityAfter)
	}
	if logs[1].StabilityAfter != 1.2 {
		t.Fatalf("抽查日志的 stability_after 期望 1.2，实际 %v", logs[1].StabilityAfter)
	}
	if logs[1].StabilityBefore != 8.0 {
		t.Fatalf("抽查日志的 stability_before 应取库里的旧值 8.0，实际 %v", logs[1].StabilityBefore)
	}
}

// TestSubmitReviewDesiredRetentionDefault desired_retention 的兜底口径：
// 缺失 / 非正 / >= 1 都落成 0.9；只有 (0,1) 区间内的值才原样保存。
// 这条决定了 FSRS 参数优化将来能不能拿到真实的期望保持率。
func TestSubmitReviewDesiredRetentionDefault(t *testing.T) {
	cases := []struct {
		name string
		send float64
		want float64
	}{
		{name: "未传用默认", send: 0, want: 0.9},
		{name: "负数用默认", send: -0.5, want: 0.9},
		{name: "等于1用默认", send: 1, want: 0.9},
		{name: "大于1用默认", send: 1.5, want: 0.9},
		{name: "区间内原样保存", send: 0.85, want: 0.85},
		{name: "极小值原样保存", send: 0.001, want: 0.001},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "genuine")

			req := newReviewRequest(wordID, 3, 2, 5, 2)
			req.DesiredRetain = tc.send
			submitReview(t, router, req)

			row := reviewRowOf(t, db, wordID)
			if row.DesiredRetention != tc.want {
				t.Fatalf("desired_retention 期望 %v，实际 %v", tc.want, row.DesiredRetention)
			}
		})
	}
}

// TestSubmitReviewInvalidPayload 请求体与参数校验：
// 不合法时必须 400 且**一行都不落库**（不能出现「状态写了、日志没写」的半截数据）。
func TestSubmitReviewInvalidPayload(t *testing.T) {
	cases := []struct {
		name string
		body []byte
	}{
		{name: "word_id 为 0", body: []byte(`{"word_id":0,"rating":3,"stability":1,"difficulty":5,"interval_days":1}`)},
		{name: "word_id 缺失", body: []byte(`{"rating":3,"stability":1,"difficulty":5,"interval_days":1}`)},
		{name: "rating 为 0", body: []byte(`{"word_id":1,"rating":0,"stability":1,"difficulty":5,"interval_days":1}`)},
		{name: "rating 为 5", body: []byte(`{"word_id":1,"rating":5,"stability":1,"difficulty":5,"interval_days":1}`)},
		{name: "stability 为负", body: []byte(`{"word_id":1,"rating":3,"stability":-1,"difficulty":5,"interval_days":1}`)},
		{name: "difficulty 为负", body: []byte(`{"word_id":1,"rating":3,"stability":1,"difficulty":-5,"interval_days":1}`)},
		{name: "interval_days 为负", body: []byte(`{"word_id":1,"rating":3,"stability":1,"difficulty":5,"interval_days":-1}`)},
		{name: "interval_days 超上限", body: []byte(`{"word_id":1,"rating":3,"stability":1,"difficulty":5,"interval_days":3650}`)},
		{name: "请求体不是 JSON", body: []byte(`not-json`)},
		{name: "请求体为空", body: []byte(``)},
		{name: "对象字段类型不对", body: []byte(`{"word_id":"abc","rating":3,"stability":1,"difficulty":5,"interval_days":1}`)},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			wordID := seedWord(t, db, "hesitate")
			// 校验发生在查词之前，所以请求里的 word_id 用真实 id 也不会落库
			body := tc.body
			if wordID != 1 {
				t.Fatalf("本用例假定第一个词条的 id 为 1，实际 %d", wordID)
			}

			resp := doJSON(t, router, http.MethodPost, "/api/reviews/submit", body)
			if resp.Status != http.StatusBadRequest {
				t.Fatalf("期望 HTTP 400，实际 %d，响应体=%s", resp.Status, string(resp.Body))
			}
			var envelope struct {
				Code    int    `json:"code"`
				Message string `json:"message"`
			}
			if err := json.Unmarshal(resp.Body, &envelope); err != nil {
				t.Fatalf("错误响应不是合法 JSON: %v，原文=%s", err, string(resp.Body))
			}
			if envelope.Code != http.StatusBadRequest {
				t.Fatalf("错误响应的 code 期望 400，实际 %d", envelope.Code)
			}

			if n := countRows(t, db, &models.WordReview{}); n != 0 {
				t.Fatalf("校验失败时不应写入 word_reviews，实际 %d 行", n)
			}
			if n := countRows(t, db, &models.ReviewLog{}); n != 0 {
				t.Fatalf("校验失败时不应写入 review_logs，实际 %d 行", n)
			}
		})
	}
}

// TestSubmitReviewRejectsMalformedLiterals 数值校验里的 NaN / Inf 分支：
// 这几个值没法用标准 JSON 表示（编码器会直接报错），所以用裸字符串构造请求体。
// 它们一旦落库会污染 due_at 计算与统计，必须在解码或校验阶段被 400 拦住。
func TestSubmitReviewRejectsMalformedLiterals(t *testing.T) {
	cases := []struct {
		name string
		body string
	}{
		{name: "stability 为字面量 NaN", body: `{"word_id":1,"rating":3,"stability":NaN,"difficulty":5,"interval_days":1}`},
		{name: "stability 为字面量 Inf", body: `{"word_id":1,"rating":3,"stability":Infinity,"difficulty":5,"interval_days":1}`},
		{name: "difficulty 为字面量 NaN", body: `{"word_id":1,"rating":3,"stability":1,"difficulty":NaN,"interval_days":1}`},
		{name: "interval_days 为字面量 Inf", body: `{"word_id":1,"rating":3,"stability":1,"difficulty":5,"interval_days":Infinity}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			seedWord(t, db, "immune")

			resp := doJSON(t, router, http.MethodPost, "/api/reviews/submit", []byte(tc.body))
			// 非法字面量在 JSON 解码阶段就失败 → 400；若解码器容错通过，也必须被数值校验拦住
			if resp.Status != http.StatusBadRequest {
				t.Fatalf("期望 HTTP 400，实际 %d，响应体=%s", resp.Status, string(resp.Body))
			}
			if n := countRows(t, db, &models.WordReview{}); n != 0 {
				t.Fatalf("非法数值不应落库，word_reviews 实际 %d 行", n)
			}
		})
	}
}

// TestSubmitReviewWordNotFound 词条不存在时 404，且不留下任何数据
func TestSubmitReviewWordNotFound(t *testing.T) {
	router, db := setupTestRouter(t)
	seedWord(t, db, "journey")

	resp := doJSON(t, router, http.MethodPost, "/api/reviews/submit",
		jsonBody(t, newReviewRequest(9999, 3, 1, 5, 1)))
	if resp.Status != http.StatusNotFound {
		t.Fatalf("期望 HTTP 404，实际 %d，响应体=%s", resp.Status, string(resp.Body))
	}
	var envelope struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	}
	if err := json.Unmarshal(resp.Body, &envelope); err != nil {
		t.Fatalf("错误响应不是合法 JSON: %v", err)
	}
	if envelope.Code != http.StatusNotFound {
		t.Fatalf("错误响应 code 期望 404，实际 %d", envelope.Code)
	}
	if n := countRows(t, db, &models.WordReview{}); n != 0 {
		t.Fatalf("词条不存在时不应写入 word_reviews，实际 %d 行", n)
	}
	if n := countRows(t, db, &models.ReviewLog{}); n != 0 {
		t.Fatalf("词条不存在时不应写入 review_logs，实际 %d 行", n)
	}
}

// TestSubmitReviewContentTypeNotJSON 记录 gin 的真实口径：ShouldBindJSON **不检查 Content-Type**，
// 只要求请求体是合法 JSON。所以 text/plain 带着合法 JSON 体时照样 200 并落库。
// 这不是缺陷（内网后台接口），但测试必须把它钉住：将来若加了 Content-Type 校验，
// 这条会立刻变红，提醒同步前端调用方。
func TestSubmitReviewContentTypeNotJSON(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "kindle")

	resp := doJSONWithHeader(t, router, http.MethodPost, "/api/reviews/submit",
		[]byte(`{"word_id":1,"rating":3,"stability":1,"difficulty":5,"interval_days":1}`), "text/plain")
	if resp.Status != http.StatusOK {
		t.Fatalf("gin 的 ShouldBindJSON 不校验 Content-Type，期望 HTTP 200，实际 %d，响应体=%s",
			resp.Status, string(resp.Body))
	}
	if n := countRows(t, db, &models.WordReview{}); n != 1 {
		t.Fatalf("合法 JSON 体应正常落库，word_reviews 实际 %d 行", n)
	}
	if row := reviewRowOf(t, db, wordID); row.Stability != 1 {
		t.Fatalf("落库的 stability 期望 1，实际 %v", row.Stability)
	}
}

// TestSubmitReviewNullNumericFields 记录另一条宽松口径：JSON 里的 null 解到 float64 是 0，
// 于是 stability/difficulty/interval_days 传 null 会被当成 0 接受（合法且落库），
// 而不是被当成「缺字段」拒绝。真正需要拦的是 NaN/Inf（见 TestSubmitReviewNaNStability）。
func TestSubmitReviewNullNumericFields(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "latent")

	resp := doJSON(t, router, http.MethodPost, "/api/reviews/submit",
		[]byte(`{"word_id":1,"rating":3,"stability":null,"difficulty":null,"interval_days":null}`))
	if resp.Status != http.StatusOK {
		t.Fatalf("null 会被解成 0 并通过校验，期望 HTTP 200，实际 %d，响应体=%s", resp.Status, string(resp.Body))
	}
	row := reviewRowOf(t, db, wordID)
	if row.Stability != 0 || row.Difficulty != 0 {
		t.Fatalf("null 应落成 0，实际 stability=%v difficulty=%v", row.Stability, row.Difficulty)
	}
	// interval_days=0 → 仍然受 600 秒下限保护，不会被算成立刻到期
	logs := logsOf(t, db, wordID)
	if len(logs) != 1 || logs[0].IntervalDays != 0 {
		t.Fatalf("期望 1 条 interval_days=0 的日志，实际 %+v", logs)
	}
	if row.DueAt == nil || !row.DueAt.After(logs[0].ReviewedAt) {
		t.Fatalf("due_at 应晚于 reviewed_at（600 秒下限），实际 due_at=%v reviewed_at=%v", row.DueAt, logs[0].ReviewedAt)
	}
}

// TestSubmitReviewDoesNotTouchOtherWords 提交只影响目标词：
// 另一张卡的状态与日志必须原样不动（多用户化之前这里最容易写错 WHERE）。
func TestSubmitReviewDoesNotTouchOtherWords(t *testing.T) {
	router, db := setupTestRouter(t)
	target := seedWord(t, db, "labor")
	other := seedWord(t, db, "margin")

	past := time.Now().Add(-24 * time.Hour)
	seedReview(t, db, other, 1.5, 7.0, &past, 4, 2)
	seedLog(t, db, other, 1, 1.0, 1.5, 1, past, false)

	submitReview(t, router, newReviewRequest(target, 3, 3, 5, 3))

	otherRow := reviewRowOf(t, db, other)
	if otherRow.Stability != 1.5 || otherRow.Difficulty != 7.0 || otherRow.Reps != 4 || otherRow.Lapses != 2 {
		t.Fatalf("其它词的状态被改动了: %+v", otherRow)
	}
	if n := len(logsOf(t, db, other)); n != 1 {
		t.Fatalf("其它词的日志数变了，期望 1，实际 %d", n)
	}
	if n := countRows(t, db, &models.ReviewLog{}); n != 2 {
		t.Fatalf("日志总数期望 2（其它词 1 + 本次 1），实际 %d", n)
	}
}
