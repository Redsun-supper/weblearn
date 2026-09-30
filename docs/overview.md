# 项目概览

> 原先在 `CLAUDE.md`，2026-09 拆分。项目结构变动时同步更新本文件。
> 速览版（一句话项目 / 端口 / 库分工）在文档地图 [`README.md`](README.md)，本文件是展开版。

「广学」是一个**学科内容展示平台**：前端静态页面 + Go 后端 API，经 Nginx 反向代理部署到 `test.lovezmx.com`。

| 组成部分 | 说明 |
|----------|------|
| 前端（根目录） | `index.html`（主页）、`main.css`（全站样式）、`main.js`（导航与缓存）、`dev-server.js`（本地开发服务器）、`image/`、`pages/`（8 个占位学科页） |
| 学科模块（`modules/<学科>/`） | 学科自己的页面 / 样式 / 逻辑 / 引擎 / 后台模块；目前只有 `modules/english/` |
| 通用后台（`admin/`） | 独立入口页（`/admin/`）：布局、侧栏导航、hash 路由、通用组件、**登录门禁（只放行 `role=admin`）** |
| 个人中心（`account/`） | 独立入口页（`/account/`）：身份卡 + 账号信息 + 登录中的设备 + 可用操作；未登录时显示登录 / 注册。**入口是站点左上角的头像**（点头像播一段扩散过场再进来），登录态是 httpOnly Cookie，页面靠 `GET /api/auth/me` 判断 |
| 后端（`backend-go/`） | Go 1.21 + Gin + **GORM/SQLite**（`database/`、`handlers/review_handlers.go`），入口 `main.go` |
| 账号系统（`backend-rust/`） | 独立 Rust 认证服务 `guangxue-auth`（axum + rusqlite，库 `auth.db`，端口 8081）：邮箱验证码**开放注册**（邀请码可选，**带邀请码注册即升级为 `admin`**）、多端登录、令牌轮换、会话与邀请码管理；**只处理 `/api/auth/*`** |
| 复习引擎（`modules/english/engine/`） | Rust/WASM crate `guangxue_wasm`：`session.rs`（会话编排）+ `fsrs_engine.rs`（FSRS 调度）+ `randomizer.rs`（随机器）+ `wordlist.rs`（词表解析）+ `card_view.rs`（词性拆分 / 例句高亮切分），浏览器内运行 |
| 部署 | Nginx 反代：`/` → 前端静态文件，`/api/auth/` → `localhost:8081`（Rust），`/api/` → `localhost:8080`（Go） |

## 两个后端、两个库（重要）

- **按前缀分流**：`/api/auth/*` → Rust 账号服务（8081），其余 `/api/*` → Go（8080）。
  本地由 `dev-server.js` 分流（`--auth-port` 对齐），线上由 Nginx 分流 —— **改任一侧端口时两边都要改**。
- **各管各的库**：账号库是 `auth.db`（Rust 独占，表结构见 `migrations/0001_init.sql`），复习库是 `guangxue.db`（Go/GORM 管）。
  **不要跨服务写对方的库**；`users.id` 只作软引用。

## 相关文档

- 复习引擎细节 → [`review-engine.md`](review-engine.md)
- 英语页界面与动画 → [`english-ui.md`](english-ui.md)
- 前端与个人中心 → [`frontend.md`](frontend.md)
- 管理后台与接口 → [`admin-api.md`](admin-api.md)
- 后端与账号系统 → [`backend-auth.md`](backend-auth.md)
- 硬性边界与工具链 → [`boundaries.md`](boundaries.md)
