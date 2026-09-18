package handlers

import (
	"net/http"
	"github.com/gin-gonic/gin"
)

// HealthCheck 健康检查：GET /api/health（供负载均衡 / 探活使用）
func HealthCheck(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"status":  "ok",
		"message": "服务器运行正常",
	})
}

// Hello 示例接口：GET /api/hello
func Hello(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"message": "欢迎使用Go后端API",
		"version": "1.0.0",
	})
}

// GetUserInfo 获取用户信息：GET /api/user/info
// TODO: 目前返回固定数据，还没有真的从数据库 / 缓存里取
func GetUserInfo(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"code":    200,
		"message": "获取成功",
		"data": gin.H{
			"id":       1,
			"username": "demo_user",
			"email":    "demo@example.com",
		},
	})
}

// UpdateUserInfo 更新用户信息：POST /api/user/update
// TODO: 占位实现——解析请求体、校验合法性、写库都还没做
func UpdateUserInfo(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"code":    200,
		"message": "更新成功",
	})
}

// GetDataList 获取数据列表：GET /api/data/list?page=1&size=10
// TODO: 目前只回显分页参数，还没有真的查库
func GetDataList(c *gin.Context) {
	page := c.DefaultQuery("page", "1")
	size := c.DefaultQuery("size", "10")

	c.JSON(http.StatusOK, gin.H{
		"code":    200,
		"message": "获取成功",
		"data": gin.H{
			"page": page,
			"size": size,
			"total": 0,
			"items": []interface{}{},
		},
	})
}

// SubmitData 提交数据：POST /api/data/submit
// TODO: 占位实现——解析请求体、校验合法性、入库都还没做
func SubmitData(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"code":    200,
		"message": "提交成功",
	})
}