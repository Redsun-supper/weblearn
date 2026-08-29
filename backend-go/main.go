package main

import (
	"log"
	"backend-go/config"
	"backend-go/routes"
)

func main() {
	// 加载配置文件
	cfg := config.LoadConfig()
	
	// 初始化路由
	router := routes.SetupRouter()
	
	// 启动服务器
	log.Printf("Go后端服务器启动在 %s:%s", cfg.Host, cfg.Port)
	log.Fatal(router.Run(cfg.Host + ":" + cfg.Port))
}