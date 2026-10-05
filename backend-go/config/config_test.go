// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestIsProductionOnlyAcceptsLowercase(t *testing.T) {
	// 与 Rust 侧 Config::is_production 同口径：只有小写 production 算生产。
	// 大小写写错时宁可当开发（多打日志），也不要让「本机调试行为突然变了」无从解释。
	cases := map[string]bool{
		"production":  true,
		"development": false,
		"Production":  false,
		"PRODUCTION":  false,
		"prod":        false,
		"":            false,
	}
	for env, want := range cases {
		cfg := &Config{Env: env}
		if got := cfg.IsProduction(); got != want {
			t.Errorf("APP_ENV=%q 的 IsProduction 期望 %v，实际 %v", env, want, got)
		}
	}
}

func TestStartupWarningsInDevelopment(t *testing.T) {
	// 开发环境不啰嗦：只提醒「密钥没配」（它会让所有要登录的接口静默 401）
	cfg := &Config{
		Env:            "development",
		Host:           "0.0.0.0", // 开发时监听全网卡是正常的
		AllowedOrigins: []string{"http://127.0.0.1:8899"},
	}
	warns := cfg.StartupWarnings()
	if len(warns) != 1 {
		t.Fatalf("开发环境只该有「密钥未配置」一条告警，实际 %d 条：%v", len(warns), warns)
	}
	if !strings.Contains(warns[0], "AUTH_JWT_SECRET") {
		t.Fatalf("告警要点名 AUTH_JWT_SECRET，实际：%s", warns[0])
	}

	cfg.JWTSecret = "已经配好了"
	if got := cfg.StartupWarnings(); len(got) != 0 {
		t.Fatalf("开发环境配好密钥后不该有告警，实际：%v", got)
	}
}

func TestStartupWarningsInProduction(t *testing.T) {
	// 生产环境的三种「配错但不至于起不来」都要被点出来
	cases := []struct {
		name string
		cfg  Config
		want string
	}{
		{
			name: "监听 0.0.0.0",
			cfg: Config{
				Env:            "production",
				Host:           "0.0.0.0",
				JWTSecret:      "已配",
				AllowedOrigins: []string{"https://test.lovezmx.com"},
			},
			want: "SERVER_HOST",
		},
		{
			name: "白名单还带着本机地址",
			cfg: Config{
				Env:            "production",
				Host:           "127.0.0.1",
				JWTSecret:      "已配",
				AllowedOrigins: []string{"http://127.0.0.1:8899"},
			},
			want: "AUTH_ALLOWED_ORIGINS",
		},
		{
			name: "密钥没配",
			cfg: Config{
				Env:            "production",
				Host:           "127.0.0.1",
				AllowedOrigins: []string{"https://test.lovezmx.com"},
			},
			want: "AUTH_JWT_SECRET",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			joined := strings.Join(tc.cfg.StartupWarnings(), "\n")
			if !strings.Contains(joined, tc.want) {
				t.Fatalf("应当提示 %s，实际告警：\n%s", tc.want, joined)
			}
		})
	}
}

func TestStartupWarningsQuietWhenProductionIsProperlyConfigured(t *testing.T) {
	// 配对了就不该有任何告警 —— 否则真上线时一堆噪音会让人忽略真正的问题
	cfg := &Config{
		Env:            "production",
		Host:           "127.0.0.1",
		JWTSecret:      "足够长的随机值",
		AllowedOrigins: []string{"https://test.lovezmx.com"},
	}
	if got := cfg.StartupWarnings(); len(got) != 0 {
		t.Fatalf("生产配置正确时不该有告警，实际：%v", got)
	}

	// localhost 与 127.0.0.1 都算「只让 Nginx 访问」
	cfg.Host = "localhost"
	if got := cfg.StartupWarnings(); len(got) != 0 {
		t.Fatalf("SERVER_HOST=localhost 不该告警，实际：%v", got)
	}
}

// TestStartupWarningsAuthDBPath 账号库（管理看板的账号侧数据源）读不到要提前喊一声。
//
// 为什么值得一条专门的用例：配错 AUTH_DB_PATH **不会**让服务起不来，症状只是看板账号侧
// 全是 0 —— 与「今天真没人注册」长得一模一样，事后从日志里翻很难对上号。
func TestStartupWarningsAuthDBPath(t *testing.T) {
	dir := t.TempDir()
	existing := filepath.Join(dir, "auth.db")
	if err := os.WriteFile(existing, []byte("占位文件（这一层只判断能不能打开读）"), 0o600); err != nil {
		t.Fatalf("造临时文件失败: %v", err)
	}
	missing := filepath.Join(dir, "not-here.db")

	cfg := &Config{Env: "development", JWTSecret: "已配", AuthDBPath: missing}
	warns := cfg.StartupWarnings()
	if len(warns) != 1 {
		t.Fatalf("账号库读不到时应当正好一条告警，实际 %d 条：%v", len(warns), warns)
	}
	// 文案要说清三件事：哪个配置项、什么后果、去看哪里
	for _, want := range []string{"AUTH_DB_PATH", missing, "管理看板"} {
		if !strings.Contains(warns[0], want) {
			t.Fatalf("告警里应当出现 %q，实际：%s", want, warns[0])
		}
	}

	// 生产环境**同样只告警**（不是致命错误）：其余项都配好时，只多出这一条
	cfg.Env = "production"
	cfg.Host = "127.0.0.1"
	cfg.AllowedOrigins = []string{"https://test.lovezmx.com"}
	prodWarns := cfg.StartupWarnings()
	if len(prodWarns) != 1 || !strings.Contains(prodWarns[0], "AUTH_DB_PATH") {
		t.Fatalf("生产环境下账号库读不到也只该有这一条告警（不致命），实际：%v", prodWarns)
	}

	// 文件存在（这一层只回答「能不能打开读」）：不再告警
	cfg.Env = "development"
	cfg.AuthDBPath = existing
	if got := cfg.StartupWarnings(); len(got) != 0 {
		t.Fatalf("账号库文件读得到时不该告警，实际：%v", got)
	}

	// 空路径 = 这份配置不管账号库（直接构造 Config 的场景），不告警：
	// LoadConfig 一定会给出默认值，所以真实启动路径上不会出现空值
	cfg.AuthDBPath = ""
	if got := cfg.StartupWarnings(); len(got) != 0 {
		t.Fatalf("空路径不该告警，实际：%v", got)
	}
}

// TestLoadConfigAuthDBPath 默认值与覆盖：默认指到账号服务的 auth.db（相对工作目录），
// 而环境变量里配了就以配的为准。
func TestLoadConfigAuthDBPath(t *testing.T) {
	t.Setenv("AUTH_DB_PATH", "")
	if got := LoadConfig().AuthDBPath; got != defaultAuthDBPath {
		t.Fatalf("AUTH_DB_PATH 未配置时期望默认值 %q，实际 %q", defaultAuthDBPath, got)
	}

	t.Setenv("AUTH_DB_PATH", "/opt/guangxue/backend-rust/auth.db")
	if got := LoadConfig().AuthDBPath; got != "/opt/guangxue/backend-rust/auth.db" {
		t.Fatalf("AUTH_DB_PATH 配了就该用配的，实际 %q", got)
	}
}
