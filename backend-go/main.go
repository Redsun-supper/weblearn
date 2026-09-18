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
	router := routes.SetupRouter(db)

	log.Printf("Go后端服务器启动在 %s:%s", cfg.Host, cfg.Port)
	log.Fatal(router.Run(cfg.Host + ":" + cfg.Port))
}
