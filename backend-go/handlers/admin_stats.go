// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package handlers

import (
	"errors"
	"fmt"
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"

	"backend-go/database"
	"backend-go/middleware"
	"backend-go/models"
)

// ============================================================================
// 管理看板 · 只读统计（/api/admin/stats/*、/api/admin/users/*）
//
// 数据来自两个库，口径必须一起钉死（docs/launch-plan.md §5.0 的定案）：
//
//   - auth.db（账号服务 backend-rust）：**只读**（mode=ro）读 users / invite_codes，
//     拿「今日新增用户 / 角色分布 / 发码与兑换」。读不到时降级成 0 + 一句原因，
//     绝不让整个看板 500 —— 账号服务重启、库被搬走都会出现这种状态（见 database.AuthDB）。
//   - guangxue.db（本服务）：复习日志与词条数。
//
// 【时区铁律】所有「今天 / 按天」一律按**北京时间（UTC+8）**切：
//   - auth.db 的时间列是 ISO8601 **UTC 文本**（`2026-09-15T08:31:13Z`，见
//     backend-rust/src/db.rs 的 TS_FORMAT），拿到手必须显式转北京时间再取日期；
//   - guangxue.db 的 reviewed_at 是 time.Time，同样转北京时间再取日期；
//   - **不要用服务器本地时区**：部署机可能是 UTC，那会让「今天」整体偏 8 小时 ——
//     北京时间的 00:00~08:00（用户最活跃的一段）会被算进前一天。
//     这条口径有专门的边界测试：UTC 的 2026-10-02T16:30:00Z 必须算作北京时间 2026-10-03。
//
// 【为什么按天分桶在 Go 侧做，而不是写进 SQL】
//   - SQLite 没有时区概念：`strftime(..., '+8 hours')` 只是字符串魔法，还会把口径藏进 SQL；
//     两个库一个存 ISO 文本、一个是 Go 时间，也没法共用同一段 SQL。
//   - 更要命的是 SQLite 比较时间靠**字典序**：glebarez 把 time.Time 写成
//     `2026-10-03 17:27:36.1460602+08:00`（带自己的偏移），拿一个 +08 的边界去比库里 +00 的行，
//     同一个瞬间会排出先后。所以 SQL 只用一个**放宽两天**的边界做粗筛（顺带还能吃到索引），
//     精确判定一律在 Go 侧按真实时间点比 —— 两个库共用同一套判定。
//
// ============================================================================

// adminCoarseFilterSlack SQL 粗筛边界相对精确起点的放宽量。
//
// 为什么要放宽：库里存的是**带时区偏移的文本**，而字典序比较不认偏移差 ——
// 全世界偏移从 -12:00 到 +14:00，同一个瞬间最多能被写出 26 小时的「文本差」。
// 正常情况下这些行都是本进程写的（偏移一致，不会有问题），但换过部署机时区、
// 或库是从别处搬来的，历史行就可能混着不同偏移。放宽两天足以覆盖 26 小时的最坏情况，
// 让粗筛**只会多带行、绝不漏行**；真正的判定在 Go 侧按真实时间点做。
//
// 代价是每个窗口多扫一两天日志（趋势 30 天 = 多 2 天），换来的是「换过时区也不出错」。
const adminCoarseFilterSlack = 2 * 24 * time.Hour

// coarseFilterStart 返回 SQL 粗筛该用的起点（比精确起点往前放宽，见 adminCoarseFilterSlack）
func coarseFilterStart(start time.Time) time.Time {
	return start.Add(-adminCoarseFilterSlack)
}

// beijingTZ 看板统计用的时区：北京时间（UTC+8）。
// 直接复用复习池那套 poolTZ()（Asia/Shanghai；容器里没装 tzdata 时回落固定 +8），
// 免得「北京时间」这个概念在两处各定义一次、其中一处被改掉而对不上。
func beijingTZ() *time.Location { return poolTZ() }

// startOfBeijingDay 返回 t 所在的北京自然日的零点（带 +8 时区信息）。
// calcStreakDays 也吃这个值：它内部按入参的 Location 取自然日，传北京时间进来才算得对。
func startOfBeijingDay(t time.Time) time.Time {
	y, m, d := t.In(beijingTZ()).Date()
	return time.Date(y, m, d, 0, 0, 0, 0, beijingTZ())
}

// beijingDay 把时间点折成北京自然日（"2006-01-02"），折线图的横轴就是它
func beijingDay(t time.Time) string {
	return t.In(beijingTZ()).Format("2006-01-02")
}

// beijingRFC3339 把时间点格式化成 RFC3339 的北京时间（2026-10-03T08:00:00+08:00）。
// 输出带 +08:00 而不是 Z：面板多数直接展示这个字符串，前端若用 new Date() 解析也会
// 按浏览器本地时区正确换算，两种用法都不会错。
func beijingRFC3339(t time.Time) string {
	return t.In(beijingTZ()).Format(time.RFC3339)
}

// adminSQLBoundary 把「北京时间某时刻」变成可以塞进 SQL 比较的边界值。
//
// ⚠️ 不能直接把带 +08 的时间点绑进 `reviewed_at >= ?`：glebarez 按
// `2006-01-02 15:04:05.999999999-07:00` 写出**带自身偏移**的文本，而 SQLite 比较时间靠字典序。
// 库里那些行是本进程（time.Now()，本地时区）写进去的，边界也换算成本地时区，偏移才对得上。
// 即便如此 SQL 也只当**粗筛**（调用方都先按 adminCoarseFilterSlack 放宽），精确判定在 Go 侧做。
func adminSQLBoundary(t time.Time) time.Time { return t.In(time.Local) }

// authTextBoundary 把时间点变成与 auth.db 存储格式同形的 UTC 文本边界
// （固定宽度 + 字面量 Z，所以字典序比较与时间先后一致）。
func authTextBoundary(t time.Time) string {
	return t.UTC().Format("2006-01-02T15:04:05Z")
}

// storedTimeLayouts 两库里时间文本的解析格式表（按现网口径到防御性格式排列）。
//
// 现网口径：
//   - auth.db 是 `2026-09-15T08:31:13Z`（UTC、秒精度、固定宽度，backend-rust/src/db.rs 的 TS_FORMAT）；
//   - guangxue.db 是 glebarez 的 `2026-10-03 17:27:36.1460602+08:00`（带自身偏移、小数秒宽度可变）。
//
// 后面几条是防御性的：万一将来带上别的精度、或者有人手工改过库，
// 也不至于让整页数字悄悄变成 0（解析不了的行会被跳过，而不是当成 1970 年）。
var storedTimeLayouts = []string{
	time.RFC3339Nano,
	"2006-01-02 15:04:05.999999999-07:00",
	"2006-01-02 15:04:05.999999999",
	"2006-01-02T15:04:05",
	"2006-01-02 15:04:05",
	"2006-01-02",
}

// parseStoredTime 解析库里读回来的时间文本；解析不了返回 ok=false（调用方跳过该行）。
// 不带时区后缀的格式按 **UTC** 解释：两个库的时间列都是 UTC 语义，
// 按本地时区解释会在非 UTC 部署机上整体偏 8 小时。
func parseStoredTime(raw string) (time.Time, bool) {
	s := strings.TrimSpace(raw)
	if s == "" {
		return time.Time{}, false
	}
	for _, layout := range storedTimeLayouts {
		if t, err := time.Parse(layout, s); err == nil {
			return t, true
		}
	}
	return time.Time{}, false
}

// ---------- 处理器与响应结构 ----------

// AdminStatsHandler 管理看板的只读统计处理器。
type AdminStatsHandler struct {
	db     *gorm.DB
	authDB *database.AuthDB
	// reviews 只为复用 calcStreakDays：连续天数只有一份实现，绝不复制第二份口径
	reviews *ReviewHandler
	// now 取当前时间；测试注入固定时刻来钉时区边界，生产就是 time.Now
	now func() time.Time
}

// NewAdminStatsHandler 创建处理器；authDB 可为「未配置路径」的句柄（看板会走降级）
func NewAdminStatsHandler(db *gorm.DB, authDB *database.AuthDB) *AdminStatsHandler {
	return &AdminStatsHandler{
		db:      db,
		authDB:  authDB,
		reviews: NewReviewHandler(db),
		now:     time.Now,
	}
}

// adminToday 今日五个数字（跨两个库，账号不可用时 new_users 为 0）
type adminToday struct {
	NewUsers    int64 `json:"new_users"`
	ActiveUsers int64 `json:"active_users"`
	Reviews     int64 `json:"reviews"`
	NewWords    int64 `json:"new_words"`
	Probes      int64 `json:"probes"`
}

// adminTotals 累计数字：账号侧五个 + 业务侧两个 + 兑换率
type adminTotals struct {
	Users            int64   `json:"users"`
	Admins           int64   `json:"admins"`
	SuperAdmins      int64   `json:"super_admins"`
	InvitesIssued    int64   `json:"invites_issued"`
	InvitesRedeemed  int64   `json:"invites_redeemed"`
	InviteConversion float64 `json:"invite_conversion"`
	Words            int64   `json:"words"`
	Reviews          int64   `json:"reviews"`
}

// adminAuthDBStatus 账号库可用性：前端据此显示提示条（available=false 时账号侧数字都是 0）
type adminAuthDBStatus struct {
	Available bool   `json:"available"`
	Error     string `json:"error"`
}

// authDBUnavailableStatus 把「读账号库这一步失败」折成可用性状态：available=false + 一句中文原因。
//
// 看板与深检（health.go）**共用这一个函数**：判定（database.AuthDB 的只读查询失败）
// 与文案（database.AuthDBUnavailableReason）必须一模一样 —— 各写一套的话，
// 同一个故障会在看板和健康检查里给出两句不同的原因，排查时反而更糊涂。
func authDBUnavailableStatus(path string, err error) adminAuthDBStatus {
	return adminAuthDBStatus{Available: false, Error: database.AuthDBUnavailableReason(path, err)}
}

// adminAccountSnapshot 一次账号库读取的全部结果；不可用时**数字全 0**、status 里带原因。
//
// 收成一个结构体是为了让「今天新增」和「累计」共享同一批查询：
// overview 与 trend 都要账号侧数字，各自再查一遍既慢又容易出现两套口径。
type adminAccountSnapshot struct {
	todayNewUsers int64
	users         int64
	admins        int64
	superAdmins   int64
	invitesIssued int64
	redeemed      int64
	status        adminAuthDBStatus
}

// adminTrendPoint 折线图的一个点（没有数据的日子也在，值是 0 —— 前端直接画，不用补洞）
type adminTrendPoint struct {
	Date        string `json:"date"`
	NewUsers    int64  `json:"new_users"`
	ActiveUsers int64  `json:"active_users"`
	Reviews     int64  `json:"reviews"`
}

// adminUserProgress 用户列表里一行的进度摘要；last_review_at 为空时是 JSON null
type adminUserProgress struct {
	UserID       uint    `json:"user_id"`
	LearnedWords int64   `json:"learned_words"`
	TotalReviews int64   `json:"total_reviews"`
	StreakDays   int     `json:"streak_days"`
	LastReviewAt *string `json:"last_review_at"`
}

// adminUserDayPoint 「最近 7 天」的一个点
type adminUserDayPoint struct {
	Date    string `json:"date"`
	Reviews int64  `json:"reviews"`
}

// adminLogRow 业务侧一次取回的日志行（分桶要用的四列）
type adminLogRow struct {
	UserID          uint      `gorm:"column:user_id"`
	ReviewedAt      time.Time `gorm:"column:reviewed_at"`
	StabilityBefore float64   `gorm:"column:stability_before"`
	IsProbe         bool      `gorm:"column:is_probe"`
}

// ---------- 业务侧（guangxue.db）取数 ----------

// windowLogs 取 [start, ∞) 的复习日志。
//
// SQL 侧把边界往前放宽（见 adminCoarseFilterSlack）只做粗筛（原因见文件头：
// 字典序比较在偏移不一致时不可靠），精确的 `reviewed_at >= start` 在 Go 侧按真实时间点比。
func (h *AdminStatsHandler) windowLogs(start time.Time) ([]adminLogRow, error) {
	var rows []adminLogRow
	err := h.db.Model(&models.ReviewLog{}).
		Select("user_id, reviewed_at, stability_before, is_probe").
		Where("reviewed_at >= ?", adminSQLBoundary(coarseFilterStart(start))).
		Scan(&rows).Error
	if err != nil {
		return nil, err
	}
	kept := rows[:0]
	for _, row := range rows {
		if !row.ReviewedAt.Before(start) {
			kept = append(kept, row)
		}
	}
	return kept, nil
}

// ---------- 账号侧（auth.db）取数 ----------

// accountSnapshot 一次性读账号库；任何一步失败都返回「不可用」（数字全 0 + 原因）。
//
// 账号侧只读 6 个数字，但**不是**一个请求一个查询：invite_codes 的总数 / 已兑换数合成一条，
// 角色分布一条，今日新增一条，users 总数一条 —— 四条都是常量级的小查询，
// 而真正贵的是每次打开连接（失败时还会重试），所以把它们放在一次快照里。
func (h *AdminStatsHandler) accountSnapshot(start time.Time) adminAccountSnapshot {
	snap := adminAccountSnapshot{status: adminAuthDBStatus{Available: true}}
	unavailable := func(err error) adminAccountSnapshot {
		return adminAccountSnapshot{status: authDBUnavailableStatus(h.authDB.Path(), err)}
	}

	var users int64
	if err := h.authDB.Select(&users, "SELECT COUNT(*) FROM users"); err != nil {
		return unavailable(err)
	}
	snap.users = users

	var roles []struct {
		Role string
		N    int64
	}
	if err := h.authDB.Select(&roles, "SELECT role, COUNT(*) AS n FROM users GROUP BY role"); err != nil {
		return unavailable(err)
	}
	for _, r := range roles {
		// 角色字面量统一走 middleware 的常量（跨服务契约，写错会让「管理员」永远显示 0）
		switch r.Role {
		case middleware.RoleAdmin:
			snap.admins = r.N
		case middleware.RoleSuperAdmin:
			snap.superAdmins = r.N
		}
	}

	createdToday, err := h.usersCreatedSince(start)
	if err != nil {
		return unavailable(err)
	}
	snap.todayNewUsers = int64(len(createdToday))

	var invites []struct {
		Total    int64
		Redeemed int64
	}
	// COALESCE：一张码都没发时 SUM(...) 返回 NULL，直接扫进 int64 会失败
	if err := h.authDB.Select(&invites,
		`SELECT COUNT(*) AS total,
		        COALESCE(SUM(CASE WHEN used_count > 0 THEN 1 ELSE 0 END), 0) AS redeemed
		   FROM invite_codes`); err != nil {
		return unavailable(err)
	}
	if len(invites) > 0 {
		snap.invitesIssued = invites[0].Total
		snap.redeemed = invites[0].Redeemed
	}
	return snap
}

// inviteConversion 兑换率 = 已兑换张数 / 发码总数，保留 2 位小数。
// 一张码都没发时给 0：绝不能算出 NaN —— JSON 里没有 NaN，序列化会直接失败，
// 前端 JSON.parse 也会抛错，整页数字全打不开。
func inviteConversion(redeemed, issued int64) float64 {
	if issued <= 0 {
		return 0
	}
	return math.Round(float64(redeemed)/float64(issued)*100) / 100
}

// usersCreatedSince 取 [start, ∞) 注册的用户时间点。
//
// created_at 是 ISO8601 UTC 文本（固定宽度 + Z，字典序 = 时间序），所以 SQL 侧直接用同格式的
// UTC 文本边界做粗筛，再在 Go 侧按真实时间点精确过滤、交由调用方按北京自然日分桶。
// 边界同样往前放宽：auth.db 的列永远是同一个格式，放宽只是防御性的（不会漏行）。
func (h *AdminStatsHandler) usersCreatedSince(start time.Time) ([]time.Time, error) {
	var raw []string
	if err := h.authDB.Select(&raw,
		"SELECT created_at FROM users WHERE created_at >= ?",
		authTextBoundary(coarseFilterStart(start))); err != nil {
		return nil, err
	}
	out := make([]time.Time, 0, len(raw))
	for _, s := range raw {
		if t, ok := parseStoredTime(s); ok && !t.Before(start) {
			out = append(out, t)
		}
	}
	return out, nil
}

// ---------- 用户进度（业务侧） ----------

// adminProgressMaxIDs 批量进度一次最多查多少个 id。
// 一页 100 行是用户列表的分页大小；再大就该翻页，而不是把 IN 列表撑成一条巨型 SQL。
const adminProgressMaxIDs = 100

// parseProgressIDs 解析 `?ids=1,2,3`。
//
// 去重后返回（同一个 id 只出一行）；重复项在**去重前**就计入上限，
// 免得用重复 id 绕过 100 的限制把查询撑大。
func parseProgressIDs(raw string) ([]uint, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil, errors.New("ids 不能为空")
	}
	parts := strings.Split(raw, ",")
	if len(parts) > adminProgressMaxIDs {
		return nil, fmt.Errorf("ids 最多 %d 个", adminProgressMaxIDs)
	}
	ids := make([]uint, 0, len(parts))
	seen := make(map[uint]struct{}, len(parts))
	for _, part := range parts {
		v, err := strconv.ParseUint(strings.TrimSpace(part), 10, 32)
		// 0 不算合法用户 id：它是 P0-1 迁移前历史数据的占位值（见 middleware.CurrentUserID）
		if err != nil || v == 0 {
			return nil, fmt.Errorf("ids 里有非法 id：%q", part)
		}
		id := uint(v)
		if _, dup := seen[id]; dup {
			continue
		}
		seen[id] = struct{}{}
		ids = append(ids, id)
	}
	if len(ids) == 0 {
		return nil, errors.New("ids 不能为空")
	}
	return ids, nil
}

// progressOf 批量算这些用户的进度摘要。
//
// 三条 GROUP BY 查询把所有数字一次算完（**不是**每人三条）：用户列表一页 100 行，
// N+1 就是 300 次往返。请求里没出现过的 id 不进 map，由调用方补零值 —— 缺行会让表格错位。
func (h *AdminStatsHandler) progressOf(ids []uint) (map[uint]adminUserProgress, error) {
	out := make(map[uint]adminUserProgress, len(ids))

	var learned []struct {
		UserID uint
		N      int64
	}
	if err := h.db.Model(&models.WordReview{}).
		Select("user_id, COUNT(*) AS n").
		Where("user_id IN ?", ids).
		Group("user_id").
		Scan(&learned).Error; err != nil {
		return nil, err
	}
	for _, row := range learned {
		if _, ok := out[row.UserID]; !ok {
			out[row.UserID] = adminUserProgress{UserID: row.UserID}
		}
		item := out[row.UserID]
		item.LearnedWords = row.N
		out[row.UserID] = item
	}

	var logged []struct {
		UserID uint
		N      int64
	}
	if err := h.db.Model(&models.ReviewLog{}).
		Select("user_id, COUNT(*) AS n").
		Where("user_id IN ?", ids).
		Group("user_id").
		Scan(&logged).Error; err != nil {
		return nil, err
	}
	for _, row := range logged {
		if _, ok := out[row.UserID]; !ok {
			out[row.UserID] = adminUserProgress{UserID: row.UserID}
		}
		item := out[row.UserID]
		item.TotalReviews = row.N
		out[row.UserID] = item
	}

	// 最近一次复习时间。⚠️ MAX(reviewed_at) 是**表达式**，SQLite 不给它 declType，
	// 驱动于是把它当文本返回（实测），所以先扫成 string 再按存储格式解析；
	// 直接写进 *time.Time 会得到一个零值时间（2026 年之前的那种静默错误）。
	//
	// 这里用的是字典序 MAX（与全库其余时间比较同一个前提：这些行都由本进程按同一时区偏移写入，
	// 所以文本序 = 时间序）。真正的边界判定都在 Go 侧做，只有这一处取「极值」必须交给 SQL。
	var last []struct {
		UserID uint
		LastAt *string
	}
	if err := h.db.Model(&models.ReviewLog{}).
		Select("user_id, MAX(reviewed_at) AS last_at").
		Where("user_id IN ?", ids).
		Group("user_id").
		Scan(&last).Error; err != nil {
		return nil, err
	}
	for _, row := range last {
		if row.LastAt == nil {
			continue
		}
		t, ok := parseStoredTime(*row.LastAt)
		if !ok {
			continue
		}
		item := out[row.UserID]
		item.UserID = row.UserID
		formatted := beijingRFC3339(t)
		item.LastReviewAt = &formatted
		out[row.UserID] = item
	}

	// 连续天数按人复用 calcStreakDays（同包内调用，绝不复制第二份口径）：
	// 它内部只取「最近 400 天的 reviewed_at」再在 Go 侧按自然日累计 ——
	// 一页 100 人是 100 次轻查询，这是为「连续天数只有一个实现」付的代价。
	startOfToday := startOfBeijingDay(h.now())
	for _, id := range ids {
		item := out[id]
		item.UserID = id
		item.StreakDays = h.reviews.calcStreakDays(id, startOfToday)
		out[id] = item
	}
	return out, nil
}

// last7Reviews 某用户最近 7 个北京自然日的复习次数（升序、没数据的日子补 0）
func (h *AdminStatsHandler) last7Reviews(userID uint, start time.Time) ([]adminUserDayPoint, error) {
	const days = 7
	points := make([]adminUserDayPoint, days)
	index := make(map[string]int, days)
	for i := 0; i < days; i++ {
		points[i] = adminUserDayPoint{Date: beijingDay(start.AddDate(0, 0, i))}
		index[points[i].Date] = i
	}

	var times []time.Time
	// 同样只是粗筛（放宽两天），精确判定在下面按时间点做
	if err := h.db.Model(&models.ReviewLog{}).
		Where("user_id = ? AND reviewed_at >= ?", userID, adminSQLBoundary(coarseFilterStart(start))).
		Pluck("reviewed_at", &times).Error; err != nil {
		return nil, err
	}
	for _, t := range times {
		if t.Before(start) {
			continue
		}
		if i, ok := index[beijingDay(t)]; ok {
			points[i].Reviews++
		}
	}
	return points, nil
}

// ---------- 接口实现 ----------

// Overview 管理看板总览：GET /api/admin/stats/overview
//
// 一个请求出全部数字（用户决策：看板不开第二个请求）。
// 账号库读不到时：`auth_db.available=false` + `error` 说明原因，账号侧 6 个数字一律 0，
// 业务侧数字照常返回、HTTP 仍是 200 —— 看板要能打开，而不是整页报错。
func (h *AdminStatsHandler) Overview(c *gin.Context) {
	start := startOfBeijingDay(h.now())

	today := adminToday{}
	logs, err := h.windowLogs(start)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	activeUsers := make(map[uint]struct{}, len(logs))
	for _, row := range logs {
		today.Reviews++
		activeUsers[row.UserID] = struct{}{}
		// 今日新学 = 当天 stability_before = 0 的日志数（docs/launch-plan.md §5.0 的口径）。
		// ⚠️ 与 /api/reviews/stats 的 today_new 有一处差异：那边还排除了 is_reset（重置重学），
		// 看板按定案只认 stability_before = 0，两者在「重置重学」多的日子会不一样。
		if row.StabilityBefore == 0 {
			today.NewWords++
		}
		if row.IsProbe {
			today.Probes++
		}
	}
	today.ActiveUsers = int64(len(activeUsers))

	totals := adminTotals{}
	var wordsTotal, reviewsTotal int64
	if err := h.db.Model(&models.Word{}).Count(&wordsTotal).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "统计失败:" + err.Error()})
		return
	}
	if err := h.db.Model(&models.ReviewLog{}).Count(&reviewsTotal).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "统计失败:" + err.Error()})
		return
	}
	totals.Words = wordsTotal
	totals.Reviews = reviewsTotal

	account := h.accountSnapshot(start)
	today.NewUsers = account.todayNewUsers
	totals.Users = account.users
	totals.Admins = account.admins
	totals.SuperAdmins = account.superAdmins
	totals.InvitesIssued = account.invitesIssued
	totals.InvitesRedeemed = account.redeemed
	totals.InviteConversion = inviteConversion(account.redeemed, account.invitesIssued)

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"today":   today,
		"totals":  totals,
		"auth_db": account.status,
	}})
}

// Trend 趋势曲线：GET /api/admin/stats/trend?days=7|30
//
// days 只接受 7 与 30（默认 7），其它值一律 400 —— 前端的两个按钮就是这两档，
// 放开任意值只会让「哪个口径被用了」变得说不清，也让缓存与测试失去确定性。
//
// points 按日期**升序**、长度恰好 days、没有数据的那天补 0：
// 前端直接连线画折线，不必自己补洞（补洞逻辑一旦写错，曲线会整段错位）。
func (h *AdminStatsHandler) Trend(c *gin.Context) {
	days := 7
	if raw := strings.TrimSpace(c.Query("days")); raw != "" {
		v, err := strconv.Atoi(raw)
		if err != nil || (v != 7 && v != 30) {
			c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "days 只接受 7 或 30"})
			return
		}
		days = v
	}

	start := startOfBeijingDay(h.now()).AddDate(0, 0, -(days - 1))
	points := make([]adminTrendPoint, days)
	index := make(map[string]int, days)
	for i := 0; i < days; i++ {
		points[i] = adminTrendPoint{Date: beijingDay(start.AddDate(0, 0, i))}
		index[points[i].Date] = i
	}

	logs, err := h.windowLogs(start)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	activeByDay := make([]map[uint]struct{}, days)
	for _, row := range logs {
		i, ok := index[beijingDay(row.ReviewedAt)]
		if !ok {
			continue // 粗筛多带回来的行（窗口之外）
		}
		points[i].Reviews++
		if activeByDay[i] == nil {
			activeByDay[i] = make(map[uint]struct{})
		}
		activeByDay[i][row.UserID] = struct{}{}
	}
	for i := range points {
		points[i].ActiveUsers = int64(len(activeByDay[i]))
	}

	// 账号侧不可用时 new_users 全 0（曲线照样画得出来），且**不影响** HTTP 状态码：
	// 曲线少一条线，好过整页打不开。
	if created, err := h.usersCreatedSince(start); err == nil {
		for _, t := range created {
			if i, ok := index[beijingDay(t)]; ok {
				points[i].NewUsers++
			}
		}
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"days":   days,
		"points": points,
	}})
}

// UsersProgress 批量进度摘要：GET /api/admin/users/progress?ids=1,2,3（最多 100 个）
//
// 给「用户列表」那一页用：Rust 出用户表（治理数据的唯一来源），Go 出当页进度，
// 两个请求并行。**请求里没出现的 id 也给一行（数字全 0）** ——
// 前端按页把两边的数据对齐渲染，缺行会让整张表格错位。
//
// 路由上与 /users/:id/progress 并存：gin 的树是「静态段优先于 :id」，
// `/users/progress` 会命中这条、`/users/7/progress` 命中单人那条（有路由测试钉住）。
func (h *AdminStatsHandler) UsersProgress(c *gin.Context) {
	ids, err := parseProgressIDs(c.Query("ids"))
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": err.Error()})
		return
	}
	progress, err := h.progressOf(ids)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	items := make([]adminUserProgress, 0, len(ids))
	for _, id := range ids {
		item := progress[id]
		// map 里没有这个 id 时给零值（数字全 0、last_review_at 为 null），
		// 但 user_id 一定要回填成请求里的那个 —— 否则前端认不出这是谁的行
		item.UserID = id
		items = append(items, item)
	}
	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"items": items,
	}})
}

// UserProgress 单人进度摘要：GET /api/admin/users/:id/progress（用户列表里点开某人）
//
// last7 固定 7 天、升序、补 0：面板上那 7 根小柱子不需要前端再做日期对齐。
func (h *AdminStatsHandler) UserProgress(c *gin.Context) {
	id, err := strconv.ParseUint(strings.TrimSpace(c.Param("id")), 10, 32)
	if err != nil || id == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "id 不合法"})
		return
	}
	userID := uint(id)

	progress, err := h.progressOf([]uint{userID})
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}
	item := progress[userID]
	item.UserID = userID

	last7, err := h.last7Reviews(userID, startOfBeijingDay(h.now()).AddDate(0, 0, -6))
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"user_id":        item.UserID,
		"learned_words":  item.LearnedWords,
		"total_reviews":  item.TotalReviews,
		"streak_days":    item.StreakDays,
		"last_review_at": item.LastReviewAt,
		"last7":          last7,
	}})
}
