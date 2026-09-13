# 广学项目 · 工作规则（CLAUDE.md）

> 本文件由 DeepSeek Harness 的 `agent-instructions` 插件在新对话的首条消息自动注入，**每个新会话只注入一次**。
> 用于统一项目的背景、边界与约定，避免每次对话重复读取整个项目。

---

## 一、项目概述

「广学」是一个**学科内容展示平台**：前端静态页面 + Go 后端 API，经 Nginx 反向代理部署到 `test.lovezmx.com`。

| 组成部分 | 说明 |
|----------|------|
| 前端（根目录） | `index.html`（主页）、`main.css`（全站样式）、`main.js`（导航与缓存）、`dev-server.js`（本地开发服务器）、`image/`、`pages/`（8 个占位学科页） |
| 学科模块（`modules/<学科>/`） | 学科自己的页面 / 样式 / 逻辑 / 引擎 / 后台模块；目前只有 `modules/english/` |
| 通用后台（`admin/`） | 独立入口页（`/admin/`）：布局、侧栏导航、hash 路由、通用组件、登录鉴权预留位 |
| 后端（`backend-go/`） | Go 1.21 + Gin + **GORM/SQLite**（`database/`、`handlers/review_handlers.go`），入口 `main.go` |
| 复习引擎（`modules/english/engine/`） | Rust/WASM crate `guangxue_wasm`：`session.rs`（会话编排）+ `fsrs_engine.rs`（FSRS 调度）+ `randomizer.rs`（随机器）+ `wordlist.rs`（词表解析）+ `card_view.rs`（词性拆分 / 例句高亮切分），浏览器内运行 |
| 部署 | Nginx 反代：`/` → 前端静态文件，`/api/` → `localhost:8080` |

### 复习引擎（词汇间隔重复）
- **职责边界（重要）**：`modules/english/english.js` 只做 DOM 渲染 / `fetch` / `localStorage` / 语音；**队列、游标、日期换算、FSRS 计算、进度统计全部在 `engine/`（Rust）里**，JS 不再保存卡片数组。
- **流程**：取 `GET /api/reviews/due` + `GET /api/reviews/new` 的 **JSON 原文** → `new ReviewSession(dueText, newText, 5)` 在 Rust 内完成「洗牌到期卡 + 抽新词 + 解析时间」→ 渲染时 `current_json()` → 评分时 `rate(rating, Date.now())` 返回**可直接 POST 的请求体** → 后端写 `word_reviews`（due_at = now + 间隔）并记 `review_logs`。
- 记忆状态 `{stability, difficulty}` 与 SQLite `word_reviews` 字段一一对应；间隔最短 10 分钟。
- 随机器：`random_indices(len, seed)` 洗牌复习顺序、`random_sample(len, count, seed)` 抽新词批次、`random_seed()` 取系统种子；由 `session.rs` 内部调用。
- `days_elapsed = floor((now - last) / 86400000)`（取 `last_review_at`，缺失回退 `due_at`，负值归零），在**评分那一刻**换算；时间串用 `js_sys::Date::parse` 解析（与改动前 `new Date(x).getTime()` 同一解析器）。
- ⚠️ **接口空结果返回 `"items": null`**（Go 的 nil 切片）：解析层必须容忍 null，否则「今天没有到期卡」这种正常状态会直接报错（已踩过一次）。
- ⚠️ `fsrs` 6.6.2 只公开 `next_states()`，内部一次算四个分支、**无单评分入口**，所以 `rate()` 是「算四个取一个」。
- 引擎是确定性算术、不依赖系统时钟；`fsrs` 的 rayon/getrandom 已通过 getrandom `wasm_js` 特性适配 wasm32。
- **复习 UI**：`modules/english/english.html` 含 `#reviewApp`，由 `english.js` 的 `initReviewApp()` 驱动（`main.js` 的 `initSubjectModule()` 动态 import）。界面为**极简全屏**风格：顶栏（今日新学 / 今日复习 / 剩余待学 + 记忆元信息）、大字号单词 + 音标胶囊、底部操作区；揭晓后例句目标词高亮 + 「词性 + 释义」。键位：空格揭晓，`Q/W/E/R`（或 `1~4`）评分，`P` 读单词，`L` 读例句（`E` 被「一般」占用）。⚠️ `engine/pkg/` 由 wasm-bindgen 生成（已 gitignore），缺失时需在 `modules/english/engine/` 下重新构建，命令见 `modules/english/README.md`。

### 前端要点
- 导航栏 `rectangle` 内含头像 + 9 个导航项，字段 `data-page="<学科页路径>"`。
- `main.js` 用 **localStorage 缓存**（键前缀 `pageCache_`，30 天过期、自动清理；`CACHE_VERSION` 在结构或路径变更时整体失效），点击导航用 `fetch` 加载并缓存，默认展示英语。
- **学科页路径**：已有独立模块的学科写成 `modules/<学科>/<学科>.html`（当前仅英语）；其余 8 门仍是 `pages/<学科>.html` 占位（`<p>敬请期待</p>`）。
- **学科模块约定**：放在 `modules/<学科>/` 下并导出初始化函数，`main.js` 的 `initSubjectModule()` 按需动态 `import()`；**学科逻辑不得回流到 `main.js`**。
- 本地起站点用仓库根目录的 `dev-server.js`（Node 内置模块实现，静态文件 + `/api` 同源代理，等价线上 Nginx 形态）；不能直接双击 `index.html`（`file://` 下 `/api` 与 WASM 模块都会失败）。

### 管理后台
- **入口**：`admin/index.html`（本地 `/admin/`）。与学生站**完全独立**：不走 `main.js`、不使用学科页的 localStorage 缓存。
- **约定**：学科后台放 `modules/<学科>/admin/`，导出 `mount(container, ctx)`（可选 `unmount()`），再到 `admin/admin.js` 的 `SUBJECT_ADMINS` 登记一行；框架用动态 `import()` 按需加载。通用能力通过 `ctx` 注入：`api` / `toast` / `confirm` / `el` / `escapeHtml` / `setTitle`。
- ⚠️ **登录未实现**：前端预留位是 `checkAuth()` 与 `#adminAuthGate`，**后端也还没有鉴权中间件**；接入登录必须两端一起做，只拦前端挡不住直接调接口的人。**在此之前不要把 `/admin/` 部署到公网**（页面上常驻提示条）。
- **英语后台**（`modules/english/admin/english-admin.js`）：词条列表（搜索 / 词书 / 单元 / 分页）、增删改查、批量导入（粘贴 → `engine` 的 `parse_word_list` 解析 → 预览 → 前端每 200 条分批 POST）。
- **词条接口**：`GET/POST /api/words`、`GET/PUT/DELETE /api/words/:id`、`GET /api/word-options`。
  - `PUT` 是全量更新；改名撞车返回 409。
  - `DELETE` **会连带删除该词的 `word_reviews` 与 `review_logs`**（日志留着会让 stats 虚高）。改错别字用 `PUT`，别删了重建。
  - `POST /api/words` 查重**大小写不敏感**。
  - `word-options` 刻意不在 `/api/words/options`，避免与 `/api/words/:id` 通配路由冲突。

### 后端要点
- API 前缀 `/api`：`/health`、`/hello`、`/user/*`、`/data/*`。
- 用户/数据相关 handler 目前多为 TODO 占位（返回固定 JSON）。
- 环境变量：`SERVER_HOST`（默认 `0.0.0.0`）、`SERVER_PORT`（默认 `8080`）、`APP_ENV`（默认 `development`）。

---

## 二、硬性边界（必须遵守）

1. **备份目录只读**：`备份/`（含 `备份1/`、`备份2/`）是存档副本，**永不修改、永不删除、永不作为建设对象**。读取/列出项目文件时一律跳过 `备份` 目录。
2. **`pages.zip`** 为二进制备份包，不作为文本读取、不修改、不展开，除非用户明确要求检查。
3. **`console.log('FAIL'`** 是一个**空文件**（0 字节），系误用重定向产生的残留：**不是合法代码，不要把它当代码，也不要试图“修复”它**；除非用户确认，不要删除，也不要修改。
4. **`go.sum` 已生成**：后端已有 `go.sum`（GORM + glebarez/sqlite 等依赖已通过 `go mod tidy` 固化）；新增依赖时用 `go mod tidy` 同步即可。
5. **`image/avatar.png`**：当前视觉增强关闭，无法查看绘制内容；按元信息（WebP，约 1330×1146）处理即可，如需主题替换先问用户。
6. **本机工具链**：Go 已装为便携版 `C:\Users\22629\go-portable\go\bin\go.exe`（go1.27.1，已 `go env -w GOPROXY=https://goproxy.cn,direct GOSUMDB=off`，直接 `go build` 即可）；Rust `cargo 1.97` 且 `wasm32-unknown-unknown` target 已装；wasm-bindgen CLI 在 `C:\Users\22629\.local\bin\wasm-bindgen-0.2.128-*\wasm-bindgen.exe`（须与 Cargo.toml 的 wasm-bindgen 版本一致 0.2.128）。
   ✅ **宿主 `cargo test` 现在可以运行**（2026-09-13 实测 42 个测试通过；本文档此前记录的「缺 mingw `as`/MSVC SDK 无法链接」已不再成立）。完整验证路径：`cargo test` → `cargo check --target wasm32-unknown-unknown` → `cargo build --target wasm32-unknown-unknown --release` → `wasm-bindgen` 生成 `pkg/` → 浏览器端到端。
   ⚠️ 但 `JsValue` 在非 wasm32 目标上未实现（调用即 `panic: function not implemented on non-wasm32 targets`，无法 unwinding 会直接 abort）：**纯计算层不要碰 `JsValue`**，把它留在 wasm 导出方法的边界上。
7. **`word_reviews` 行是懒创建**：单词由 `POST /api/words` 写入 `words` 表；首次提交复习时才创建对应 `word_reviews` 行。`/api/reviews/new` = 无复习行的词。

---

## 三、代码风格约定

- 注释与面向用户文案使用**中文**。
- 前端 JS 保持 **ES5** 风格（与现有 `main.js` 一致），不使用 `let`/`const`/箭头函数等 ES6+ 语法，除非用户明确要求升级。
- 路径统一使用**相对路径**；有独立模块的学科放 `modules/<学科>/`（页面命名 `<学科>.html`），其余占位页仍在 `pages/<学科>.html`。
- 改动前**先读取目标文件**，再改动；改动后确认工作区是否被破坏。
- 检查文件内容用 read 工具，不用 `cat`；查找用 grep/glob 工具。

---

## 四、工作流程约定

1. 开工前先明确目标；涉及多步时用待办列表（todo）拆分。
2. 涉及后端 API 改动需同时更新 `README.md`（根目录与 `backend-go/`）中的接口/环境变量说明。
3. 修改文件后，在回复中给出**主要/变更文件的完整路径**（格式化为内联代码）。
4. 若对需求有歧义，先确认再动手，不要擅自大改结构。

---

## 五、支持与许可

项目由作者在爱发电维护；许可证 MIT。涉及对外发布或引用时保留作者信息。
