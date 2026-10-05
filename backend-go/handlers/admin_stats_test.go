// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package handlers

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/database"
	"backend-go/middleware"
)

// 本文件盯住管理看板的几条硬口径：
//   - **时区**：所有「今天 / 按天」按北京时间（UTC+8）切。下面的固定时钟特意选在北京时间的
//     凌晨（UTC 还停在前一天），任何按 UTC 或按服务器本地时区切天的实现都会当场算错；
//   - 账号库读不到时 **overview 仍 200**、账号侧数字全 0 且带原因，业务侧数字照常；
//   - trend 的补零 / 升序 / days 只认 7 与 30；
//   - 批量 progress：请求里没出现的 id 也要有一行（否则前端表格错位）、超限 400。

// ---------- 固定时钟与装置 ----------

// 固定的「现在」：北京时间 2026-10-03 02:00（= UTC 2026-10-02T18:00Z）。
//
// ⚠️ 选这个时刻是刻意的：此刻 UTC 的日期还是 10-02，而北京时间的自然日已经是 10-03。
// 于是「北京时间的今天」在 UTC 口径下会是「昨天」，任何按 UTC（或按部署机本地时区，
// 生产上可能是 UTC）切天的实现都会被下面的用例抓出来。
const adminTestNowISO = "2026-10-02T18:00:00Z"

// 边界数据里用到的两个时刻（与 adminTestNowISO 配套）：
const (
	// adminTestTodayEarlyUTC 北京时间 2026-10-03 00:30 —— 属于「今天」
	adminTestTodayEarlyUTC = "2026-10-02T16:30:00Z"
	// adminTestYesterdayLateUTC 北京时间 2026-10-02 23:30 —— 属于「昨天」
	adminTestYesterdayLateUTC = "2026-10-02T15:30:00Z"
)

// mustTime 解析一个 RFC3339 时刻（写死时刻是为了让「哪一天」不随跑测试的时间漂移）
func mustTime(t *testing.T, raw string) time.Time {
	t.Helper()
	parsed, err := time.Parse(time.RFC3339, raw)
	if err != nil {
		t.Fatalf("测试时刻 %q 解析失败: %v", raw, err)
	}
	return parsed
}

// setupAdminRouter 起一套「内存业务库 + 临时 auth.db + 只挂管理看板路由」的引擎。
//
// 为什么不复用 setupTestRouter：看板要**固定时钟**（时区边界必须钉住「现在」），
// 而那份装置里的处理器是内部构造的、注入不进 now。路由表本身的真伪由 routes 包的测试盯
// （它调真实的 SetupRouter）；这里只复刻那四条路径与 RequireAdmin。
func setupAdminRouter(t *testing.T, now time.Time) (*gin.Engine, *gorm.DB, string) {
	t.Helper()
	authPath := newTempAuthDB(t)
	router, db := setupAdminRouterAt(t, now, authPath)
	return router, db, authPath
}

// setupAdminRouterAt 是指定账号库路径的版本（不存在的路径用来测降级）
func setupAdminRouterAt(t *testing.T, now time.Time, authPath string) (*gin.Engine, *gorm.DB) {
	t.Helper()
	gin.SetMode(gin.TestMode)

	db := newTestDB(t)
	authDB := database.NewAuthDB(authPath)
	// 句柄持有只读连接，用例结束必须关掉：Windows 上留着连接会让 t.TempDir 的清理失败
	t.Cleanup(func() { _ = authDB.Close() })
	h := NewAdminStatsHandler(db, authDB)
	if !now.IsZero() {
		h.now = func() time.Time { return now }
	}

	router := gin.New()
	router.Use(gin.Recovery())
	api := router.Group("/api")
	api.Use(middleware.CSRFGuard([]string{testOrigin}))
	admin := api.Group("/admin", middleware.RequireAdmin(testJWTSecret))
	{
		admin.GET("/stats/overview", h.Overview)
		admin.GET("/stats/trend", h.Trend)
		admin.GET("/users/progress", h.UsersProgress)
		admin.GET("/users/:id/progress", h.UserProgress)
	}
	return router, db
}

// 临时账号库的表结构：与 backend-rust/migrations/0001_init.sql（users / invite_codes，
// 后者另含 0002_launch.sql 的 grant_role / batch_id）逐列对齐。
// 列名写全是有意的：看板里写错列名时测试会当场报 no such column，而不是给出一个「还好是 0」的数字。
const (
	adminTestUsersDDL = `CREATE TABLE users (
		id                INTEGER PRIMARY KEY AUTOINCREMENT,
		email             TEXT    NOT NULL UNIQUE,
		username          TEXT,
		password_hash     TEXT    NOT NULL,
		role              TEXT    NOT NULL DEFAULT 'user',
		status            TEXT    NOT NULL DEFAULT 'active',
		email_verified_at TEXT,
		failed_attempts   INTEGER NOT NULL DEFAULT 0,
		locked_until      TEXT,
		last_login_at     TEXT,
		created_at        TEXT    NOT NULL,
		updated_at        TEXT    NOT NULL
	)`
	adminTestInviteCodesDDL = `CREATE TABLE invite_codes (
		id         INTEGER PRIMARY KEY AUTOINCREMENT,
		code       TEXT    NOT NULL UNIQUE,
		note       TEXT    NOT NULL DEFAULT '',
		max_uses   INTEGER NOT NULL DEFAULT 1,
		used_count INTEGER NOT NULL DEFAULT 0,
		expires_at TEXT,
		disabled   INTEGER NOT NULL DEFAULT 0,
		created_by INTEGER,
		created_at TEXT    NOT NULL,
		grant_role TEXT    NOT NULL DEFAULT 'user',
		batch_id   TEXT    NOT NULL DEFAULT ''
	)`
)

// newTempAuthDB 在**临时目录**里造一个与账号服务同结构的 auth.db，返回它的路径。
// 绝不碰仓库里真实的 auth.db（那是账号服务的数据）。
// 顺手把 journal_mode 设成 WAL：账号服务线上就是 WAL，只读打开要在这个模式下也成立。
func newTempAuthDB(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "auth.db")
	db, err := gorm.Open(sqlite.Open(path), &gorm.Config{Logger: logger.Default.LogMode(logger.Silent)})
	if err != nil {
		t.Fatalf("建临时账号库失败: %v", err)
	}
	for _, stmt := range []string{adminTestUsersDDL, adminTestInviteCodesDDL, "PRAGMA journal_mode=WAL"} {
		if err := db.Exec(stmt).Error; err != nil {
			t.Fatalf("初始化临时账号库失败: %v（语句：%s）", err, stmt)
		}
	}
	if sqlDB, dbErr := db.DB(); dbErr == nil {
		_ = sqlDB.Close()
	}
	return path
}

// withAuthDB 开一条**可写**连接改临时账号库（模拟账号服务在写），用例结束自动关闭。
// 被看板读的那一侧走 database.AuthDB（只读），两边互不干扰。
func withAuthDB(t *testing.T, path string) *gorm.DB {
	t.Helper()
	db, err := gorm.Open(sqlite.Open(path), &gorm.Config{Logger: logger.Default.LogMode(logger.Silent)})
	if err != nil {
		t.Fatalf("打开临时账号库失败: %v", err)
	}
	t.Cleanup(func() {
		if sqlDB, dbErr := db.DB(); dbErr == nil {
			_ = sqlDB.Close()
		}
	})
	return db
}

// seedAuthUser 插一个账号用户；createdAt 用**ISO8601 UTC 文本**，与账号服务的存储口径一字不差。
func seedAuthUser(t *testing.T, path, email, role, createdAt string) {
	t.Helper()
	db := withAuthDB(t, path)
	if err := db.Exec(
		`INSERT INTO users (email, password_hash, role, created_at, updated_at) VALUES (?, 'argon2id-placeholder', ?, ?, ?)`,
		email, role, createdAt, createdAt).Error; err != nil {
		t.Fatalf("造账号用户 %s 失败: %v", email, err)
	}
}

// seedInvite 插一张邀请码；usedCount > 0 表示这张码已经兑换过（看板据此算兑换率）
func seedInvite(t *testing.T, path, code string, usedCount int) {
	t.Helper()
	db := withAuthDB(t, path)
	if err := db.Exec(
		`INSERT INTO invite_codes (code, note, max_uses, used_count, disabled, created_at, grant_role, batch_id)
		 VALUES (?, '', 1, ?, 0, ?, 'user', '')`,
		code, usedCount, "2026-09-15T08:31:13Z").Error; err != nil {
		t.Fatalf("造邀请码 %s 失败: %v", code, err)
	}
}

// ---------- 断言小工具（本文件专用，避免与 setup_test.go 的同名工具打架） ----------

// subMap 取 data 里的一个子对象
func subMap(t *testing.T, data map[string]interface{}, key string) map[string]interface{} {
	t.Helper()
	raw, ok := data[key]
	if !ok {
		t.Fatalf("响应 data 里没有 %q 字段，实际字段=%v", key, mapKeys(data))
	}
	obj, ok := raw.(map[string]interface{})
	if !ok {
		t.Fatalf("字段 %q 期望对象，实际类型=%T（值=%v）", key, raw, raw)
	}
	return obj
}

// listOf 取 data 里的一个对象数组（points / items / last7）
func listOf(t *testing.T, data map[string]interface{}, key string) []map[string]interface{} {
	t.Helper()
	raw, ok := data[key]
	if !ok {
		t.Fatalf("响应 data 里没有 %q 字段，实际字段=%v", key, mapKeys(data))
	}
	list, ok := raw.([]interface{})
	if !ok {
		t.Fatalf("字段 %q 期望数组，实际类型=%T", key, raw)
	}
	out := make([]map[string]interface{}, 0, len(list))
	for i, item := range list {
		obj, ok := item.(map[string]interface{})
		if !ok {
			t.Fatalf("%s[%d] 不是对象，实际类型=%T", key, i, item)
		}
		out = append(out, obj)
	}
	return out
}

// assertBool 断言布尔字段
func assertBool(t *testing.T, data map[string]interface{}, key string, want bool) {
	t.Helper()
	got, ok := data[key]
	if !ok {
		t.Fatalf("响应缺少字段 %q，实际字段=%v", key, mapKeys(data))
	}
	b, ok := got.(bool)
	if !ok {
		t.Fatalf("字段 %q 期望布尔，实际类型=%T（值=%v）", key, got, got)
	}
	if b != want {
		t.Fatalf("字段 %q 期望 %v，实际 %v", key, want, got)
	}
}

// assertString 断言字符串字段
func assertString(t *testing.T, data map[string]interface{}, key, want string) {
	t.Helper()
	got, ok := data[key]
	if !ok {
		t.Fatalf("响应缺少字段 %q（值为 nil 也算缺）—— JSON null 与字段缺失在排查时不是一回事", key)
	}
	s, ok := got.(string)
	if !ok {
		t.Fatalf("字段 %q 期望字符串，实际类型=%T（值=%v）", key, got, got)
	}
	if s != want {
		t.Fatalf("字段 %q 期望 %q，实际 %q", key, want, s)
	}
}

// assertNull 断言字段是 JSON null（missing id 的 last_review_at 必须是 null，不能是零值时间字符串）
func assertNull(t *testing.T, data map[string]interface{}, key string) {
	t.Helper()
	got, ok := data[key]
	if !ok {
		t.Fatalf("响应缺少字段 %q（前端按字段存在与否渲染，缺字段与 null 不是一回事）", key)
	}
	if got != nil {
		t.Fatalf("字段 %q 期望 null，实际 %v", key, got)
	}
}

// ---------- overview ----------

// TestAdminOverviewBeijingDayBoundary 本批最容易写错的一条：**按北京时间切天**。
//
// 固定「现在」= UTC 2026-10-02T18:00Z（北京 10-03 02:00），然后摆两批数据：
//   - 北京 10-03 00:30（UTC 10-02T16:30Z）→ 必须算「今天」；
//   - 北京 10-02 23:30（UTC 10-02T15:30Z）→ 必须算「昨天」。
//
// 两者的 **UTC 日期都是 10-02**：所以一个按 UTC（或按部署机本地时区 = UTC）切天的实现
// 会把它们都算成「今天」，这里会立刻红。
func TestAdminOverviewBeijingDayBoundary(t *testing.T) {
	router, db, authPath := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	seedAuthUser(t, authPath, "today@example.com", "user", adminTestTodayEarlyUTC)
	seedAuthUser(t, authPath, "yesterday@example.com", "user", adminTestYesterdayLateUTC)
	// 北京 10-02 00:30（UTC 10-01T16:30Z）：也是昨天，防止「只要不是今天就算昨天」这种凑巧对上的实现
	seedAuthUser(t, authPath, "before@example.com", "user", "2026-10-01T16:30:00Z")

	wordID := seedWord(t, db, "boundary-word")
	seedLogFor(t, db, 1, wordID, 3, 0, 1.5, 1.5, mustTime(t, adminTestTodayEarlyUTC), true)
	seedLogFor(t, db, 2, wordID, 3, 5, 6, 1.5, mustTime(t, adminTestYesterdayLateUTC), false)

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/overview", nil))
	if code != http.StatusOK {
		t.Fatalf("overview 的 code 期望 200，实际 %d", code)
	}

	today := subMap(t, data, "today")
	assertInt(t, today, "new_users", 1)    // 只有北京 10-03 00:30 那个算今天
	assertInt(t, today, "reviews", 1)      // 昨天的复习不算今天
	assertInt(t, today, "active_users", 1) // 昨天那条日志的用户不算今日活跃
	assertInt(t, today, "new_words", 1)    // 今天那条 stability_before = 0
	assertInt(t, today, "probes", 1)       // 今天那条是抽查

	auth := subMap(t, data, "auth_db")
	assertBool(t, auth, "available", true)
	assertString(t, auth, "error", "")
}

// TestAdminOverviewTotalsAndConversion 累计数字与兑换率：
// 角色分布按 role 精确匹配（admin / super_admin），兑换率保留 2 位小数。
func TestAdminOverviewTotalsAndConversion(t *testing.T) {
	router, db, authPath := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	seedAuthUser(t, authPath, "root@example.com", middleware.RoleSuperAdmin, "2026-09-15T08:31:13Z")
	seedAuthUser(t, authPath, "admin1@example.com", middleware.RoleAdmin, "2026-09-15T08:31:47Z")
	seedAuthUser(t, authPath, "admin2@example.com", middleware.RoleAdmin, "2026-09-15T08:32:42Z")
	for i := 0; i < 4; i++ {
		seedAuthUser(t, authPath, fmt.Sprintf("user%d@example.com", i), "user", "2026-09-16T08:00:00Z")
	}
	// 4 张码、2 张已兑换 → 0.5
	seedInvite(t, authPath, "CODE-A", 1)
	seedInvite(t, authPath, "CODE-B", 3)
	seedInvite(t, authPath, "CODE-C", 0)
	seedInvite(t, authPath, "CODE-D", 0)

	wordID := seedWord(t, db, "w1")
	seedWord(t, db, "w2")
	seedWord(t, db, "w3")
	seedLog(t, db, wordID, 3, 0, 1.5, 1.5, mustTime(t, adminTestTodayEarlyUTC), false)
	seedLog(t, db, wordID, 3, 1.5, 3, 1.5, mustTime(t, adminTestYesterdayLateUTC), false)

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/overview", nil))
	if code != http.StatusOK {
		t.Fatalf("overview 的 code 期望 200，实际 %d", code)
	}
	totals := subMap(t, data, "totals")
	assertInt(t, totals, "users", 7)
	assertInt(t, totals, "admins", 2)
	assertInt(t, totals, "super_admins", 1)
	assertInt(t, totals, "invites_issued", 4)
	assertInt(t, totals, "invites_redeemed", 2)
	assertFloat(t, totals, "invite_conversion", 0.5)
	assertInt(t, totals, "words", 3)
	assertInt(t, totals, "reviews", 2)
}

// TestAdminOverviewConversionIsZeroWithoutInvites 一张码都没发时兑换率给 0（**不能是 NaN**）：
// JSON 里没有 NaN，序列化会直接失败，前端 JSON.parse 也会抛错——整页数字会一起打不开。
func TestAdminOverviewConversionIsZeroWithoutInvites(t *testing.T) {
	router, _, authPath := setupAdminRouter(t, mustTime(t, adminTestNowISO))
	seedAuthUser(t, authPath, "only@example.com", "user", "2026-09-15T08:31:13Z")

	resp := doJSON(t, router, http.MethodGet, "/api/admin/stats/overview", nil)
	if strings.Contains(string(resp.Body), "NaN") {
		t.Fatalf("响应体里出现了 NaN：%s", truncateBody(resp.Body))
	}
	data, code := decodeData(t, resp)
	if code != http.StatusOK {
		t.Fatalf("overview 的 code 期望 200，实际 %d", code)
	}
	totals := subMap(t, data, "totals")
	assertInt(t, totals, "invites_issued", 0)
	assertInt(t, totals, "invites_redeemed", 0)
	assertFloat(t, totals, "invite_conversion", 0)
}

// TestAdminOverviewDegradesWhenAuthDBUnavailable 账号库读不到时：
// HTTP 仍 200、code 仍 200、账号侧数字全 0 + error 非空，**业务侧数字照常**。
// 这是看板的主路径之一（账号服务重启、库被搬走都会这样），不能整页 500。
func TestAdminOverviewDegradesWhenAuthDBUnavailable(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "not-here", "auth.db")
	router, db := setupAdminRouterAt(t, mustTime(t, adminTestNowISO), missing)

	wordID := seedWord(t, db, "still-counted")
	seedLogFor(t, db, 1, wordID, 3, 0, 1.5, 1.5, mustTime(t, adminTestTodayEarlyUTC), false)
	seedLogFor(t, db, 2, wordID, 4, 0, 1.5, 1.5, mustTime(t, adminTestTodayEarlyUTC), false)

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/overview", nil))
	if code != http.StatusOK {
		t.Fatalf("账号库不可用时 overview 的 code 仍应是 200，实际 %d", code)
	}

	auth := subMap(t, data, "auth_db")
	assertBool(t, auth, "available", false)
	message, _ := auth["error"].(string)
	for _, want := range []string{"AUTH_DB_PATH", missing, "账号侧数字一律显示 0"} {
		if !strings.Contains(message, want) {
			t.Fatalf("auth_db.error 里应当出现 %q，实际：%q", want, message)
		}
	}

	today := subMap(t, data, "today")
	assertInt(t, today, "new_users", 0)
	assertInt(t, today, "reviews", 2)
	assertInt(t, today, "active_users", 2)
	assertInt(t, today, "new_words", 2)

	totals := subMap(t, data, "totals")
	assertInt(t, totals, "users", 0)
	assertInt(t, totals, "admins", 0)
	assertInt(t, totals, "super_admins", 0)
	assertInt(t, totals, "invites_issued", 0)
	assertInt(t, totals, "invites_redeemed", 0)
	assertFloat(t, totals, "invite_conversion", 0)
	assertInt(t, totals, "words", 1)
	assertInt(t, totals, "reviews", 2)

	// 只读打开**不能**把不存在的账号库顺手建出来（那会变成一个「读得到但任何表都没有」的空库）
	if _, err := os.Stat(missing); err == nil {
		t.Fatalf("只读访问把不存在的账号库创建出来了：%s", missing)
	}
}

// TestAdminOverviewReadsWhileAuthWriterIsOpen 账号服务**在跑**（库被另一个连接以可写方式打开、
// 且是 WAL 模式）时只读连接照样读得到 —— 这是线上最常见的状态，也是「只读打开 WAL 库」唯一
// 真正要成立的时刻（-shm/-wal 由写入方维护着）。
func TestAdminOverviewReadsWhileAuthWriterIsOpen(t *testing.T) {
	router, _, authPath := setupAdminRouter(t, mustTime(t, adminTestNowISO))
	seedAuthUser(t, authPath, "live@example.com", "user", adminTestTodayEarlyUTC)

	// 一直开着这条可写连接，模拟账号服务持有库不放
	writer := withAuthDB(t, authPath)
	if err := writer.Exec(
		`INSERT INTO invite_codes (code, note, max_uses, used_count, disabled, created_at, grant_role, batch_id)
		 VALUES ('LIVE-CODE', '', 1, 1, 0, '2026-09-15T08:31:13Z', 'user', '')`).Error; err != nil {
		t.Fatalf("账号服务侧写码失败: %v", err)
	}

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/overview", nil))
	if code != http.StatusOK {
		t.Fatalf("overview 的 code 期望 200，实际 %d", code)
	}
	auth := subMap(t, data, "auth_db")
	assertBool(t, auth, "available", true)
	assertString(t, auth, "error", "")
	// 刚写进去的那张码也要看得见（只读连接不能读到一份陈旧的快照）
	assertInt(t, subMap(t, data, "today"), "new_users", 1)
	totals := subMap(t, data, "totals")
	assertInt(t, totals, "users", 1)
	assertInt(t, totals, "invites_issued", 1)
	assertInt(t, totals, "invites_redeemed", 1)
	assertFloat(t, totals, "invite_conversion", 1)
}

// ---------- trend ----------

// TestAdminTrendDaysValidation days 只接受 7 与 30（默认 7），其它值一律 400
func TestAdminTrendDaysValidation(t *testing.T) {
	router, _, _ := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	ok := map[string]int{"": 7, "?days=7": 7, "?days=30": 30}
	for query, wantDays := range ok {
		t.Run("接受"+query, func(t *testing.T) {
			data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/trend"+query, nil))
			if code != http.StatusOK {
				t.Fatalf("trend%s 的 code 期望 200，实际 %d", query, code)
			}
			assertInt(t, data, "days", int64(wantDays))
			if got := len(listOf(t, data, "points")); got != wantDays {
				t.Fatalf("trend%s 的 points 期望 %d 个，实际 %d", query, wantDays, got)
			}
		})
	}

	for _, query := range []string{"?days=1", "?days=8", "?days=0", "?days=-7", "?days=abc", "?days=30.0"} {
		t.Run("拒绝"+query, func(t *testing.T) {
			resp := doJSON(t, router, http.MethodGet, "/api/admin/stats/trend"+query, nil)
			if resp.Status != http.StatusBadRequest {
				t.Fatalf("trend%s 期望 400，实际 %d，响应体=%s", query, resp.Status, truncateBody(resp.Body))
			}
		})
	}
}

// TestAdminTrendPaddedAscending 折线图的数据形状：升序、长度恰好 days、没有数据的日子补 0。
// 前端直接连线画图，一旦少一天或顺序乱了，整条曲线会静默错位。
func TestAdminTrendPaddedAscending(t *testing.T) {
	router, db, authPath := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	wordID := seedWord(t, db, "trend-word")
	// 北京 10-03（今天）：两条日志、两个不同用户
	seedLogFor(t, db, 1, wordID, 3, 0, 1.5, 1.5, mustTime(t, "2026-10-02T16:30:00Z"), false)
	seedLogFor(t, db, 2, wordID, 3, 0, 1.5, 1.5, mustTime(t, "2026-10-02T17:00:00Z"), false)
	// 北京 10-01 01:00
	seedLogFor(t, db, 1, wordID, 3, 0, 1.5, 1.5, mustTime(t, "2026-09-30T17:00:00Z"), false)
	// 窗口外（北京 09-20）：一个点都不该出现在 7 天的曲线上
	seedLogFor(t, db, 1, wordID, 3, 0, 1.5, 1.5, mustTime(t, "2026-09-20T02:00:00Z"), false)

	seedAuthUser(t, authPath, "d2@example.com", "user", "2026-10-01T16:30:00Z")  // 北京 10-02
	seedAuthUser(t, authPath, "d3@example.com", "user", "2026-10-02T16:30:00Z")  // 北京 10-03
	seedAuthUser(t, authPath, "old@example.com", "user", "2026-09-01T00:00:00Z") // 窗口外

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/trend?days=7", nil))
	if code != http.StatusOK {
		t.Fatalf("trend 的 code 期望 200，实际 %d", code)
	}
	assertInt(t, data, "days", 7)

	points := listOf(t, data, "points")
	if len(points) != 7 {
		t.Fatalf("points 期望 7 个，实际 %d", len(points))
	}
	// 升序、且正好是「今天往前 7 天」这一段（北京时间的 09-27 ~ 10-03）
	wantDates := []string{"2026-09-27", "2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03"}
	var reviewSum int64
	for i, want := range wantDates {
		assertString(t, points[i], "date", want)
		reviewSum += int64(points[i]["reviews"].(float64))
	}
	if reviewSum != 3 {
		t.Fatalf("7 天内的复习总数期望 3（窗口外那条不该被算进来），实际 %d", reviewSum)
	}

	// 没有数据的前四天必须是 0（补零），而不是缺行
	for i := 0; i < 4; i++ {
		assertInt(t, points[i], "reviews", 0)
		assertInt(t, points[i], "active_users", 0)
		assertInt(t, points[i], "new_users", 0)
	}
	assertInt(t, points[4], "reviews", 1) // 10-01
	assertInt(t, points[4], "active_users", 1)
	assertInt(t, points[4], "new_users", 0)
	assertInt(t, points[5], "reviews", 0) // 10-02：没有复习，但有一个注册
	assertInt(t, points[5], "new_users", 1)
	assertInt(t, points[6], "reviews", 2) // 10-03
	assertInt(t, points[6], "active_users", 2)
	assertInt(t, points[6], "new_users", 1)
}

// TestAdminTrendDegradesWhenAuthDBUnavailable 账号库不可用时 new_users 全 0，但曲线照常返回
// （reviews / active_users 来自业务库，必须还在）。
func TestAdminTrendDegradesWhenAuthDBUnavailable(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "gone", "auth.db")
	router, db := setupAdminRouterAt(t, mustTime(t, adminTestNowISO), missing)

	wordID := seedWord(t, db, "trend-degraded")
	seedLog(t, db, wordID, 3, 0, 1.5, 1.5, mustTime(t, adminTestTodayEarlyUTC), false)

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/stats/trend?days=7", nil))
	if code != http.StatusOK {
		t.Fatalf("账号库不可用时 trend 的 code 仍应是 200，实际 %d", code)
	}
	points := listOf(t, data, "points")
	assertInt(t, points[6], "reviews", 1)
	assertInt(t, points[6], "active_users", 1)
	for i := range points {
		assertInt(t, points[i], "new_users", 0)
	}
}

// ---------- 批量进度 ----------

// TestAdminUsersProgressBatch 批量进度的三条硬要求：
// 请求里**没有的 id 也要出现在 items 里**（数字全 0，否则前端表格错位）、
// 不存在的 id 给 0、返回顺序与请求顺序一致。
func TestAdminUsersProgressBatch(t *testing.T) {
	router, db, _ := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	w1 := seedWord(t, db, "p1")
	w2 := seedWord(t, db, "p2")
	// 用户 1：学了两个词、两次复习（今天 + 昨天 → 连续 2 天）
	seedReviewFor(t, db, 1, w1, 1.5, 5, nil, 1, 0)
	seedReviewFor(t, db, 1, w2, 1.5, 5, nil, 1, 0)
	seedLogFor(t, db, 1, w1, 3, 0, 1.5, 1.5, mustTime(t, "2026-10-02T16:30:00Z"), false) // 北京 10-03 00:30
	seedLogFor(t, db, 1, w2, 3, 1.5, 3, 1.5, mustTime(t, "2026-10-02T02:00:00Z"), false) // 北京 10-02 10:00
	// 用户 2：学了一个词、今天复习一次
	seedReviewFor(t, db, 2, w1, 1.5, 5, nil, 1, 0)
	seedLogFor(t, db, 2, w1, 3, 0, 1.5, 1.5, mustTime(t, adminTestTodayEarlyUTC), false)

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/users/progress?ids=1,2,999", nil))
	if code != http.StatusOK {
		t.Fatalf("批量进度的 code 期望 200，实际 %d", code)
	}
	items := listOf(t, data, "items")
	if len(items) != 3 {
		t.Fatalf("items 期望 3 行（请求里没有数据的 id 也要有一行），实际 %d：%v", len(items), items)
	}

	assertInt(t, items[0], "user_id", 1)
	assertInt(t, items[0], "learned_words", 2)
	assertInt(t, items[0], "total_reviews", 2)
	assertInt(t, items[0], "streak_days", 2)
	// 最近一次复习 = 北京 10-03 00:30，输出成带 +08:00 的 RFC3339
	assertString(t, items[0], "last_review_at", "2026-10-03T00:30:00+08:00")

	assertInt(t, items[1], "user_id", 2)
	assertInt(t, items[1], "learned_words", 1)
	assertInt(t, items[1], "total_reviews", 1)
	assertInt(t, items[1], "streak_days", 1)
	assertString(t, items[1], "last_review_at", "2026-10-03T00:30:00+08:00")

	// 账号库里有、业务库里没有数据的用户：数字全 0，且 last_review_at 是 null
	assertInt(t, items[2], "user_id", 999)
	assertInt(t, items[2], "learned_words", 0)
	assertInt(t, items[2], "total_reviews", 0)
	assertInt(t, items[2], "streak_days", 0)
	assertNull(t, items[2], "last_review_at")
}

// TestAdminUsersProgressValidation ids 非法 / 超限一律 400；
// 重复 id 去重（去重前就计入 100 的上限，免得用重复项绕过限制）。
func TestAdminUsersProgressValidation(t *testing.T) {
	router, _, _ := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	bad := []string{"", "ids=", "ids=abc", "ids=0", "ids=-1", "ids=1,,2", "ids=1,abc", "ids=1.5"}
	for _, query := range bad {
		t.Run("拒绝 "+query, func(t *testing.T) {
			resp := doJSON(t, router, http.MethodGet, "/api/admin/users/progress?"+query, nil)
			if resp.Status != http.StatusBadRequest {
				t.Fatalf("?%s 期望 400，实际 %d，响应体=%s", query, resp.Status, truncateBody(resp.Body))
			}
		})
	}

	// 100 个 id：放行
	hundred := make([]string, 0, 100)
	for i := 1; i <= 100; i++ {
		hundred = append(hundred, itoa(int64(i)))
	}
	data, code := decodeData(t, doJSON(t, router, http.MethodGet,
		"/api/admin/users/progress?ids="+strings.Join(hundred, ","), nil))
	if code != http.StatusOK {
		t.Fatalf("100 个 id 应当放行，code 实际 %d", code)
	}
	if got := len(listOf(t, data, "items")); got != 100 {
		t.Fatalf("100 个 id 期望 100 行，实际 %d", got)
	}

	// 101 个 id：400
	over := append(append([]string{}, hundred...), "101")
	resp := doJSON(t, router, http.MethodGet, "/api/admin/users/progress?ids="+strings.Join(over, ","), nil)
	if resp.Status != http.StatusBadRequest {
		t.Fatalf("101 个 id 期望 400，实际 %d，响应体=%s", resp.Status, truncateBody(resp.Body))
	}

	// 重复 id 去重后只出一行
	data, _ = decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/users/progress?ids=1,1,2", nil))
	items := listOf(t, data, "items")
	if len(items) != 2 {
		t.Fatalf("重复 id 应当去重成 2 行，实际 %d", len(items))
	}
	assertInt(t, items[0], "user_id", 1)
	assertInt(t, items[1], "user_id", 2)
}

// ---------- 单人进度 ----------

// TestAdminUserProgressSingle 单人摘要：字段齐全、last7 固定 7 天升序补 0；
// 不存在的 id 也给 0（面板点开一个没学过的人不能报错），非数字 / 0 才 400。
func TestAdminUserProgressSingle(t *testing.T) {
	router, db, _ := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	wordID := seedWord(t, db, "single")
	seedReviewFor(t, db, 1, wordID, 1.5, 5, nil, 1, 0)
	// 北京 10-03 两条、10-02 一条、09-28 一条（都在最近 7 天里：09-27 ~ 10-03）
	seedLogFor(t, db, 1, wordID, 3, 0, 1.5, 1.5, mustTime(t, "2026-10-02T16:30:00Z"), false)
	seedLogFor(t, db, 1, wordID, 4, 1.5, 3, 1.5, mustTime(t, "2026-10-02T17:30:00Z"), false)
	seedLogFor(t, db, 1, wordID, 3, 3, 6, 1.5, mustTime(t, "2026-10-02T02:00:00Z"), false)
	seedLogFor(t, db, 1, wordID, 3, 6, 9, 1.5, mustTime(t, "2026-09-27T18:00:00Z"), false) // 北京 09-28 02:00

	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/users/1/progress", nil))
	if code != http.StatusOK {
		t.Fatalf("单人进度的 code 期望 200，实际 %d", code)
	}
	assertInt(t, data, "user_id", 1)
	assertInt(t, data, "learned_words", 1)
	assertInt(t, data, "total_reviews", 4)
	assertInt(t, data, "streak_days", 2)
	assertString(t, data, "last_review_at", "2026-10-03T01:30:00+08:00")

	points := listOf(t, data, "last7")
	if len(points) != 7 {
		t.Fatalf("last7 期望 7 天，实际 %d", len(points))
	}
	wantDates := []string{"2026-09-27", "2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03"}
	wantReviews := []int64{0, 1, 0, 0, 0, 1, 2}
	for i := range wantDates {
		assertString(t, points[i], "date", wantDates[i])
		assertInt(t, points[i], "reviews", wantReviews[i])
	}

	// 没学过的人：0 + 空曲线，仍然是 200（面板点开谁都不该报错）
	data, code = decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/users/999/progress", nil))
	if code != http.StatusOK {
		t.Fatalf("不存在用户的单人进度也应当 200，实际 %d", code)
	}
	assertInt(t, data, "user_id", 999)
	assertInt(t, data, "learned_words", 0)
	assertInt(t, data, "total_reviews", 0)
	assertInt(t, data, "streak_days", 0)
	assertNull(t, data, "last_review_at")
	if got := len(listOf(t, data, "last7")); got != 7 {
		t.Fatalf("不存在用户的 last7 也必须是 7 天（补 0），实际 %d", got)
	}

	// 非数字 / 0 → 400
	for _, id := range []string{"abc", "0", "-1", "1.5"} {
		t.Run("拒绝 id="+id, func(t *testing.T) {
			resp := doJSON(t, router, http.MethodGet, "/api/admin/users/"+id+"/progress", nil)
			if resp.Status != http.StatusBadRequest {
				t.Fatalf("id=%s 期望 400，实际 %d，响应体=%s", id, resp.Status, truncateBody(resp.Body))
			}
		})
	}
}

// TestAdminRoutesRequireAdmin 看板整组要「能进后台」的角色：
// 未登录 401（error=unauthenticated）、普通用户 403（error=forbidden）、
// admin 与 super_admin 都放行 —— 这条同时钉住「用的是 RequireAdmin 而不是 RequireSuperAdmin」。
func TestAdminRoutesRequireAdmin(t *testing.T) {
	router, _, _ := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	paths := []string{
		"/api/admin/stats/overview",
		"/api/admin/stats/trend?days=7",
		"/api/admin/users/progress?ids=1",
		"/api/admin/users/1/progress",
	}
	for _, path := range paths {
		t.Run("未登录 "+path, func(t *testing.T) {
			resp := doJSONAnonymous(t, router, http.MethodGet, path, nil)
			if resp.Status != http.StatusUnauthorized {
				t.Fatalf("%s 未登录期望 401，实际 %d，响应体=%s", path, resp.Status, truncateBody(resp.Body))
			}
			assertEnvelopeError(t, resp, "unauthenticated")
		})
		t.Run("普通用户 "+path, func(t *testing.T) {
			resp := doJSONAs(t, router, http.MethodGet, path, nil, "user")
			if resp.Status != http.StatusForbidden {
				t.Fatalf("%s 普通用户期望 403，实际 %d，响应体=%s", path, resp.Status, truncateBody(resp.Body))
			}
			assertEnvelopeError(t, resp, "forbidden")
		})
		for _, role := range []string{middleware.RoleAdmin, middleware.RoleSuperAdmin} {
			t.Run("角色 "+role+" "+path, func(t *testing.T) {
				resp := doJSONAs(t, router, http.MethodGet, path, nil, role)
				if resp.Status != http.StatusOK {
					t.Fatalf("%s 角色 %s 期望 200，实际 %d，响应体=%s", path, role, resp.Status, truncateBody(resp.Body))
				}
			})
		}
	}
}

// assertEnvelopeError 断言错误信封里的 error 字段（前端按它分支：unauthenticated → 去登录）
func assertEnvelopeError(t *testing.T, resp jsonResponse, want string) {
	t.Helper()
	var body map[string]interface{}
	if err := json.Unmarshal(resp.Body, &body); err != nil {
		t.Fatalf("错误响应不是合法 JSON: %v，原文=%s", err, truncateBody(resp.Body))
	}
	if body["error"] != want {
		t.Fatalf("error 字段期望 %q，实际 %v（响应体=%s）", want, body["error"], truncateBody(resp.Body))
	}
}

// TestAdminProgressRoutesCoexist 批量接口 `/users/progress` 与单人接口 `/users/:id/progress`
// 在 gin 的路由树里**并存**（静态段优先于通配段），两条都要真的通到各自的处理器：
// 批量那条返回 items，单人那条返回 last7 —— 光看状态码分不出来，所以按响应字段判定。
func TestAdminProgressRoutesCoexist(t *testing.T) {
	router, _, _ := setupAdminRouter(t, mustTime(t, adminTestNowISO))

	batch, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/users/progress?ids=1", nil))
	if code != http.StatusOK {
		t.Fatalf("批量接口期望 200，实际 %d", code)
	}
	if _, ok := batch["items"]; !ok {
		t.Fatalf("/users/progress 没走到批量处理器（响应里没有 items）：%v", mapKeys(batch))
	}

	single, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/admin/users/1/progress", nil))
	if code != http.StatusOK {
		t.Fatalf("单人接口期望 200，实际 %d", code)
	}
	if _, ok := single["last7"]; !ok {
		t.Fatalf("/users/1/progress 没走到单人处理器（响应里没有 last7）：%v", mapKeys(single))
	}
}
