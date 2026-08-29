package config

import (
	"os"
)

// Config 服务器配置结构体
type Config struct {
	Host string
	Port string
	Env  string // 运行环境：development 或 production
}

// LoadConfig 加载配置
// 优先从环境变量读取，如果没有则使用默认值
func LoadConfig() *Config {
	cfg := &Config{
		Host: getEnv("SERVER_HOST", "0.0.0.0"),
		Port: getEnv("SERVER_PORT", "8080"),
		Env:  getEnv("APP_ENV", "development"),
	}
	return cfg
}

// getEnv 获取环境变量，如果不存在则返回默认值
func getEnv(key, defaultValue string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return defaultValue
}