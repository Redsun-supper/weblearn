// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package handlers

import (
	"github.com/gin-gonic/gin"
	"net/http"
)

// apiVersion 本服务对外报告的版本号：GET /api/hello 的 version 与
// GET /api/health?deep=1 里 deep.version 共用这一个常量。
// 抽出来是因为外部监控会把深检里的 version 记进告警上下文，两处各写一遍字面量
// 迟早会出现「hello 说 1.1.0、健康检查说 1.0.0」这种自相矛盾。
const apiVersion = "1.0.0"

// Hello 示例接口：GET /api/hello
func Hello(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"message": "欢迎使用Go后端API",
		"version": apiVersion,
	})
}

// 健康检查在 health.go：浅检（GET /api/health）与深检（?deep=1）共用 /api/health 一条路由，
// 原来的 HealthCheck 函数已随之删除 —— 浅检的响应形状一字未改，但只留一份实现。

// 这里原先还有 GetUserInfo / UpdateUserInfo / GetDataList / SubmitData 四个占位处理器
// （对应 /api/user/*、/api/data/*）：返回的是假数据，且与账号服务 auth.db 里的 users 表
// 同名不同源，留着必被误用，已连同 models.User / models.DataItem 一起删除。
// 「当前用户是谁」统一走账号服务的 /api/auth/me。
