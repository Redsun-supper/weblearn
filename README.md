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
│  │  location /            → 前端静态文件（仓库根目录）          │  │
│  │  location /api/auth/   → Rust 账号系统 (localhost:8081)     │  │
│  │  location /api/        → Go 主后端    (localhost:8080)      │  │
│  └───────────────────────────────────────────────────────────┘  │
────────────────────────────┬────────────────────────────────────┘
                             │
        ┌────────────────────┼─────────────────────┐
        ▼                    ▼                     ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────────────┐
│  前端 (Frontend)  │ │  后端 (Backend)   │ │  账号系统 (Auth)          │
│                  │ │                  │ │                          │
│  index.html      │ │  Go (Gin+GORM)   │ │  Rust (axum + SQLite)    │
│  main.css        │ │                  │ │                          │
│  main.js         │ │  ├── main.go     │ │  ├── src/core/  纯逻辑    │
│  image/          │ │  ├── config/     │ │  ├── src/service.rs      │
│  pages/          │ │  ├── routes/     │ │  ├── src/store/  SQLite  │
│  modules/        │ │  ├── handlers/   │ │  ├── src/http/   axum    │
│                  │ │  ├── models/     │ │  └── src/mail/           │
│  特性:            │ │  └── database/   │ │                          │
│  - 学科导航       │ │                  │ │  特性:                    │
│  - localStorage  │ │  特性:            │ │  - 邮箱注册（邀请码+验证码）│
│  - 30天自动清理   │ │  - 词汇复习 API   │ │  - 多端同时登录            │
│  - 默认显示英语   │ │  - 词条管理       │ │  - Argon2id + 令牌轮换     │
│                  │ │  - FSRS 调度     │ │  - 会话管理与邀请码管理      │
└──────────────────┘ └──────────────────┘ └──────────────────────────┘
```

---

## 目录结构

```
e:\porject\4/
├── index.html                   # 主页面入口（仓库根目录即站点根）
├── main.css                     # 全站样式（导航栏、内容容器等）
├── main.js                      # 导航交互与 localStorage 缓存管理
├── dev-server.js                # 本地开发服务器（静态文件 + /api 双上游代理，仅开发用）
├── image/
│   └── avatar.png               # 头像图片
├── pages/                       # 尚未建模块的学科占位页（其余 8 门，均为「敬请期待」）
│   ├── chinese.html             # 语文
│   ├── math.html                # 数学
│   ├── physics.html             # 物理
│   ├── chemistry.html           # 化学
│   ├── biology.html             # 生物
│   ├── history.html             # 历史
│   ├── politics.html            # 政治
│   └── geography.html           # 地理
│
├── account/                     # 个人中心（账号 / 设备 / 操作，独立入口；入口是左上角头像）
│   ├── index.html               #   登录、注册、当前账号与登录中的设备
│   ├── account.css              #   只作用于账号页
│   └── account.js               #   交互逻辑（ES5）：问 /api/auth/me 决定显示哪一块
│
├── admin/                       # 通用后台骨架（独立入口，与学生站互不影响）
│   ├── index.html               #   入口页（布局 + 鉴权遮罩预留位）
│   ├── admin.css                #   后台样式与通用组件（表格/表单/按钮/空状态/提示条）
│   ├── admin.js                 #   框架：侧栏导航、hash 路由、学科后台按需加载、鉴权预留
│   └── README.md                #   后台约定与「新增学科后台」步骤
│
├── modules/                     # 学科模块（每个学科一个目录，独立管理）
│   └── english/                 # 英语模块（唯一已实现的学科）
│       ├── english.html         #   学科页片段（由 main.js 注入 #contentContainer）
│       ├── english.css          #   模块样式（只作用于英语页）
│       ├── english.js           #   学生端逻辑：DOM / fetch / 存储 / 语音
│       ├── README.md            #   模块说明
│       ├── admin/               #   词条管理后台模块（列表/搜索/编辑/批量导入）
│       │   └── english-admin.js
│       └── engine/              #   Rust/WASM 引擎（队列编排 + FSRS 调度 + 随机化 + 词表解析 + 卡片文本）
│           ├── Cargo.toml
│           ├── src/
│           │   ├── lib.rs           # 模块导出
│           │   ├── session.rs       # 会话编排（ReviewSession：队列/游标/评分/进度/卡片输出）
│           │   ├── fsrs_engine.rs   # FSRS 调度计算
│           │   ├── randomizer.rs    # 随机器（洗牌/抽样/种子）
│           │   ├── wordlist.rs      # 词表文本解析（后台批量导入用）
│           │   └── card_view.rs     # 卡片文本：多释义拆分（一个词性一块）与例句高亮切分
│           └── README.md            # 引擎说明与构建命令
│
├── docs/                        # 项目文档：CLAUDE.md 的细节全在这里（**先读 docs/README.md**）
│   ├── README.md                #   文档地图：什么时候读哪份
│   ├── overview.md              #   组成部分、端口与数据库分工
│   ├── review-engine.md         #   每日队列口径、抽卡、FSRS、wasm 方法一览
│   ├── english-ui.md            #   复习 UI、起始页与过场、沉浸模式
│   ├── frontend.md              #   导航 / 缓存 / 学科页路径 / 头像入口
│   ├── admin-api.md             #   后台约定、/api/words*、/api/reviews/*
│   ├── backend-auth.md          #   Go 与 Rust 两个服务、邀请码口径、验证方式
│   ├── boundaries.md            #   硬性红线的完整说明与本机工具链（含验证/备份脚本）
│   └── roadmap.md               #   **未来规划**：阶段表、进度记录、已定案决策与六项议题的成本 × 收益分析
│
├── scripts/                     # 本机 PowerShell 脚本（开发与运维）
│   ├── verify.ps1               #   一键验证：Go 构建/vet/测试 + 账号服务 + 引擎 + wasm32 目标检查
│   ├── verify-auth.ps1          #   跨服务鉴权联调：临时库起两个真服务，验 Cookie 本地验签全链路
│   ├── backup.ps1               #   数据库安全快照（VACUUM INTO；底层是 backend-go/cmd/backup）
│   └── clean-build-cache.ps1    #   清理 target/ 编译缓存（带白名单安全闸）
│
├── backend-go/                  # Go后端服务
│   ├── main.go                  # 程序入口
│   ├── go.mod                   # Go模块定义
│   ├── config/
│   │   └── config.go            # 配置管理
│   ├── database/
│   │   └── database.go          # SQLite 连接与自动迁移
│   ├── cmd/
│   │   ├── seed/
│   │   │   └── main.go          # 词表导入命令（JSON → words 表）
│   │   ├── inspect/
│   │   │   └── main.go          # 查看各词的记忆状态（排查用）
│   │   └── backup/
│   │       └── main.go          # 数据库快照命令（VACUUM INTO，被 scripts/backup.ps1 调用）
│   ├── seed/
│   │   └── words_english.json   # 英语种子词表（100 词）
│   ├── routes/
│   │   └── routes.go            # 路由定义
│   ├── handlers/
│   │   ├── handlers.go          # 请求处理器
│   │   ├── review_handlers.go   # 词汇复习（FSRS）处理器
│   │   └── *_test.go            # 关键路径测试（内存库 + httptest，见 docs/roadmap.md 阶段 1）
│   ├── models/
│   │   └── models.go            # 数据模型（含 Word/WordReview/ReviewLog）
│   ├── utils/
│   │   └── utils.go             # 工具函数
│   └── README.md                # 后端说明文档
│
├── backend-rust/                # 账号系统（Rust 认证服务，与 Go 后端各自独立）
│   ├── Cargo.toml               # 包 guangxue-auth，三个 bin：服务 / seed-admin / invite
│   ├── .env.example             # 全部 AUTH_* 环境变量示例（.env 已 gitignore）
│   ├── migrations/
│   │   └── 0001_init.sql        # auth.db 表结构（编译期内嵌）
│   ├── scripts/
│   │   └── smoke.ps1            # 端到端冒烟脚本（真实 HTTP + 真实 Cookie，24 项）
│   ├── tests/                   # 集成测试：auth_flow / email_code / invite / multi_device / security / admin
│   ├── src/
│   │   ├── main.rs              # HTTP 服务入口（默认 127.0.0.1:8081）
│   │   ├── lib.rs               # AppState 装配与路由挂载
│   │   ├── config.rs            # 环境变量 → Config（生产环境强校验）
│   │   ├── error.rs             # 统一错误 → {code,message,error}
│   │   ├── clock.rs             # 时钟注入（测试可控制「过期」这类分支）
│   │   ├── db.rs                # SQLite 连接 / PRAGMA / 迁移
│   │   ├── models.rs            # 数据行与对外输出结构
│   │   ├── store/               # 事务边界与全部 SQL
│   │   ├── service.rs           # 业务规则（注册/登录/刷新/会话/邀请码）
│   │   ├── rate_limit.rs        # 内存滑动窗口限流
│   │   ├── core/                # 纯逻辑：密码 / 令牌 / 邀请码 / 验证码 / 校验
│   │   ├── mail/                # 邮件发送（开发模式只打日志，配好 SMTP 即启用）
│   │   ├── http/                # axum 路由、Cookie、CSRF、鉴权提取器
│   │   └── bin/                 # seed-admin / invite 命令行工具
│   └── README.md                # 账号系统说明（接口、安全设计、环境变量、部署）
│
├── CLAUDE.md                    # 项目规则（**保护文件：非特殊要求不得修改**）
├── TODO.md                      # 已决定暂缓的问题与候选方案（**不是漏掉的 bug**）
├── README.md                    # 项目总说明（本文件）
├── pages.zip                    # 早期 pages/ 的存档（只读）
└── 备份/                        # 历史整目录备份（只读；已写进 .gitignore，但早期文件仍在版本库里）
```

---

## 学科模块约定

每个学科的东西放在一个目录里（`modules/<学科>/`），便于独立管理与演进：

| 文件 | 职责 |
|------|------|
| `<学科>.html` | 学科页片段，由 `main.js` fetch 后注入 `#contentContainer` |
| `<学科>.css` | 模块样式，只作用于该学科页内的元素 |
| `<学科>.js` | 模块逻辑（ES module），导出初始化函数，由 `main.js` **按需动态 import**；可选导出 `unmount()` 做切页清理 |
| `engine/` | 该学科的 Rust/WASM 引擎（纯计算：调度、随机化、统计等） |
| `admin/` | 该学科的管理后台模块，由根目录 `admin/` 的通用后台按需加载 |

`main.js` 里只保留一处学科相关代码——`initSubjectModule(pageName)`：按 `data-page`
判断并动态 `import()` 对应模块。**好处是学科逻辑不会回流到 `main.js`，且只有真正进入
该学科页才会加载它的模块与引擎。**

切换学科前，`main.js` 会调用上一个模块可选的 `unmount()`（模块没导出就跳过）。
这是框架层面的收口点：学科页可能往 `body` / `document` 上挂全局状态
（例如英语复习页的沉浸模式会给 `body` 加 `is-immersive` 来隐藏导航栏），
内容容器被换掉后这些状态不会自己消失，必须由模块自己收回。

> 目前只有 `modules/english/` 建好了；其余 8 门仍是 `pages/<学科>.html` 占位页。
> 将来把某门学科做成模块时，把它从 `pages/` 移进 `modules/<学科>/`，
> 并在 `index.html` 的 `data-page` 与 `main.js` 的 `ENGLISH_PAGE` 旁登记即可。

---

## 管理后台

独立入口，本地地址 **http://127.0.0.1:8899/admin/**。

```
admin/                 通用骨架（布局、侧栏导航、hash 路由、通用组件、鉴权预留位）
modules/<学科>/admin/  各学科自己的后台模块，按需动态加载
```

| 项 | 说明 |
|----|------|
| 入口 | `admin/index.html`，与学生站完全独立（不走 `main.js`，也不使用学科页的 localStorage 缓存） |
| 新增学科后台 | 在 `modules/<学科>/admin/` 写模块并导出 `mount(container, ctx)`，再到 `admin/admin.js` 的 `SUBJECT_ADMINS` 登记一行 |
| 通用能力 | `ctx.api` / `toast` / `confirm` / `el` / `escapeHtml` / `setTitle`，避免各学科重复实现 |
| 英语后台 | 词条列表（搜索 / 词书 / 单元筛选 / 分页）、新增编辑删除、**多释义编辑**（一个词性一块，每块可带自己的例句与译文）、**批量导入**（粘贴词表 → 引擎解析 → 预览 → 分批导入） |

> ✅ **登录门禁已接入**：打开 `/admin/` 会先调 `GET /api/auth/me`，只有 `role=admin` 的账号
> 才渲染后台，否则显示登录表单（普通账号会提示「不是管理员」）。顶栏显示当前账号与「退出登录」。
> ⚠️ 但这只是**界面层**的门禁：Go 侧的 `/api/words` 写接口**还没有服务端鉴权**，
> 直接调接口（例如 `curl -X PUT /api/words/1`）仍然能改数据。
> **在给 Go 补上鉴权中间件之前，仍然不要把 `/admin/` 部署到公网**——那一期要用同一个
> `AUTH_JWT_SECRET` 验签并查 `auth.db` 的会话。详见 [`admin/README.md`](admin/README.md)。

> 💡 批量导入的解析规则（支持制表符 / 竖线 / 逗号 / 空格、注释行、重复与错误行提示）由
> 引擎的 `wordlist.rs` 实现，有 16 个单元测试覆盖各种粘贴格式。

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
| GET | `/api/words` | 词条列表（搜索 / 词书 / 单元筛选 + 分页） | ✅ 可用 |
| POST | `/api/words` | 批量添加词条（已存在跳过） | ✅ 可用 |
| GET | `/api/words/:id` | 获取单个词条 | ✅ 可用 |
| PUT | `/api/words/:id` | 更新词条内容 | ✅ 可用 |
| DELETE | `/api/words/:id` | 删除词条（连带复习状态与日志） | ✅ 可用 |
| GET | `/api/word-options` | 已有词书 / 单元列表 | ✅ 可用 |
| GET | `/api/reviews/due` | 到期复习卡列表（只给已到期的） | ✅ 可用 |
| GET | `/api/reviews/new` | 未加入复习的新词（新词候选） | ✅ 可用 |
| GET | `/api/reviews/queue` | 复习队列：**整库按紧迫度排序**（含未到期），分页 | ✅ 可用 |
| GET | `/api/reviews/probes` | 每日抽查候选：**到期最远**的已学词 | ✅ 可用 |
| POST | `/api/reviews/submit` | 提交复习结果（持久化 FSRS 状态） | ✅ 可用 |
| GET | `/api/reviews/stats` | 复习统计（词库概览 / 今日进度 / 连续天数 / 记忆保持率） | ✅ 可用 |

### 账号系统 API（Rust 认证服务，详见 [`backend-rust/README.md`](backend-rust/README.md)）

| 方法 | 路径 | 说明 | 状态 |
|------|------|------|------|
| GET | `/api/auth/health` | 账号服务健康检查 | ✅ 可用 |
| POST | `/api/auth/email-code` | 发注册验证码（邀请码可选：填了先校验；不填直接发） | ✅ 可用 |
| POST | `/api/auth/register` | 邮箱 + 邀请码 + 验证码注册，成功即登录 | ✅ 可用 |
| POST | `/api/auth/login` | 登录（**每次登录新建会话 → 多端同时在线**） | ✅ 可用 |
| POST | `/api/auth/refresh` | 轮换 refresh 令牌；旧令牌重放会吊销整条轮换链 | ✅ 可用 |
| POST | `/api/auth/logout` | 只登出当前这个端 | ✅ 可用 |
| GET | `/api/auth/me` | 当前登录用户 | ✅ 可用 |
| GET | `/api/auth/sessions` | 我的活跃会话（设备名 / IP / 最近活跃 / 是否当前端） | ✅ 可用 |
| POST | `/api/auth/logout-all` | 登出全部端（`keep_current` 可只踢其他端） | ✅ 可用 |
| GET/POST | `/api/auth/admin/invites` | 邀请码列表 / 生成（明文只在生成时返回一次） | ✅ 可用 |
| POST | `/api/auth/admin/invites/{id}/disable` | 停用邀请码 | ✅ 可用 |
| GET | `/api/auth/dev/codes` | 读取验证码（**仅 development**，生产环境路由不注册） | ✅ 可用 |

> 账号接口的响应同样使用 `{code, message, data}` 信封，失败时额外带机器可读的
> `error` 字段（如 `invalid_invite`、`bad_credentials`、`account_locked`），详见
> [`backend-rust/README.md`](backend-rust/README.md) 的错误码表。

词条字段（`/api/words*`、`/api/reviews/*` 通用）：

| 字段 | 说明 |
|------|------|
| `word` / `phonetic` / `meaning` / `example` | 单词 / 音标 / 释义 / 例句 |
| `example_translation` | 例句的中文翻译（可空） |
| `senses` | 多释义数组 `[{pos, meaning, example, translation}]`（可空；空数组即「没有结构化义项」） |
| `book` / `unit` | 词书 / 单元（后台分组用，可空） |

> `senses` 为空时，复习界面由引擎按 `meaning` 里的词性标签自动分块；
> 后端保证「空」序列化成 `[]` 而不是 `null`（`models.WordSenses` 的 `Scan` / `MarshalJSON`）。

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

### 3. 启动账号系统（登录 / 注册相关）

```bash
cd backend-rust
cargo run --release   # 监听 127.0.0.1:8081，首次启动自动建库、迁移并创建管理员
```

初始管理员 `2262997289@qq.com` / `7289HR_RedSun`（**首次登录后请改密**）。
`dev-server.js` 会把 `/api/auth/*` 分流到这个服务，其余 `/api/*` 仍走 Go。

本期还没有登录页面，功能验证用冒烟脚本或 curl：

```powershell
pwsh backend-rust/scripts/smoke.ps1                          # 直连 8081
pwsh backend-rust/scripts/smoke.ps1 -BaseUrl http://127.0.0.1:8899   # 经 dev-server 代理
```

邮件默认「只打日志不发信」，本地联调时验证码从服务端日志或
`GET /api/auth/dev/codes?email=...` 取（该接口只在 development 存在）。

### 4. 启动前端服务器

```bash
# 在仓库根目录执行
node dev-server.js                   # 默认 http://127.0.0.1:8899
node dev-server.js --port 9000        # 前端端口被占用时换一个
node dev-server.js --api-port 8081    # 后端换了端口时对齐
```

### 5. 打开浏览器

访问 **http://127.0.0.1:8899** 即可。首页默认加载英语页，下拉即是「单词复习」。

其它入口：

| 地址 | 用途 |
|------|------|
| http://127.0.0.1:8899/ | 学生站（**左上角头像**是个人中心入口；复习页默认沉浸，先点「显示导航栏」才看得到） |
| http://127.0.0.1:8899/account/ | 个人中心：账号信息 / 登录中的设备 / 可用操作（未登录时是登录 / 注册） |
| http://127.0.0.1:8899/admin/ | 后台管理（需管理员账号登录） |

初始管理员：`2262997289@qq.com` / `7289HR_RedSun`（生产环境请务必改密）。

### 常见问题

| 现象 | 原因与处理 |
|------|-----------|
| 页面提示「复习功能加载失败」 | 后端没启动或端口不是 8080；`dev-server.js` 控制台会打印 `[proxy error]` 说明具体原因 |
| 提示「词库为空」或「暂无需要复习的单词」 | 还没导入词表，执行 `go run ./cmd/seed`（见上一步） |
| 改了学科页（`modules/<学科>/*.html` 或 `pages/*.html`）却不生效 | 学科页被 **localStorage 缓存了 30 天**：DevTools → Application → Local Storage 删除 `pageCache_*` 键，或在 Console 执行 `localStorage.clear()` 后刷新；也可以把 `main.js` 里的 `CACHE_VERSION` +1 强制全体用户失效 |
| 改了 `main.js` / `main.css` 却不生效 | 浏览器 HTTP 缓存。`dev-server.js` 已发送 `Cache-Control: no-store`；若仍异常请硬刷新（Ctrl+F5） |
| 端口 8080 / 8899 被占用 | 后端用环境变量换端口（如 `SERVER_PORT=8081`），前端用 `--api-port 8081` 对齐；前端自身用 `--port` 换 |
| 想要与线上完全一致的形态 | 用 Nginx 反向代理：`root` 指向仓库根目录、`proxy_pass` 指向 `127.0.0.1:8080`（见「部署说明」） |
| 登录/注册接口报 502 | 账号服务没启动。`dev-server.js` 会打印 `[proxy error] ... → 账号系统(Rust)`；先 `cd backend-rust && cargo run --release` |
| 重启服务后登录态全部失效 | 开发环境没设 `AUTH_JWT_SECRET`，每次启动都会随机生成密钥（正式部署务必固定它） |
| 发验证码提示「操作过于频繁」 | 同邮箱 60 秒只能发一次、每小时 5 次；同 IP 每小时 20 次。可用 `AUTH_RL_*` 调整 |

> 💡 `dev-server.js` 仅用于本地开发，部署时无需上传（线上由 Nginx 承担同样的职责）。

### 编译缓存与磁盘占用

`target/` 是可再生的 Rust 编译缓存（调试符号 + 增量编译），两个 crate（账号服务 + wasm 引擎）加起来能到 **4.5 GB** —— 约为仓库源码体积的 300 倍。不过它**全部被 `.gitignore` 排除**，不进版本库、也不会传到云端，只占本地磁盘。

```powershell
# 先看会删什么（不真删）
pwsh scripts/clean-build-cache.ps1 -DryRun

# 只删 debug 缓存（推荐：约释放 3.8 GB；release 二进制与浏览器用的 pkg/ 都保留）
pwsh scripts/clean-build-cache.ps1

# 彻底清空（约 4.5 GB；需先停掉账号服务，否则正在运行的 exe 删不掉）
pwsh scripts/clean-build-cache.ps1 -All
```

> 仓库之外还有一处缓存：Go 构建缓存在 `%LOCALAPPDATA%\go-build`（本机约 230 MB），需要时用 `go clean -cache` 清理。
> 删完缓存后首次 `cargo test` / `cargo build --release` 会重新完整编译，慢一次属正常。

### 一键验证与数据库备份

改完代码想确认「没弄坏」：

```powershell
# 一条命令跑完：Go 构建/vet/测试 + 账号服务测试 + 引擎测试 + wasm32 目标检查（全绿约 45 秒）
pwsh scripts/verify.ps1

# 只要其中一段
pwsh scripts/verify.ps1 -Only go       # all | go | rust | engine
pwsh scripts/verify.ps1 -IncludeSmoke  # 额外跑账号服务 smoke.ps1 的 24 项（需服务已启动）
```

验证鉴权改动（**跨服务**：账号服务发 Cookie → Go 本地验签）：

```powershell
# 用临时库起两个真服务（默认 18081 账号服务 / 18080 Go），13 项检查全绿约 8 秒
pwsh scripts/verify-auth.ps1
```

> 两侧的单元测试各自 mock 自己的密钥，密钥/算法/容差对不上时它们**全绿**，只有这个脚本能发现 —— 所以它验证的是「匿名 401 / 账号服务发的 Cookie 200 / 普通用户写词条 403 / 外站 Origin 403」这条完整链路。

备份数据库（**不要用「复制 `.db` 文件」的方式**：`auth.db` 带着 3.7 MB 未 checkpoint 的 WAL，直接拷可能得到空库）：

```powershell
# 对 guangxue.db 与 backend-rust/auth.db 执行 SQLite VACUUM INTO 快照 → backups/<时间戳>/
pwsh scripts/backup.ps1

# 只保留最近 5 份快照
pwsh scripts/backup.ps1 -Keep 5
```

> ⚠️ 实测：磁盘上的 `backend-rust/auth.db` 只有 4 KB，而安全快照出来是 **124 KB** —— 那 97% 就在 WAL 里，只有 `VACUUM INTO` 才拿得到。

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
    {"word": "apple", "phonetic": "/ˈæp.əl/", "meaning": "n. 苹果", "example": "I eat an apple.",
     "example_translation": "我吃一个苹果。", "subject": "english"},
    {"word": "benefit", "phonetic": "/ˈben.ɪ.fɪt/", "meaning": "n. 好处；益处 v. 有益于",
     "example": "Exercise has many benefits.", "example_translation": "锻炼有很多好处。",
     "senses": [
       {"pos": "n.", "meaning": "好处；益处", "example": "Exercise has many benefits.", "translation": "锻炼有很多好处。"},
       {"pos": "v.", "meaning": "有益于", "example": "Regular exercise benefits your heart.", "translation": "规律锻炼对心脏有益。"}
     ], "subject": "english"}
  ]
}
```

- `subject` 省略时默认为 `english`，为将来其他学科的词汇留出扩展位。
- `example_translation` 与 `senses` 都是可选的：`senses` 留空时，复习界面会按 `meaning` 里的
  词性标签自动分块（上面 `benefit` 那行不写 `senses` 也会显示成名词、动词两块）。
- `words.word` 是唯一索引，重复单词自动跳过，因此命令**可反复执行**（幂等）。
- 换成自己的词表（中考 / 高考 / 四六级等）时，保持同样的 JSON 结构即可。
- SQLite 文件与词表都是本地数据：`*.db` 已在 `.gitignore` 中忽略，词表 JSON 则在版本控制内。

---

## 复习页交互（英语）

界面为「极简全屏」风格：顶栏是沉浸模式开关、今日统计与当前卡片的记忆元信息，主体只有大字号单词与音标，
底部是操作区。揭晓后，例句中的目标词会高亮（支持常见变形：例句里的 `applied` 也能对应词条 `apply`），
例句下方是它的中文翻译，再往下是**一条条释义块**（一个词性一块）。

### 两套动效

**换卡**：旧卡淡出上移 → 新卡淡入下移（150ms + 240ms）。过渡期间评分按钮隐藏、键盘评分被锁，
避免连点把第二次评分落到下一张卡上。

**揭晓**：不是整块一起淡入（那太死板），而是排了一个节拍——

- 「点击显示答案」按钮**缩小淡出**（不是直接消失）；
- 主例句浮现 → 译文跟上 → 一条条释义块错开浮现，
  每块里的目标词高亮再晚一点扫过去，像荧光笔划过去；
- 按钮退场动画播完的**同一刻**，四个评分按钮从它原来的位置依次顶上来（浮起 + 由小变大），
  中间既没有空档也不重叠。整段约 0.65 秒。

> ⚠️ 换人必须是「先藏掉揭晓按钮，再放出评分按钮」：两者是底部操作区里相邻的两个块，
> 同时显示会让操作区变高，把上面的单词顶上去（那正是之前修掉的毛病）。

- 节拍的**时刻**在 `english.js` 的 `playRevealAnimation()` 里（`REVEAL_*` 常量），
  **动作**（时长/缓动/起止状态）在 `english.css` 的 `revealUp` / `hitSweep` 关键帧里，两边靠常量对齐。
- 单词本身**不参与任何动画**：揭晓时它必须待在原位不动。
- 系统开启「减少动态效果」时，两套动效全部关闭，内容直接显示。

| 操作 | 说明 |
|------|------|
| 空格 / 回车 | 显示答案（揭晓例句与释义，并放出评分按钮） |
| `Q` / `W` / `E` / `R` | 评分：陌生 / 困难 / 一般 / 简单（**仅在显示答案之后生效**） |
| `1` / `2` / `3` / `4` | 同上，等效的备选键位 |
| `P` | 朗读当前单词 |
| `L` | 朗读当前例句（`E` 被「一般」占用，故用 `L`） |
| `Esc` | 切换沉浸模式（等同点顶栏那个开关） |
| 自动朗读 | 底部开关，**默认开启**；关掉后保存在 localStorage（`reviewAutoSpeak`） |

顶栏右侧的记忆元信息（难度 / 稳定性 / 状态 / 复习次数 / 上次 / **预计记住**）中，
「预计记住」由引擎用 FSRS 可提取率算出；新词没有记忆状态，只显示「状态 新词」。

### 沉浸模式（隐藏站点导航栏）

进复习页默认进入沉浸模式：站点顶部导航栏收起来，复习界面占满整屏（`body.is-immersive` + `main.css`
里的通用规则），点顶栏左上角的「显示导航栏」（或按 `Esc`）即可叫回来，选择记在 localStorage
（`reviewImmersive`）。切到别的学科时，`main.js` 会调用英语模块导出的 `unmount()` 把这个状态收回去，
否则导航栏会跟着消失到别的学科页上。

> 实现约定：学科页只负责「挂/摘 `body` 上的类」，样式一律写在 `main.css`（通用）与学科 CSS（自己的留白）里，
> 学科逻辑不回流到 `main.js`。

### 多释义与例句翻译

一个词条可以有多个义项，复习时**一个词性显示一块**（名词一块、动词一块），每块还能带自己的例句与译文：

- 词库里的历史数据把多个义项写在一行（`n. 好处；益处 v. 有益于`）时，引擎会**按词性标签自动拆开**，
  不填结构化数据也能得到多块效果；
- 在后台「编辑词条 → 多释义」里逐条填写时，以结构化数据为准（可覆盖自动拆分），
  每块可填自己的例句与例句翻译；
- 例句的中文翻译有两个落点：词条级（`words.example_translation`，配主例句）与释义级
  （`senses[].translation`，配该义项自己的例句）；没填就不显示那一行。
- 存储：多释义在 SQLite 里是 `words.senses` 一列 JSON 文本（`models.WordSenses`），
  不单开子表——释义永远跟着词条一起读写，存 JSON 省掉一次 join，传输也少带 `id`/`word_id`。

### 每日队列：5 个新词 + 5 个抽查 + 无限复习

一天的开局不是「把所有到期的抓过来」，而是三段拼接（`session.rs` 的 `plan_day`）：

| 段 | 内容 | 顺序 |
|----|------|------|
| ① 新词 5 个 | 词库里还没有复习记录的词 | 无放回随机抽 |
| ② 抽查 5 个 | 已学词里**到期最远**的，且最近 7 天没抽查过 | 越轮不到复习的越先抽 |
| ③ 复习区 | 其余已学词，按 `due_at` 升序 | 池首过期的严格最旧优先；未到期的在**最靠前 10 张**里随机抽 |

### 无限复习：评完的卡按新到期时间插回池子

队列不再是一条走到头就结束的线，而是一个**始终按到期时间排序的池子**：

- 每评完一张，引擎就用「现在 + 刚算出的间隔」作为它的新到期时间，**按顺序插回池子**；
- 因此池子永远不会空，**没有「今天复习完成」那个结束页**，想复习多久就复习多久；
- 同一张卡不会立刻又出现：**本轮已经评过的卡不再抽**，至少隔一整轮才会再来
  （词库很小、池子比保护窗口还小时自动放行，保证永远有卡可出）；
- 池子里每张卡都评过一次 = **一轮**。过完一轮时左下角小字会闪一句
  「本轮已过一遍 · 可以继续」，然后接着下一轮。

状态行也从「队列剩余 N 张」改成了「**本轮已复习 N 张**」——在无限复习里「剩余」已经没有意义了。

> 实现细节见 `modules/english/engine/README.md`；踩过的两个坑（窗口不放宽会把后面的卡饿死、
> 结算轮次的顺序会让「刚评过」的保护失效）都在那里记着，并各有一条单元测试锁住。

**为什么要抽查那 5 个**：只按到期时间出题的话，间隔被拉到几十天的词永远轮不到。
在 3500 词的词库里，很可能前 50 个天天快要过期、天天出现，后 50 个一直不动——
长期不露面的词就是记忆盲区。每天固定抽 5 个「最轮不到」的词提前确认，等于给整库做抽样体检。

抽查卡评分时**按新卡重算**（丢掉原来的 stability/difficulty，天数按 0 算），
所以它的间隔会被压缩回几天，很快回来重新标定；请求体里带 `is_probe: true`，
后端记进 `review_logs.is_probe`（日志里的 `stability_before` 仍是真实旧值，
所以「今日新学」统计不会被污染）。

> ⚠️ **每日配额记在浏览器 localStorage**（键 `reviewDailyPlan`），不是服务端。
> 因为现在没有登录系统、服务端只有一份共享词库：若在服务端按「每天 5 个」算，
> 等于**全站每天共放 5 个新词**，你先学了别人就没得学。
> 代价是换设备或清缓存会重置——等有了用户系统再迁到服务端。

左下角有一行很淡的小字显示今天的计划进度（`新词 3/5 · 抽查 2/5`），
计划走完的那一刻会**带动效切换**成「计划完成 · 进入复习阶段」。计划与复习是两段不同的阶段：
计划只有 10 张（每天一次），走完之后进入**无限复习**——池子按到期时间循环，没有尽头。

> **设计要点：为什么答案默认隐藏？**
> 进入卡片时只显示单词与音标，释义/例句保持隐藏，评分必须发生在揭晓之后。
> 因为 FSRS 的 `stability` / `difficulty` 依赖「是否真的想起来」这一真实信号：
> 若一边看着答案一边打分，评分会系统性偏高，记忆状态随之失真，
> `review_logs` 也就失去了后续参数优化（`compute_parameters`）的价值。

---

## 账号系统（登录 / 注册）

账号系统是一个**独立的 Rust 服务**（`backend-rust/`，axum + rusqlite，数据库 `auth.db`），
只负责 `/api/auth/*`；Go 主后端继续负责词汇复习接口。线上 Nginx 与本地 `dev-server.js`
都按前缀分流，两边形态一致。

### 注册：邮箱验证码是门槛，邀请码是「升级券」

1. `POST /api/auth/email-code`：给该邮箱发 6 位验证码（10 分钟有效、最多试 5 次，
   重发会让旧码立即失效）。**邀请码可选**——填了就先校验（未停用 / 未过期 / 没用完），
   让用户在发码这一步就拿到「码不对」的反馈；不填直接发码。
2. `POST /api/auth/register`：验证码 + 邮箱 + 密码即可建号，**注册完就是登录状态**。
   - **不带邀请码** → 普通用户（`role=user`），也就是**开放注册**；
   - **带邀请码** → 注册即**升级为管理员**（`role=admin`）。将来同一个入口还会承载
     积分 / 礼物之类的兑换（后台靠邀请码里的 `-` 前缀区分用途）。
   - 邀请码用量在**同一事务**里占位，所以同一枚码被并发使用时只有一个能成功；
     多个邀请码用**空格**分隔，一张张依次核销（同一个码写两遍只扣一次）。

> ⚠️ 邀请码现在等同于「管理员授权」：拿到码的人注册出来就是管理员。
> 所以码只发给信得过的人，用完可以 `POST /api/auth/admin/invites/{id}/disable` 停用。
> 邀请码由管理员生成：`POST /api/auth/admin/invites`（明文码只在生成响应里出现一次），
> 或命令行 `cargo run --release --bin invite -- create --count 3`。

### 多端同时登录

每次登录都新建一行会话（浏览器各自拿到自己的 HttpOnly Cookie），端与端互不影响：

- `GET /api/auth/sessions` 能看到全部在线端（设备名、IP、创建时间、最近活跃、是否当前端）；
- `POST /api/auth/logout` 只登出当前端；`POST /api/auth/logout-all` 一键踢掉全部端
  （带 `{"keep_current": true}` 时保留当前端）；
- refresh 令牌**每次使用都轮换**；一枚已用过的令牌再次出现即判定重放，
  **整条轮换链立即吊销**，窃取者与本人都会掉线重新登录。

### 密码与登录态

- 密码用 **Argon2id** 哈希（OWASP 推荐参数），库里只有 PHC 串，永不存明文；
- 登录态放 **httpOnly Cookie**（JS 读不到，XSS 也偷不走）：access 15 分钟（HS256 JWT）、
  refresh 30 天（库中只存 SHA-256 摘要）；
- 受保护接口每次都会校验会话表，所以**登出与踢端是即时生效的**，不必等 access 过期；
- 防爆破：账号连续失败 5 次锁 15 分钟（落库），另有 IP / 邮箱维度的滑动窗口限流；
- 权限等级：`users.role`（`user`/`admin`）已实际使用——**带邀请码注册即为 `admin`**；
  `users.status` 与更细的 RBAC 仍是预留字段。

### 页面与门禁（前端怎么用这套接口）

| 入口 | 做什么 |
|------|--------|
| `account/`（个人中心） | 打开先问 `GET /api/auth/me`：已登录显示**身份卡**（头像 + 昵称 + 角色徽章）、账号信息、**登录中的设备列表**（可「登出其他设备」或「退出登录」）、可用操作（管理员多一个进后台的按钮）；未登录显示登录 / 注册表单，登录表单下面的小字里「注册」二字**可点击**，点了直接切到注册标签页。**邀请码可留空**（直接注册成普通用户），填了注册后就是管理员；多个邀请码用空格分隔。注册表单的「获取验证码」带 60 秒倒计时，**本地开发会自动调 `/api/auth/dev/codes` 把验证码填进表单**（生产环境该接口不存在，静默忽略）。所有组件按顺序入场（`data-enter` + `playEnter()`）；**标签栏的选中高亮是滑动的滑块**（`.acc-tabs-thumb`，切标签时滑过去），表单按点击方向从侧边滑入，同时**白色卡片会平滑地向下延伸 / 向上回缩**到新表单的高度（`animateCardHeight()`：量旧高 → 换内容 → 量新高 → 过渡 → 收尾还原成自动高度）；带 `?from=avatar` 进来时跳过身份卡的入场，与首页过场衔接 |
| `admin/`（后台） | 打开先 `checkAuth()` 问服务端：`role=admin` 才渲染后台，否则只显示登录表单（普通账号会明确提示「不是管理员」）。顶栏显示当前账号与「退出登录」 |
| 站点左上角**头像** | 个人中心的入口（原来是右上角的「登录 / 注册」文字链接）：点击后以头像为圆心扩散一层遮罩盖满全屏，再跳到 `account/?from=avatar`。已登录时头像右下角亮一个绿点、`title` 显示昵称；未登录时是「登录 / 注册 · 个人中心」。⚠️ 它在 `.rectangle` 里，**沉浸模式下会随导航栏一起隐藏**（复习页默认沉浸） |

三处都只做**界面层**的门禁：真正的权限必须由服务端判定。服务端侧已经补齐（阶段 2）：Rust 账号服务的 `/api/auth/admin/*` 要
`role=admin`，Go 侧的 `/api/words` 写接口走 `RequireAdmin`、`/api/reviews/*` 走 `RequireUser`，两边共享同一把
`AUTH_JWT_SECRET` 本地验签（见 [`backend-go/README.md`](backend-go/README.md)）。
⚠️ 但**多用户隔离还没做**——`word_reviews` 仍按单词全局唯一，第二个用户会覆盖第一个人的进度，
上线前必须先做这件事，见 [`docs/launch-plan.md`](docs/launch-plan.md) 的 P0-1。

### 邮件

默认 `AUTH_MAIL_MODE=log`：验证码只打到服务端日志（并可由开发调试接口读取），不发信。
拿到邮箱 SMTP 授权码后配上 `AUTH_MAIL_MODE=smtp` 与 `AUTH_SMTP_*` 即可真实发送
（QQ 邮箱要填**设置里生成的授权码**，不是登录密码）。

> ⚠️ **上线必须走 HTTPS**：传输安全完全交给 TLS。浏览器端自己加密并不能防中间人
> （攻击者可以篡改下发的 JS 与公钥），后端端口也不要直接暴露公网。

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

    # 账号系统（Rust 认证服务）
    # 放在 /api/ 之外独立分流；nginx 前缀匹配取最长者，所以 /api/auth/* 一定命中这里
    location /api/auth/ {
        proxy_pass http://localhost:8081;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
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

### 启动账号系统（Rust 认证服务）

```bash
cd backend-rust
cargo build --release           # 产出 target/release/guangxue-auth.exe

# 生产启动（务必显式提供密钥与管理员密码，见 backend-rust/README.md）
APP_ENV=production \
AUTH_JWT_SECRET='一串足够长的随机值' \
AUTH_ADMIN_PASSWORD='管理员密码' \
./target/release/guangxue-auth
```

> ⚠️ `APP_ENV=production` 时有两条硬性检查：必须显式提供 `AUTH_JWT_SECRET` 与
> `AUTH_ADMIN_PASSWORD`，且调试接口 `AUTH_DEV_ENDPOINTS` 必须关闭 —— 不满足会直接启动失败，
> 不允许带着开发默认值上线。

### 环境变量

**Go 主后端**

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `SERVER_HOST` | 监听地址 | `0.0.0.0` |
| `SERVER_PORT` | 端口 | `8080` |
| `APP_ENV` | 运行环境 | `development` |

**Rust 账号系统**（完整清单见 [`backend-rust/README.md`](backend-rust/README.md)）

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `APP_ENV` | `production` 时启用强校验 | `development` |
| `AUTH_HOST` / `AUTH_PORT` | 监听地址与端口 | `127.0.0.1` / `8081` |
| `AUTH_DB_PATH` | SQLite 文件 | `auth.db` |
| `AUTH_JWT_SECRET` | 令牌签名密钥（生产必填） | 开发随机生成 |
| `AUTH_ACCESS_TTL_SECONDS` / `AUTH_REFRESH_TTL_DAYS` | 会话有效期 | `900` / `30` |
| `AUTH_COOKIE_SECURE` | Cookie 是否带 Secure | 随 `APP_ENV` |
| `AUTH_ALLOWED_ORIGINS` | CSRF 来源白名单 | `http://127.0.0.1:8899,http://localhost:8899` |
| `AUTH_MAIL_MODE` | `log` 只打日志 / `smtp` 真发信 | `log` |
| `AUTH_SMTP_*` | SMTP 主机/端口/账号/授权码/发件人/加密方式 | — |
| `AUTH_ADMIN_EMAIL` / `AUTH_ADMIN_PASSWORD` | 初始管理员 | `2262997289@qq.com` / 开发默认密码 |
| `AUTH_SEED_ADMIN` | 启动时确保管理员存在（幂等） | 随 `APP_ENV` |
| `AUTH_ARGON2_M_COST` / `_T_COST` / `_P_COST` | 密码哈希参数 | `19456` / `2` / `1` |
| `AUTH_RL_*` / `AUTH_LOCK_THRESHOLD` / `AUTH_LOCK_MINUTES` | 限流与锁定 | 见 `.env.example` |

---

## 开发计划

> 「下一步做什么、为什么是这个顺序」的分析与阶段表见 [`docs/roadmap.md`](docs/roadmap.md)；本节只记结果。

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
- [x] 计算下沉 Rust：会话编排、日期换算、FSRS 调度、进度统计移入引擎（现为 118 个单元测试）
- [x] 通用后台骨架 `admin/`（布局 / 导航 / 路由 / 通用组件 + 登录鉴权预留位）
- [x] 英语后台：词条增删改查 + 批量导入（粘贴词表 → Rust 解析 → 预览 → 分批导入）
- [x] 词书 / 单元分组（`words.book` / `words.unit` + 列表筛选）
- [x] 登录鉴权（前端 `checkAuth()` 门禁 + 后端鉴权中间件：`/api/reviews/*` 需登录、词条写接口需管理员）
- [ ] 按词书 / 单元限定复习范围（目前复习队列不看分组）
- [x] 主动回忆流程（先回想 → 显示答案 → 评分，避免「看着答案打分」污染 FSRS 状态）
- [x] 单词 / 例句发音（Web Speech API）与键盘快捷键（空格、Q/W/E/R 或 1~4、P、L）
- [x] 复习界面改版（极简全屏：顶栏统计与记忆元信息、大字号单词、例句目标词高亮、粉彩评分按钮）
- [x] 自动朗读默认开启（首次交互后自动补读，绕过浏览器对语音的拦截）
- [x] 换卡与揭晓过渡动效（尊重系统的「减少动态效果」设置）
- [x] 取消每日新词上限：队列抽干后自动补词，可一直学到词库学完
- [x] 每日队列改为「5 个新词 + 5 个抽查 + 无限复习」（`plan_day`），杜绝「间隔长的词永远轮不到」
- [x] 无限复习：评完的卡按新到期时间插回池子，没有结束页；过完一整轮时轻提示「本轮已过一遍」
- [x] 抽查卡按新卡重算记忆状态，`review_logs.is_probe` 留痕（供日后排除出参数拟合）
- [x] 左下角每日计划进度小字 + 计划完成后的动效切换
- [x] 例句变形匹配（`applied` 也能对应词条 `apply`；原形优先）
- [x] 沉浸模式（默认隐藏站点导航栏，全屏复习；`Esc` 或顶栏开关切换，切学科自动收回）
- [x] 一词多义：复习界面一个词性一块（历史数据按词性标签自动分块，后台可逐条结构化录入）
- [x] 例句中文翻译（词条级 `example_translation` + 释义级 `senses[].translation`，后台可编辑、批量导入第 5 列可带）
- [x] 复习统计面板（今日进度、连续天数、记忆保持率、本轮进度条）
- [x] 英语种子词表与导入命令（`backend-go/cmd/seed`，100 词）
- [x] **账号系统（Rust 认证服务）**：邮箱注册（管理员邀请码 + 邮箱验证码）、多端同时登录、
      令牌轮换与重放检测、会话管理、邀请码管理（`backend-rust/`）
- [x] 账号系统安全基线：Argon2id 密码哈希、httpOnly Cookie 登录态、CSRF 来源校验、
      账号锁定与限流、审计日志（不含密码与验证码明文）
- [x] 账号系统与 Go 后端按前缀分流（`/api/auth/*` → 8081），本地 `dev-server.js` 与线上 Nginx 同形态
- [x] 个人中心页面 `account/`（身份卡 / 账号信息 / 登录中的设备 / 可用操作，本地开发自动回填验证码，全部组件带入场动效）
- [x] 后台登录门禁（`admin/` 只放行 `role=admin`）与站点左上角头像入口（点头像扩散过场进个人中心，含登录态圆点）
- [ ] 权限等级系统（RBAC）—— 三级角色（超级管理员 / 管理员 / 用户）与按等级发码的完整方案见 [`docs/launch-plan.md`](docs/launch-plan.md) 的 P0-5
- [x] 词条写接口鉴权（Go 侧用同一 JWT 密钥本地验签：`/api/reviews/*` 需登录、`/api/words` 写接口需管理员）
- [ ] 用户与复习数据绑定（`word_reviews.user_id`）、每日配额从 localStorage 迁到服务端 —— **上线前置，见 `docs/launch-plan.md` P0-1**
- [ ] 改密 / 找回密码 / 多方式登录（`user_identities` 表已预留）
- [ ] 复习页面 UI（进阶：完整词义卡交互、统计曲线图表等）
- [ ] FSRS 参数优化（基于 review_logs 的 compute_parameters）

---

## 支持作者

如果这个项目对你有帮助，欢迎在爱发电支持我！

[⚡ 在爱发电支持我](https://afdian.com/a/hr_redsun)

---

## 许可证

MIT License