package handlers

import (
	"net/http"
	"github.com/gin-gonic/gin"
)

// HealthCheck 健康检查接口
// 用途：验证服务器是否正常运行，常用于负载均衡器健康检测
// 请求方式：GET /api/health
// 返回：JSON格式的状态信息
func HealthCheck(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"status":  "ok",
		"message": "服务器运行正常",
	})
}

// Hello 欢迎接口
// 用途：示例接口，展示如何返回JSON响应
// 请求方式：GET /api/hello
// 返回：JSON格式的欢迎信息
func Hello(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"message": "欢迎使用Go后端API",
		"version": "1.0.0",
	})
}

// GetUserInfo 获取用户信息接口
// 用途：返回当前用户的基本信息
// 请求方式：GET /api/user/info
// 返回：JSON格式的用户信息
func GetUserInfo(c *gin.Context) {
	// TODO: 从数据库或缓存中获取用户信息
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

// UpdateUserInfo 更新用户信息接口
// 用途：接收客户端提交的用户信息并更新
// 请求方式：POST /api/user/update
// 请求体：JSON格式的用户信息
// 返回：JSON格式的更新结果
func UpdateUserInfo(c *gin.Context) {
	// TODO: 解析请求体中的用户信息
	// TODO: 验证数据合法性
	// TODO: 更新数据库中的用户信息
	
	c.JSON(http.StatusOK, gin.H{
		"code":    200,
		"message": "更新成功",
	})
}

// GetDataList 获取数据列表接口
// 用途：返回分页的数据列表
// 请求方式：GET /api/data/list?page=1&size=10
// 返回：JSON格式的数据列表
func GetDataList(c *gin.Context) {
	// 获取分页参数
	page := c.DefaultQuery("page", "1")
	size := c.DefaultQuery("size", "10")
	
	// TODO: 从数据库中查询数据
	
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

// SubmitData 提交数据接口
// 用途：接收客户端提交的数据并保存
// 请求方式：POST /api/data/submit
// 请求体：JSON格式的数据
// 返回：JSON格式的提交结果
func SubmitData(c *gin.Context) {
	// TODO: 解析请求体中的数据
	// TODO: 验证数据合法性
	// TODO: 保存到数据库
	
	c.JSON(http.StatusOK, gin.H{
		"code":    200,
		"message": "提交成功",
	})
}