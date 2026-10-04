package config

import (
	"bufio"
	"os"
	"strings"
)

// 允许来源（CSRF 白名单）默认值：与账号服务 backend-rust/src/config.rs 的默认值保持一致
// ——本机开发时浏览器访问的是 dev-server（127.0.0.1:8899），它会带这两个 Origin 之一。
const defaultAllowedOrigins = "http://127.0.0.1:8899,http://localhost:8899"

// Config 服务器配置
type Config struct {
	Host   string
	Port   string
	Env    string // 运行环境：development 或 production
	DBPath string // SQLite 数据库文件路径

	// 以下两项与账号服务（backend-rust）共享，用于本地验签登录令牌：
	// Go 自己用同一把密钥验签，不查库、也不回头问账号服务。
	// 口径与理由见 docs/backend-auth.md 与 docs/roadmap.md 第 2 节。
	JWTSecret      string   // AUTH_JWT_SECRET：HS256 共享密钥，必须与账号服务完全一致
	AllowedOrigins []string // AUTH_ALLOWED_ORIGINS：CSRF 白名单（浏览器 Origin）
}

// LoadConfig 加载配置：先读 .env 文件，再读环境变量，缺失时用默认值
// （SERVER_HOST / SERVER_PORT / APP_ENV / DB_PATH / AUTH_JWT_SECRET / AUTH_ALLOWED_ORIGINS）
//
// 为什么要读 .env：AUTH_JWT_SECRET 必须与账号服务一致，而 .env 已被 .gitignore 排除，
// 把密钥写在那里就不必每开一个终端手工 export（账号服务那边用 dotenvy 做同一件事）。
// 真实环境变量优先级更高（.env 不覆盖已有值），所以生产上用 systemd / 容器注入环境变量时，
// 这台机器上可以根本没有 .env 文件。
func LoadConfig() *Config {
	loadEnvFile()

	cfg := &Config{
		Host:           getEnv("SERVER_HOST", "0.0.0.0"),
		Port:           getEnv("SERVER_PORT", "8080"),
		Env:            getEnv("APP_ENV", "development"),
		DBPath:         getEnv("DB_PATH", "guangxue.db"),
		JWTSecret:      os.Getenv("AUTH_JWT_SECRET"),
		AllowedOrigins: splitList(getEnv("AUTH_ALLOWED_ORIGINS", defaultAllowedOrigins)),
	}
	return cfg
}

func getEnv(key, defaultValue string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return defaultValue
}

// IsProduction 是否生产环境。
// 与 Rust 侧（`Config::is_production`）保持同一口径：**只有小写 `production` 算生产**，
// 大小写写错的值一律当开发环境 —— 免得「本机调试时行为突然变了」变成查不出原因的问题。
func (c *Config) IsProduction() bool {
	return c.Env == "production"
}

// StartupWarnings 返回启动期该提醒的问题（空切片 = 没问题）。
//
// 为什么要有它：这几项配错**不会让服务起不来**，表现是「某类请求一直 401」
// 或「加固措施其实没生效」，事后从日志里翻很难对上号。
// 生产环境的每一项都会写成日志告警。
func (c *Config) StartupWarnings() []string {
	var warns []string
	if c.JWTSecret == "" {
		warns = append(warns, "AUTH_JWT_SECRET 未配置：所有需要登录的接口都会返回 401。"+
			"请在 backend-go/.env 里填上与账号服务 backend-rust/.env 相同的密钥（模板见 .env.example）")
	}
	if c.IsProduction() {
		// 生产应该只让 Nginx 访问，不该把 8080 暴露到公网
		if c.Host != "127.0.0.1" && c.Host != "localhost" {
			warns = append(warns, "生产环境建议把 SERVER_HOST 改成 127.0.0.1（只让 Nginx 访问），"+
				"当前是 "+c.Host+"：8080 会直接暴露在公网上，绕过 HTTPS 与 Nginx 的限流")
		}
		// 白名单漏配的后果是**所有写请求 403**（它同时是 CSRF 白名单与 CORS 依据）
		for _, origin := range c.AllowedOrigins {
			if strings.HasPrefix(origin, "http://127.0.0.1") || strings.HasPrefix(origin, "http://localhost") {
				warns = append(warns, "生产环境 AUTH_ALLOWED_ORIGINS 还带着本机地址（"+origin+"）："+
					"站点域名的写请求会一律 403。两个后端（backend-rust/.env 与 backend-go/.env）都要改成线上域名")
				break
			}
		}
	}
	return warns
}

// splitList 把 "a,b , c" 切成 ["a","b","c"]（跳过空项），用于逗号分隔的配置
func splitList(raw string) []string {
	parts := strings.Split(raw, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// loadEnvFile 按候选顺序找第一个存在的 .env 并载入。
//
// 候选顺序：$GX_ENV_FILE（显式指定，用于非常规启动方式）→ ./.env → ./backend-go/.env。
// 后两个是为了兼容「在 backend-go/ 里启动」和「在仓库根目录启动」两种习惯。
//
// 找不到文件不算错误：本机可以不配 .env（此时 AUTH_JWT_SECRET 为空，
// 所有需要登录的接口一律 401，main.go 启动时会打印醒目提示）。
func loadEnvFile() {
	for _, path := range envFileCandidates() {
		f, err := os.Open(path)
		if err != nil {
			continue
		}
		loadEnv(f)
		_ = f.Close()
		return
	}
}

func envFileCandidates() []string {
	candidates := make([]string, 0, 3)
	if custom := os.Getenv("GX_ENV_FILE"); custom != "" {
		candidates = append(candidates, custom)
	}
	return append(candidates, ".env", "backend-go/.env")
}

// loadEnv 解析 .env 的最小子集：KEY=VALUE 一行一条，# 开头是注释，允许 export 前缀，
// 值两端的成对引号会被去掉。
//
// 故意不做变量插值、多行值与 ${} 展开：本文件只有一个用途——放共享密钥与白名单，
// 支持多了反而看不出哪条生效。（账号服务用的是 dotenvy，规则比这里宽一点。）
func loadEnv(f *os.File) {
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		line = strings.TrimPrefix(line, "export ")

		key, value, found := strings.Cut(line, "=")
		if !found {
			continue
		}
		key = strings.TrimSpace(key)
		if key == "" {
			continue
		}
		value = unquote(strings.TrimSpace(value))

		// 已有环境变量优先：.env 只补空缺，不覆盖（生产注入的环境变量说了算）
		if _, exists := os.LookupEnv(key); exists {
			continue
		}
		_ = os.Setenv(key, value)
	}
}

// unquote 去掉成对的单/双引号
func unquote(value string) string {
	if len(value) >= 2 {
		if (value[0] == '"' && value[len(value)-1] == '"') || (value[0] == '\'' && value[len(value)-1] == '\'') {
			return value[1 : len(value)-1]
		}
	}
	return value
}
