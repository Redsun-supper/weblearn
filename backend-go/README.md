# Go后端服务

## 项目结构

```
backend-go/
├── main.go              ← 程序入口
├── go.mod               ← Go模块定义
── go.sum               ← 依赖校验文件
├── config/              ← 配置管理
│   └── config.go
├── database/            ← SQLite 连接与自动迁移
│   └── database.go
├── cmd/
│   └── seed/            ← 词表导入命令（JSON → words 表）
│       └── main.go
├── seed/
│   └── words_english.json  ← 英语种子词表（100 词）
── routes/              ← 路由定义
│   └── routes.go
├── handlers/            ← 请求处理器
│   ├── handlers.go
│   └── review_handlers.go  ← 词汇复习（FSRS）处理器
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
| GET | /api/words | 词条列表（limit/offset/subject/book/unit/search） |
| POST | /api/words | 批量添加词条（已存在的跳过） |
| GET | /api/words/:id | 获取单个词条 |
| PUT | /api/words/:id | 更新词条内容（全量；改名冲突返回 409） |
| DELETE | /api/words/:id | 删除词条（连带删除其复习状态与日志） |
| GET | /api/word-options | 词条中已使用的词书 / 单元列表（后台筛选下拉用） |
| GET | /api/reviews/due | 到期复习卡列表（limit/now） |
| GET | /api/reviews/new | 尚未加入复习的新词 |
| POST | /api/reviews/submit | 提交复习结果（FSRS 状态持久化） |
| GET | /api/reviews/stats | 复习统计 |

### 词条管理接口说明（供后台使用）

| 接口 | 要点 |
|------|------|
| `GET /api/words` | 返回 `{items, total, limit, offset}`；`search` 同时匹配**单词与释义**；`book`/`unit` 为精确匹配筛选 |
| `PUT /api/words/:id` | **全量更新**（后台表单会把所有字段一起提交，未填即清空）；`word` 为空返回 400，与其它词条重名返回 409 |
| `DELETE /api/words/:id` | ⚠️ **会连带删除该词的 `word_reviews` 与 `review_logs`**：日志留着会变成指向不存在词条的脏数据，使 `/api/reviews/stats` 的累计次数与保持率虚高。只是改错别字请用 `PUT`，不要删了重建 |
| `POST /api/words` | 查重**大小写不敏感**（`Abandon` 与 `abandon` 视为同一个词）；先一次性取回现有单词建索引再分批插入，适合一次导入上千词 |
| `GET /api/word-options` | 放在 `/api/word-options` 而非 `/api/words/options`，是为了避开与 `/api/words/:id` 的通配路由冲突 |

### `/api/reviews/stats` 返回字段

| 字段 | 说明 |
|------|------|
| `total_words` | 词库总词数 |
| `new_words` | 尚未加入复习的新词数 |
| `due_cards` | 当前到期待复习数 |
| `reviewed_words` | 已加入复习（有记忆状态）的词数 |
| `total_reviews` | 累计复习次数（`review_logs` 行数） |
| `today_reviewed` | 今日已复习次数（服务器本地时区当天零点起算） |
| `streak_days` | 连续复习天数；今日尚未复习时从昨天起算，避免当天开始即显示断签 |
| `retention_rate` | 记忆保持率 = 非「忘记」评分占比（0~1，三位小数） |

## 词表导入

复习功能需要 `words` 表中有词条。用 `cmd/seed` 从 JSON 词表导入：

```bash
cd backend-go

# 默认使用 seed/words_english.json，数据库取环境变量 DB_PATH（默认 guangxue.db）
go run ./cmd/seed

# 指定词表与数据库
go run ./cmd/seed -file seed/my_words.json -db guangxue.db
```

词表 JSON 结构（`subject` 可省略，默认 `english`）：

```json
{"words":[{"word":"apple","phonetic":"/ˈæp.əl/","meaning":"n. 苹果","example":"I eat an apple.","subject":"english"}]}
```

`words.word` 为唯一索引，重复词条自动跳过，因此该命令可反复执行。

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
# 注意：proxy_pass 末尾不要带 /，否则 /api/xxx 会被改写成 /xxx 而 404
location /api/ {
    proxy_pass http://localhost:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```