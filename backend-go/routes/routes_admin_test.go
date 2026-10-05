// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package routes

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"backend-go/middleware"
)

// 本文件盯住管理看板那组路由的**装配**（不是业务口径——那在 handlers 包测）：
//   - 四条路径都真的挂在 gin 的路由表上；
//   - 整组挂在 RequireAdmin 后面：未登录 401、普通用户 403、admin 与 super_admin 放行
//     （这一条同时钉住「用的是 RequireAdmin，不是 RequireSuperAdmin」）；
//   - `/users/progress` 与 `/users/:id/progress` 这对**看起来会撞**的路径真的各走各的处理器。

// adminPaths 是这一批对外承诺的全部路径（README / 前端 panel 都按这个形状调）
var adminPaths = []string{
	"/api/admin/stats/overview",
	"/api/admin/stats/trend?days=7",
	"/api/admin/users/progress?ids=1",
	"/api/admin/users/1/progress",
}

// TestRouterAdminPathsRegistered 四条路径必须在真实路由表里（查 router.Routes() 而不是发请求看状态码：
// 看板接口在空配置下也会 200，光看状态码分不出「路由没注册」和「注册了但走了降级」）。
func TestRouterAdminPathsRegistered(t *testing.T) {
	router := setupRoutesTest(t)

	registered := make(map[string]bool)
	for _, r := range router.Routes() {
		registered[r.Method+" "+r.Path] = true
	}
	want := []string{
		"GET /api/admin/stats/overview",
		"GET /api/admin/stats/trend",
		"GET /api/admin/users/progress",
		"GET /api/admin/users/:id/progress",
	}
	for _, route := range want {
		if !registered[route] {
			t.Errorf("路由 %q 未注册——路由表被改动或漏注册了", route)
		}
	}
}

// TestRouterAdminRequiresAdmin 看板整组的准入：
// 未登录 401（error=unauthenticated）→ 普通用户 403（error=forbidden）→ 管理员 / 超管 200。
//
// ⚠️ 超管必须放行的原因：这一组挂的是 RequireAdmin。若有人图省事改成 RequireSuperAdmin，
// 症状是「管理员打不开看板」，而两边的单元测试各自都能绿——这里直接把它钉住。
func TestRouterAdminRequiresAdmin(t *testing.T) {
	router := setupRoutesTest(t)

	for _, path := range adminPaths {
		t.Run("未登录 "+path, func(t *testing.T) {
			rec := doRoutes(router, http.MethodGet, path, nil, "", routesTestOrigin, "")
			if rec.Code != http.StatusUnauthorized {
				t.Fatalf("未登录访问 %s 期望 401，实际 %d，响应体=%s", path, rec.Code, rec.Body.String())
			}
			assertErrorField(t, rec, "unauthenticated")
		})

		t.Run("普通用户 "+path, func(t *testing.T) {
			rec := doRoutes(router, http.MethodGet, path, nil, signRoutesToken(t, "user"), routesTestOrigin, "")
			if rec.Code != http.StatusForbidden {
				t.Fatalf("普通用户访问 %s 期望 403，实际 %d，响应体=%s", path, rec.Code, rec.Body.String())
			}
			assertErrorField(t, rec, "forbidden")
		})

		for _, role := range []string{middleware.RoleAdmin, middleware.RoleSuperAdmin} {
			t.Run(role+" "+path, func(t *testing.T) {
				rec := doRoutes(router, http.MethodGet, path, nil, signRoutesToken(t, role), routesTestOrigin, "")
				if rec.Code != http.StatusOK {
					t.Fatalf("角色 %s 访问 %s 期望 200，实际 %d，响应体=%s", role, path, rec.Code, rec.Body.String())
				}
			})
		}
	}
}

// TestRouterAdminProgressPathsCoexist 本批唯一一处「路由可能打架」的地方：
//
// 批量接口是 /users/progress，单人接口是 /users/:id/progress —— 同一个位置既要静态段
// "progress"、又要把它当 :id 用。实测 gin 1.9 允许这样注册：匹配时**静态子节点优先**，
// 静态分支走不通才回退到 :id（tree.go 的 getValue 会给带通配子节点的位置留一份 skippedNode
// 快照用于回溯），所以两条路径都能通，不需要把单人接口挪成 /user-progress/:id。
//
// 这条用例同时钉住两件事：注册不 panic、两条各自走到对的处理器
// （按响应字段区分：批量有 items，单人有 last7 —— 状态码都是 200，光看码分不出来）。
func TestRouterAdminProgressPathsCoexist(t *testing.T) {
	router := setupRoutesTest(t)
	admin := signRoutesToken(t, middleware.RoleAdmin)

	batch := decodeAdminData(t, doRoutes(router, http.MethodGet, "/api/admin/users/progress?ids=1,2", nil, admin, routesTestOrigin, ""))
	if _, ok := batch["items"]; !ok {
		t.Fatalf("/api/admin/users/progress 没走到批量处理器（响应 data 里没有 items）：%v", batch)
	}

	single := decodeAdminData(t, doRoutes(router, http.MethodGet, "/api/admin/users/1/progress", nil, admin, routesTestOrigin, ""))
	if _, ok := single["last7"]; !ok {
		t.Fatalf("/api/admin/users/1/progress 没走到单人处理器（响应 data 里没有 last7）：%v", single)
	}

	// :id 不是数字时由单人处理器回 400（顺带证明 /users/<段>/progress 确实被通配段接走了）
	rec := doRoutes(router, http.MethodGet, "/api/admin/users/abc/progress", nil, admin, routesTestOrigin, "")
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("/api/admin/users/abc/progress 期望 400（单人处理器判非法 id），实际 %d，响应体=%s",
			rec.Code, rec.Body.String())
	}
}

// TestRouterAdminDegradesWithoutAuthDBPath 没配 AUTH_DB_PATH（这里传空 = 本测试装置的口径）时：
// 看板照常 200，只是账号侧标记为不可用 —— 看板打不开比数字是 0 严重得多，
// 而「上游账号服务没起来」在生产上是常态。
func TestRouterAdminDegradesWithoutAuthDBPath(t *testing.T) {
	router := setupRoutesTest(t)
	admin := signRoutesToken(t, middleware.RoleAdmin)

	data := decodeAdminData(t, doRoutes(router, http.MethodGet, "/api/admin/stats/overview", nil, admin, routesTestOrigin, ""))
	authDB, ok := data["auth_db"].(map[string]interface{})
	if !ok {
		t.Fatalf("overview 的 data 里没有 auth_db 对象：%v", data)
	}
	if available, _ := authDB["available"].(bool); available {
		t.Fatalf("没配 AUTH_DB_PATH 时 auth_db.available 应当是 false，实际 %v", authDB)
	}
	if message, _ := authDB["error"].(string); message == "" {
		t.Fatal("auth_db.error 不能为空——管理员要能一眼看出「账号侧为什么全是 0」")
	}
}

// decodeAdminData 解析 {code,message,data} 信封里的 data（顺带断言 HTTP 200 与信封里的 code 200）
func decodeAdminData(t *testing.T, rec *httptest.ResponseRecorder) map[string]interface{} {
	t.Helper()
	if rec.Code != http.StatusOK {
		t.Fatalf("期望 HTTP 200，实际 %d，响应体=%s", rec.Code, rec.Body.String())
	}
	var envelope struct {
		Code int                    `json:"code"`
		Data map[string]interface{} `json:"data"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &envelope); err != nil {
		t.Fatalf("响应不是合法 JSON: %v，原文=%s", err, rec.Body.String())
	}
	if envelope.Code != http.StatusOK {
		t.Fatalf("信封 code 期望 200，实际 %d，响应体=%s", envelope.Code, rec.Body.String())
	}
	return envelope.Data
}
