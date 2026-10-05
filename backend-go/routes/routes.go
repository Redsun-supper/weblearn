// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package routes

import (
	"backend-go/config"
	"backend-go/database"
	"backend-go/handlers"
	"backend-go/middleware"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

// SetupRouter 装配全部路由；db 与 cfg 由 main 注入
// （db 供词汇复习接口用；cfg 提供登录令牌的验签密钥与 CSRF 白名单）。
func SetupRouter(db *gorm.DB, cfg *config.Config) *gin.Engine {
	// 生产不能跑在 debug 模式：gin 在 debug 下会打印路由表、每次请求一行
	// `[GIN] 200 | 1.2ms | 1.2.3.4 | GET /api/...`，日志量翻好几倍，
	// 而且这些行会进 journald 长期留着。
	gin.SetMode(ginMode(cfg.Env))

	r := gin.Default()

	api := r.Group("/api")
	// CSRF 闸门挂在整个 /api 上（只读方法它自己会放行），改动状态的请求必须过这道闸
	api.Use(middleware.CSRFGuard(cfg.AllowedOrigins))
	{
		// 健康检查：浅检（默认）与深检（?deep=1）同一条路径，见 handlers/health.go。
		// 账号库句柄同样是惰性打开的：这里只记路径，起服务时不会碰 auth.db（与看板一致）。
		// 深检里账号库读不到**不算故障**（P1 口径），只有业务库查不了才回 503 ——
		// 外部监控只看状态码，所以这条区分必须在服务端做掉。
		hh := handlers.NewHealthHandler(db, database.NewAuthDB(cfg.AuthDBPath))
		api.GET("/health", hh.Check)

		api.GET("/hello", handlers.Hello)

		// 这里原先还有 /api/user/* 与 /api/data/* 四条占位接口（配 models.User / models.DataItem
		// 两张空表）：它们与账号服务 auth.db 里的 users 表同名不同源，留着必被误用，已整体删除。
		// 「当前用户是谁」统一走账号服务的 /api/auth/me。

		// ---- 词汇间隔复习（FSRS）相关路由 ----
		rv := handlers.NewReviewHandler(db)
		// 词条管理：集合用 GET/POST，单条用 GET/PUT/DELETE（供后台增删改查）
		// 读接口保持公开（后台列表与筛选下拉不需要登录），写接口要求管理员
		words := api.Group("/words")
		{
			words.GET("", rv.ListWords)
			words.POST("", middleware.RequireAdmin(cfg.JWTSecret), rv.AddWords)
			words.GET("/:id", rv.GetWord)
			words.PUT("/:id", middleware.RequireAdmin(cfg.JWTSecret), rv.UpdateWord)
			words.DELETE("/:id", middleware.RequireAdmin(cfg.JWTSecret), rv.DeleteWord)
		}
		// 复习进度是「个人的东西」，读写都要登录：未登录一律 401，
		// 前端收到 401 就引导去账号页登录（见 modules/english/english.js 的 gotoLogin）
		reviews := api.Group("/reviews", middleware.RequireUser(cfg.JWTSecret))
		{
			reviews.GET("/due", rv.DueReviews)
			reviews.GET("/new", rv.NewWords)
			reviews.GET("/queue", rv.QueueReviews)     // 整库按紧迫度排序（含未到期）
			reviews.GET("/probes", rv.ProbeCandidates) // 每日抽查候选（到期最远的）
			reviews.POST("/submit", rv.SubmitReview)
			reviews.GET("/stats", rv.ReviewStats)
		}
		// 词条里已使用的词书 / 单元列表（后台筛选下拉用）
		api.GET("/word-options", rv.WordOptions)

		// ---- 管理看板（只读统计，见 docs/launch-plan.md §5.0）----
		// 整组挂 RequireAdmin：没登录 401、登录了但不是 admin / super_admin 403。
		// 用 RequireAdmin 而**不是** RequireSuperAdmin：看板只是只读统计，管理员就该能看；
		// 发码 / 调权限 / 封号那类治理动作才收口到超管（那些接口在账号服务里）。
		//
		// ⚠️ `/users/progress` 与 `/users/:id/progress` 看着会撞（同一个位置既要有静态段
		// "progress"、又要把它当 :id 用），实测 gin 1.9 允许这样写：匹配时**静态子节点优先**，
		// 静态分支走不通才回退到 :id（tree.go 的 getValue 会给带通配子节点的位置留一份
		// skippedNode 快照用于回溯）。于是 `/users/progress` 命中批量接口、
		// `/users/7/progress` 命中单人接口 —— 两条路径都有路由测试钉住
		// （routes_admin_test.go），改这段之前先跑它们。
		// 之所以不改成 `/user-progress/:id` 之类的绕法：前端（批 3）与 launch-plan 记的就是这个形状。
		admin := api.Group("/admin", middleware.RequireAdmin(cfg.JWTSecret))
		{
			// 账号库句柄是惰性打开的：这里只记路径，起服务时不会碰 auth.db
			as := handlers.NewAdminStatsHandler(db, database.NewAuthDB(cfg.AuthDBPath))
			admin.GET("/stats/overview", as.Overview)
			admin.GET("/stats/trend", as.Trend)
			admin.GET("/users/progress", as.UsersProgress)
			admin.GET("/users/:id/progress", as.UserProgress)
		}
	}

	return r
}

// ginMode 把 APP_ENV 映射成 gin 的运行模式。
//
// 映射规则与 Rust 侧（`Config::is_production`）保持一致：**只有小写 `production`
// 算生产**，其余（development / 拼错的值）一律当开发环境。
// 刻意不做「非 development 就算生产」：把 `APP_ENV=Production` 这种大小写写错的值
// 当成生产，会让「本机调试时日志突然消失」变成一个查不出原因的问题。
func ginMode(appEnv string) string {
	if appEnv == "production" {
		return gin.ReleaseMode
	}
	return gin.DebugMode
}
