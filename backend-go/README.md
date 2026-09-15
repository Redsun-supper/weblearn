# Go后端服务

> **账号系统不在本目录**：登录 / 注册 / 会话相关接口由独立的 Rust 认证服务承载
> （`backend-rust/`，监听 127.0.0.1:8081，只处理 `/api/auth/*`），数据库也是独立的
> `auth.db`。本服务继续负责词汇复习与词条管理，用 `guangxue.db`。
> 本地 `dev-server.js` 与线上 Nginx 都按前缀把两者分流，详见
> [`../backend-rust/README.md`](../backend-rust/README.md)。
>
> ⚠️ `/api/user/info`、`/api/user/update` 仍是**占位接口**（返回固定 JSON，不读写数据库），
> 它们与账号系统无关，已被 `/api/auth/me` 取代；将来要么删除、要么改成校验会话后
> 返回真实用户。目前**没有任何接口校验登录态**。

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
| GET | /api/reviews/queue | 复习队列：**所有已学词**按 `due_at` 升序（**含未到期**），分页带 `total` |
| GET | /api/reviews/probes | 每日抽查候选：已学词按 `due_at` **倒序**（越轮不到复习的越靠前） |
| POST | /api/reviews/submit | 提交复习结果（FSRS 状态持久化） |
| GET | /api/reviews/stats | 复习统计 |

### 复习队列接口说明（供学生端调度使用）

`queue` 与 `probes` 是同一份数据的两端：

- `queue` 给「整库按紧迫度排好序」的列表（`due_at ASC`），客户端引擎在这里面切「已过期 / 未到期」，
  再做梯度乱序（未到期每 10 个一块、块内打乱）。**到期与否只影响顺序，不影响是否有资格出现**——
  想多学就能一直往下翻，所以它同时承担了「到期卡」和「未到期但想提前复习」两种需求。
- `probes` 给「到期最远」的那几个，用于**每日抽查**：每天固定抽 5 个最轮不到复习的词提前确认，
  避免「总是快要过期的那些天天出现、间隔长的永远不出现」。客户端还会按 localStorage 里
  最近抽过的词再过滤一次，所以这里按 `due_at DESC` 多给几条候选。
- 两者字段与 `/api/reviews/due` 一致（复用 `dueCard` 结构），客户端解析代码无需区分。

`POST /api/reviews/submit` 请求体多一个 `is_probe`（默认 `false`）：抽查卡按新卡重算记忆状态，
其 `stability_after` 会明显低于 `before`。日志里记下这个标记，日后做 FSRS 参数优化
（`compute_parameters`）时应当排除这批记录，否则参数会被带偏。
⚠️ 日志里的 `stability_before` 取的是**库里那一行的真实旧值**，不是引擎的输入状态——
所以抽查不会被「今日新学」的统计（按 `stability_before = 0` 判定）误算成新词。

### 词条管理接口说明（供后台使用）

| 接口 | 要点 |
|------|------|
| `GET /api/words` | 返回 `{items, total, limit, offset}`；`search` 同时匹配**单词与释义**；`book`/`unit` 为精确匹配筛选 |
| `PUT /api/words/:id` | **全量更新**（后台表单会把所有字段一起提交，未填即清空）；`word` 为空返回 400，与其它词条重名返回 409 |
| `DELETE /api/words/:id` | ⚠️ **会连带删除该词的 `word_reviews` 与 `review_logs`**：日志留着会变成指向不存在词条的脏数据，使 `/api/reviews/stats` 的累计次数与保持率虚高。只是改错别字请用 `PUT`，不要删了重建 |
| `POST /api/words` | 查重**大小写不敏感**（`Abandon` 与 `abandon` 视为同一个词）；先一次性取回现有单词建索引再分批插入，适合一次导入上千词 |
| `GET /api/word-options` | 放在 `/api/word-options` 而非 `/api/words/options`，是为了避开与 `/api/words/:id` 的通配路由冲突 |

### 词条字段（`words` 表）

| 字段 | 说明 |
|------|------|
| `word` / `phonetic` / `meaning` / `example` | 单词 / 音标 / 释义 / 例句（`word` 唯一索引） |
| `example_translation` | 例句的中文翻译（可空） |
| `senses` | 多释义数组（可空）：`[{"pos":"n.","meaning":"好处；益处","example":"...","translation":"..."}]` |
| `book` / `unit` | 词书 / 单元，后台分组与筛选用（可空） |

`senses` 在 SQLite 里是**一列 JSON 文本**（`models.WordSenses`，实现了 `Value` / `Scan` / `MarshalJSON`）。

- 为什么不单开子表：释义永远跟着词条一起读写、从不单独查询，存 JSON 省掉一次 join，传输也少带 `id` / `word_id`；代价是不能用 SQL 直接查某条释义（本项目无此需求）。
- ⚠️ **空值必须序列化成 `[]` 而不是 `null`**（`Scan` 里先把接收者重置为非 nil 空切片）：前端 Rust 引擎按数组解析，遇到 `null` 会直接报错（`"items": null` 已经踩过一次）。
- 写入时 `normalizeSenses` 会去空白并丢掉「词性与释义都空」的行，后台表单留空的行不会进库。
- 读取时对坏 JSON 宽容（按「没有多释义」处理）：手工改坏一行的 JSON 不该让整个复习页打不开。

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

词表 JSON 结构（`subject` 可省略，默认 `english`；`example_translation` 与 `senses` 都可选）：

```json
{"words":[{"word":"apple","phonetic":"/ˈæp.əl/","meaning":"n. 苹果","example":"I eat an apple.",
           "example_translation":"我吃一个苹果。","subject":"english"},
          {"word":"benefit","phonetic":"/ˈben.ɪ.fɪt/","meaning":"n. 好处；益处 v. 有益于",
           "example":"Exercise has many benefits.","example_translation":"锻炼有很多好处。",
           "senses":[{"pos":"n.","meaning":"好处；益处"},{"pos":"v.","meaning":"有益于"}]}]}
```

`senses` 留空时不影响使用：复习界面会按 `meaning` 里的词性标签自动分块展示。

`words.word` 为唯一索引，重复词条自动跳过，因此该命令可反复执行。

## 数据库（SQLite）

- 使用 [GORM](https://gorm.io) + [glebarez/sqlite](https://github.com/glebarez/sqlite)（纯 Go，无需 CGO）。
- 首次启动自动建表：`words`（词条）、`word_reviews`（每词 FSRS 记忆状态）、`review_logs`（复习日志）。
- `AutoMigrate` 只新增缺失的表与**列**，不动已有数据：给 `words` 加 `example_translation` / `senses` 时，
  老库里的行会被补上 NULL（读出来即「没有翻译 / 没有多释义」），无需手工迁移。
- 数据文件路径由环境变量 `DB_PATH` 控制，默认 `guangxue.db`。

### 词条示例

```bash
curl -X POST http://localhost:8080/api/words -H "Content-Type: application/json" -d '{
  "words": [
    {"word":"apple","phonetic":"/ˈæp.əl/","meaning":"n. 苹果","example":"I eat an apple.",
     "example_translation":"我吃一个苹果。"},
    {"word":"benefit","phonetic":"/ˈben.ɪ.fɪt/","meaning":"n. 好处；益处 v. 有益于",
     "senses":[{"pos":"n.","meaning":"好处；益处","example":"Exercise has many benefits.","translation":"锻炼有很多好处。"}]}
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