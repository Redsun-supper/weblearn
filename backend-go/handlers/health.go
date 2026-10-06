// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
package handlers

import (
	"net/http"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"

	"backend-go/database"
)

// ============================================================================
// 健康检查：GET /api/health（浅检 + 深检）
//
// 外部监控（guangxue-monitor，每 5 分钟一次）**只靠 HTTP 状态码判断**，
// 所以「本服务自己坏了」必须回 **503**，而不是 200 再带一句错误文案 ——
// 对只看状态码的监控来说，200 + 文案与一切正常没有任何区别，告警永远不会响。
//
// 但「账号库读不到」**不算坏**：那是 P1 已经定死的口径（账号库读不到时看板降级成
// 数字全 0 + 一句原因，绝不让整页报错）。深检只把它当降级信息留在 `deep.auth_db` 里，
// status 仍是 "ok"、HTTP 仍是 200。两件事混在一起，会让「账号服务重启」这种日常状态
// 每 5 分钟报一次警，真正的故障反而被淹没。
//
// 于是判定只有一条：**自身检查**（业务库查询）失败才算 degraded → 503。
//
// 浅检（不带 deep 参数）**逐字保持原样** `{"status":"ok","message":"服务器运行正常"}`：
// 旧前端、部署脚本（docs/deploy-runbook.md 里的 curl）与 routes 包的用例都按这个形状解析。
// ============================================================================

// HealthHandler 健康检查处理器。
//
// db 是业务库（guangxue.db）；authDB 是账号库的**只读惰性句柄**（起服务时不碰文件，
// 与 /api/admin/stats/* 用的是同一个 database.AuthDB 类型与同一套判定）。
type HealthHandler struct {
	db        *gorm.DB
	authDB    *database.AuthDB
	startedAt time.Time
}

// NewHealthHandler 创建处理器；startedAt 取构造时刻，深检据此报 uptime_seconds
func NewHealthHandler(db *gorm.DB, authDB *database.AuthDB) *HealthHandler {
	return &HealthHandler{db: db, authDB: authDB, startedAt: time.Now()}
}

// healthDeep 深检结果。字段名是给监控/排障脚本看的固定契约，别再改名。
type healthDeep struct {
	// Database "ok" 或 "error: <原因>"。只有它不是 ok 时整体才会 503
	Database string `json:"database"`
	// MigrationVersion 业务库当前的迁移版本（SQLite 的 user_version）。
	// 纯信息项：读不到一律报 0，**绝不**因为它失败而降级（见 migrationVersion）
	MigrationVersion int64 `json:"migration_version"`
	// AuthDB 账号库可用性：读不到只是降级信息，不影响 status / HTTP 状态码
	AuthDB healthAuthDBInfo `json:"auth_db"`
	// UptimeSeconds 本进程已运行的秒数
	UptimeSeconds int64 `json:"uptime_seconds"`
	// Version 与 /api/hello 同一个常量（apiVersion），不是从库或环境变量里读的
	Version string `json:"version"`
}

// healthAuthDBInfo 深检里的账号库状态。
//
// 可用时 error 是 **JSON null**（不是空串）：这一份要给监控脚本读，
// `null` 比 `""` 好判断「到底有没有出问题」。看板的 adminAuthDBStatus 保持原样
// （恒为字符串），两处的判定与文案仍共用同一套代码（authDBUnavailableStatus）。
type healthAuthDBInfo struct {
	Available bool    `json:"available"`
	Error     *string `json:"error"`
}

// Check 健康检查：GET /api/health[?deep=1|true]
func (h *HealthHandler) Check(c *gin.Context) {
	// 浅检：字段、取值、HTTP 状态码都与改造前**一字不差**，一个字段都不能多
	if !deepRequested(c) {
		c.JSON(http.StatusOK, gin.H{
			"status":  "ok",
			"message": "服务器运行正常",
		})
		return
	}

	deep := healthDeep{
		Database:         "ok",
		MigrationVersion: h.migrationVersion(),
		UptimeSeconds:    int64(time.Since(h.startedAt).Seconds()),
		Version:          apiVersion,
	}

	// 账号库先探：它读不到只是降级信息（P1 口径），**不**影响 status / HTTP 状态码。
	// 放在业务库之前是有意的：即使业务库坏了、要回 503，响应里的 deep 也要带上账号侧状态 ——
	// 排障时最想知道的恰恰是「两个库是不是一起出问题了」。
	deep.AuthDB = healthAuthDBInfo{Available: true}
	if status := probeAuthDB(h.authDB); !status.Available {
		reason := status.Error
		deep.AuthDB = healthAuthDBInfo{Available: false, Error: &reason}
	}

	// 业务库：深检里唯一会让整体降级的一项。
	// 用真查询（SELECT 1）而不是只看文件在不在 —— 库被删、连接被关、文件损坏
	// 都可能出现「路径还在但查不了」，而那正是监控要报警的状态。
	var one int
	if err := h.db.Raw("SELECT 1").Scan(&one).Error; err != nil {
		deep.Database = "error: " + err.Error()
		// 文案直接说清是哪一项坏了：监控告警里只会贴这一句，写「服务异常」等于没写
		c.JSON(http.StatusServiceUnavailable, gin.H{
			"status":  "degraded",
			"message": "业务库（guangxue.db）不可用：" + err.Error(),
			"deep":    deep,
		})
		return
	}

	c.JSON(http.StatusOK, gin.H{
		"status":  "ok",
		"message": "服务器运行正常",
		"deep":    deep,
	})
}

// deepRequested 这次请求要不要做深检。
//
// 只认 `?deep=1` 与 `?deep=true`（忽略大小写与首尾空白），其余一律浅检：
// `?deep=0` / `?deep=yes` / `?deep=` 这类模棱两可的写法按「不要深检」处理，
// 免得「哪些值算真」变成一件要靠读代码才知道的事。
func deepRequested(c *gin.Context) bool {
	switch strings.ToLower(strings.TrimSpace(c.Query("deep"))) {
	case "1", "true":
		return true
	default:
		return false
	}
}

// migrationVersion 读业务库当前的迁移版本。
//
// 读不到就报 0 且**不降级**：它只是给人看的信息项。
// 口径是 SQLite 的 `PRAGMA user_version` —— 与账号服务记迁移进度用的是同一个约定
// （backend-rust/src/db.rs）。⚠️ guangxue.db 的表结构由 GORM AutoMigrate 维护、没有版本链，
// 谁都没写过这个值，所以现在（以及可预见的将来）读回来就是 0，含义是「这个库没被打过版本标记」，
// **不是「库坏了」**。要让这个数字有意义，得由 cmd/migrate 在迁移完成后写一个版本号，
// 那时深检自动就会报出真值（读的就是这个 PRAGMA）。
func (h *HealthHandler) migrationVersion() int64 {
	version, err := database.MigrationVersion(h.db)
	if err != nil {
		return 0
	}
	return version
}

// probeAuthDB 探测账号库现在读不读得到。
//
// 走 database.AuthDB.Select 发一条**真查询**，不用 os.Stat 看文件在不在：
// 账号服务正在写、-wal/-shm 伴随文件被清掉、库被换了一份，都会出现「文件在但读不了」，
// 而那正是要报出去的状态。成败折成 adminAuthDBStatus，与看板共用
// authDBUnavailableStatus —— 同一个故障在看板与深检里必须给出同一句原因。
//
// 查询用 `SELECT 1`（看板的首查是 `SELECT COUNT(*) FROM users`）：深检只问
// 「这个库能不能读」，不关心表结构，账号服务还没建表时不该被深检报成「账号库读不到」。
func probeAuthDB(authDB *database.AuthDB) adminAuthDBStatus {
	var one int
	if err := authDB.Select(&one, "SELECT 1"); err != nil {
		return authDBUnavailableStatus(authDB.Path(), err)
	}
	return adminAuthDBStatus{Available: true}
}
