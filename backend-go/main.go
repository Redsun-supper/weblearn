package main

import (
	"log"
	"backend-go/config"
	"backend-go/database"
	"backend-go/routes"
)

func main() {
	// 加载配置文件
	cfg := config.LoadConfig()

	// 初始化数据库（SQLite：建表/迁移；首次运行自动创建 guangxue.db）
	db := database.Init(cfg.DBPath)

	// 初始化路由（带数据库连接）
	router := routes.SetupRouter(db)

	// 启动服务器
	log.Printf("Go后端服务器启动在 %s:%s", cfg.Host, cfg.Port)
	log.Fatal(router.Run(cfg.Host + ":" + cfg.Port))
}
