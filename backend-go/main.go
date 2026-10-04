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

	// 配错但**不至于起不来**的那几项在这里统一告警（密钥为空、生产却监听 0.0.0.0、
	// 白名单还带着本机地址）。理由与每一条的后果见 Config.StartupWarnings 的注释。
	for _, warn := range cfg.StartupWarnings() {
		log.Printf("⚠️ %s", warn)
	}

	log.Printf("Go 后端启动：%s:%s（APP_ENV=%s）", cfg.Host, cfg.Port, cfg.Env)
	log.Fatal(router.Run(cfg.Host + ":" + cfg.Port))
}
