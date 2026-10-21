// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
package handlers

// GET /api/reviews/history 的口径测试（个人中心「学习统计」面板的数据源）。
//
// 覆盖：必须登录 / 默认 84 天窗口与缺日补零 / 同词多次评分只算一个词 /
// 评分分布 / 窗口外不进来 / 按用户隔离 / days 参数与错误值。

import (
	"encoding/json"
	"net/http"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
)

type historyResponse struct {
	Code int `json:"code"`
	Data struct {
		Days   int            `json:"days"`
		Points []historyPoint `json:"points"`
	} `json:"data"`
}

// historyRaw 直接以当前登录用户身份请求，返回原始响应（断言 HTTP 状态码用）
func historyRaw(t *testing.T, router *gin.Engine, path string) jsonResponse {
	t.Helper()
	return doRequest(t, router, "GET", path, nil, testRequest{Role: "user", Origin: testOrigin})
}

// getHistory 请求并解析响应体（断言业务口径用）
func getHistory(t *testing.T, router *gin.Engine, path string) historyResponse {
	t.Helper()
	res := historyRaw(t, router, path)
	var out historyResponse
	if err := json.Unmarshal(res.Body, &out); err != nil {
		t.Fatalf("解析响应失败: %v，原文=%s", err, truncateBody(res.Body))
	}
	return out
}

// pointOf 按日期串找那一天；找不到直接失败（比自己在测试里算下标可靠）
func pointOf(t *testing.T, res historyResponse, date string) historyPoint {
	t.Helper()
	for _, p := range res.Data.Points {
		if p.Date == date {
			return p
		}
	}
	t.Fatalf("返回里没有 %s 这一天（窗口 %d 天）", date, res.Data.Days)
	return historyPoint{}
}

func TestHistoryRequiresLogin(t *testing.T) {
	router, _ := setupTestRouter(t)
	// 不带 Cookie、带本站 Origin：先过 CSRF 闸门，401 才能确定来自登录校验
	res := doJSONAnonymous(t, router, "GET", "/api/reviews/history", nil)
	if res.Status != http.StatusUnauthorized {
		t.Fatalf("未登录应 401，实际 %d body=%s", res.Status, res.Body)
	}
}

func TestHistoryDefaultWindowAndZeroFill(t *testing.T) {
	router, db := setupTestRouter(t)
	w1 := seedWord(t, db, "alpha")

	today := startOfBeijingDay(time.Now())
	seedLog(t, db, w1, 3, 0, 2.0, 2, today.Add(9*time.Hour), false)

	res := getHistory(t, router, "/api/reviews/history")
	if res.Code != http.StatusOK {
		t.Fatalf("应 200，实际 %d body=%v", res.Code, res.Data)
	}
	if res.Data.Days != historyDefaultDays {
		t.Fatalf("默认窗口应 %d 天，实际 %d", historyDefaultDays, res.Data.Days)
	}
	if len(res.Data.Points) != res.Data.Days {
		t.Fatalf("points 数应与天数一致：期望 %d，实际 %d", res.Data.Days, len(res.Data.Points))
	}

	todayKey := beijingDay(today)
	last := res.Data.Points[len(res.Data.Points)-1]
	if last.Date != todayKey {
		t.Fatalf("最后一天应等于今天 %s，实际 %s", todayKey, last.Date)
	}
	if last.Words != 1 || last.Reviews != 1 {
		t.Fatalf("今天应有 1 词 1 次，实际 words=%d reviews=%d", last.Words, last.Reviews)
	}

	// 三天前没有任何记录 → 必须是补 0 的那一格（前端自己补洞的话，这一条会红）
	threeAgo := pointOf(t, res, beijingDay(today.AddDate(0, 0, -3)))
	if threeAgo.Words != 0 || threeAgo.Reviews != 0 || threeAgo.Good != 0 {
		t.Fatalf("三天前应是全 0，实际 %+v", threeAgo)
	}

	// 窗口最前一天：默认窗口正好 84 天，它也是当天起点，补 0
	oldest := res.Data.Points[0]
	if oldest.Date != beijingDay(today.AddDate(0, 0, -(historyDefaultDays-1))) {
		t.Fatalf("第一天的日期不对：%s", oldest.Date)
	}
	if oldest.Words != 0 {
		t.Fatalf("窗口最前一天应为 0，实际 %d", oldest.Words)
	}
}

func TestHistoryCountsDistinctWordsAndRatings(t *testing.T) {
	router, db := setupTestRouter(t)
	w1 := seedWord(t, db, "beta")
	w2 := seedWord(t, db, "gamma")

	today := startOfBeijingDay(time.Now())
	// 同一个词评了两次 + 另一个词一次：reviews=3，words=2
	seedLog(t, db, w1, 1, 0, 1.5, 1, today.Add(time.Hour), false) // Again
	seedLog(t, db, w1, 3, 1.5, 3.0, 2, today.Add(2*time.Hour), false) // Good
	seedLog(t, db, w2, 4, 0, 5.0, 5, today.Add(3*time.Hour), false)  // Easy

	res := getHistory(t, router, "/api/reviews/history?days=7")
	if res.Data.Days != 7 || len(res.Data.Points) != 7 {
		t.Fatalf("days=7 应返回 7 天，实际 %d 天 / %d 格", res.Data.Days, len(res.Data.Points))
	}
	p := pointOf(t, res, beijingDay(today))
	if p.Words != 2 {
		t.Fatalf("同词多次评分只算一个词：期望 2，实际 %d", p.Words)
	}
	if p.Reviews != 3 {
		t.Fatalf("评分次数应 3，实际 %d", p.Reviews)
	}
	if p.Again != 1 || p.Hard != 0 || p.Good != 1 || p.Easy != 1 {
		t.Fatalf("评分分布不对：%+v", p)
	}
}

func TestHistoryExcludesRowsOutsideWindow(t *testing.T) {
	router, db := setupTestRouter(t)
	w1 := seedWord(t, db, "delta")
	w2 := seedWord(t, db, "epsilon")

	today := startOfBeijingDay(time.Now())
	seedLog(t, db, w1, 3, 0, 2, 2, today.Add(time.Hour), false)
	seedLog(t, db, w2, 3, 0, 2, 2, today.AddDate(0, 0, -100).Add(time.Hour), false)

	// 默认 84 天窗口：100 天前那条不该进来
	res := getHistory(t, router, "/api/reviews/history")
	farKey := beijingDay(today.AddDate(0, 0, -100))
	var totalReviews int64
	for _, p := range res.Data.Points {
		if p.Date == farKey {
			t.Fatalf("100 天前在 84 天窗口外，这一天不该出现: %+v", p)
		}
		totalReviews += p.Reviews
	}
	if totalReviews != 1 {
		t.Fatalf("窗口内只应有今天那 1 次复习，实际 %d", totalReviews)
	}

	// 窗口放到 365 天：它就进来了（证明不是「日期对不上导致永远取不到」）
	wide := getHistory(t, router, "/api/reviews/history?days=365")
	if wide.Code != http.StatusOK || wide.Data.Days != 365 {
		t.Fatalf("days=365 应正常返回，实际 code=%d days=%d", wide.Code, wide.Data.Days)
	}
	far := pointOf(t, wide, beijingDay(today.AddDate(0, 0, -100)))
	if far.Reviews != 1 || far.Words != 1 {
		t.Fatalf("365 天窗口内应有 100 天前那条，实际 %+v", far)
	}
}

func TestHistoryIsPerUser(t *testing.T) {
	router, db := setupTestRouter(t)
	w1 := seedWord(t, db, "zeta")

	today := startOfBeijingDay(time.Now())
	seedLog(t, db, w1, 3, 0, 2, 2, today.Add(time.Hour), false)
	// 别人的同一天记录：不能算到我头上
	seedLogFor(t, db, testOtherUserID, w1, 4, 0, 5, 5, today.Add(4*time.Hour), false)

	res := getHistory(t, router, "/api/reviews/history?days=7")
	p := pointOf(t, res, beijingDay(today))
	if p.Words != 1 || p.Reviews != 1 || p.Easy != 0 {
		t.Fatalf("只应统计当前用户：期望 words=1 reviews=1 easy=0，实际 %+v", p)
	}

	// 换成那个人自己的令牌，才看得到他那条（doJSONAsUser 只换令牌里的 sub）
	other := doJSONAsUser(t, router, "GET", "/api/reviews/history?days=7", nil, testOtherUserID, "user")
	if other.Status != http.StatusOK {
		t.Fatalf("别人的身份应能查自己的历史，实际 %d", other.Status)
	}
	var body historyResponse
	if err := json.Unmarshal(other.Body, &body); err != nil {
		t.Fatalf("解析响应失败: %v", err)
	}
	p2 := pointOf(t, body, beijingDay(today))
	if p2.Words != 1 || p2.Reviews != 1 || p2.Easy != 1 {
		t.Fatalf("别人的历史里应只有他那条：%+v", p2)
	}
}

func TestHistoryRejectsBadDays(t *testing.T) {
	router, _ := setupTestRouter(t)
	for _, raw := range []string{"0", "-1", "366", "abc", "7.5", ""} {
		res := historyRaw(t, router, "/api/reviews/history?days="+raw)
		if raw == "" {
			// 空值 = 用默认窗口，不该报错
			if res.Status != http.StatusOK {
				t.Fatalf("days 空值应回落到默认窗口，实际 %d", res.Status)
			}
			continue
		}
		if res.Status != http.StatusBadRequest {
			t.Fatalf("days=%q 应 400，实际 %d", raw, res.Status)
		}
	}
}
