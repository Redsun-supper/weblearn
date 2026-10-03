package middleware

import (
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/golang-jwt/jwt/v5"
)

// 本文件只测中间件本身的口径（不经过业务处理器）：
//   - 登录校验：缺令牌 / 签名不对 / 算法被改 / 过期 / 过期但在容差内 / 密钥没配；
//   - 角色准入：普通用户碰管理员接口是 403（不是 401）；
//   - CSRF：Origin 白名单、无 Origin 时的 JSON 要求、只读方法放行。
//
// 为什么这些用例值钱：它们钉住的都是「静默放行」类缺陷——算法降级、密钥没配时放行、
// 跨站写操作被放过，这几类问题在手工点页面时几乎发现不了。

const (
	testSecret = "test-only-jwt-secret-for-middleware"
	testOrigin = "http://127.0.0.1:8899"
)

// newProbeRouter 挂一个最小路由：guard 通过就回显中间件写进 Context 的登录态。
// 回显是必要的——只断言 200 的话，「通过了但没写用户 id」这种错会被漏掉。
func newProbeRouter(guard gin.HandlerFunc) *gin.Engine {
	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.Use(gin.Recovery())

	handler := func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{
			"user_id":    c.GetInt64(CtxUserID),
			"session_id": c.GetInt64(CtxSessionID),
			"role":       c.GetString(CtxRole),
		})
	}
	r.GET("/probe", guard, handler)
	r.POST("/probe", guard, func(c *gin.Context) { handler(c) })
	// HEAD / OPTIONS 也登记上：gin 不会为 GET 自动补 HEAD，若这里不登记，
	// 「只读方法放行」的用例就会因为 404（而不是 200）失败，把中间件的问题掩盖成路由的问题。
	r.HEAD("/probe", guard, func(c *gin.Context) { c.Status(http.StatusOK) })
	r.OPTIONS("/probe", guard, func(c *gin.Context) { c.Status(http.StatusNoContent) })
	return r
}

// signToken 签一张与账号服务同构的令牌；expOffset 决定过期时间（负数 = 已过期多久）。
func signToken(t *testing.T, secret, role string, expOffset time.Duration) string {
	t.Helper()
	now := time.Now()
	claims := &AccessClaims{
		Sub:  7,
		Sid:  42,
		Role: role,
		Iat:  now.Add(-time.Minute).Unix(),
		Exp:  now.Add(expOffset).Unix(),
		Jti:  "test-jti",
	}
	token, err := jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString([]byte(secret))
	if err != nil {
		t.Fatalf("签发测试令牌失败: %v", err)
	}
	return token
}

// noneAlgToken 手工拼一个 alg=none 的令牌。
// 为什么要自己拼：jwt 库不允许用 none 签名，而「alg=none 攻击」正是要防的东西，
// 只能绕开库自造一个来验证 WithValidMethods 真的挡住了它。
func noneAlgToken(t *testing.T, role string) string {
	t.Helper()
	header := base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"none","typ":"JWT"}`))
	payload, err := json.Marshal(&AccessClaims{
		Sub:  7,
		Sid:  42,
		Role: role,
		Iat:  time.Now().Add(-time.Minute).Unix(),
		Exp:  time.Now().Add(time.Hour).Unix(),
		Jti:  "none-jti",
	})
	if err != nil {
		t.Fatalf("序列化 payload 失败: %v", err)
	}
	body := base64.RawURLEncoding.EncodeToString(payload)
	return header + "." + body + "."
}

// probe 发一次请求：token 为空表示不带 Cookie，extraHeaders 用来加 Origin / Content-Type。
func probe(router *gin.Engine, method, token string, extraHeaders map[string]string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, "/probe", nil)
	if token != "" {
		req.AddCookie(&http.Cookie{Name: AccessCookieName, Value: token})
	}
	for k, v := range extraHeaders {
		req.Header.Set(k, v)
	}
	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, req)
	return rec
}

// ---------- 登录校验 ----------

// TestRequireUserRejectsMissingCookie 没带 Cookie 时必须 401，
// 且响应体里要有机器可读的 error 字段（前端按它判断「该去登录」）。
func TestRequireUserRejectsMissingCookie(t *testing.T) {
	router := newProbeRouter(RequireUser(testSecret))

	rec := probe(router, http.MethodGet, "", nil)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("没带 Cookie 期望 401，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	var body map[string]interface{}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("401 响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if body["error"] != "unauthenticated" {
		t.Fatalf("error 字段期望 unauthenticated（与账号服务同口径），实际 %v", body["error"])
	}
	if body["code"] != float64(http.StatusUnauthorized) {
		t.Fatalf("code 字段期望 401，实际 %v", body["code"])
	}
}

// TestRequireUserAcceptsValidToken 有效令牌放行，并且用户 id / 会话 id / 角色
// 都要真的写进 Context（处理器靠它取「当前用户是谁」）。
func TestRequireUserAcceptsValidToken(t *testing.T) {
	router := newProbeRouter(RequireUser(testSecret))

	rec := probe(router, http.MethodGet, signToken(t, testSecret, "user", time.Hour), nil)
	if rec.Code != http.StatusOK {
		t.Fatalf("有效令牌期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	var body map[string]interface{}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if body["user_id"] != float64(7) || body["session_id"] != float64(42) {
		t.Fatalf("Context 里的 user_id/session_id 期望 7/42，实际 %v/%v", body["user_id"], body["session_id"])
	}
	if body["role"] != "user" {
		t.Fatalf("Context 里的 role 期望 user，实际 %v", body["role"])
	}
}

// TestRequireUserRejectsWrongSecret 用别的密钥签出来的令牌必须被拒——
// 这是「Go 与账号服务共享同一把 AUTH_JWT_SECRET」这条约定的守门测试：
// 两边密钥不一致时，这里必须红，而不是让请求悄悄通过。
func TestRequireUserRejectsWrongSecret(t *testing.T) {
	router := newProbeRouter(RequireUser(testSecret))

	rec := probe(router, http.MethodGet, signToken(t, "another-secret", "user", time.Hour), nil)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("签名不匹配期望 401，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRequireUserRejectsAlgNone 防「算法降级」：alg=none 的令牌不能被接受。
func TestRequireUserRejectsAlgNone(t *testing.T) {
	router := newProbeRouter(RequireUser(testSecret))

	rec := probe(router, http.MethodGet, noneAlgToken(t, "admin"), nil)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("alg=none 必须被拒（期望 401），实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRequireUserRejectsEmptySecret 服务端没配密钥（本机漏写 .env）时，
// 连「用空密钥签出来的令牌」也不能放行——否则任何人都能自签一张万能令牌。
func TestRequireUserRejectsEmptySecret(t *testing.T) {
	router := newProbeRouter(RequireUser(""))

	rec := probe(router, http.MethodGet, signToken(t, "", "admin", time.Hour), nil)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("服务端密钥为空时必须一律 401，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRequireUserExpiryLeeway 过期判定必须与账号服务的 LEEWAY_SECONDS=60 对齐：
// 刚过期 30 秒仍然放行，过期 2 分钟则拒绝。
// 容差比对不齐会表现为「偶发 401」——客户端刚刷新完令牌，Go 这边却认为是过期。
func TestRequireUserExpiryLeeway(t *testing.T) {
	router := newProbeRouter(RequireUser(testSecret))

	cases := []struct {
		name     string
		offset   time.Duration
		wantCode int
	}{
		{name: "过期 30 秒（在 60 秒容差内）", offset: -30 * time.Second, wantCode: http.StatusOK},
		{name: "过期 120 秒（超出容差）", offset: -120 * time.Second, wantCode: http.StatusUnauthorized},
		{name: "还有 1 小时", offset: time.Hour, wantCode: http.StatusOK},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			rec := probe(router, http.MethodGet, signToken(t, testSecret, "user", tc.offset), nil)
			if rec.Code != tc.wantCode {
				t.Fatalf("exp 偏移 %s 期望 %d，实际 %d，响应体=%s",
					tc.offset, tc.wantCode, rec.Code, rec.Body.String())
			}
		})
	}
}

// ---------- 角色准入 ----------

// TestRequireAdminRoleGate 已登录但角色不是管理员 → 403（不是 401）：
// 401 的意思是「你没登录」，会让前端把人拉去登录页；这里人已经登录了，是权限不够。
func TestRequireAdminRoleGate(t *testing.T) {
	router := newProbeRouter(RequireAdmin(testSecret))

	rec := probe(router, http.MethodGet, signToken(t, testSecret, "user", time.Hour), nil)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("普通用户访问管理员接口期望 403，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	var body map[string]interface{}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("403 响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if body["error"] != "forbidden" {
		t.Fatalf("error 字段期望 forbidden，实际 %v", body["error"])
	}

	rec = probe(router, http.MethodGet, signToken(t, testSecret, "admin", time.Hour), nil)
	if rec.Code != http.StatusOK {
		t.Fatalf("管理员期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}

	// P0-5：超管也能进后台（改造前这里会是 403 —— 「超管被自己的后台挡在门外」）
	rec = probe(router, http.MethodGet, signToken(t, testSecret, RoleSuperAdmin, time.Hour), nil)
	if rec.Code != http.StatusOK {
		t.Fatalf("超级管理员期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	if !CanEnterAdmin(RoleAdmin) || !CanEnterAdmin(RoleSuperAdmin) {
		t.Fatal("CanEnterAdmin 应当同时放行 admin 与 super_admin")
	}
	if CanEnterAdmin("user") || CanEnterAdmin("root") {
		t.Fatal("CanEnterAdmin 不该放行 user / 未知角色")
	}

	// 没登录时是 401 而不是 403（前端要能把「去登录」和「没权限」区分开）
	rec = probe(router, http.MethodGet, "", nil)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("未登录访问管理员接口期望 401，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRequireSuperAdminRoleGate 治理动作只放行超管：
// 普通管理员在这里必须是 403 —— 这是「管理员 = 内容/运营角色，碰不到权限」的实现口径。
func TestRequireSuperAdminRoleGate(t *testing.T) {
	router := newProbeRouter(RequireSuperAdmin(testSecret))

	rec := probe(router, http.MethodGet, signToken(t, testSecret, RoleSuperAdmin, time.Hour), nil)
	if rec.Code != http.StatusOK {
		t.Fatalf("超管期望 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}

	// ⚠️ 关键断言：管理员不是超管，不能做治理动作
	rec = probe(router, http.MethodGet, signToken(t, testSecret, RoleAdmin, time.Hour), nil)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("普通管理员访问治理接口期望 403，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}

	rec = probe(router, http.MethodGet, signToken(t, testSecret, "user", time.Hour), nil)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("普通用户访问治理接口期望 403，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}

	rec = probe(router, http.MethodGet, "", nil)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("未登录访问治理接口期望 401，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
}

// TestRoleStringsMatchTheAuthService 角色字面量是**跨服务契约**。
//
// 这条测试的价值不在于「测出什么逻辑」，而在于**把改动卡在编译期之外的那一处**：
// 账号服务（Rust）写进 users.role 的字符串，Go 这边靠字面量比对。
// 有人把 "super_admin" 改成 "superadmin" 时，两侧各自的测试都不会红
// （各自 mock 自己的字符串），只有线上会出现「后台能进、接口 403」。
func TestRoleStringsMatchTheAuthService(t *testing.T) {
	if RoleAdmin != "admin" {
		t.Fatalf("RoleAdmin 必须与 Rust 的 ROLE_ADMIN 一致，实际 %q", RoleAdmin)
	}
	if RoleSuperAdmin != "super_admin" {
		t.Fatalf("RoleSuperAdmin 必须与 Rust 的 ROLE_SUPER_ADMIN 一致，实际 %q", RoleSuperAdmin)
	}
	// 有下划线、全小写 —— 这是账号服务 0001_init.sql 注释里预留的写法
	if !strings.Contains(RoleSuperAdmin, "_") {
		t.Fatal("super_admin 带下划线，写成 superadmin 就与账号服务对不上了")
	}
}

// ---------- CSRF ----------

// TestCSRFGuardOriginWhitelist 带 Origin 的写请求必须命中白名单，不在名单里一律 403。
func TestCSRFGuardOriginWhitelist(t *testing.T) {
	router := newProbeRouter(CSRFGuard([]string{testOrigin, "http://localhost:8899"}))

	cases := []struct {
		name     string
		origin   string
		wantCode int
	}{
		{name: "白名单内（127.0.0.1）", origin: testOrigin, wantCode: http.StatusOK},
		{name: "白名单内（localhost）", origin: "http://localhost:8899", wantCode: http.StatusOK},
		{name: "外站", origin: "http://evil.example.com", wantCode: http.StatusForbidden},
		{name: "端口不对", origin: "http://127.0.0.1:9000", wantCode: http.StatusForbidden},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			rec := probe(router, http.MethodPost, "", map[string]string{
				"Origin":       tc.origin,
				"Content-Type": "application/json",
			})
			if rec.Code != tc.wantCode {
				t.Fatalf("Origin=%s 期望 %d，实际 %d，响应体=%s",
					tc.origin, tc.wantCode, rec.Code, rec.Body.String())
			}
		})
	}
}

// TestCSRFGuardRequiresJSONWithoutOrigin 不带 Origin 时只认 JSON：
// 跨站表单发不出 application/json，这一条就足以挡住表单型 CSRF。
func TestCSRFGuardRequiresJSONWithoutOrigin(t *testing.T) {
	router := newProbeRouter(CSRFGuard([]string{testOrigin}))

	cases := []struct {
		name        string
		contentType string
		wantCode    int
	}{
		{name: "application/json", contentType: "application/json", wantCode: http.StatusOK},
		{name: "application/json; charset=utf-8", contentType: "application/json; charset=utf-8", wantCode: http.StatusOK},
		{name: "表单", contentType: "application/x-www-form-urlencoded", wantCode: http.StatusForbidden},
		{name: "multipart", contentType: "multipart/form-data", wantCode: http.StatusForbidden},
		{name: "text/plain", contentType: "text/plain", wantCode: http.StatusForbidden},
		{name: "完全不带 Content-Type", contentType: "", wantCode: http.StatusForbidden},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			headers := map[string]string{}
			if tc.contentType != "" {
				headers["Content-Type"] = tc.contentType
			}
			// 显式删掉 Origin：probe 默认不加，这里就是「非浏览器」场景
			rec := probe(router, http.MethodPost, "", headers)
			if rec.Code != tc.wantCode {
				t.Fatalf("Content-Type=%q 且无 Origin 时期望 %d，实际 %d，响应体=%s",
					tc.contentType, tc.wantCode, rec.Code, rec.Body.String())
			}
		})
	}
}

// TestCSRFGuardAllowsSafeMethods GET / HEAD / OPTIONS 不改变状态，即使 Origin 不在白名单也放行。
// （跨站读走的是 CORS 那套限制，不该由 CSRF 闸门兜。）
func TestCSRFGuardAllowsSafeMethods(t *testing.T) {
	router := newProbeRouter(CSRFGuard([]string{testOrigin}))

	cases := []struct {
		method   string
		wantCode int
	}{
		{method: http.MethodGet, wantCode: http.StatusOK},
		{method: http.MethodHead, wantCode: http.StatusOK},
		{method: http.MethodOptions, wantCode: http.StatusNoContent},
	}
	for _, tc := range cases {
		t.Run(tc.method, func(t *testing.T) {
			rec := probe(router, tc.method, "", map[string]string{"Origin": "http://evil.example.com"})
			if rec.Code != tc.wantCode {
				t.Fatalf("%s 应放行（期望 %d），实际 %d，响应体=%s", tc.method, tc.wantCode, rec.Code, rec.Body.String())
			}
		})
	}
}
