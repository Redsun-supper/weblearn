# 后端与账号系统

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `backend-go/`、`backend-rust/`、`account/` 前读一遍。
> 词条 / 复习接口清单在 [`admin-api.md`](admin-api.md)。

## Go 主后端（`backend-go/`）

- API 前缀 `/api`：`/health`（`?deep=1` 会真查一次库，坏则 **503**，供监控命令用）、`/hello`、`/words`（读公开、写要管理员）、`/word-options`、`/reviews/*`（**全部要登录**）、`/admin/*`（**全部要管理员**，P1 新增，见下）。`/user/*`、`/data/*` 四条占位路由与 `users` / `data_items` 两张表已在阶段 2 删除——它们与 `auth.db` 同名不同源，留着必然被误用；真正的用户信息是 Rust 侧的 `/api/auth/me`。
- 📊 **管理看板接口（P1，`handlers/admin_stats.go`，整组 `RequireAdmin`）**：`GET /api/admin/stats/overview`（今日 5 个 + 累计 8 个 + `auth_db.available`，**一个请求出全部数字**）、`GET /api/admin/stats/trend?days=7|30`（`points` 已按**北京自然日**升序、缺失的天补 0，前端不再补洞）、`GET /api/admin/users/progress?ids=1,2,3`（≤100 个，给用户列表补当页进度）、`GET /api/admin/users/:id/progress`（单人 + `last7`）。口径：时间一律折成 **UTC+8** 再切天（`auth.db` 存 UTC 文本、`guangxue.db` 存带时区的时间）；「今日新学」= `stability_before = 0`、「今日抽查」= `is_probe`、「今日活跃」= 当天有 `review_logs` 的 distinct `user_id`。**账号库读不到时接口仍 200**：账号侧数字全按 0 + `auth_db.available=false` + 一句人话原因（看板照常能看复习侧）。
- 🔐 **Go 侧鉴权 = 共享密钥本地验签**（阶段 2 实现，`backend-go/middleware/auth.go`）：不查库、不回调 Rust，直接从 Cookie `gx_access` 取令牌，用与账号服务**同一把 `AUTH_JWT_SECRET`** 验 HS256。钉死的三件事必须与 Rust 对齐，否则会静默 401：① 只接受 `HS256`（`jwt.WithValidMethods`，防 `alg=none` / 算法降级）；② 过期容差 60 秒（对应 `backend-rust/src/core/token.rs` 的 `LEEWAY_SECONDS`）；③ 载荷字段名与类型照抄 `AccessClaims`（`sub`/`sid` 是 JSON 数字、`role` 取 `"user"`/`"admin"`）。
- **两级门槛**：`middleware.RequireUser` 挂 `/api/reviews/*` 整组，`middleware.RequireAdmin` 挂 `/api/words` 的 `POST` / `PUT /:id` / `DELETE /:id`。失败响应与 Rust 逐字对齐：401 `{"code":401,"message":"请先登录","error":"unauthenticated"}`、403 `{"code":403,"message":"没有权限","error":"forbidden"}`——前端按 `error` 字段分支（401 → 去登录）。
- **CSRF 闸门**（`middleware.CSRFGuard`，挂在 `/api` 整组）：写方法（非 `GET`/`HEAD`/`OPTIONS`）必须带白名单内的 `Origin`；没有 `Origin` 时才退回「`Content-Type` 必须 `application/json`」这条规则（跨站表单发不出 JSON）。白名单取 `AUTH_ALLOWED_ORIGINS`，默认与 Rust 相同：`http://127.0.0.1:8899,http://localhost:8899`。
- 环境变量：`SERVER_HOST`（默认 `0.0.0.0`）、`SERVER_PORT`（默认 `8080`）、`APP_ENV`（默认 `development`）、`DB_PATH`（默认 `guangxue.db`，相对进程工作目录）、`AUTH_JWT_SECRET`（**无默认值**，空则所有需登录的接口一律 401 并在启动时打印告警）、`AUTH_ALLOWED_ORIGINS`。
- **Go 自己读 `.env`**（`config.loadEnvFile`，约 30 行的最小 dotenv 子集，零依赖）：查找顺序 `$GX_ENV_FILE` → `./.env` → `./backend-go/.env`，取第一个存在的；**已存在的真实环境变量优先**，文件不覆盖它。模板是 `backend-go/.env.example`，本机真实文件 `backend-go/.env` 与 `backend-rust/.env` 里写**同一把** `AUTH_JWT_SECRET`（两个文件都已 gitignore）。改密钥要两边同时改，改完重启两个服务。
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
  - `is_plausible` 只是廉价预筛（字母数字 + `-`，去掉 `-` 后 8~32 位），刻意比生成/自定义码的校验**宽**：用途前缀 `ADMIN`/`POINTS` 含 I/O/U，而且老库里可能还有 Crockford 时代发的码。
  - ⚠️ **两个已知代价（用户明确接受）**：① 邀请码现在等于「管理员授权」，只发给信得过的人；② `/api/auth/email-code` 对任意邮箱都会发码，**它是本服务唯一的对外发信面**，现在靠两层限流兜着（P2 起）：应用内「同邮箱 1 次/分、5 次/时」+「同 IP 200 次/时」（IP 维度刻意放宽 —— 上百人共用校园/公司出口 IP 时，旧的 20 次/时会被自己人打满），外加 Nginx 的 `limit_req`（`gx_auth` 30r/m，单个 IP）。**真要开放注册前仍建议加图形验证码。**
- **三级权限（P0-5 定案、P1 落地）**：`user` < `admin` < `super_admin`（判定在 `models.rs` 的 `role_rank` / `can_enter_admin` / `is_super_role`，两侧的常量字符串必须逐字一致）。
  - `admin`：管词条（`/api/words` 写接口）、看管理看板、**只读**用户列表（服务端把邮箱就地打码成 `22***@qq.com` 并回 `email_masked=true`）；
  - `super_admin`：发码 / 停用 / 按批停用 / 整批发邮件、调角色、封禁解封、强制下线、读审计日志；
  - 两条自锁保护：不能降级「最后一个超管」、也不能封禁「最后一个超管」（都返回 400，否则没人能进后台了）。
- **管理接口（P1，实现在 `http/admin.rs`）**：

  | 接口 | 准入 | 说明 |
  |---|---|---|
  | `GET /api/auth/config` | **匿名** | 只回 `{require_invite}`；注册表单靠它决定「邀请码是不是必填」（强制邀请制下没有码连验证码都发不出去） |
  | `GET /api/auth/admin/audit?action=&actor=&from=&to=&page=&size=` | 超管 | 回 `{items,total,page,size,actions}`；**动作清单跟着列表一起回**（前端不硬编码、也不多发一个请求）；`from`/`to` 依次试 RFC3339 → `YYYY-MM-DD HH:MM:SS` → 纯日期（纯日期当当天 00:00 UTC） |
  | `GET /api/auth/admin/users?role=&status=&keyword=&page=&size=` | 管理员（只读） | `keyword` 搜邮箱/昵称（拼 LIKE 前先剔掉 `%` 与 `_`）；非超管邮箱脱敏且 `email_masked=true`；带 `last_login_at` |
  | `POST /api/auth/admin/invites` | 超管 | 生成邀请码。**不传 `custom_code`** = 系统随机生成 `count` 张（1~50，16 位，字符表 **A-Z 与 0-9**）；**传了 `custom_code`** = 用超管指定的那串码出 1 张（正好 16 位、只允许 A-Z 与 0-9，自动转大写并抹掉手写的 `-`，否则 400 `invalid_params`）。`expires_in_days` ≤ 0 = **永不过期**（源码里就是 `None`）。文件里那张码已存在时**不覆盖也不新建**：默认回 409 `invite_code_taken`，`data` 里带 `{code, existing}`（那张码的状态 / 已用次数 / 谁用过）；带 `allow_existing:true` 重发才**沿用**它，且只改 `max_uses` 与 `expires_at`，`used_count` / `disabled` / `grant_role` / 兑换记录一个字都不动。响应带 `{items,codes,grant_role,custom,reused}` |
  | `POST /api/auth/admin/invites/{id}/disable` | 超管 | 停用一张码（幂等） |
  | `POST /api/auth/admin/invite-batches/{batch_id}/disable` | 超管 | 按批停用，幂等，回 `{batch_id,disabled}`；批次不存在 → 404 |
  | `POST /api/auth/admin/invites/{id}/reset` | 超管 | **重新启用一张已用过的码**：把 `used_count` 清零（码还能被兑换），回 `{id,cleared}`；`invite_uses` 的兑换记录**不删**（谁用过仍然查得到），每次写一条 `invite_reset` 审计；码不存在 → 404，本来没被用过 → `cleared=0`（幂等）。⚠️ 这是**主动降安全**的动作，面板上必须带风险确认 |
  | `POST /api/auth/admin/invite-mail` | 超管 | body `{pairs:[{invite_id,email}]}`，**一对一配对**、≤50 条、逐封独立发送（一封失败不影响其余），回 `{items,sent,failed,mail_mode}` |
  | `POST /api/auth/admin/users/{id}/logout-all` | 超管 | 吊销某人的全部会话（`revoked_reason="admin_revoke"`），审计动作 `user_logout_all` |

  ⚠️ **路由命名**：`invites/{id}/disable` 与「按批停用」不能挤在同一层 —— `invites/batch/...`、`invites/email` 会让静态段与 `{id}` 冲突，症状是一类解释不清的 404；所以按批走 `invite-batches/{batch_id}/disable`、发邮件走 `invite-mail`。
- **审计日志**：`audit_logs` 只记动作与目标，**绝不记密码、验证码、邀请码明文**（发邮件那条只记 id 与统计）。动作名就是库里的字符串，共 16 个：`register`、`login_ok`、`login_fail`、`logout`、`logout_all`、`admin_seed`、`password_change`、`invite_create`、`invite_disable`、`invite_disable_batch`、`invite_reset`、`invite_use`、`invite_email`、`role_change`、`user_status`、`user_logout_all`（注意是 `login_ok` / `login_fail`，**没有** `login` 这个动作）。面板的筛选下拉直接吃接口回的 `actions`（`SELECT DISTINCT`），所以前端不硬编码这份清单。保留期 `AUTH_AUDIT_RETENTION_DAYS`（默认 **180** 天，`0` = 永不清理）：启动时清一次，之后每 24 小时一次（`main.rs` 里的 `tokio::spawn` + `interval`，失败只记 `tracing::warn!` 不影响服务）。
- 验证方式：`cargo test --all-targets`（**198 项全过**：76 单元 + 6（`guangxue-monitor`）+ 116 集成；其中 P1 新增 `tests/audit.rs` 15 项、`tests/invite.rs` 里自定义邀请码 6 项，P2 新增限流解析 4 项与监控命令 6 项）→ `cargo build --release` → `pwsh scripts/smoke.ps1`（24 项端到端）→ `pwsh scripts/verify-auth.ps1`（跨服务联调：脚本总计 **71 项、0 失败、约 15 秒**；其中 P1 新增 18 项、重置已用过的码 4 项、自定义邀请码 6 项，P2 新增 10 项，样式 3 项 —— 深度健康、浅检形状不变、`AUTH_RL_LOGIN_IP=2/60` 真的生效（401/401/429）、格式错拒绝启动、监控命令的 0/1 退出码与「开始故障 / 不重复提醒 / 已恢复」状态机、三份样式表都声明了 `color-scheme: light`）。
- ⚠️ 本地没设 `AUTH_JWT_SECRET` 时每次重启都会随机生成密钥（旧令牌全失效）；`APP_ENV=production` 下必须显式提供它和管理员密码，并关闭 `AUTH_DEV_ENDPOINTS`，否则启动失败。

## 个人中心（`account/`）

**前端入口**：个人中心 `account/`。**布局与后台 `admin/` 同一套**（2026-10 用户要求改成那样）：
未登录是整屏居中的**登录门禁**（`.acc-gate`，检测中那一屏与表单占同一块地方，所以登录态确认前后
没有任何布局位移 —— 这也是它不再需要「舞台高度」那套量高逻辑的原因）；已登录是
**左侧深色侧栏**（头像 + 品牌 + 导航 + 页脚）+ **右侧主区**（顶栏 + 内容），
内容按 **hash 路由**切换：`#/profile`（账号信息）/ `#/devices`（登录过得设备）/ `#/actions`（可用操作）。

- **顶栏右侧那一排**（`.acc-topbar-right`）：服务端连通性小字 `#backendStatus` → 当前账号
  `#topUserName` →「管理后台」按钮 `#accAdmin`（**只有管理员可见**，`hidden` 由 `account.js` 摘）
  →「关于作者」按钮 `#accAbout`（所有登录用户可见）。**2026-10 用户要求**在个人中心顶栏右侧加
  「关于作者」，它指向站点法律声明页 `../legal/index.html` —— 作者署名、账号与仓库链接都在那一页，
  它同时也是 `NOTICE` 附加条款第 3 条要求保留的那个显式入口页（首页左上角「大冬呱」是同一页的
  另一个入口，两处都不得删）。
- **加一个页面** = `account.js` 的 `NAV_GROUPS` 加一项（`{id, name}`）+ `index.html` 里加一个
  `id="panel" + 首字母大写`（如 `panelProfile`）的 `.acc-panel`，导航与路由会自动带上它。
  面板元素 id 由 `panelIdOf()` 统一推导，不要在别处硬编码拼字符串。
- **主窗口固定、只有内容区滚**（2026-10 用户要求「主窗口固定内容滚动不影响主窗口」）：
  `.acc-layout` 是 `height: 100vh; overflow: hidden` 的固定框架，`.acc-main` 与 `.acc-content`
  配 `min-height: 0`，**唯一的滚动容器是 `.acc-content`**（`overflow-y: auto`）——
  设备列表再长，侧栏与顶栏都钉在原地，整页不会跟着滚，也不会出现双滚动条。
  ⚠️ 别把 `overflow` 挪到 `.acc-layout` / `body` 上，那正是要避免的「主窗口跟着滚」。
- 底色仍用**整页深色渐变**（不是后台的浅灰）：从首页头像扩散过来的那块遮罩就是这串渐变，
  改浅了接缝会立刻显出来。
- **头像与首页那颗同尺寸同位**：侧栏里 `.acc-sidebar-avatar` 是 64×64、3px 白边、
  9px 圆角，位置 **(65, 18)** —— 与首页 `.rounded-square` **逐像素重合**（上下来自侧栏的
  18px 内边距；左右是 `margin-left: calc(50% - 32px - 3px)`：栏心 100 - 半宽 32 - 3px 光学偏移，
  首页那边写的是 `left: 65px`）。那 3px 是为了「**看起来**居中」：头像带 3px 白边 + 朝右下的
  投影，几何正中（68）看着偏右，用户看实机截图提过「这不像是居中，向左一点」。
  所以点头像进来时那颗头像**根本不动**，过场只剩「白底扩散」（返回时收圈），没有任何形变。
  ⚠️ 窄屏（≤920px 侧栏横过来）两边**都回到 30px**（侧栏不再是 200px，也就没有「栏心」了）——
  改这里请同时改 `main.css` 的 `.rounded-square`（它末尾有同断点的覆盖规则），两个断点都要量。
- **侧栏自己的左右内边距是 0**（`padding: 18px 0`）：导航那列要**通栏**（选中/悬停的底色
  铺满整条）、蓝色竖条贴在最左缘（`.acc-nav-item::before` 的 `left: 0`），而栏里的东西又要
  **在 200px 内居中** —— 所以左右留白交给各个区块自己写：头像用
  `calc(50% - 32px - 3px)`（比几何正中再左 3px，见上），品牌与页脚靠
  `align-items: center` + 左右 30px 内边距，中线落在栏心 x=100 **附近**（品牌 -3、页脚 -8）。
  ⚠️ **导航那三条是例外：它们左对齐**（2026-10 用户要求「向左靠齐」）——
  `.acc-nav-item` 是 `display: flex` + `padding: 10px 20px 10px 30px`，文字左缘落在 x=30
  （与品牌块的左内边距同一条线），右侧 20px 留给「会话台数」那小字（`.acc-nav-note` 用
  `margin-left: auto` 顶到右边，不再用 float —— flex 容器里 float 不参与布局）。
  居中时三个词长短不一、左缘参差，左对齐后就齐了。
  2026-10 用户的要求依次是「1 位置侧边栏居中、头像区也居中、主界面的头像改成个人中心头像的位置」→
  「就往左一点使其看起来居中」（头像 -3px）→「头像下面的字也改下」（品牌 -3px）→
  页脚「向左靠一点从而居中」「再向左挪一点」「在挪一点」（三档挪到 -8px）→「向左靠齐」（导航左对齐）。
  再早那一版是「左缘 30px 对齐」，更早是侧栏 `padding: 18px 30px` + 导航 `margin: 0 -30px`
  反抵消，都已取代。窄屏那条横排导航也是左对齐（左右各 16px）。
- 本地开发会自动从 `/api/auth/dev/codes` 回填验证码；带 `?from=avatar` 进来时**跳过整页入场动效**，
  与首页的扩散过场衔接。
- **登录 / 注册标签栏的选中高亮是会滑动的滑块** `.acc-tabs-thumb`——宽度用 `calc(50% - 3px)` 算、
  靠 `translateX(100%)` 换位，不需要 JS 量像素；切标签时表单按点击方向用 `is-enter-right` /
  `is-enter-left` 轻轻浮入，并且**白卡高度会平滑延伸 / 回缩**（`animateCardHeight()`：
  量旧高 → 换内容 → 量新高 → 过渡到新高 → 收尾**必须清掉行内 height/overflow** 还原成自动高度，
  否则报错文案或窄屏换行撑高的内容会被裁掉；量新高前也要先清掉上一轮的行内高度，
  否则量到的是被 `overflow:hidden` 裁过的值；卡片还是 `hidden` 时不要量、不要播，交给入场动效）。
- **会话台数显示在两处**：侧栏「登录过得设备」那项的右侧小字（`#navDevicesCount`）与页面标题旁
  （`#devicesCount`）。`renderNav()` 会重建导航，所以重渲染后要把数字补回去（同一个渲染函数负责两处）。
- `/api/auth/me` 用的是**最短显示时长** `MIN_CHECK_MS = 600`：本地它只要几毫秒，不等一下的话
  检测屏会一闪而过。
- **检测屏 → 白卡是「接力」不是「切换」**：检测屏 `#accCheck` 是**绝对定位叠在白卡应该在的位置**
  （水平/竖直居中），所以它不会被弹入的白卡挤到旁边去 —— 早先它在 flex 行里占位，白卡一出现
  就被挤到左边，看着像「左边还在转圈、右边突然冒出一张卡」。换场由 `showGate()` 掐表下达：
  `0ms` 挂 `is-leaving`（`accCheckOut`：淡出 + 缩到 0.94 + 模糊 4px）→ `TIMING.gateDelay`(40ms)
  白卡解除 `hidden` 并挂 `is-arriving`（`accCardIn`：从 0.9 倍 + 下移 10px 弹出，缓动 c5 带一点回弹）
  → 两者交叠三四百毫秒。⚠️ `screenFade`(400) **必须明显大于** `gateDelay`(40)：反过来就会出现
  「检测屏已淡到 0、白卡还是 opacity: 0」的空档，那正是用户报的「生硬地弹出来」（2026-10 实测踩过）。
- 其余入口：后台门禁 `admin/`、站点左上角头像（点它进个人中心）。三处都只做界面层门禁，服务端判定仍是权威。

- 首页头像入口与扩散过场的实现细节见 [`frontend.md`](frontend.md)。
- 布局与后台同一套，**通用组件的写法照 `admin/admin.css` 那一套来**（卡片、按钮、表单、徽章）。
  两个**壳**（`account.css` / `admin.css`）各自独立、不共享样式表，改一边不会影响另一边；
  ⚠️ **唯一的例外是管理面板模块**（`admin/panels/*.js`）：它们是壳无关的（个人中心与后台都能挂同一份），
  所以样式跟着模块走，共用 `admin/panels/panels.css`（`pn-` 前缀，浅色卡片风，**零动效**）。
  给面板加样式改那个文件，别改两个壳的 CSS。窄屏断点在 `920px`（侧栏横过来放到顶部）。
- 📌 **待处理问题统一记在根目录 [`TODO.md`](../TODO.md)**：沉浸模式下的「返回」反向动画（暂缓，含三个候选方案）、头像 2.6MB 的缩略图（等用户点头，涉及素材）、转场时长的人工对齐。**接手账号 / 转场相关改动前先读它一遍**，别把已决定暂缓的事又当成漏掉的 bug。
