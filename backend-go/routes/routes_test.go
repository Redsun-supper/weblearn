// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package routes

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/golang-jwt/jwt/v5"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/config"
	"backend-go/middleware"
	"backend-go/models"
)

// 本文件覆盖 routes.SetupRouter 的真实装配结果（handlers 包的测试用的是等价的手写路由表，
// 一旦有人在 routes.go 里漏注册、改错路径、漏挂中间件，那边是发现不了的）：
//   - /api/health 这条最简单的通路端到端能返回；
//   - 所有对外承诺的「方法 + 路径」都真的挂在 gin 的路由表上，已删除的占位接口确实不在；
//   - 空库下 /api/word-options 返回的是 [] 而不是 null；
//   - 登录门槛（/api/reviews/* 要登录、词条写接口要管理员）与 CSRF 闸门真的接在路由上。

const (
	routesTestSecret = "test-only-jwt-secret-for-routes"
	routesTestOrigin = "http://127.0.0.1:8899"
)

// testDBCounter 与 handlers 包同款：每个用例一个独立的内存库名，用例之间互不干扰
var testDBCounter atomic.Int64

// setupRoutesTest 建一个只装了生产 models 的内存库，返回按生产方式装配好的路由
func setupRoutesTest(t *testing.T) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)

	dsn := fmt.Sprintf("file:guangxue_routes_test_%d?mode=memory&cache=shared", testDBCounter.Add(1))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{
		Logger: logger.Default.LogMode(logger.Silent),
	})
	if err != nil {
		t.Fatalf("打开内存数据库失败: %v", err)
	}
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("取 sql.DB 失败: %v", err)
	}
	// 内存库靠常驻连接存活，连接池压到 1 保证所有查询看到同一个库
	sqlDB.SetMaxOpenConns(1)
	sqlDB.SetMaxIdleConns(1)
	t.Cleanup(func() { _ = sqlDB.Close() })

	if err := db.AutoMigrate(
		&models.Word{},
		&models.WordReview{},
		&models.ReviewLog{},
	); err != nil {
		t.Fatalf("迁移测试库失败: %v", err)
	}
	return SetupRouter(db, testConfig())
}

// testConfig 只填与中间件相关的两项：验签密钥与 CSRF 白名单
func testConfig() *config.Config {
	return &config.Config{
		Env:            "test",
		JWTSecret:      routesTestSecret,
		AllowedOrigins: []string{routesTestOrigin},
	}
}

// signRoutesToken 签一张测试令牌（与账号服务同构：sub/sid/role/iat/exp/jti）
func signRoutesToken(t *testing.T, role string) string {
	t.Helper()
	now := time.Now()
	claims := &middleware.AccessClaims{
		Sub:  1,
		Sid:  2,
		Role: role,
		Iat:  now.Add(-time.Minute).Unix(),
		Exp:  now.Add(time.Hour).Unix(),
		Jti:  "routes-test",
	}
	token, err := jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString([]byte(routesTestSecret))
	if err != nil {
		t.Fatalf("签发测试令牌失败: %v", err)
	}
	return token
}

// doRoutes 发一次请求；token 为空表示不带 Cookie，origin 为空表示不带 Origin
func doRoutes(router *gin.Engine, method, path string, body []byte, token, origin, contentType string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, path, bytes.NewReader(body))
	if token != "" {
		req.AddCookie(&http.Cookie{Name: middleware.AccessCookieName, Value: token})
	}
	if origin != "" {
		req.Header.Set("Origin", origin)
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, req)
	return rec
}

// TestRouterHealthEndpoint 走真实路由的 health 通路：状态码、字段名与取值都要对得上。
// health 的响应体没有 code/data 包裹（与业务接口的结构不同），前端启动时会探这条接口，容易被改错。
// 它是少数几个**不需要登录**的接口，所以这条用例同时钉住「探活不会被鉴权挡掉」。
func TestRouterHealthEndpoint(t *testing.T) {
	router := setupRoutesTest(t)

	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/api/health", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("GET /api/health 期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	var body map[string]interface{}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("health 响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if body["status"] != "ok" {
		t.Fatalf("health 的 status 期望 ok，实际 %v", body["status"])
	}
	if _, ok := body["message"]; !ok {
		t.Fatalf("health 响应缺少 message 字段，实际字段=%v", body)
	}
}

// TestRouterRegistersAllPaths 路由表回归：每条已在 README / 前端里使用的「方法 + 路径」
// 都必须真的注册在 gin 的引擎上。
// 这里查 router.Routes() 而不是发请求看状态码——因为 /api/words/:id 这类接口在空库下
// 本身就会返回 404（“词条不存在”），用状态码判断会把「路由没注册」和「资源不存在」混为一谈。
func TestRouterRegistersAllPaths(t *testing.T) {
	router := setupRoutesTest(t)

	registered := make(map[string]bool)
	for _, r := range router.Routes() {
		registered[r.Method+" "+r.Path] = true
	}

	want := []string{
		"GET /api/health",
		"GET /api/hello",
		"GET /api/word-options",
		"GET /api/words",
		"POST /api/words",
		"GET /api/words/:id",
		"PUT /api/words/:id",
		"DELETE /api/words/:id",
		"GET /api/reviews/due",
		"GET /api/reviews/new",
		"GET /api/reviews/queue",
		"GET /api/reviews/probes",
		"GET /api/reviews/stats",
		"POST /api/reviews/submit",
	}
	for _, route := range want {
		if !registered[route] {
			t.Errorf("路由 %q 未注册——路由表被改动或漏注册了", route)
		}
	}

	// 反向断言：已删除的占位接口不能复活（它们与账号服务的 users 表同名不同源，
	// 一旦有人照着旧文档加回来，这里会立刻红）
	gone := []string{
		"GET /api/user/info",
		"POST /api/user/update",
		"GET /api/data/list",
		"POST /api/data/submit",
	}
	for _, route := range gone {
		if registered[route] {
			t.Errorf("占位路由 %q 不该存在（已随 models.User / models.DataItem 一起删除）", route)
		}
	}
}

// TestRouterWordsDetailRoutesReachable 给「路由确实生效」补一条真请求的旁证：
// /api/words/:id 命中处理器后，空库下应返回 404 + JSON（{"code":404,"message":"词条不存在"}），
// 而不是 gin 默认的纯文本 404 页面。两者状态码相同，只能靠响应体格式区分。
// DELETE 需要管理员令牌，所以要带着 Cookie 发——否则会先被中间件拦成 401。
func TestRouterWordsDetailRoutesReachable(t *testing.T) {
	router := setupRoutesTest(t)
	admin := signRoutesToken(t, middleware.RoleAdmin)

	cases := []struct {
		method string
		token  string
	}{
		{method: http.MethodGet, token: ""},
		{method: http.MethodDelete, token: admin},
	}
	for _, tc := range cases {
		t.Run(tc.method+" /api/words/1", func(t *testing.T) {
			rec := doRoutes(router, tc.method, "/api/words/1", nil, tc.token, routesTestOrigin, "")
			if rec.Code != http.StatusNotFound {
				t.Fatalf("空库下 %s /api/words/1 期望 404，实际 %d，响应体=%s", tc.method, rec.Code, rec.Body.String())
			}
			if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "application/json") {
				t.Fatalf("404 应来自业务处理器（JSON），实际 Content-Type=%q，响应体=%s", ct, rec.Body.String())
			}
		})
	}
}

// TestRouterWordOptionsNotEmptyList 空库下 /api/word-options 的两个列表必须是 JSON 数组而不是 null：
// 前端（modules/english/admin/english-admin.js）直接按数组用，null 会把它打挂。
// 注意该接口只返回 books / units，没有 subjects（词条所属主题没有做下拉）。
func TestRouterWordOptionsNotEmptyList(t *testing.T) {
	router := setupRoutesTest(t)

	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/api/word-options", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("GET /api/word-options 期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}

	var envelope struct {
		Code int `json:"code"`
		Data struct {
			Books []string `json:"books"`
			Units []string `json:"units"`
		} `json:"data"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &envelope); err != nil {
		t.Fatalf("word-options 响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if envelope.Data.Books == nil || envelope.Data.Units == nil {
		t.Fatalf("空库下 books / units 都必须是 []（不能是 null），实际=%s", rec.Body.String())
	}
}

// TestRouterReviewsRequireLogin 复习接口（读 + 写）全部要登录：
// 未登录 401 且响应体带 error=unauthenticated（前端据此跳登录页），带令牌则正常放行。
func TestRouterReviewsRequireLogin(t *testing.T) {
	router := setupRoutesTest(t)

	readPaths := []string{
		"/api/reviews/due",
		"/api/reviews/new",
		"/api/reviews/queue",
		"/api/reviews/probes",
		"/api/reviews/stats",
	}
	for _, path := range readPaths {
		t.Run("未登录 GET "+path, func(t *testing.T) {
			rec := doRoutes(router, http.MethodGet, path, nil, "", routesTestOrigin, "")
			if rec.Code != http.StatusUnauthorized {
				t.Fatalf("未登录访问 %s 期望 401，实际 %d，响应体=%s", path, rec.Code, rec.Body.String())
			}
			assertErrorField(t, rec, "unauthenticated")
		})
	}

	t.Run("未登录 POST /api/reviews/submit", func(t *testing.T) {
		rec := doRoutes(router, http.MethodPost, "/api/reviews/submit", []byte(`{"word_id":1,"rating":3}`),
			"", routesTestOrigin, "application/json")
		if rec.Code != http.StatusUnauthorized {
			t.Fatalf("未登录提交复习期望 401，实际 %d，响应体=%s", rec.Code, rec.Body.String())
		}
		assertErrorField(t, rec, "unauthenticated")
	})

	t.Run("已登录 GET /api/reviews/stats", func(t *testing.T) {
		rec := doRoutes(router, http.MethodGet, "/api/reviews/stats", nil,
			signRoutesToken(t, "user"), routesTestOrigin, "")
		if rec.Code != http.StatusOK {
			t.Fatalf("已登录访问统计期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
		}
	})
}

// TestRouterWordsWriteRequiresAdmin 词条写接口要管理员：
// 普通用户 403（error=forbidden，说明「登录了但没权限」）、管理员放行（不返回 401/403）。
func TestRouterWordsWriteRequiresAdmin(t *testing.T) {
	router := setupRoutesTest(t)
	body := []byte(`{"words":[{"word":"admin-probe"}]}`)

	rec := doRoutes(router, http.MethodPost, "/api/words", body,
		signRoutesToken(t, "user"), routesTestOrigin, "application/json")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("普通用户写词条期望 403，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	assertErrorField(t, rec, "forbidden")

	rec = doRoutes(router, http.MethodPost, "/api/words", body,
		signRoutesToken(t, middleware.RoleAdmin), routesTestOrigin, "application/json")
	if rec.Code == http.StatusUnauthorized || rec.Code == http.StatusForbidden {
		t.Fatalf("管理员写词条不该被拦，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRouterCSRFBlocksCrossSiteWrite 跨站写请求（Origin 不在白名单）要被 CSRF 闸门挡在业务处理器之前。
// 用「删词条」这类真实写接口验证，而不是拿一个假路由：要证明闸门确实挂在 /api 上。
func TestRouterCSRFBlocksCrossSiteWrite(t *testing.T) {
	router := setupRoutesTest(t)
	admin := signRoutesToken(t, middleware.RoleAdmin)

	rec := doRoutes(router, http.MethodPost, "/api/reviews/submit", []byte(`{"word_id":1,"rating":3}`),
		signRoutesToken(t, "user"), "http://evil.example.com", "application/json")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("外站 Origin 提交复习期望 403，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	assertErrorField(t, rec, "forbidden")

	rec = doRoutes(router, http.MethodPost, "/api/words", []byte(`{"words":[{"word":"csrf-probe"}]}`),
		admin, "http://evil.example.com", "application/json")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("外站 Origin 写词条期望 403，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// assertErrorField 断言错误响应体里的 error 字段（前端按它分支：unauthenticated → 去登录）。
func assertErrorField(t *testing.T, rec *httptest.ResponseRecorder, want string) {
	t.Helper()
	var body map[string]interface{}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("错误响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if body["error"] != want {
		t.Fatalf("error 字段期望 %q，实际 %v（响应体=%s）", want, body["error"], rec.Body.String())
	}
}

// TestGinModeFollowsAppEnv 生产不能跑在 gin 的 debug 模式：
// debug 会打印路由表、每个请求一行 [GIN]，日志量翻好几倍且会进 journald 长期留着。
//
// 口径与 Rust 侧的 is_production 一致：**只有小写 `production` 算生产**，
// 其余（含大小写写错的值）一律当开发 —— 免得「本机调试时日志突然消失」查不出原因。
func TestGinModeFollowsAppEnv(t *testing.T) {
	cases := map[string]string{
		"production":  gin.ReleaseMode,
		"development": gin.DebugMode,
		"Production":  gin.DebugMode, // 大小写写错 → 当开发，宁可多打日志
		"prod":        gin.DebugMode,
		"":            gin.DebugMode,
	}
	for env, want := range cases {
		if got := ginMode(env); got != want {
			t.Errorf("APP_ENV=%q 应当用 %s，实际 %s", env, want, got)
		}
	}
}

// TestRouterProductionModeStillServesHealth 生产模式（APP_ENV=production）下路由照常工作。
// 这条盯的是「切 ReleaseMode 时把东西切坏了」——SetMode 是全局状态，最容易顺手改错。
func TestRouterProductionModeStillServesHealth(t *testing.T) {
	t.Cleanup(func() { gin.SetMode(gin.TestMode) })

	dsn := fmt.Sprintf("file:guangxue_routes_prod_%d?mode=memory&cache=shared", testDBCounter.Add(1))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{Logger: logger.Default.LogMode(logger.Silent)})
	if err != nil {
		t.Fatalf("打开内存数据库失败: %v", err)
	}
	sqlDB, _ := db.DB()
	sqlDB.SetMaxOpenConns(1)
	t.Cleanup(func() { _ = sqlDB.Close() })

	cfg := testConfig()
	cfg.Env = "production"
	router := SetupRouter(db, cfg)

	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/api/health", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("生产模式下 GET /api/health 期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRouterPathPrefixBoundary 是给 Nginx 反代配错准备的护栏。
//
// 线上靠**路径前缀**分流：/api/auth/* 转给账号服务（8081）、/api/* 转给 Go（8080）。
// 所以 Go 这边**绝不能**有「也能被 /api/auth 前缀匹配上」的路径 —— 一旦有，
// 就会出现「本机一切正常、线上某几个接口 404 或 401」这种最难查的现象。
// 这条用例把边界钉住：这些路径必须压根不存在（gin 默认 404 纯文本）。
func TestRouterPathPrefixBoundary(t *testing.T) {
	router := setupRoutesTest(t)

	for _, path := range []string{"/api/auth/health", "/apiauth/health", "/api-auth/health", "/health"} {
		t.Run(path, func(t *testing.T) {
			rec := httptest.NewRecorder()
			router.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, path, nil))
			if rec.Code != http.StatusNotFound {
				t.Fatalf("%s 不该由 Go 处理（该由 Nginx 分流给账号服务，或根本不存在），实际 %d，响应体=%s",
					path, rec.Code, rec.Body.String())
			}
			// 必须是 gin 自己的 404（纯文本），而不是某个业务处理器的 JSON 404
			if ct := rec.Header().Get("Content-Type"); strings.HasPrefix(ct, "application/json") {
				t.Fatalf("%s 被某个业务处理器接走了（Content-Type=%s），前缀边界被破坏", path, ct)
			}
		})
	}
}
