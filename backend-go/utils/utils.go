// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package utils

import (
	"fmt"
	"time"
)

// FormatTime 格式化为 "2006-01-02 15:04:05"
func FormatTime(t time.Time) string {
	return t.Format("2006-01-02 15:04:05")
}

// GetCurrentTime 取当前时间的格式化字符串（记录创建时间用）
func GetCurrentTime() string {
	return FormatTime(time.Now())
}

// GenerateID 以当前时间戳（纳秒）生成 ID
func GenerateID() string {
	return fmt.Sprintf("%d", time.Now().UnixNano())
}

// ValidateEmail 最基本的邮箱格式校验：只要求含 '@'
// TODO: 需要更严格时再上正则
func ValidateEmail(email string) bool {
	if len(email) == 0 {
		return false
	}
	for i := 0; i < len(email); i++ {
		if email[i] == '@' {
			return true
		}
	}
	return false
}
