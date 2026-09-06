# 广学项目 · 工作规则（CLAUDE.md）

> 本文件由 DeepSeek Harness 的 `agent-instructions` 插件在新对话的首条消息自动注入，**每个新会话只注入一次**。
> 用于统一项目的背景、边界与约定，避免每次对话重复读取整个项目。

---

## 一、项目概述

「广学」是一个**学科内容展示平台**：前端静态页面 + Go 后端 API，经 Nginx 反向代理部署到 `test.lovezmx.com`。

| 组成部分 | 说明 |
|----------|------|
| 前端（根目录） | `index.html`（主页）、`main.css`（样式）、`main.js`（交互与缓存）、`image/`、`pages/`（9 个学科页） |
| 后端（`backend-go/`） | Go 1.21 + Gin + **GORM/SQLite**（`database/`、`handlers/review_handlers.go`），入口 `main.go` |
| 复习引擎（`frontend-rust/`） | Rust/WASM crate `guangxue_wasm`：`fsrs_engine.rs`（FSRS 调度）+ `randomizer.rs`（随机器），浏览器内运行 |
| 部署 | Nginx 反代：`/` → 前端静态文件，`/api/` → `localhost:8080` |

### 复习引擎（词汇间隔重复）
- **流程**：前端运行时调 `GET /api/reviews/due`（到期卡）+ `GET /api/reviews/new`（新词）→ WASM 引擎 `fsrs_next_states(state_json, retention, days_elapsed)` 计算四种评分下一状态 → 选分支后 `POST /api/reviews/submit` 持久化 → 后端写入 `word_reviews`（due_at = now + 间隔）并记 `review_logs`。
- 记忆状态 `{stability, difficulty}` 与 SQLite `word_reviews` 字段一一对应；间隔最短 10 分钟。
- 随机器：`random_indices(len, seed)` 洗牌复习顺序、`random_sample(len, count, seed)` 抽新词批次、`random_seed()` 取系统种子。
- 引擎是确定性算术、不依赖系统时钟；`fsrs` 的 rayon/getrandom 已通过 getrandom `wasm_js` 特性适配 wasm32。
- **复习 UI（简单版）**：`pages/english.html` 含 `#reviewApp`，由 `main.js` 中 `initReviewApp()` 驱动（动态 import `frontend-rust/pkg/guangxue_wasm.js`，调 `/api/reviews/*`）。⚠️ `frontend-rust/pkg/` 由 wasm-bindgen 生成（已 gitignore），若缺失需重新执行：`wasm-bindgen --target web --out-dir pkg --out-name guangxue_wasm target/wasm32-unknown-unknown/release/guangxue_wasm.wasm`。

### 前端要点
- 导航栏 `rectangle` 内含头像 + 9 个导航项，字段 `data-page="pages/<学科>.html"`。
- `main.js` 用 **localStorage 缓存**（键前缀 `pageCache_`，30 天过期、自动清理），点击导航用 `fetch` 加载并缓存，默认展示英语。
- 学科页命名 `pages/<学科>.html`，其余 7 门为占位（`<p>敬请期待</p>`），仅 `english.html` 已有内容。

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
6. **本机工具链（2026-09 现状）**：Go 已装为便携版 `C:\Users\22629\go-portable\go\bin\go.exe`（go1.27.1，已 `go env -w GOPROXY=https://goproxy.cn,direct GOSUMDB=off`，直接 `go build` 即可）；wasm-bindgen CLI 在 `C:\Users\22629\.local\bin\wasm-bindgen-0.2.128-*\wasm-bindgen.exe`（须与 Cargo.toml 的 wasm-bindgen 版本一致 0.2.128）。宿主 `cargo test` 仍无法链接（缺 mingw `as`/MSVC SDK），验证路径：`cargo check` + `cargo build --target wasm32-unknown-unknown --release` + Node 跑 `target/nodejs-pkg` 功能测试。
7. **`word_reviews` 行是懒创建**：单词由 `POST /api/words` 写入 `words` 表；首次提交复习时才创建对应 `word_reviews` 行。`/api/reviews/new` = 无复习行的词。

---

## 三、代码风格约定

- 注释与面向用户文案使用**中文**。
- 前端 JS 保持 **ES5** 风格（与现有 `main.js` 一致），不使用 `let`/`const`/箭头函数等 ES6+ 语法，除非用户明确要求升级。
- 路径统一使用**相对路径**；新增学科页放入 `pages/`，命名 `<学科>.html`。
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
