# 广学

## ⚡ 我的爱发电主页

如果这个项目对你有帮助，欢迎在爱发电支持我！

[👉 在爱发电支持我](https://afdian.com/a/hr_redsun)

---

一个学科内容展示平台，支持多科目内容加载与本地缓存。

---

## 项目架构

```
┌─────────────────────────────────────────────────────────────────┐
│                         用户浏览器                                │
│                    (HTTPS: test.lovezmx.com)                     │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Nginx 反向代理                             │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  location /        → 前端静态文件 (frontend/)              │  │
│  │  location /api/    → Go后端服务 (localhost:8080)           │  │
│  └───────────────────────────────────────────────────────────┘  │
────────────────────────────┬────────────────────────────────────┘
                             │
              ┌──────────────┴──────────────┐
              ▼                              ▼
┌─────────────────────────┐    ┌─────────────────────────┐
│       前端 (Frontend)    │    │      后端 (Backend)      │
│                         │    │                         │
│  index.html             │    │  Go (Gin框架)           │
│  main.css               │    │                         │
│  main.js                │    │  ├── main.go            │
│  image/                 │    │  ├── config/            │
│  pages/                 │    │  ├── routes/            │
│                         │    │  ├── handlers/          │
│  特性:                   │    │  ├── models/           │
│  - 学科导航              │    │  └── utils/            │
│  - localStorage缓存      │    │                         │
│  - 30天自动清理          │    │  特性:                   │
│  - 默认显示英语          │    │  - RESTful API          │
│                         │    │  - 健康检查              │
│                         │    │  - 用户管理              │
└─────────────────────────    │  - 数据管理              │
                               └─────────────────────────┘
```

---

## 目录结构

```
e:\porject\4/
├── frontend/                    # 前端静态文件（根目录即站点根）
│   ├── index.html               # 主页面入口
│   ├── main.css                 # 全站样式（导航栏、内容容器等）
│   ├── main.js                  # 导航交互与 localStorage 缓存管理
│   ├── image/                   # 图片资源
│   │   └── avatar.png           # 头像图片
│   └── pages/                   # 尚未建模块的学科占位页（其余 8 门，均为「敬请期待」）
│       ├── chinese.html         # 语文
│       ├── math.html            # 数学
│       ├── physics.html         # 物理
│       ├── chemistry.html       # 化学
│       ├── biology.html         # 生物
│       ├── history.html         # 历史
│       ├── politics.html        # 政治
│       └── geography.html       # 地理
│
├── dev-server.js                # 本地开发服务器（静态文件 + /api 反向代理，仅开发用）
│
├── modules/                     # 学科模块（每个学科一个目录，独立管理）
│   └── english/                 # 英语模块（唯一已实现的学科）
│       ├── english.html         #   学科页片段（由 main.js 注入 #contentContainer）
│       ├── english.css          #   模块样式（只作用于英语页）
│       ├── english.js           #   模块逻辑：DOM / fetch / 存储 / 语音
│       ├── README.md            #   模块说明
│       └── engine/              #   Rust/WASM 引擎（队列编排 + FSRS 调度 + 随机化）
│           ├── Cargo.toml
│           ├── src/
│           │   ├── lib.rs           # 模块导出
│           │   ├── session.rs       # 会话编排（ReviewSession：队列/游标/评分/进度）
│           │   ├── fsrs_engine.rs   # FSRS 调度计算
│           │   └── randomizer.rs    # 随机器（洗牌/抽样/种子）
│           └── README.md            # 引擎说明与构建命令
│
├── backend-go/                  # Go后端服务
│   ├── main.go                  # 程序入口
│   ├── go.mod                   # Go模块定义
│   ├── config/
│   │   └── config.go            # 配置管理
│   ├── database/
│   │   └── database.go          # SQLite 连接与自动迁移
│   ├── cmd/
│   │   └── seed/
│   │       └── main.go          # 词表导入命令（JSON → words 表）
│   ├── seed/
│   │   └── words_english.json   # 英语种子词表（100 词）
│   ├── routes/
│   │   └── routes.go            # 路由定义
│   ├── handlers/
│   │   ├── handlers.go          # 请求处理器
│   │   └── review_handlers.go   # 词汇复习（FSRS）处理器
│   ├── models/
│   │   └── models.go            # 数据模型（含 Word/WordReview/ReviewLog）
│   ├── utils/
│   │   └── utils.go             # 工具函数
│   └── README.md                # 后端说明文档
│
└── README.md                    # 项目总说明（本文件）
```

---

## 学科模块约定

每个学科的东西放在一个目录里（`modules/<学科>/`），便于独立管理与演进：

| 文件 | 职责 |
|------|------|
| `<学科>.html` | 学科页片段，由 `main.js` fetch 后注入 `#contentContainer` |
| `<学科>.css` | 模块样式，只作用于该学科页内的元素 |
| `<学科>.js` | 模块逻辑（ES module），导出初始化函数，由 `main.js` **按需动态 import** |
| `engine/` | 该学科的 Rust/WASM 引擎（纯计算：调度、随机化、统计等） |

`main.js` 里只保留一处学科相关代码——`initSubjectModule(pageName)`：按 `data-page`
判断并动态 `import()` 对应模块。**好处是学科逻辑不会回流到 `main.js`，且只有真正进入
该学科页才会加载它的模块与引擎。**

> 目前只有 `modules/english/` 建好了；其余 8 门仍是 `pages/<学科>.html` 占位页。
> 将来把某门学科做成模块时，把它从 `pages/` 移进 `modules/<学科>/`，
> 并在 `index.html` 的 `data-page` 与 `main.js` 的 `ENGLISH_PAGE` 旁登记即可。

---

## 技术栈

### 前端
| 技术 | 用途 |
|------|------|
| HTML5 | 页面结构 |
| CSS3 | 样式与布局 |
| JavaScript (ES5) | 交互逻辑 |
| localStorage | 本地缓存（30天过期） |
| fetch API | HTTP请求 |

### 后端
| 技术 | 用途 |
|------|------|
| Go 1.21 | 后端语言 |
| Gin | Web框架 |
| GORM + SQLite (glebarez) | 数据持久化（纯 Go，无 CGO） |
| Nginx | 反向代理与静态文件服务 |

### 复习引擎
| 技术 | 用途 |
|------|------|
| Rust + wasm-bindgen | 前端 WASM 模块（在浏览器中运行） |
| fsrs v6 (fsrs-rs) | FSRS 间隔复习调度算法 |
| serde / serde_json | 引擎 JSON 数据交换 |

---

## 请求流程

```
用户点击学科按钮
        │
        ▼
┌──────────────────┐
│ 检查localStorage  │
│ 是否有缓存？       │
└─────────────────┘
         │
    ────┴────┐
    │         │
   有         无
    │         │
    ▼         ▼
──────┐  ┌──────────┐
│直接  │  │fetch请求 │
│显示  │  │服务器    │
└──────┘  └────┬─────┘
               │
               ▼
         ┌──────────┐
         │保存到    │
         │localStorage│
         └────┬─────┘
              │
              ▼
         ┌──────┐
         │显示  │
         │内容  │
         └──────
```

---

## API接口

### 后端API（Go）

| 方法 | 路径 | 说明 | 状态 |
|------|------|------|------|
| GET | `/api/health` | 健康检查 | ✅ 可用 |
| GET | `/api/hello` | 欢迎信息 | ✅ 可用 |
| GET | `/api/user/info` | 获取用户信息 | 🔄 待实现 |
| POST | `/api/user/update` | 更新用户信息 | 🔄 待实现 |
| GET | `/api/data/list` | 获取数据列表 | 🔄 待实现 |
| POST | `/api/data/submit` | 提交数据 | 🔄 待实现 |
| GET | `/api/words` | 单词列表 | ✅ 可用 |
| POST | `/api/words` | 批量添加单词 | ✅ 可用 |
| GET | `/api/reviews/due` | 到期复习卡列表 | ✅ 可用 |
| GET | `/api/reviews/new` | 未加入复习的新词 | ✅ 可用 |
| POST | `/api/reviews/submit` | 提交复习结果（持久化 FSRS 状态） | ✅ 可用 |
| GET | `/api/reviews/stats` | 复习统计（词库概览 / 今日进度 / 连续天数 / 记忆保持率） | ✅ 可用 |

---

## 缓存机制

### localStorage缓存策略

```
┌─────────────────────────────────────────────────────┐
│                   缓存生命周期                        │
│                                                     │
│  首次访问 → 请求服务器 → 保存到localStorage          │
│       │                                             │
│       ▼                                             │
│  再次访问 → 直接读取localStorage（零带宽）            │
│       │                                             │
│       ▼                                             │
│  30天未访问 → 自动清理过期缓存                       │
│                                                     │
│  存储空间不足 → 自动清理过期缓存后重试                │
└─────────────────────────────────────────────────────┘
```

### 缓存数据结构

| 键名 | 用途 |
|------|------|
| `pageCache_<学科页路径>` | 存储页面HTML内容（如 `pageCache_modules/english/english.html`） |
| `pageCache_meta` | 存储各页面最后访问时间 |
| `pageCache_version` | 缓存结构版本号；版本升级时一次性清除所有旧页面缓存 |

> ⚠️ 修改学科页结构（例如给英语页新增按钮）后，必须把 `main.js` 里的 `CACHE_VERSION` +1，
> 否则老用户会继续用 localStorage 中的旧页面结构，导致新脚本找不到对应元素、功能不可用。

---

## 本地启动测试

前端使用**绝对路径**请求 `/api/*`（线上由 Nginx 反向代理），因此**不能直接双击 `index.html` 打开**：
`file://` 协议下 `/api` 请求会失败，WASM 的 ES 模块加载也会被浏览器拦截。
本地必须让「静态文件」与「API」处于**同一个 origin**，仓库根目录的 `dev-server.js` 就是为此准备的
（只用 Node 内置模块，无需 `npm install`）。

### 1. 导入词库（首次，否则英语复习没有词）

```bash
cd backend-go
go run ./cmd/seed
```

### 2. 启动后端

```bash
cd backend-go
go run main.go        # 监听 0.0.0.0:8080
```

### 3. 启动前端服务器

```bash
# 在仓库根目录执行
node dev-server.js                   # 默认 http://127.0.0.1:8899
node dev-server.js --port 9000        # 前端端口被占用时换一个
node dev-server.js --api-port 8081    # 后端换了端口时对齐
```

### 4. 打开浏览器

访问 **http://127.0.0.1:8899** 即可。首页默认加载英语页，下拉即是「单词复习」。

### 常见问题

| 现象 | 原因与处理 |
|------|-----------|
| 页面提示「复习功能加载失败」 | 后端没启动或端口不是 8080；`dev-server.js` 控制台会打印 `[proxy error]` 说明具体原因 |
| 提示「词库为空」或「暂无需要复习的单词」 | 还没导入词表，执行 `go run ./cmd/seed`（见上一步） |
| 改了学科页（`modules/<学科>/*.html` 或 `pages/*.html`）却不生效 | 学科页被 **localStorage 缓存了 30 天**：DevTools → Application → Local Storage 删除 `pageCache_*` 键，或在 Console 执行 `localStorage.clear()` 后刷新；也可以把 `main.js` 里的 `CACHE_VERSION` +1 强制全体用户失效 |
| 改了 `main.js` / `main.css` 却不生效 | 浏览器 HTTP 缓存。`dev-server.js` 已发送 `Cache-Control: no-store`；若仍异常请硬刷新（Ctrl+F5） |
| 端口 8080 / 8899 被占用 | 后端用环境变量换端口（如 `SERVER_PORT=8081`），前端用 `--api-port 8081` 对齐；前端自身用 `--port` 换 |
| 想要与线上完全一致的形态 | 用 Nginx 反向代理：`root` 指向仓库根目录、`proxy_pass` 指向 `127.0.0.1:8080`（见「部署说明」） |

> 💡 `dev-server.js` 仅用于本地开发，部署时无需上传（线上由 Nginx 承担同样的职责）。

---

## 单词表导入（本地开发）

英语复习依赖 `words` 表中的词条。首次跑复习功能前，先导入词表：

```bash
cd backend-go

# 使用仓库自带的英语种子词表（100 词，seed/words_english.json）
go run ./cmd/seed

# 或指定自己的词表与数据库
go run ./cmd/seed -file my_words.json -db guangxue.db
```

词表 JSON 结构：

```json
{
  "words": [
    {"word": "apple", "phonetic": "/ˈæp.əl/", "meaning": "n. 苹果", "example": "I eat an apple.", "subject": "english"}
  ]
}
```

- `subject` 省略时默认为 `english`，为将来其他学科的词汇留出扩展位。
- `words.word` 是唯一索引，重复单词自动跳过，因此命令**可反复执行**（幂等）。
- 换成自己的词表（中考 / 高考 / 四六级等）时，保持同样的 JSON 结构即可。
- SQLite 文件与词表都是本地数据：`*.db` 已在 `.gitignore` 中忽略，词表 JSON 则在版本控制内。

---

## 复习页交互（英语）

| 操作 | 说明 |
|------|------|
| 空格 / 回车 | 显示答案（揭晓释义与例句，并放出评分按钮） |
| `1` / `2` / `3` / `4` | 评分：忘记 / 困难 / 良好 / 简单（**仅在显示答案之后生效**） |
| `P` | 朗读当前单词 |
| `E` | 朗读当前例句 |
| 自动朗读 | 卡片右上角开关，选择状态保存在 localStorage（`reviewAutoSpeak`） |

> **设计要点：为什么答案默认隐藏？**
> 进入卡片时只显示单词与音标，释义/例句保持隐藏，评分必须发生在揭晓之后。
> 因为 FSRS 的 `stability` / `difficulty` 依赖「是否真的想起来」这一真实信号：
> 若一边看着答案一边打分，评分会系统性偏高，记忆状态随之失真，
> `review_logs` 也就失去了后续参数优化（`compute_parameters`）的价值。

---

## 部署说明

### 服务器目录结构

```
/var/www/test.lovezmx.com/
├── frontend/          # 前端文件
├── backend-go/        # 后端代码
── nginx.conf         # Nginx配置
```

### Nginx配置示例

```nginx
server {
    listen 443 ssl;
    server_name test.lovezmx.com;

    # SSL证书配置
    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    # 前端静态文件
    location / {
        root /var/www/test.lovezmx.com/frontend;
        index index.html;
        try_files $uri $uri/ /index.html;
    }

    # Go后端API
    # 注意：proxy_pass 末尾不要带 / —— 带斜杠时 nginx 会把匹配到的 /api/ 前缀替换掉，
    # /api/health 会被转发成 /health，与后端注册的 /api/health 不匹配而 404
    location /api/ {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}
```

### 启动后端服务

```bash
# 安装依赖
cd backend-go
go mod tidy

# 开发模式运行
go run main.go

# 生产模式构建
go build -o server main.go
./server
```

### 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `SERVER_HOST` | 监听地址 | `0.0.0.0` |
| `SERVER_PORT` | 端口 | `8080` |
| `APP_ENV` | 运行环境 | `development` |

---

## 开发计划

- [x] 前端基础架构
- [x] 学科导航功能
- [x] localStorage缓存机制
- [x] 30天自动清理
- [x] Go后端基础框架
- [x] 词汇间隔复习引擎（Rust/WASM：FSRS 调度 + 随机器）
- [x] 数据库集成（SQLite + GORM，自动迁移完成）
- [ ] 用户认证系统
- [ ] 学科内容完善
- [ ] 响应式优化
- [x] 复习页面 UI（简单版：`modules/english/english.html` + WASM 引擎）
- [x] 英语模块独立成 `modules/english/`（页面/样式/逻辑/引擎集中管理）
- [x] 计算下沉 Rust：会话编排、日期换算、FSRS 调度、进度统计移入引擎（26 个单元测试）
- [x] 主动回忆流程（先回想 → 显示答案 → 评分，避免「看着答案打分」污染 FSRS 状态）
- [x] 单词 / 例句发音（Web Speech API）与键盘快捷键（空格、1~4、P、E）
- [x] 复习统计面板（今日进度、连续天数、记忆保持率、本轮进度条）
- [x] 英语种子词表与导入命令（`backend-go/cmd/seed`，100 词）
- [ ] 复习页面 UI（进阶：完整词义卡交互、统计曲线图表等）
- [ ] FSRS 参数优化（基于 review_logs 的 compute_parameters）

---

## 支持作者

如果这个项目对你有帮助，欢迎在爱发电支持我！

[⚡ 在爱发电支持我](https://afdian.com/a/hr_redsun)

---

## 许可证

MIT License