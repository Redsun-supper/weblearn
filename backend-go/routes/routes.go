package routes

import (
	"backend-go/config"
	"backend-go/handlers"
	"backend-go/middleware"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

// SetupRouter 装配全部路由；db 与 cfg 由 main 注入
// （db 供词汇复习接口用；cfg 提供登录令牌的验签密钥与 CSRF 白名单）。
func SetupRouter(db *gorm.DB, cfg *config.Config) *gin.Engine {
	// ⚠️ 没有按 APP_ENV 切到 gin.ReleaseMode，production 下同样会输出 gin 的调试日志
	r := gin.Default()

	api := r.Group("/api")
	// CSRF 闸门挂在整个 /api 上（只读方法它自己会放行），改动状态的请求必须过这道闸
	api.Use(middleware.CSRFGuard(cfg.AllowedOrigins))
	{
		api.GET("/health", handlers.HealthCheck)

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
	}

	return r
}
