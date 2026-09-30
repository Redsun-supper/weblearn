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

	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/models"
)

// 本文件覆盖 routes.SetupRouter 的真实装配结果（handlers 包的测试用的是等价的手写路由表，
// 一旦有人在 routes.go 里漏注册或改错路径，那边是发现不了的）：
//   - /api/health 这条最简单的通路端到端能返回；
//   - 所有对外承诺的「方法 + 路径」都真的挂在 gin 的路由表上；
//   - 空库下 /api/word-options 返回的是 [] 而不是 null。

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
		&models.User{},
		&models.DataItem{},
		&models.Word{},
		&models.WordReview{},
		&models.ReviewLog{},
	); err != nil {
		t.Fatalf("迁移测试库失败: %v", err)
	}
	return SetupRouter(db)
}

// TestRouterHealthEndpoint 走真实路由的 health 通路：状态码、字段名与取值都要对得上。
// health 的响应体没有 code/data 包裹（与业务接口的结构不同），前端启动时会探这条接口，容易被改错。
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
		"GET /api/user/info",
		"POST /api/user/update",
		"GET /api/data/list",
		"POST /api/data/submit",
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
}

// TestRouterWordsDetailRoutesReachable 给「路由确实生效」补一条真请求的旁证：
// /api/words/:id 命中处理器后，空库下应返回 404 + JSON（{"code":404,"message":"词条不存在"}），
// 而不是 gin 默认的纯文本 404 页面。两者状态码相同，只能靠响应体格式区分。
func TestRouterWordsDetailRoutesReachable(t *testing.T) {
	router := setupRoutesTest(t)

	cases := []struct {
		method string
		path   string
	}{
		{method: http.MethodGet, path: "/api/words/1"},
		{method: http.MethodDelete, path: "/api/words/1"},
	}
	for _, tc := range cases {
		t.Run(tc.method+" "+tc.path, func(t *testing.T) {
			rec := httptest.NewRecorder()
			router.ServeHTTP(rec, httptest.NewRequest(tc.method, tc.path, bytes.NewReader(nil)))
			if rec.Code != http.StatusNotFound {
				t.Fatalf("空库下 %s %s 期望 404，实际 %d，响应体=%s", tc.method, tc.path, rec.Code, rec.Body.String())
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
