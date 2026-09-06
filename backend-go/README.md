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
| GET | /api/words | 单词列表（limit/offset/subject） |
| POST | /api/words | 批量添加单词 |
| GET | /api/reviews/due | 到期复习卡列表（limit/now） |
| GET | /api/reviews/new | 尚未加入复习的新词 |
| POST | /api/reviews/submit | 提交复习结果（FSRS 状态持久化） |
| GET | /api/reviews/stats | 复习统计 |

## 数据库（SQLite）

- 使用 [GORM](https://gorm.io) + [glebarez/sqlite](https://github.com/glebarez/sqlite)（纯 Go，无需 CGO）。
- 首次启动自动建表：`words`（词条）、`word_reviews`（每词 FSRS 记忆状态）、`review_logs`（复习日志）。
- 数据文件路径由环境变量 `DB_PATH` 控制，默认 `guangxue.db`。

### 词条示例

```bash
curl -X POST http://localhost:8080/api/words -H "Content-Type: application/json" -d '{
  "words": [
    {"word":"apple","phonetic":"/ˈæp.əl/","meaning":"n. 苹果","example":"I eat an apple."}
  ]
}'
```

### 复习流程

1. 前端调 `GET /api/reviews/due` 拿到期卡，`GET /api/reviews/new` 拿新词（用随机器抽批次）。
2. 用户评分后，前端 Rust/WASM 引擎用 `fsrs_next_states` 计算新记忆状态。
3. 前端调 `POST /api/reviews/submit` 持久化（服务端换算 due_at 并写日志）。

## 环境变量

| 变量名 | 说明 | 默认值 |
|--------|------|--------|
| SERVER_HOST | 服务器监听地址 | 0.0.0.0 |
| SERVER_PORT | 服务器端口 | 8080 |
| APP_ENV | 运行环境 | development |
| DB_PATH | SQLite 数据库文件路径 | guangxue.db |

## Nginx反向代理配置

```nginx
location /api/ {
    proxy_pass http://localhost:8080/;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```