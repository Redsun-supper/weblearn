package utils

import (
	"fmt"
	"time"
)

// FormatTime 格式化时间
// 用途：将时间对象转换为指定格式的字符串
// 参数：t - 要格式化的时间
// 返回：格式化后的时间字符串，格式为 "2006-01-02 15:04:05"
func FormatTime(t time.Time) string {
	return t.Format("2006-01-02 15:04:05")
}

// GetCurrentTime 获取当前时间字符串
// 用途：获取当前时间的格式化字符串，常用于记录创建时间
// 返回：当前时间的字符串表示
func GetCurrentTime() string {
	return FormatTime(time.Now())
}

// GenerateID 生成唯一ID
// 用途：基于时间戳生成唯一标识符
// 返回：唯一ID字符串
func GenerateID() string {
	return fmt.Sprintf("%d", time.Now().UnixNano())
}

// ValidateEmail 验证邮箱格式
// 用途：检查邮箱地址是否符合基本格式要求
// 参数：email - 要验证的邮箱地址
// 返回：true表示格式正确，false表示格式错误
func ValidateEmail(email string) bool {
	// 简单的邮箱格式验证
	// TODO: 可以使用正则表达式进行更严格的验证
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