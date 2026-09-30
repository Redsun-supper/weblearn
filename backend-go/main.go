package main

import (
	"log"
	"backend-go/config"
	"backend-go/database"
	"backend-go/routes"
)

// main 启动顺序：加载配置 → 打开 SQLite（首次运行自动创建 guangxue.db 并建表/迁移）
// → 装配路由 → 监听端口。
func main() {
	cfg := config.LoadConfig()
	db := database.Init(cfg.DBPath)
	router := routes.SetupRouter(db, cfg)

	// 密钥没配时把话说清楚：复习接口会一律 401，现象看着像「登录了却一直要登录」，
	// 而真正的原因是 Go 与账号服务没拿到同一把 AUTH_JWT_SECRET
	if cfg.JWTSecret == "" {
		log.Printf("⚠️ AUTH_JWT_SECRET 未配置：所有需要登录的接口都会返回 401。" +
			"请在 backend-go/.env 里填上与账号服务 backend-rust/.env 相同的密钥（模板见 .env.example）")
	}

	log.Printf("Go后端服务器启动在 %s:%s", cfg.Host, cfg.Port)
	log.Fatal(router.Run(cfg.Host + ":" + cfg.Port))
}
