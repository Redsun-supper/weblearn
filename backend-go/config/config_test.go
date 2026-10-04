// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package config

import (
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
