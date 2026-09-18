package config

import (
	"os"
)

// Config 服务器配置
type Config struct {
	Host   string
	Port   string
	Env    string // 运行环境：development 或 production
	DBPath string // SQLite 数据库文件路径
}

// LoadConfig 加载配置：优先读环境变量，缺失时用默认值
// （SERVER_HOST / SERVER_PORT / APP_ENV / DB_PATH）
func LoadConfig() *Config {
	cfg := &Config{
		Host:   getEnv("SERVER_HOST", "0.0.0.0"),
		Port:   getEnv("SERVER_PORT", "8080"),
		Env:    getEnv("APP_ENV", "development"),
		DBPath: getEnv("DB_PATH", "guangxue.db"),
	}
	return cfg
}

func getEnv(key, defaultValue string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return defaultValue
}