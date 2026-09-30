# 后端与账号系统

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `backend-go/`、`backend-rust/`、`account/` 前读一遍。
> 词条 / 复习接口清单在 [`admin-api.md`](admin-api.md)。

## Go 主后端（`backend-go/`）

- API 前缀 `/api`：`/health`、`/hello`、`/user/*`、`/data/*`。
- 用户/数据相关 handler 目前多为 TODO 占位（返回固定 JSON）。⚠️ `/api/user/*` 与账号系统无关（真正的账号接口是 Rust 侧的 `/api/auth/me`），且**Go 侧目前没有任何鉴权中间件**，`/api/words` 的写接口是公开的。
- 环境变量：`SERVER_HOST`（默认 `0.0.0.0`）、`SERVER_PORT`（默认 `8080`）、`APP_ENV`（默认 `development`）、`DB_PATH`（默认 `guangxue.db`，相对进程工作目录）。
- 端口与库的分工见 [`overview.md`](overview.md)。

## Rust 账号系统（`backend-rust/`）

- **两个后端，按前缀分流**：`/api/auth/*` → Rust 账号服务（8081），其余 `/api/*` → Go（8080）。本地由 `dev-server.js` 分流（`--auth-port` 对齐），线上由 Nginx 分流。改任一侧端口时两边都要改。
- **两个库，各管各的**：账号库是 `auth.db`（Rust 独占，表结构见 `migrations/0001_init.sql`），复习库是 `guangxue.db`（Go/GORM 管）。不要跨服务写对方的库；`users.id` 只作软引用。
- **分层**：`core/`（纯逻辑，注入时钟，全部带单测）→ `store/`（单连接 + 事务 + SQL）→ `service.rs`（业务规则，HTTP 与 CLI 共用）→ `http/`（axum 路由 / Cookie / CSRF / 提取器）。
- ⚠️ 两条踩过的坑：① `store::write` 的约定是「闭包返回 `Err` 就回滚」，**业务拒绝要用 `TxOutcome::Reject`**（否则验证码试错次数会被回滚掉，等于没有防爆破）；② **别在持有数据库锁时做 Argon2 或发信**（连接全局串行，会把所有请求堵住）。
- 响应沿用 `{code, message, data}` 信封，失败多一个 `error` 字段；鉴权一律用 `AuthUser` / `AdminUser` 提取器，别在 handler 里自己解析 Cookie。
- **邀请码的定位（2026-09 改，别按旧口径理解）**：邀请码**不再是注册门槛**，而是「兑换券 / 授权书」——
  - 不填 → 邮箱验证码是唯一门槛，注册出来是普通用户（`role=user`，即**开放注册**）；
  - 填了 → 逐个校验（未停用 / 未过期 / 没用完，任一不合法就整体拒绝，不静默降级），注册成功后 `role=admin`。将来同一入口承载积分 / 礼物兑换，后台靠邀请码里的 **`-` 前缀**区分用途（所以**规范化时绝不能把 `-` 抹掉**）。
  - **多个邀请码用空白分隔**（`invite::split_codes`，顺带去重，防止同一个码被扣两次）；`-` 是码自身的格式、不是分隔符。管理员手抄成 `XXXX-XXXX-XXXX-XXXX` 的老写法靠 `service::find_invite` 兜底（先按原样查，查不到再去掉 `-` 查一次）。
  - `is_plausible` 只是廉价预筛（字母数字 + `-`，去掉 `-` 后 8~32 位），**刻意不套生成时那套 Crockford 字母表**：用途前缀 `ADMIN`/`POINTS` 含 I/O/U，套了就永远传不进去。
  - ⚠️ **两个已知代价（用户明确接受）**：① 邀请码现在等于「管理员授权」，只发给信得过的人；② `/api/auth/email-code` 对任意邮箱都会发码，**它是本服务唯一的对外发信面**，目前只有限流兜着（同邮箱 1 次/分、5 次/时；同 IP 20 次/时）——要上真 SMTP 前建议再加图形验证码或网关层限流。
- 权限等级**只预留** `users.role` / `users.status`，还没写判定逻辑；唯一例外是邀请码管理接口的 `role == 'admin'` 准入。
- 验证方式：`cargo test`（90 项）→ `cargo build --release` → `pwsh scripts/smoke.ps1`（24 项端到端，直连或经 8899 代理都行）。
- ⚠️ 本地没设 `AUTH_JWT_SECRET` 时每次重启都会随机生成密钥（旧令牌全失效）；`APP_ENV=production` 下必须显式提供它和管理员密码，并关闭 `AUTH_DEV_ENDPOINTS`，否则启动失败。

## 个人中心（`account/`）

**前端入口**：个人中心 `account/`（身份卡 + 账号信息 + 设备列表 + 操作；本地开发会自动从 `/api/auth/dev/codes` 回填验证码；带 `?from=avatar` 进来时跳过身份卡的入场动效，与首页的扩散过场衔接；**登录 / 注册标签栏的选中高亮是会滑动的滑块** `.acc-tabs-thumb`——宽度用 `calc((100% - 16px) / 2)` 算、靠 `translateX(calc(100% + 8px))` 换位，不需要 JS 量像素；切标签时表单按点击方向用 `is-enter-right` / `is-enter-left` 从侧边滑入，并且**白卡高度会平滑延伸 / 回缩**（`animateCardHeight()`：量旧高 → 换内容 → 量新高 → 过渡到新高 → 收尾**必须清掉行内 height/overflow** 还原成自动高度，否则报错文案或窄屏换行撑高的内容会被裁掉；量新高前也要先清掉上一轮的行内高度，否则量到的是被 `overflow:hidden` 裁过的值；卡片还是 `hidden` 时不要量、不要播，交给入场动效））、后台门禁 `admin/`、站点左上角头像（点它进个人中心）。三处都只做界面层门禁，服务端判定仍是权威。

- 首页头像入口与扩散过场的实现细节见 [`frontend.md`](frontend.md)。
- 📌 **待处理问题统一记在根目录 [`TODO.md`](../TODO.md)**：沉浸模式下的「返回」反向动画（暂缓，含三个候选方案）、头像 2.6MB 的缩略图（等用户点头，涉及素材）、登录后画面是临时的、转场时长的人工对齐。**接手账号 / 转场相关改动前先读它一遍**，别把已决定暂缓的事又当成漏掉的 bug。
