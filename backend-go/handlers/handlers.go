// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package handlers

import (
	"github.com/gin-gonic/gin"
	"net/http"
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

// 这里原先还有 GetUserInfo / UpdateUserInfo / GetDataList / SubmitData 四个占位处理器
// （对应 /api/user/*、/api/data/*）：返回的是假数据，且与账号服务 auth.db 里的 users 表
// 同名不同源，留着必被误用，已连同 models.User / models.DataItem 一起删除。
// 「当前用户是谁」统一走账号服务的 /api/auth/me。
