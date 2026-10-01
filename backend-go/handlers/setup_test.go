package handlers

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/golang-jwt/jwt/v5"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"backend-go/middleware"
	"backend-go/models"
)

// 本包测试用的固定密钥与来源：与生产同款的 middleware 包吃这两个值。
// 用固定值而不是随机值，是为了让「签名不对」「角色不对」这类用例一眼能看懂。
const (
	testJWTSecret = "test-only-jwt-secret-for-handlers"
	testOrigin    = "http://127.0.0.1:8899"

	// testUserID 是装置默认的「当前登录用户」；testOtherUserID 用来验证「各人的进度互不可见」。
	// 值本身任意（Go 侧只把它当账号服务 auth.db 的 users.id 用），但必须 > 0：0 是 P0-1 迁移前
	// 历史数据的占位值，middleware.CurrentUserID 会直接把它判成「未登录」。
	testUserID      = 1
	testOtherUserID = 2
)

// 本文件是 handlers 包测试的公共装置：内存库 + 生产路由 + 造数与请求小工具。
//
// 口径说明（为什么这么搭）：
//   - 建表必须走 AutoMigrate 且复用生产同款 models，不另写一份 DDL，否则测试库与线上库会悄悄漂移；
//   - 内存库命名用「进程内自增计数」，不能用 t.Name()——测试名里带斜杠会让 DSN 的 file: 段
//     没法按「文件名 + 查询串」解析（sqlite 驱动会直接报错），而且隔离性也不如独立名字；
//   - 用 cache=shared 保证 GORM 连接池里新开的连接看到的是同一个内存库（mode=memory 在连接
//     全关时会销毁，所以还要把连接池压到 1，见下面 SetMaxOpenConns 的注释）；
//   - 一律走 HTTP 层（httptest + 生产路由），不直接调私有函数，这样测住的是「接口口径」
//     而不是「实现细节」，将来重构内部函数测试不会虚假地红。

// testDBCounter 给每个测试实例分配独立的内存库名字，保证用例之间互不干扰
var testDBCounter atomic.Int64

// setupTestRouter 起一套「内存 SQLite + 生产路由」的测试环境。
// 返回的 *gorm.DB 用来直接造数据 / 核对落库结果（跨过 HTTP 层看真实写入）。
func setupTestRouter(t *testing.T) (*gin.Engine, *gorm.DB) {
	t.Helper()

	// 测试环境关掉 gin 的启动日志与请求日志，只保留 panic 兜底
	gin.SetMode(gin.TestMode)

	name := fmt.Sprintf("guangxue_test_%d", testDBCounter.Add(1))
	dsn := "file:" + name + "?mode=memory&cache=shared"
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{
		// 测试里 ErrRecordNotFound 是正常控制流（判断新词 / 新卡），不打印
		Logger: logger.Default.LogMode(logger.Silent),
	})
	if err != nil {
		t.Fatalf("打开内存数据库失败: %v", err)
	}

	// 表结构走生产同一套 AutoMigrate。故意逐表列出而不是 db.AutoMigrate(&models.User{}, ...)，
	// 是为了让「某张表没建上」在测试里直接暴露，而不是拖到查询时报 no such table。
	if err := db.AutoMigrate(
		&models.Word{},
		&models.WordReview{},
		&models.ReviewLog{},
	); err != nil {
		t.Fatalf("内存库自动迁移失败: %v", err)
	}

	// 连接池压到 1 条：内存库靠这条常驻连接活着（连接全关则库被销毁），
	// 顺带让所有查询串行，避免共享缓存下的并发写干扰断言。
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("取出底层 sql.DB 失败: %v", err)
	}
	sqlDB.SetMaxOpenConns(1)
	sqlDB.SetMaxIdleConns(1)
	t.Cleanup(func() {
		_ = sqlDB.Close()
	})

	router := newTestRouter(t, db)
	return router, db
}

// newTestRouter 只挂生产路由与生产中间件，不挂 gin.Default() 的 Logger 中间件
// （Default 会把每个请求的日志打到 stdout，淹没测试输出）。
func newTestRouter(t *testing.T, db *gorm.DB) *gin.Engine {
	t.Helper()

	// 完全复刻 routes.SetupRouter 的路径、处理器绑定与中间件（CSRF 闸门 + 登录校验）。
	// 这里没有直接调用 routes.SetupRouter：它用的是 gin.Default()，会把每个请求的
	// 访问日志与启动横幅打到测试输出里。因此本文件维护了一份路由表的副本——
	// 它是**测试专用的有意副本**：路由表或中间件变动时这里要同步，否则用例会因为
	// 404 / 401 / 403 变红，从而逼着人把它改回去（比「悄悄测不到」安全）。
	// 反过来，routes 包的测试会调真实的 SetupRouter，两边互为补丁。
	router := gin.New()
	router.Use(gin.Recovery())

	api := router.Group("/api")
	api.Use(middleware.CSRFGuard([]string{testOrigin}))
	rv := NewReviewHandler(db)
	api.GET("/health", HealthCheck)
	api.GET("/hello", Hello)
	words := api.Group("/words")
	{
		words.GET("", rv.ListWords)
		words.POST("", middleware.RequireAdmin(testJWTSecret), rv.AddWords)
		words.GET("/:id", rv.GetWord)
		words.PUT("/:id", middleware.RequireAdmin(testJWTSecret), rv.UpdateWord)
		words.DELETE("/:id", middleware.RequireAdmin(testJWTSecret), rv.DeleteWord)
	}
	reviews := api.Group("/reviews", middleware.RequireUser(testJWTSecret))
	{
		reviews.GET("/due", rv.DueReviews)
		reviews.GET("/new", rv.NewWords)
		reviews.GET("/queue", rv.QueueReviews)
		reviews.GET("/probes", rv.ProbeCandidates)
		reviews.POST("/submit", rv.SubmitReview)
		reviews.GET("/stats", rv.ReviewStats)
	}
	api.GET("/word-options", rv.WordOptions)
	return router
}

// ---------- 造数 ----------

// seedWord 往 words 表插一条词条，返回其 id。
// book / unit 走默认值（只影响后台分组），需要的话用 seedWordWith。
func seedWord(t *testing.T, db *gorm.DB, word string) uint {
	t.Helper()
	return seedWordWith(t, db, models.Word{Word: word})
}

// seedWordWith 插入自定义字段的词条，Subject 留空时与 POST /api/words 一样补成 english
func seedWordWith(t *testing.T, db *gorm.DB, w models.Word) uint {
	t.Helper()
	if w.Subject == "" {
		w.Subject = "english"
	}
	if err := db.Create(&w).Error; err != nil {
		t.Fatalf("造词条 %q 失败: %v", w.Word, err)
	}
	return w.ID
}

// seedReview 直接插一条 FSRS 记忆状态（绕过 HTTP，用于构造「已学 / 到期 / 未到期」的初始局面）。
// 归属 testUserID（装置默认的当前登录用户）；要造别人的进度用 seedReviewFor。
// dueAt 传 nil 表示 due_at 为空——生产里不会出现（提交时必写），用来测各接口对脏数据的过滤。
func seedReview(t *testing.T, db *gorm.DB, wordID uint, stability, difficulty float64, dueAt *time.Time, reps, lapses uint) {
	t.Helper()
	seedReviewFor(t, db, testUserID, wordID, stability, difficulty, dueAt, reps, lapses)
}

// seedReviewFor 是指定归属用户的版本：P0-1 之后，同一个词每个用户各有一行进度。
func seedReviewFor(t *testing.T, db *gorm.DB, userID, wordID uint, stability, difficulty float64, dueAt *time.Time, reps, lapses uint) {
	t.Helper()
	row := models.WordReview{
		UserID:           userID,
		WordID:           wordID,
		Stability:        stability,
		Difficulty:       difficulty,
		DesiredRetention: 0.9,
		DueAt:            dueAt,
		Reps:             reps,
		Lapses:           lapses,
	}
	if dueAt != nil {
		row.LastReviewAt = dueAt
	}
	if err := db.Create(&row).Error; err != nil {
		t.Fatalf("造复习状态 user_id=%d word_id=%d 失败: %v", userID, wordID, err)
	}
}

// seedLog 直接插一条复习日志（归属 testUserID；要造别人的日志用 seedLogFor）。
// reviewedAt 由调用方指定成「今天 / 昨天 / …」的自然日时间点——跨天口径就靠这个构造，
// 不去 mock 系统时钟：被测代码（ReviewStats）本来就用 time.Now() 算今日零点，
// 我们只把「历史数据」摆到它该在的位置，这样测的是真实的时间比较逻辑。
func seedLog(t *testing.T, db *gorm.DB, wordID uint, rating uint8, stabilityBefore, stabilityAfter, intervalDays float64, reviewedAt time.Time, isProbe bool) {
	t.Helper()
	seedLogFor(t, db, testUserID, wordID, rating, stabilityBefore, stabilityAfter, intervalDays, reviewedAt, isProbe)
}

// seedLogFor 是指定归属用户的复习日志。
func seedLogFor(t *testing.T, db *gorm.DB, userID, wordID uint, rating uint8, stabilityBefore, stabilityAfter, intervalDays float64, reviewedAt time.Time, isProbe bool) {
	t.Helper()
	row := models.ReviewLog{
		UserID:          userID,
		WordID:          wordID,
		Rating:          rating,
		StabilityBefore: stabilityBefore,
		StabilityAfter:  stabilityAfter,
		DifficultyAfter: 5,
		IntervalDays:    intervalDays,
		ReviewedAt:      reviewedAt,
		IsProbe:         isProbe,
	}
	if err := db.Create(&row).Error; err != nil {
		t.Fatalf("造复习日志 user_id=%d word_id=%d 失败: %v", userID, wordID, err)
	}
}

// ---------- 鉴权小工具 ----------

// testAccessToken 签一张 testUserID 的令牌（绝大多数用例用它）
func testAccessToken(t *testing.T, role string) string {
	t.Helper()
	return testAccessTokenFor(t, testUserID, role)
}

// testAccessTokenFor 签一张用 testJWTSecret 签名的访问令牌，结构与账号服务的 AccessClaims 保持同构
// （sub/sid 是 JSON 数字、role 取 "user"/"admin"、exp 带一小时有效期、jti 固定便于排查）。
// 用固定结构而不是随机值，是为了让「签名不对」「角色不对」这类用例一眼能看懂差别在哪。
// userID 就是令牌里的 sub：Go 侧一切「这是谁」的判断都只认它（请求体里没有 user_id 字段）。
func testAccessTokenFor(t *testing.T, userID int64, role string) string {
	t.Helper()

	now := time.Now()
	claims := &middleware.AccessClaims{
		Sub:  userID,
		Sid:  userID*10 + 1, // 会话 id 与用户绑一下，排查时一眼能对上
		Role: role,
		Iat:  now.Add(-time.Minute).Unix(),
		Exp:  now.Add(time.Hour).Unix(),
		Jti:  "handlers-test",
	}
	token, err := jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString([]byte(testJWTSecret))
	if err != nil {
		t.Fatalf("签发测试令牌失败: %v", err)
	}
	return token
}

// ---------- 请求与断言小工具 ----------

// doJSON 发一个 JSON 请求（body 为 nil 时无请求体），返回响应。
// 元组用命名参数，调用处一眼能看出哪个是状态码。
type jsonResponse struct {
	Status int
	Body   []byte
}

// testRequest 描述一次请求的身份与头部；零值 = 匿名且不带 Origin（专门用来测中间件拦截）。
type testRequest struct {
	UserID      int64  // 令牌里的 sub（0 = 用 testUserID）；只在 Role 非空时有意义
	Role        string // 空串表示不带 Cookie（匿名）；否则用该角色签一张令牌
	Origin      string // 空串表示不带 Origin
	ContentType string
}

// doRequest 是本文件唯一的发请求实现，其余小工具都是它的薄封装。
// 走 HTTP 层、走真实路由，因此中间件（CSRF 闸门、登录校验）与处理器都会被真实触发。
func doRequest(t *testing.T, router *gin.Engine, method, path string, body []byte, opts testRequest) jsonResponse {
	t.Helper()

	var reader *bytes.Reader
	if body == nil {
		reader = bytes.NewReader(nil)
	} else {
		reader = bytes.NewReader(body)
	}
	req := httptest.NewRequest(method, path, reader)
	if opts.Role != "" {
		uid := opts.UserID
		if uid == 0 {
			uid = testUserID
		}
		req.AddCookie(&http.Cookie{
			Name:  middleware.AccessCookieName,
			Value: testAccessTokenFor(t, uid, opts.Role),
		})
	}
	if opts.Origin != "" {
		req.Header.Set("Origin", opts.Origin)
	}
	if opts.ContentType != "" {
		req.Header.Set("Content-Type", opts.ContentType)
	}
	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, req)
	return jsonResponse{Status: rec.Code, Body: rec.Body.Bytes()}
}

// doJSON 默认以**管理员**身份、带本站 Origin 发 JSON 请求。
// 阶段 2 之后复习接口要登录、词条写接口要管理员，默认这一档能让绝大多数用例
// 专注在业务口径上；需要验证拦截行为时用 doJSONAnonymous / doJSONAs。
func doJSON(t *testing.T, router *gin.Engine, method, path string, body []byte) jsonResponse {
	t.Helper()
	return doRequest(t, router, method, path, body, testRequest{
		Role:        middleware.RoleAdmin,
		Origin:      testOrigin,
		ContentType: "application/json",
	})
}

// doJSONAs 以指定角色发 JSON 请求（middleware.RoleAdmin / "user"）。
// 用来区分「没登录」（401）与「登录了但没权限」（403）两种失败。
func doJSONAs(t *testing.T, router *gin.Engine, method, path string, body []byte, role string) jsonResponse {
	t.Helper()
	return doRequest(t, router, method, path, body, testRequest{
		Role:        role,
		Origin:      testOrigin,
		ContentType: "application/json",
	})
}

// doJSONAnonymous 不带 Cookie、但带本站 Origin 的 JSON 请求。
// 带上 Origin 是为了让请求先过 CSRF 闸门，这样 401 才能确定来自登录校验而不是 CSRF。
func doJSONAnonymous(t *testing.T, router *gin.Engine, method, path string, body []byte) jsonResponse {
	t.Helper()
	return doRequest(t, router, method, path, body, testRequest{
		Origin:      testOrigin,
		ContentType: "application/json",
	})
}

// doJSONAsUser 以指定用户（令牌里的 sub）+ 指定角色发 JSON 请求。
// 「进度按人隔离」的用例靠它：同一个词、同一个接口，只换 sub，就该看到另一份数据。
func doJSONAsUser(t *testing.T, router *gin.Engine, method, path string, body []byte, userID int64, role string) jsonResponse {
	t.Helper()
	return doRequest(t, router, method, path, body, testRequest{
		UserID:      userID,
		Role:        role,
		Origin:      testOrigin,
		ContentType: "application/json",
	})
}

// doJSONWithHeader 指定 Content-Type；传空串表示不带该头（用于测请求体格式错误的兜底）。
// 身份仍是管理员 + 本站 Origin，所以这类用例验证的是处理器的宽松口径，而不是中间件。
func doJSONWithHeader(t *testing.T, router *gin.Engine, method, path string, body []byte, contentType string) jsonResponse {
	t.Helper()
	return doRequest(t, router, method, path, body, testRequest{
		Role:        middleware.RoleAdmin,
		Origin:      testOrigin,
		ContentType: contentType,
	})
}

// jsonBody 把请求结构体编码成 JSON（编码失败直接判失败，避免悄悄发出空体）
func jsonBody(t *testing.T, v interface{}) []byte {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatalf("序列化请求体失败: %v", err)
	}
	return b
}

// decodeData 解析统一响应壳 {code,message,data:...} 里的 data；第二个返回值是顶层 code。
// 响应如果不是合法 JSON，直接判失败并带上原文（排查接口 500 时最有用）。
func decodeData(t *testing.T, resp jsonResponse) (map[string]interface{}, int) {
	t.Helper()
	if resp.Status != http.StatusOK {
		t.Fatalf("期望 HTTP 200，实际 %d，响应体=%s", resp.Status, string(resp.Body))
	}
	var envelope struct {
		Code    int                    `json:"code"`
		Message string                 `json:"message"`
		Data    map[string]interface{} `json:"data"`
	}
	if err := json.Unmarshal(resp.Body, &envelope); err != nil {
		t.Fatalf("响应不是合法 JSON: %v，原文=%s", err, truncateBody(resp.Body))
	}
	return envelope.Data, envelope.Code
}

// decodeFlat 解析没有 data 包裹的响应（如 /api/health）
func decodeFlat(t *testing.T, resp jsonResponse) map[string]interface{} {
	t.Helper()
	if resp.Status != http.StatusOK {
		t.Fatalf("期望 HTTP 200，实际 %d，响应体=%s", resp.Status, string(resp.Body))
	}
	var out map[string]interface{}
	if err := json.Unmarshal(resp.Body, &out); err != nil {
		t.Fatalf("响应不是合法 JSON: %v，原文=%s", err, truncateBody(resp.Body))
	}
	return out
}

// items 取出 data.items 数组；JSON 的 null 会被当成空数组（前端也要吃这个口径）
func items(t *testing.T, data map[string]interface{}) []map[string]interface{} {
	t.Helper()
	raw, ok := data["items"]
	if !ok {
		t.Fatalf("响应 data 里没有 items 字段，实际字段=%v", mapKeys(data))
	}
	if raw == nil {
		return nil
	}
	list, ok := raw.([]interface{})
	if !ok {
		t.Fatalf("items 不是数组，实际类型=%T", raw)
	}
	out := make([]map[string]interface{}, 0, len(list))
	for i, item := range list {
		obj, ok := item.(map[string]interface{})
		if !ok {
			t.Fatalf("items[%d] 不是对象，实际类型=%T", i, item)
		}
		out = append(out, obj)
	}
	return out
}

// getStats 调一次 /api/reviews/stats 并返回 data
func getStats(t *testing.T, router *gin.Engine) map[string]interface{} {
	t.Helper()
	data, code := decodeData(t, doJSON(t, router, http.MethodGet, "/api/reviews/stats", nil))
	if code != http.StatusOK {
		t.Fatalf("stats 的 code 期望 200，实际 %d", code)
	}
	return data
}

// itoa 拼查询串用（避免在测试里到处 import strconv）
func itoa(v int64) string {
	return strconv.FormatInt(v, 10)
}

// submitReview 提交一次复习并返回 data
func submitReview(t *testing.T, router *gin.Engine, req submitReviewRequest) map[string]interface{} {
	t.Helper()
	data, code := decodeData(t, doJSON(t, router, http.MethodPost, "/api/reviews/submit", jsonBody(t, req)))
	if code != http.StatusOK {
		t.Fatalf("submit 的 code 期望 200，实际 %d", code)
	}
	return data
}

// ---------- 数值 / 判等 ----------

func toInt(t *testing.T, v interface{}) int {
	t.Helper()
	f, ok := v.(float64)
	if !ok {
		t.Fatalf("期望 JSON 数字，实际类型=%T（值=%v）", v, v)
	}
	return int(f)
}

func toInt64(t *testing.T, v interface{}) int64 {
	t.Helper()
	f, ok := v.(float64)
	if !ok {
		t.Fatalf("期望 JSON 数字，实际类型=%T（值=%v）", v, v)
	}
	return int64(f)
}

func toFloat(t *testing.T, v interface{}) float64 {
	t.Helper()
	f, ok := v.(float64)
	if !ok {
		t.Fatalf("期望 JSON 数字，实际类型=%T（值=%v）", v, v)
	}
	return f
}

// assertInt 断言字段等于期望值。
// JSON 数字统一解成 float64，比较前换算成 int64：float64 能精确表示 2^53 以内的整数，
// 而业务里的计数与 id 远小于这个量级，所以不会有精度问题。
func assertInt(t *testing.T, data map[string]interface{}, key string, want int64) {
	t.Helper()
	got, ok := data[key]
	if !ok {
		t.Fatalf("响应缺少字段 %q，实际字段=%v", key, mapKeys(data))
	}
	f, ok := got.(float64)
	if !ok {
		t.Fatalf("字段 %q 期望数字，实际类型=%T（值=%v）", key, got, got)
	}
	if int64(f) != want {
		t.Fatalf("字段 %q 期望 %d，实际 %v", key, want, got)
	}
}

// assertFloat 断言浮点字段（近似比较）
func assertFloat(t *testing.T, data map[string]interface{}, key string, want float64) {
	t.Helper()
	got, ok := data[key]
	if !ok {
		t.Fatalf("响应缺少字段 %q，实际字段=%v", key, mapKeys(data))
	}
	f, ok := got.(float64)
	if !ok {
		t.Fatalf("字段 %q 期望数字，实际类型=%T（值=%v）", key, got, got)
	}
	if diff := f - want; diff > 1e-9 || diff < -1e-9 {
		t.Fatalf("字段 %q 期望 %v，实际 %v", key, want, got)
	}
}

// assertHasSuffix 断言字段是字符串且以给定后缀结尾（用于校验响应体里的消息文案）
func assertHasSuffix(t *testing.T, data map[string]interface{}, key, suffix string) {
	t.Helper()
	got, ok := data[key]
	if !ok {
		t.Fatalf("响应缺少字段 %q，实际字段=%v", key, mapKeys(data))
	}
	s, ok := got.(string)
	if !ok {
		t.Fatalf("字段 %q 期望字符串，实际类型=%T（值=%v）", key, got, got)
	}
	if !strings.HasSuffix(s, suffix) {
		t.Fatalf("字段 %q 期望以 %q 结尾，实际 %q", key, suffix, s)
	}
}

func mapKeys(m map[string]interface{}) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	return keys
}

func truncateBody(b []byte) string {
	const maxLen = 500
	s := string(b)
	if len(s) > maxLen {
		return s[:maxLen] + "..."
	}
	return s
}

// ---------- 时间工具 ----------

// startOfToday 今日零点（服务器本地时区），与 ReviewStats 内部口径保持一致
func startOfToday() time.Time {
	now := time.Now()
	return time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
}

// daysAgoAt 取 daysAgo 天前的本地 timeOfDay 时刻（例如「昨天 10:00」）。
// 跨天口径全部用它构造，不用 mock 时钟；dayOffset 为 0 就是今天。
func daysAgoAt(daysAgo int, hour, minute int) time.Time {
	day := startOfToday().AddDate(0, 0, -daysAgo)
	return time.Date(day.Year(), day.Month(), day.Day(), hour, minute, 0, 0, day.Location())
}

// localDay 把时间点格式化成自然日，用于日志里打印「数据落在哪一天」
func localDay(t time.Time) string {
	return t.Format("2006-01-02")
}

// ---------- 落库核对 ----------

// reviewRowOf 按 word_id 取记忆状态行；找不到直接判失败
func reviewRowOf(t *testing.T, db *gorm.DB, wordID uint) models.WordReview {
	t.Helper()
	var row models.WordReview
	if err := db.Where("word_id = ?", wordID).First(&row).Error; err != nil {
		t.Fatalf("读取 word_id=%d 的记忆状态失败: %v", wordID, err)
	}
	return row
}

// logsOf 按 word_id 取复习日志（按 id 升序，即提交先后顺序）
func logsOf(t *testing.T, db *gorm.DB, wordID uint) []models.ReviewLog {
	t.Helper()
	var rows []models.ReviewLog
	if err := db.Where("word_id = ?", wordID).Order("id ASC").Find(&rows).Error; err != nil {
		t.Fatalf("读取 word_id=%d 的复习日志失败: %v", wordID, err)
	}
	return rows
}

// countRows 统计一张表的行数（传 &models.X{}）
func countRows(t *testing.T, db *gorm.DB, model interface{}) int64 {
	t.Helper()
	var n int64
	if err := db.Model(model).Count(&n).Error; err != nil {
		t.Fatalf("统计行数失败: %v", err)
	}
	return n
}

// reviewRowOfUser 按 (user_id, word_id) 取记忆状态行。
// P0-1 之后同一个词每个用户各有一行，所以核对落库必须带上用户，不能用只按 word_id 的版本。
func reviewRowOfUser(t *testing.T, db *gorm.DB, userID, wordID uint) models.WordReview {
	t.Helper()
	var row models.WordReview
	if err := db.Where("user_id = ? AND word_id = ?", userID, wordID).First(&row).Error; err != nil {
		t.Fatalf("读取 user_id=%d word_id=%d 的记忆状态失败: %v", userID, wordID, err)
	}
	return row
}

// logsOfUser 按 (user_id, word_id) 取复习日志（按 id 升序，即提交先后顺序）
func logsOfUser(t *testing.T, db *gorm.DB, userID, wordID uint) []models.ReviewLog {
	t.Helper()
	var rows []models.ReviewLog
	if err := db.Where("user_id = ? AND word_id = ?", userID, wordID).Order("id ASC").Find(&rows).Error; err != nil {
		t.Fatalf("读取 user_id=%d word_id=%d 的复习日志失败: %v", userID, wordID, err)
	}
	return rows
}

// countReviewsOfUser 统计某个用户在 word_reviews 里的行数（核对「各人各一行」）
func countReviewsOfUser(t *testing.T, db *gorm.DB, userID uint) int64 {
	t.Helper()
	var n int64
	if err := db.Model(&models.WordReview{}).Where("user_id = ?", userID).Count(&n).Error; err != nil {
		t.Fatalf("统计 user_id=%d 的进度行数失败: %v", userID, err)
	}
	return n
}
