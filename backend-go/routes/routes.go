package routes

import (
	"backend-go/handlers"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

// SetupRouter 装配全部路由；db 由 main 注入，供词汇复习接口使用。
func SetupRouter(db *gorm.DB) *gin.Engine {
	// ⚠️ 没有按 APP_ENV 切到 gin.ReleaseMode，production 下同样会输出 gin 的调试日志
	r := gin.Default()

	api := r.Group("/api")
	{
		api.GET("/health", handlers.HealthCheck)

		api.GET("/hello", handlers.Hello)

		user := api.Group("/user")
		{
			user.GET("/info", handlers.GetUserInfo)
			user.POST("/update", handlers.UpdateUserInfo)
		}

		data := api.Group("/data")
		{
			data.GET("/list", handlers.GetDataList)
			data.POST("/submit", handlers.SubmitData)
		}

		// ---- 词汇间隔复习（FSRS）相关路由 ----
		rv := handlers.NewReviewHandler(db)
		// 词条管理：集合用 GET/POST，单条用 GET/PUT/DELETE（供后台增删改查）
		words := api.Group("/words")
		{
			words.GET("", rv.ListWords)
			words.POST("", rv.AddWords)
			words.GET("/:id", rv.GetWord)
			words.PUT("/:id", rv.UpdateWord)
			words.DELETE("/:id", rv.DeleteWord)
		}
		reviews := api.Group("/reviews")
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
