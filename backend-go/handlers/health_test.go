// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package handlers

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"

	"backend-go/database"
	"backend-go/middleware"
)

// 本文件盯住健康检查的几条硬口径（外部监控只看 HTTP 状态码，所以状态码本身就是契约）：
//   - 浅检（不带 deep 参数）的响应体**逐字**与改造前一致，不允许多个字段出来；
//   - 深检（?deep=1 / ?deep=true）200 + deep 各项齐全；
//   - **业务库查不了 → 503 + status=degraded**（监控只认状态码，这里必须是 503）；
//   - **账号库读不到 → 仍 200 + status=ok**（P1 定案：账号侧故障只降级，不报错）；
//   - migration_version 是信息项，读的是库里的 user_version，失败也只报 0。

// setupHealthRouter 起一套「内存业务库 + 指定账号库路径 + 只挂 /api/health」的引擎。
//
// 不复用 setupTestRouter：这里要给深检换不同的账号库路径（正常 / 不存在），
// 而那份装置里的账号库路径是写死的空串。路由表本身的真伪由 routes 包的用例盯（它调真实 SetupRouter）。
func setupHealthRouter(t *testing.T, authPath string) (*gin.Engine, *gorm.DB) {
	t.Helper()
	gin.SetMode(gin.TestMode)

	db := newTestDB(t)
	authDB := database.NewAuthDB(authPath)
	// 句柄持有只读连接，用例结束必须关掉：Windows 上留着连接会让 t.TempDir 的清理失败
	t.Cleanup(func() { _ = authDB.Close() })

	h := NewHealthHandler(db, authDB)
	router := gin.New()
	router.Use(gin.Recovery())
	api := router.Group("/api")
	api.Use(middleware.CSRFGuard([]string{testOrigin}))
	api.GET("/health", h.Check)
	return router, db
}

// healthBody 解析 /api/health 的响应体（浅检与深检都没有 code/data 信封）。
//
// 与 setup_test.go 的 decodeFlat 只差一点：深检降级时是 **503**，decodeFlat 会把非 200
// 直接判失败，而这里要能拿到 503 的响应体做断言。
func healthBody(t *testing.T, resp jsonResponse) map[string]interface{} {
	t.Helper()
	var body map[string]interface{}
	if err := json.Unmarshal(resp.Body, &body); err != nil {
		t.Fatalf("health 响应不是合法 JSON: %v，原文=%s", err, truncateBody(resp.Body))
	}
	return body
}

// healthDeepOf 取深检响应里的 deep 对象（没有这个字段就直接判失败）
func healthDeepOf(t *testing.T, body map[string]interface{}) map[string]interface{} {
	t.Helper()
	return healthSubOf(t, body, "deep")
}

// healthAuthOf 取 deep.auth_db 对象
func healthAuthOf(t *testing.T, deep map[string]interface{}) map[string]interface{} {
	t.Helper()
	return healthSubOf(t, deep, "auth_db")
}

// healthSubOf 取一个子对象（字段缺失或类型不对都当场判失败，不留「还好是 nil」的余地）
func healthSubOf(t *testing.T, obj map[string]interface{}, key string) map[string]interface{} {
	t.Helper()
	raw, ok := obj[key]
	if !ok {
		t.Fatalf("响应缺少 %q 字段，实际字段=%v（响应体=%v）", key, mapKeys(obj), obj)
	}
	sub, ok := raw.(map[string]interface{})
	if !ok {
		t.Fatalf("字段 %q 期望对象，实际类型=%T（值=%v）", key, raw, raw)
	}
	return sub
}

// getHealth 发一次 /api/health 请求（path 里自己带查询串）
func getHealth(t *testing.T, router *gin.Engine, path string) jsonResponse {
	t.Helper()
	return doJSON(t, router, http.MethodGet, path, nil)
}

// ---------- 浅检 ----------

// TestHealthShallowKeepsLegacyShape 浅检（默认）必须与改造**一字不差**：
// 只有 status / message 两个字段、HTTP 200。
//
// 这里直接比字节而不是比字段：按字段断言的话，「顺手多塞一个 deep」这种改动会溜过去，
// 而旧前端（admin/ 面板的连通状态）与 docs/deploy-runbook.md 里的 curl 都是按这个形状解析的。
// `?deep=` 系列的其它取值也走浅检：只有 1 / true 算深检，模棱两可的写法一律不深检。
func TestHealthShallowKeepsLegacyShape(t *testing.T) {
	router, _ := setupHealthRouter(t, "")

	const wantBody = `{"message":"服务器运行正常","status":"ok"}`
	for _, path := range []string{"/api/health", "/api/health?deep=", "/api/health?deep=0", "/api/health?deep=yes"} {
		t.Run(path, func(t *testing.T) {
			resp := getHealth(t, router, path)
			if resp.Status != http.StatusOK {
				t.Fatalf("%s 期望 HTTP 200，实际 %d，响应体=%s", path, resp.Status, truncateBody(resp.Body))
			}
			if got := string(resp.Body); got != wantBody {
				t.Fatalf("%s 的响应体必须逐字保持原样\n期望：%s\n实际：%s", path, wantBody, got)
			}
		})
	}
}

// ---------- 深检（一切正常） ----------

// TestHealthDeepOK 深检的健康路径：200 + deep 各项齐全且取值正确。
// 账号库这里是一个真实的临时 auth.db（与看板用例共用同一套建库装置）。
func TestHealthDeepOK(t *testing.T) {
	router, _ := setupHealthRouter(t, newTempAuthDB(t))

	// 1 / true 两种写法都要能进深检
	for _, query := range []string{"?deep=1", "?deep=true", "?deep=TRUE"} {
		t.Run(query, func(t *testing.T) {
			resp := getHealth(t, router, "/api/health"+query)
			if resp.Status != http.StatusOK {
				t.Fatalf("%s 期望 HTTP 200，实际 %d，响应体=%s", query, resp.Status, truncateBody(resp.Body))
			}

			body := healthBody(t, resp)
			assertString(t, body, "status", "ok")
			assertString(t, body, "message", "服务器运行正常")

			deep := healthDeepOf(t, body)
			assertString(t, deep, "database", "ok")
			// 版本必须与 /api/hello 同源（同一个常量），改一处忘一处会让监控里的版本号自相矛盾
			assertString(t, deep, "version", "1.0.0")

			// uptime 是「已经跑了多久」，只要不是负数（时钟倒退才会为负）
			if uptime := toInt64(t, deep["uptime_seconds"]); uptime < 0 {
				t.Fatalf("uptime_seconds 不该是负数，实际 %d", uptime)
			}
			// migration_version 是信息项：字段必须在、必须是数字（值本身见下一条用例）
			if _, ok := deep["migration_version"].(float64); !ok {
				t.Fatalf("migration_version 期望 JSON 数字，实际 %T（值=%v）",
					deep["migration_version"], deep["migration_version"])
			}

			auth := healthAuthOf(t, deep)
			assertBool(t, auth, "available", true)
			// 可用时 error 必须是 **JSON null**（不是空串）：监控脚本按 null 判断「没出问题」
			assertNull(t, auth, "error")
		})
	}
}

// TestHealthDeepReportsMigrationVersion 深检报的 migration_version 必须**真的来自库里**，
// 而不是写死的 0：这里先把测试库的 user_version 改成 7，再要求深检报 7。
//
// 口径来源：账号服务用 `PRAGMA user_version` 记迁移进度（backend-rust/src/db.rs），
// 业务库沿用同一个约定（见 database.MigrationVersion）。guangxue.db 目前没人写过这个值，
// 所以线上读回来是 0 —— 那是「没打过版本标记」，本用例同时钉住「0 也不能降级」。
func TestHealthDeepReportsMigrationVersion(t *testing.T) {
	router, db := setupHealthRouter(t, newTempAuthDB(t))

	// 没打过标记的库：报 0，且**仍然 200 / status=ok**（信息项读不到不等于坏）
	resp := getHealth(t, router, "/api/health?deep=1")
	if resp.Status != http.StatusOK {
		t.Fatalf("未被标记的库不该让深检降级，实际 %d，响应体=%s", resp.Status, truncateBody(resp.Body))
	}
	assertInt(t, healthDeepOf(t, healthBody(t, resp)), "migration_version", 0)

	if err := db.Exec("PRAGMA user_version = 7").Error; err != nil {
		t.Fatalf("写入测试库的 user_version 失败: %v", err)
	}

	resp = getHealth(t, router, "/api/health?deep=1")
	if resp.Status != http.StatusOK {
		t.Fatalf("期望 HTTP 200，实际 %d，响应体=%s", resp.Status, truncateBody(resp.Body))
	}
	assertInt(t, healthDeepOf(t, healthBody(t, resp)), "migration_version", 7)
}

// ---------- 深检（账号库读不到：降级但不失败） ----------

// TestHealthDeepWithoutAuthDBStillHealthy 账号库读不到时：HTTP **仍 200**、status **仍 ok**、
// 业务库照样报 ok，只有 deep.auth_db 标成不可用并给出原因。
//
// 这是 P1 已定案的口径（账号库读不到时看板降级成 0 + 一句原因，不让整页报错）。
// 深检若在这里回 503，「账号服务重启」这种日常状态会每 5 分钟报一次警。
func TestHealthDeepWithoutAuthDBStillHealthy(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "not-here", "auth.db")
	router, _ := setupHealthRouter(t, missing)

	resp := getHealth(t, router, "/api/health?deep=1")
	if resp.Status != http.StatusOK {
		t.Fatalf("账号库读不到不该让深检失败（P1 口径），实际 %d，响应体=%s",
			resp.Status, truncateBody(resp.Body))
	}

	body := healthBody(t, resp)
	assertString(t, body, "status", "ok")
	assertString(t, body, "message", "服务器运行正常")

	deep := healthDeepOf(t, body)
	assertString(t, deep, "database", "ok")

	auth := healthAuthOf(t, deep)
	assertBool(t, auth, "available", false)
	reason, _ := auth["error"].(string)
	if strings.TrimSpace(reason) == "" {
		t.Fatalf("auth_db.available=false 时 error 必须给出原因，实际 %v", auth["error"])
	}
	// 原因与看板同一句（共用 database.AuthDBUnavailableReason）：要能看出读的是哪个库
	if !strings.Contains(reason, missing) {
		t.Fatalf("auth_db.error 里应当出现账号库路径 %q，实际：%q", missing, reason)
	}

	// 只读打开**不能**把不存在的账号库顺手建出来（那会变成一个「读得到但任何表都没有」的空库）
	if _, err := os.Stat(missing); err == nil {
		t.Fatalf("只读访问把不存在的账号库创建出来了：%s", missing)
	}
}

// ---------- 深检（业务库坏掉：503） ----------

// TestHealthDeepDegradedWhenBusinessDBBroken 业务库查不了时：
// **HTTP 503** + status=degraded + message 说清是业务库坏了。
//
// 状态码是这条用例的重点：外部监控只认状态码，回 200 带一句「不可用」等于没告警。
// 做法是把底层 *sql.DB 关掉，让 SELECT 1 真失败（比换一个「指向不存在文件的句柄」可靠 ——
// glebarez 驱动默认会 CREATE，指向不存在的文件反而不报错）。
func TestHealthDeepDegradedWhenBusinessDBBroken(t *testing.T) {
	router, db := setupHealthRouter(t, newTempAuthDB(t))

	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("取底层 sql.DB 失败: %v", err)
	}
	if err := sqlDB.Close(); err != nil {
		t.Fatalf("关闭测试库失败: %v", err)
	}

	resp := getHealth(t, router, "/api/health?deep=1")
	if resp.Status != http.StatusServiceUnavailable {
		t.Fatalf("业务库不可用时深检必须回 503，实际 %d，响应体=%s", resp.Status, truncateBody(resp.Body))
	}

	body := healthBody(t, resp)
	assertString(t, body, "status", "degraded")

	message, _ := body["message"].(string)
	if !strings.Contains(message, "业务库") || !strings.Contains(message, "guangxue.db") {
		t.Fatalf("message 要说清是哪一项坏了（中文、带上库名），实际：%q", message)
	}

	deep := healthDeepOf(t, body)
	databaseField, _ := deep["database"].(string)
	if !strings.HasPrefix(databaseField, "error:") {
		t.Fatalf("deep.database 期望以 %q 开头，实际 %q", "error:", databaseField)
	}

	// 账号库是好的：这一条同时证明 503 只由「自身检查」触发，没有牵连账号侧
	assertBool(t, healthAuthOf(t, deep), "available", true)
}
