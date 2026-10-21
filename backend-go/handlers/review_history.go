// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
package handlers

// 学习统计的按天明细：GET /api/reviews/history?days=84
//
// 给个人中心的「学习统计」面板用：热力图看每天的复习量，评分分布看「忘记」的占比。
// ⚠️ 与管理看板的 /api/admin/stats/trend 是两回事：那个是**全站**口径（跨用户、给管理员看），
// 这个只统计当前登录用户，且必须登录（挂在 /reviews 组上，未登录一律 401）。
//
// 刻意与管理看板保持一致的几处口径：
//   - 「今天」按服务端 Asia/Shanghai 自然日切（poolTZ；理由见 ReviewStats 上的说明）；
//   - 缺失日期补 0，前端不用自己补洞；
//   - `words` = 当天复习过的**不同词数**（与左下角「本轮已复习」同口径），
//     `reviews` = 当天的评分次数 —— 同一个词一天可能评多次，两个数字在复习量大的时候会分叉。

import (
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"backend-go/models"
)

const (
	historyDefaultDays = 84    // 12 周，热力图的标准窗口
	historyMaxDays     = 365   // 再长就是「全部历史」，给个上限防止刷参数
)

// 一天的统计（热力图的一格 + 评分分布的一份）
type historyPoint struct {
	Date    string `json:"date"`    // 北京自然日 "2006-01-02"
	Words   int64  `json:"words"`   // 当天复习过的不同词数
	Reviews int64  `json:"reviews"` // 当天的评分次数
	Again   int64  `json:"again"`   // 评分 1=Again
	Hard    int64  `json:"hard"`    // 评分 2=Hard
	Good    int64  `json:"good"`    // 评分 3=Good
	Easy    int64  `json:"easy"`    // 评分 4=Easy
}

// historyLogRow 是窗口内需要的几列（不取 IsDaily / IsProbe：面板用不到，
// 少取一列就少一份内存，365 天的窗口下这点差别开始有意义）。
type historyLogRow struct {
	UserID     uint      `gorm:"column:user_id"`
	WordID     uint      `gorm:"column:word_id"`
	Rating     uint8     `gorm:"column:rating"`
	ReviewedAt time.Time `gorm:"column:reviewed_at"`
}

// ReviewHistory 按天聚合当前用户的复习历史。
func (h *ReviewHandler) ReviewHistory(c *gin.Context) {
	userID, ok := currentUser(c)
	if !ok {
		return
	}

	days := historyDefaultDays
	if raw := strings.TrimSpace(c.Query("days")); raw != "" {
		v, err := strconv.Atoi(raw)
		if err != nil || v < 1 || v > historyMaxDays {
			c.JSON(http.StatusBadRequest, gin.H{"code": 400, "message": "days 只接受 1~365 天"})
			return
		}
		days = v
	}

	start := startOfBeijingDay(time.Now()).AddDate(0, 0, -(days - 1))
	points := make([]historyPoint, days)
	index := make(map[string]int, days)
	for i := 0; i < days; i++ {
		points[i] = historyPoint{Date: beijingDay(start.AddDate(0, 0, i))}
		index[points[i].Date] = i
	}

	// 窗口日志：粗筛后按边界再卡一次（与 admin 的 trend 同一套理由 ——
	// SQLite 里存的是带偏移的时间文本，字典序比较只在同格式下可靠）。
	var rows []historyLogRow
	if err := h.db.Model(&models.ReviewLog{}).
		Select("user_id, word_id, rating, reviewed_at").
		Where("user_id = ? AND reviewed_at >= ?", userID, adminSQLBoundary(coarseFilterStart(start))).
		Scan(&rows).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"code": 500, "message": "查询失败:" + err.Error()})
		return
	}

	words := make([]map[uint]struct{}, days)
	for _, row := range rows {
		if row.ReviewedAt.Before(start) {
			continue // 粗筛多带回来的行
		}
		i, ok := index[beijingDay(row.ReviewedAt)]
		if !ok {
			continue
		}
		points[i].Reviews++
		if words[i] == nil {
			words[i] = make(map[uint]struct{})
		}
		words[i][row.WordID] = struct{}{}
		switch row.Rating {
		case 1:
			points[i].Again++
		case 2:
			points[i].Hard++
		case 3:
			points[i].Good++
		case 4:
			points[i].Easy++
		}
	}
	for i := range points {
		points[i].Words = int64(len(words[i]))
	}

	c.JSON(http.StatusOK, gin.H{"code": 200, "message": "获取成功", "data": gin.H{
		"days":   days,
		"points": points,
	}})
}
