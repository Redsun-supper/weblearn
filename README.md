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

> ⚠️ **当前没有登录校验**：任何能访问 `/admin/` 的人都能修改词条，**接入登录前请勿部署到公网**。
> 前端预留位是 `admin/admin.js` 的 `checkAuth()` 与 `admin/index.html` 的 `#adminAuthGate`；
> 但后端目前也没有鉴权中间件，**登录实现时两端要一起做**（只拦前端挡不住直接调接口的人）。
> 详见 [`admin/README.md`](admin/README.md)。

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
- [x] 计算下沉 Rust：会话编排、日期换算、FSRS 调度、进度统计移入引擎（42 个单元测试）
- [x] 通用后台骨架 `admin/`（布局 / 导航 / 路由 / 通用组件 + 登录鉴权预留位）
- [x] 英语后台：词条增删改查 + 批量导入（粘贴词表 → Rust 解析 → 预览 → 分批导入）
- [x] 词书 / 单元分组（`words.book` / `words.unit` + 列表筛选）
- [ ] 登录鉴权（前端 `checkAuth()` + 后端鉴权中间件，需两端一起做）
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
- [ ] 复习页面 UI（进阶：完整词义卡交互、统计曲线图表等）
- [ ] FSRS 参数优化（基于 review_logs 的 compute_parameters）

---

## 支持作者

如果这个项目对你有帮助，欢迎在爱发电支持我！

[⚡ 在爱发电支持我](https://afdian.com/a/hr_redsun)

---

## 许可证

MIT License