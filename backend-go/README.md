# Go后端服务

## 项目结构

```
backend-go/
├── main.go              ← 程序入口
├── go.mod               ← Go模块定义
── go.sum               ← 依赖校验文件
├── config/              ← 配置管理
│   └── config.go
── routes/              ← 路由定义
│   └── routes.go
├── handlers/            ← 请求处理器
│   └── handlers.go
├── models/              ← 数据模型
│   └── models.go
── utils/               ← 工具函数
    └── utils.go
```

## 快速开始

### 1. 安装依赖

```bash
cd backend-go
go mod tidy
```

### 2. 运行开发服务器

```bash
go run main.go
```

服务器将在 `http://localhost:8080` 启动

### 3. 构建生产版本

```bash
go build -o server main.go
./server
```

## API接口

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | /api/health | 健康检查 |
| GET | /api/hello | 欢迎信息 |
| GET | /api/user/info | 获取用户信息 |
| POST | /api/user/update | 更新用户信息 |
| GET | /api/data/list | 获取数据列表 |
| POST | /api/data/submit | 提交数据 |

## 环境变量

| 变量名 | 说明 | 默认值 |
|--------|------|--------|
| SERVER_HOST | 服务器监听地址 | 0.0.0.0 |
| SERVER_PORT | 服务器端口 | 8080 |
| APP_ENV | 运行环境 | development |

## Nginx反向代理配置

```nginx
location /api/ {
    proxy_pass http://localhost:8080/;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```