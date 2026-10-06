// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
package handlers

// P0-1「复习进度按人隔离」的回归测试。
//
// 为什么单独一个文件：这一组用例测的不是某个接口的业务口径，而是一条横切性质——
// **同一个接口、同一个词，换个用户就必须看到另一份数据**。改造前 word_reviews 的唯一键是
// word_id（一个词全局只有一行），下面每条用例都会红，这正是它们存在的意义。
//
// 词库共享、进度私有：`words` 表没有 user_id，`word_reviews` / `review_logs` 才有。

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"

	"backend-go/database"
	"backend-go/models"
)

// submitAs 以指定用户身份提交一次复习，要求 HTTP 200。
func submitAs(t *testing.T, router *gin.Engine, userID int64, req submitReviewRequest) {
	t.Helper()
	resp := doJSONAsUser(t, router, http.MethodPost, "/api/reviews/submit", jsonBody(t, req), userID, "user")
	if resp.Status != http.StatusOK {
		t.Fatalf("user %d 提交复习期望 200，实际 %d，响应体=%s", userID, resp.Status, truncateBody(resp.Body))
	}
}

// getDataAs 以指定用户身份 GET 一个复习接口，返回统一响应壳里的 data。
func getDataAs(t *testing.T, router *gin.Engine, userID int64, path string) map[string]interface{} {
	t.Helper()
	data, _ := decodeData(t, doJSONAsUser(t, router, http.MethodGet, path, nil, userID, "user"))
	return data
}

// TestSubmitReviewKeepsOneRowPerUser 两个用户复习同一个词，各留各的进度。
// 改造前这里只会有一行（后写的覆盖先写的），所以这条用例同时盯着「行数」与「字段没被改」。
func TestSubmitReviewKeepsOneRowPerUser(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "isolated")

	// 第一个用户：5 天后到期；第二个用户：Again（间隔下限 10 分钟）+ 一次遗忘
	submitAs(t, router, testUserID, newReviewRequest(wordID, 3, 5.0, 5.0, 5))
	submitAs(t, router, testOtherUserID, newReviewRequest(wordID, 1, 1.0, 8.0, 0))

	if got := countRows(t, db, &models.WordReview{}); got != 2 {
		t.Fatalf("两个用户复习同一个词后 word_reviews 应有 2 行，实际 %d 行", got)
	}
	if got := countReviewsOfUser(t, db, testOtherUserID); got != 1 {
		t.Fatalf("第二个用户应恰好有 1 行进度，实际 %d 行", got)
	}

	first := reviewRowOfUser(t, db, testUserID, wordID)
	second := reviewRowOfUser(t, db, testOtherUserID, wordID)

	// 后提交的人不能改到前一个人的状态
	assertFloat(t, map[string]interface{}{"stability": first.Stability}, "stability", 5.0)
	assertFloat(t, map[string]interface{}{"difficulty": first.Difficulty}, "difficulty", 5.0)
	assertFloat(t, map[string]interface{}{"stability": second.Stability}, "stability", 1.0)
	assertFloat(t, map[string]interface{}{"difficulty": second.Difficulty}, "difficulty", 8.0)
	if first.Reps != 1 || second.Reps != 1 {
		t.Fatalf("reps 期望各为 1，实际 first=%d / second=%d", first.Reps, second.Reps)
	}
	if first.Lapses != 0 || second.Lapses != 1 {
		t.Fatalf("lapses 期望 first=0 / second=1，实际 first=%d / second=%d", first.Lapses, second.Lapses)
	}
	if first.DueAt == nil || second.DueAt == nil {
		t.Fatal("两个用户的 due_at 都不应为 NULL")
	}
	if !first.DueAt.After(*second.DueAt) {
		t.Fatalf("5 天间隔的到期时间应当晚于 Again 的：user %d=%s，user %d=%s",
			testUserID, first.DueAt, testOtherUserID, second.DueAt)
	}

	// 日志同样分人：每人各一条，归属必须写对
	if got := len(logsOfUser(t, db, testUserID, wordID)); got != 1 {
		t.Fatalf("第一个用户应有 1 条日志，实际 %d 条", got)
	}
	logs := logsOfUser(t, db, testOtherUserID, wordID)
	if len(logs) != 1 {
		t.Fatalf("第二个用户应有 1 条日志，实际 %d 条", len(logs))
	}
	if logs[0].UserID != testOtherUserID || logs[0].Rating != 1 {
		t.Fatalf("第二个用户的日志内容不对：user_id=%d rating=%d（期望 %d / 1）",
			logs[0].UserID, logs[0].Rating, testOtherUserID)
	}
}

// TestReviewStatsIsolatedPerUser 统计接口只算当前用户的数据，但词库总数保持全局。
func TestReviewStatsIsolatedPerUser(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "someone-elses-word")

	past := time.Now().Add(-time.Hour)
	seedReviewFor(t, db, testOtherUserID, wordID, 2, 5, &past, 1, 0)
	seedLogFor(t, db, testOtherUserID, wordID, 3, 0, 2.0, 2.0, time.Now().Add(-time.Minute), false)

	mine := getDataAs(t, router, testUserID, "/api/reviews/stats")
	assertInt(t, mine, "total_words", 1) // 词库共享：总词数对谁都一样
	assertInt(t, mine, "new_words", 1)   // 但对我来说这个词还是新词
	assertInt(t, mine, "reviewed_words", 0)
	assertInt(t, mine, "due_cards", 0)
	assertInt(t, mine, "total_reviews", 0)
	assertInt(t, mine, "today_new", 0)
	assertInt(t, mine, "today_reviewed", 0)
	assertInt(t, mine, "streak_days", 0)

	theirs := getDataAs(t, router, testOtherUserID, "/api/reviews/stats")
	assertInt(t, theirs, "total_words", 1)
	assertInt(t, theirs, "new_words", 0)
	assertInt(t, theirs, "reviewed_words", 1)
	assertInt(t, theirs, "due_cards", 1)
	assertInt(t, theirs, "total_reviews", 1)
	assertInt(t, theirs, "today_new", 1)
	assertInt(t, theirs, "streak_days", 1)
}

// TestNewWordsIsolatedPerUser 单一循环池之后，「新词」这个概念没了：池子对每个人都是**整个词库**，
// 区别只在**进度**归谁。这条用例钉住的就是这个语义（旧版这里比的是 new 池的条数）。
func TestNewWordsIsolatedPerUser(t *testing.T) {
	router, db := setupTestRouter(t)
	learnedByOther := seedWord(t, db, "learned-by-other")
	untouched := seedWord(t, db, "untouched")

	past := time.Now().Add(-time.Hour)
	seedReviewFor(t, db, testOtherUserID, learnedByOther, 2, 5, &past, 1, 0)

	// 池子对两个用户都是 2 个词（词库共享、池子即全表）
	for _, user := range []int64{testUserID, testOtherUserID} {
		data := getDataAs(t, router, user, "/api/reviews/queue?limit=50")
		if got := len(items(t, data)); got != 2 {
			t.Fatalf("用户 %d 的池子应含全部 2 个词，实际 %d 个", user, got)
		}
		assertInt(t, data, "total", 2)
	}

	// 但「谁有进度」是按人的：别人学过的词，我这里 has_review 必须是 false
	cardOf := func(user int64, wordID uint) map[string]interface{} {
		t.Helper()
		for _, item := range items(t, getDataAs(t, router, user, "/api/reviews/queue?limit=50")) {
			if uint(toInt64(t, item["id"])) == wordID {
				return item
			}
		}
		t.Fatalf("用户 %d 的池子里找不到词 %d", user, wordID)
		return nil
	}
	if cardOf(testUserID, learnedByOther)["has_review"] != false {
		t.Fatalf("别人学过的词在我这里不该有进度（has_review 应为 false）")
	}
	if cardOf(testOtherUserID, learnedByOther)["has_review"] != true {
		t.Fatalf("学过的人自己应当有进度（has_review 应为 true）")
	}
	if cardOf(testUserID, untouched)["has_review"] != false {
		t.Fatalf("没人学过的词 has_review 应为 false")
	}

	// 已废弃的 new 接口对谁都不再返回候选
	for _, user := range []int64{testUserID, testOtherUserID} {
		if got := len(items(t, getDataAs(t, router, user, "/api/reviews/new?limit=50"))); got != 0 {
			t.Fatalf("new 接口在新模型下应为空，用户 %d 拿到 %d 条", user, got)
		}
	}
}

// TestDueQueueProbesIsolatedPerUser 到期卡、复习队列与抽查候选都只看自己的进度。
// 注意：队列的 total 现在是**池子总词数**（对所有人一样），所以「隔离」要看
// due / has_review / probes，而不是看 total 是否为 0。
func TestDueQueueProbesIsolatedPerUser(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "due-for-me")

	past := time.Now().Add(-time.Hour)
	seedReviewFor(t, db, testUserID, wordID, 2, 5, &past, 1, 0)

	if got := len(items(t, getDataAs(t, router, testUserID, "/api/reviews/due"))); got != 1 {
		t.Fatalf("第一个用户应有 1 张到期卡，实际 %d 张", got)
	}
	if got := len(items(t, getDataAs(t, router, testOtherUserID, "/api/reviews/due"))); got != 0 {
		t.Fatalf("第二个用户没有任何进度，不该有到期卡，实际 %d 张", got)
	}

	// 队列对两个人都给出同一个池子（total 相同），但进度标记按人区分
	theirs := getDataAs(t, router, testOtherUserID, "/api/reviews/queue?limit=100")
	mine := getDataAs(t, router, testUserID, "/api/reviews/queue?limit=100")
	assertInt(t, theirs, "total", 1)
	assertInt(t, mine, "total", 1)
	if items(t, theirs)[0]["has_review"] != false {
		t.Fatalf("第二个用户没学过这个词，has_review 应为 false")
	}
	if items(t, mine)[0]["has_review"] != true {
		t.Fatalf("第一个用户学过这个词，has_review 应为 true")
	}

	// 抽查（池尾诊断）只给「有进度行」的词，所以第二个用户是空的
	if got := len(items(t, getDataAs(t, router, testOtherUserID, "/api/reviews/probes"))); got != 0 {
		t.Fatalf("第二个用户的抽查候选应为空，实际 %d 张", got)
	}
	if got := len(items(t, getDataAs(t, router, testUserID, "/api/reviews/probes"))); got != 1 {
		t.Fatalf("第一个用户的抽查候选应有 1 张，实际 %d 张", got)
	}
}

// TestDeleteWordClearsEveryUsersProgress 删词条是跨用户操作：词条共享，删掉它就等于
// 删掉所有人对这个词的进度（P0-1 决策表里明确保留这个语义，不做软删除）。
func TestDeleteWordClearsEveryUsersProgress(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "shared-word")

	submitAs(t, router, testUserID, newReviewRequest(wordID, 3, 5, 5, 5))
	submitAs(t, router, testOtherUserID, newReviewRequest(wordID, 3, 5, 5, 5))
	if got := countRows(t, db, &models.WordReview{}); got != 2 {
		t.Fatalf("删除前应有 2 行进度，实际 %d 行", got)
	}

	data, _ := decodeData(t, doJSON(t, router, http.MethodDelete, "/api/words/"+itoa(int64(wordID)), nil))
	assertInt(t, data, "removed_reviews", 2)
	assertInt(t, data, "removed_logs", 2)

	if got := countRows(t, db, &models.WordReview{}); got != 0 {
		t.Fatalf("删除词条后进度行应清空，实际还剩 %d 行", got)
	}
	if got := countRows(t, db, &models.ReviewLog{}); got != 0 {
		t.Fatalf("删除词条后日志应清空，实际还剩 %d 行", got)
	}
}

// TestOrphanProgressIsInvisibleToEveryUser user_id = 0 是 P0-1 迁移前历史数据的占位值
// （迁移会把它们清掉）。万一有漏网的，它们既不能算到任何真实用户头上，也不能挡住新词判断。
func TestOrphanProgressIsInvisibleToEveryUser(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "orphan")

	past := time.Now().Add(-time.Hour)
	if err := db.Create(&models.WordReview{UserID: 0, WordID: wordID, Stability: 2, Difficulty: 5, DueAt: &past}).Error; err != nil {
		t.Fatalf("造无归属进度失败: %v", err)
	}
	if err := db.Create(&models.ReviewLog{UserID: 0, WordID: wordID, Rating: 3, ReviewedAt: time.Now()}).Error; err != nil {
		t.Fatalf("造无归属日志失败: %v", err)
	}

	stats := getDataAs(t, router, testUserID, "/api/reviews/stats")
	assertInt(t, stats, "reviewed_words", 0)
	assertInt(t, stats, "new_words", 1)
	assertInt(t, stats, "due_cards", 0)
	assertInt(t, stats, "total_reviews", 0)
}

// TestSchemaUsesPerUserUniqueIndex 盯住表结构本身：唯一键必须是 (user_id, word_id)，
// 而 P0-1 之前那条 UNIQUE(word_id) 必须不存在——它留着的话，第二个用户复习同一个词
// 就会在运行时撞 UNIQUE 约束（GORM 的 AutoMigrate 只补新索引、不删旧索引，所以这条要测）。
func TestSchemaUsesPerUserUniqueIndex(t *testing.T) {
	_, db := setupTestRouter(t)

	var idx struct {
		SQL string
	}
	if err := db.Raw(
		`SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?`, database.ReviewUserWordIndex,
	).Scan(&idx).Error; err != nil {
		t.Fatalf("查询索引失败: %v", err)
	}
	if !strings.Contains(idx.SQL, "user_id") || !strings.Contains(idx.SQL, "word_id") {
		t.Fatalf("索引 %s 的定义不对（应同时包含 user_id 与 word_id）：%q", database.ReviewUserWordIndex, idx.SQL)
	}

	legacy, err := database.IndexPresent(db, database.LegacyReviewUniqueIndex)
	if err != nil {
		t.Fatalf("查询旧索引失败: %v", err)
	}
	if legacy {
		t.Fatalf("旧索引 %s（UNIQUE(word_id)）不应该存在：它会让第二个用户复习同一个词时写失败",
			database.LegacyReviewUniqueIndex)
	}
}
