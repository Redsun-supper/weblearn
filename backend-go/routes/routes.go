package routes

import (
	"backend-go/handlers"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

// SetupRouter 初始化并配置所有路由
// db 为 SQLite 数据库连接（由 main 注入），用于词汇复习相关接口
func SetupRouter(db *gorm.DB) *gin.Engine {
	// 根据环境变量设置运行模式
	// production模式下不输出调试日志
	// development模式下输出详细日志
	r := gin.Default()

	// API路由组，所有API接口都以/api为前缀
	api := r.Group("/api")
	{
		// 健康检查接口，用于验证服务器是否正常运行
		api.GET("/health", handlers.HealthCheck)

		// 示例接口，返回欢迎信息
		api.GET("/hello", handlers.Hello)

		// 用户相关路由
		user := api.Group("/user")
		{
			user.GET("/info", handlers.GetUserInfo)
			user.POST("/update", handlers.UpdateUserInfo)
		}

		// 数据相关路由
		data := api.Group("/data")
		{
			data.GET("/list", handlers.GetDataList)
			data.POST("/submit", handlers.SubmitData)
		}

		// ---- 词汇间隔复习（FSRS）相关路由 ----
		rv := handlers.NewReviewHandler(db)
		words := api.Group("/words")
		{
			words.GET("", rv.ListWords)
			words.POST("", rv.AddWords)
		}
		reviews := api.Group("/reviews")
		{
			reviews.GET("/due", rv.DueReviews)
			reviews.GET("/new", rv.NewWords)
			reviews.POST("/submit", rv.SubmitReview)
			reviews.GET("/stats", rv.ReviewStats)
		}
	}

	return r
}
