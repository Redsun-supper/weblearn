# guangxue-auth — 广学账号系统（Rust 认证服务）

位置：`backend-rust/`。**独立于 Go 主后端的认证服务**，负责 `/api/auth/*`：
邮箱注册（管理员派发的邀请码 + 邮箱验证码）、多端同时登录、令牌轮换与会话管理。

线上由 Nginx 把 `/api/auth/` 分流到本服务（127.0.0.1:8081），本地由仓库根目录的
`dev-server.js` 做同样的事；Go 后端继续负责词汇复习接口，两者各管一段，互不干扰。

```
浏览器
  ├─ /api/auth/*  →  Rust 认证服务 (axum + SQLite → auth.db)      ← 本目录
  └─ /api/*       →  Go 主后端   (Gin + GORM  → guangxue.db)      ← ../backend-go
```

---

## 一、快速开始

```bash
cd backend-rust

# 1) 跑测试（90 项：41 单元 + 49 集成）
cargo test

# 2) 起服务（默认 127.0.0.1:8081，首次启动自动建库、迁移、创建管理员）
cargo run --release

# 3) 另开一个终端：端到端冒烟（24 项断言，含多端登录与重放检测）
pwsh scripts/smoke.ps1
#    也可以经 dev-server 代理跑，顺便验证分流：
pwsh scripts/smoke.ps1 -BaseUrl http://127.0.0.1:8899
```

### 配套的前端入口（不需要额外配置）

`dev-server.js` 会把 `/api/auth/*` 分流到本服务，页面同源直接用 httpOnly Cookie：

| 地址 | 用途 |
|------|------|
| http://127.0.0.1:8899/account/ | 个人中心：身份卡 / 账号信息 / 登录中的设备 / 可用操作（未登录时是登录 / 注册） |
| http://127.0.0.1:8899/admin/ | 后台（只放行 `role=admin`） |
| http://127.0.0.1:8899/ | 学生站，**左上角头像**是个人中心入口（点头像播一段扩散过场） |

本地开发（`AUTH_MAIL_MODE=log`）时账号页会自动调 `/api/auth/dev/codes` 把验证码填进注册表单。

首次启动会打印：

```
广学 · 账号系统（Rust 认证服务）已启动
  监听地址    : http://127.0.0.1:8081
  数据库      : auth.db
  邮件模式    : log（只打日志，不发信）
  ...
  ⚠️ 未设置 AUTH_JWT_SECRET，本次已随机生成：重启后旧令牌会失效（正式部署请固定它）
  ⚠️ 正在使用默认管理员密码，请首次登录后立即更换（生产环境会拒绝启动）
```

**初始管理员**：`2262997289@qq.com` / `7289HR_RedSun`（`role=admin`，邮箱视为已验证）。
密码可用 `AUTH_ADMIN_PASSWORD` 覆盖；`--reset-password` 可重置（见下方 CLI）。

---

## 二、接口一览

全部挂在 `/api/auth` 下。响应沿用 Go 侧的信封 `{code, message, data}`，
失败时多一个机器可读的 `error` 字段（如 `invalid_invite`）。

| 方法 | 路径 | 鉴权 | 说明 |
|------|------|------|------|
| GET | `/health` | 无 | 服务健康检查（含邮件模式、调试接口开关） |
| POST | `/email-code` | 无 | 发注册验证码。邀请码**可选**：填了就先校验有效再发信；不填直接发（开放注册） |
| POST | `/register` | 无 | `{email, email_code, password, username?, device_label?, invite_code?}`，成功即登录；**带邀请码 → `role=admin`** |
| POST | `/login` | 无 | `{email, password, device_label?}`，**每次登录新建一个会话**（多端并存） |
| POST | `/refresh` | refresh Cookie | 轮换 refresh + 换发 access；命中已轮换过的令牌即判重放 |
| POST | `/logout` | 会话（幂等） | 只吊销当前这个端，并清 Cookie |
| GET | `/me` | 会话 | 当前用户 `{id,email,username,role,status,email_verified_at,created_at}` |
| GET | `/sessions` | 会话 | 我的活跃会话（设备名 / IP / UA / 创建时间 / 最近活跃 / 是否当前端） |
| POST | `/logout-all` | 会话 | 吊销全部端，可选 `{keep_current:true}` 只踢其他端 |
| GET | `/admin/invites` | 会话 + `role=admin` | 邀请码列表，支持 `status=unused\|used\|expired\|disabled` + 分页 |
| POST | `/admin/invites` | 会话 + `role=admin` | `{count,max_uses,expires_in_days,note}`；**明文码只在这次响应里出现** |
| POST | `/admin/invites/{id}/disable` | 会话 + `role=admin` | 停用邀请码 |
| GET | `/dev/codes?email=` | 仅 development | 取最近一次发给该邮箱的验证码（本地无 SMTP 时联调用） |

### 错误码（`error` 字段）

| `error` | HTTP | 场景 |
|---------|------|------|
| `invalid_params` | 400 | 邮箱格式 / 密码强度 / 参数缺失 |
| `invalid_invite` | 400 | 邀请码不存在、被停用（**只在填了邀请码时才会出现**） |
| `invite_expired` / `invite_exhausted` | 400 | 邀请码过期 / 用尽 |
| `invalid_code` / `code_expired` / `code_attempts_exceeded` | 400 | 验证码错误 / 过期 / 试错超限 |
| `bad_credentials` | 401 | 密码错误（**与「账号不存在」同一文案**，避免枚举邮箱） |
| `unauthenticated` | 401 | 未登录、access 过期、会话被吊销、refresh 重放 |
| `forbidden` | 403 | 账号停用、角色不足、CSRF 来源校验失败 |
| `email_taken` | 409 | 邮箱已注册 |
| `account_locked` / `rate_limited` | 429 | 账号锁定 / 触发限流 |
| `not_found` | 404 | 记录不存在（如停用一个不存在的邀请码） |
| `mail_failed` | 502 | 邮件发送失败 |
| `internal` | 500 | 服务内部错误（细节只进服务端日志） |

### 响应示例

```jsonc
// POST /api/auth/login 200
{
  "code": 200,
  "message": "登录成功",
  "data": {
    "user": { "id": 2, "email": "a@example.com", "username": "小明", "role": "user",
              "status": "active", "email_verified_at": "2026-09-15T08:31:13Z",
              "created_at": "2026-09-15T08:31:13Z" },
    "device_label": "Windows · Chrome",
    "access_expires_in": 900,
    "refresh_expires_in": 2592000
  }
}
// 令牌本身只出现在 Set-Cookie 里，响应体里没有
```

---

## 三、安全设计（为什么这么做）

| 面向 | 做法与理由 |
|------|-----------|
| 密码存储 | **Argon2id**（默认 `m=19456 KiB, t=2, p=1`，即 OWASP 推荐值），每个密码独立 16 字节盐，库里只有 PHC 串；参数写在哈希里，日后调参不会让老哈希失效 |
| 传输 | 线上靠 **HTTPS（Nginx 终止 TLS）**；本地 `http://127.0.0.1` 仅用于回环调试。明文密码只存在于 TLS 通道与服务器内存 |
| 登录态 | **httpOnly Cookie**：JS 读不到令牌（XSS 也偷不走）。access 是 15 分钟 HS256 JWT，refresh 是 32 字节随机串（库里只存 SHA-256 摘要） |
| 即时吊销 | 受保护接口除验签外还要查 `sessions`（未吊销、未过期）+ 用户 `status='active'`，所以**登出后 access 立刻失效**，不用等 15 分钟 |
| 多端登录 | 每次登录新建一行会话，端与端互不影响；`/sessions` 可看到设备名与最近活跃；`/logout-all` 可一键踢掉全部端 |
| 重放防护 | refresh 每次使用都轮换；**已轮换过的令牌再次出现即判定重放**，整条轮换链（含当前端）一起吊销 |
| 暴力破解 | ① 账号维度：连续失败 5 次锁 15 分钟（落库，重启也绕不过）；② IP/邮箱维度：滑动窗口限流（登录 20 次/15 分、发码 1 次/分且 5 次/时、发码 IP 20 次/时、注册 IP 10 次/时） |
| 邮件滥用 | ⚠️ 注册开放后 `/email-code` 对任意邮箱都会发码，**这是本服务目前唯一的对外发信面**。现有防线只有限流（同邮箱 1 次/分、5 次/时；同 IP 20 次/时）。要上线真 SMTP 前建议再加一层（图形验证码 / 预热白名单 / 网关层限流） |
| 验证码 | 6 位数字，库里只存 **HMAC-SHA256(pepper, email:code)**；10 分钟有效、最多 5 次尝试；重发会让旧码立即失效 |
| CSRF | `SameSite=Lax` + 非 GET 请求校验 `Origin` 白名单；没有 `Origin` 时要求 `Content-Type: application/json`（跨站表单发不出 JSON） |
| 计时攻击 | 账号不存在时也执行一次同参数 Argon2 校验（`dummy_verify`），响应时间与「密码错误」接近；验证码/HMAC 比对用恒定时间比较 |
| 审计 | `audit_logs` 记录注册/登录成败/登出/重放/邀请码操作；**不含密码与验证码明文**；日志里的邮箱打码（`22***@qq.com`） |
| 权限等级 | 只预留 `users.role`（`user`/`admin`）与 `users.status`（`active`/`disabled`）字段，**不实现 RBAC**；唯一的准入检查是邀请码管理接口的 `role == 'admin'` |

> ⚠️ **「加密上传」的口径**：本项目不做应用层信封加密，传输安全完全交给 TLS。
> 原因：在无 HTTPS 的前提下，浏览器端加密并不能防中间人——攻击者可以篡改下发的
> JS 与公钥，密钥本身也在明文链路上。**上线必须用 HTTPS**，并且后端端口不要直接暴露公网。

### 已知限制（刻意留白，别当成 bug）

1. **限流是单进程内存实现**：多开实例时每个实例各算一份，进程重启即清零。
   真要多实例部署，换成 Redis 或网关层限流。
2. **SQLite 单写者**：用 WAL + `busy_timeout=5s` 缓解，并发写入一大就需要换 Postgres。
3. **调试接口会泄露验证码**：只在 `APP_ENV=development` 注册该路由；生产环境若把
   `AUTH_DEV_ENDPOINTS=true` 打开会**直接启动失败**（配置层强制）。
4. 没有登录系统以外的账号功能：**改密 / 找回密码 / 强制改密 / 删除账号**都还没做
   （CLI 的 `seed-admin --reset-password` 是唯一的改密手段）。

---

## 四、数据库（`auth.db`）

独立于 Go 的 `guangxue.db`，由本服务独占（避免两套 ORM 抢 schema）。
`users.id` 是**跨服务的软引用**：Go 侧 `word_reviews.user_id` / `review_logs.user_id` 指的就是它
（值来自访问令牌的 `sub`，见 [`../docs/backend-auth.md`](../docs/backend-auth.md) 的「Go 主后端」一节）。
⚠️ 两边不共享事务、也不做外键约束：删账号不会自动清掉那个人的复习进度。

| 表 | 用途 |
|----|------|
| `users` | 账号：邮箱（唯一）、Argon2id 密码哈希、`role`/`status`（预留）、失败次数与锁定时间 |
| `user_identities` | 登录标识 `(provider, identifier)` 唯一 —— **多方式登录的预留位**，本期只写 `provider='email'` |
| `sessions` | 一行 = 一个端的登录；`family_id` + `replaced_by` 记录 refresh 轮换链 |
| `invite_codes` / `invite_uses` | 邀请码与使用留痕（码存明文：它是管理员要核对、转发的短期凭据） |
| `email_codes` | 验证码（只存 HMAC 摘要）+ 尝试次数 |
| `audit_logs` | 审计 |

迁移用 `PRAGMA user_version` 记版本，SQL 在 `migrations/0001_init.sql`（编译期内嵌）。
**时间列统一是 UTC、秒精度、固定宽度的 RFC3339 文本**（`2026-09-15T15:53:22Z`）：
SQLite 里比较时间靠字典序，一旦带上可变长度的小数秒（`12:00:00Z` vs `12:00:00.5Z`）顺序就会错。

---

## 五、环境变量

完整示例见 `.env.example`（可复制成 `.env`，已 gitignore）。要点：

| 变量 | 默认 | 说明 |
|------|------|------|
| `APP_ENV` | `development` | `production` 会启用两条硬性检查（见下） |
| `AUTH_HOST` / `AUTH_PORT` | `127.0.0.1` / `8081` | 默认只监听回环，公网由 Nginx 进 |
| `AUTH_DB_PATH` | `auth.db` | SQLite 文件 |
| `AUTH_JWT_SECRET` | 开发随机生成 | **production 必填**；开发缺失时随机生成并告警（重启后旧令牌失效） |
| `AUTH_CODE_PEPPER` | 取 JWT 密钥 | 验证码 HMAC 的 pepper |
| `AUTH_ACCESS_TTL_SECONDS` / `AUTH_REFRESH_TTL_DAYS` | `900` / `30` | 令牌有效期 |
| `AUTH_COOKIE_SECURE` | 随 `APP_ENV` | 本地 http 必须为 false，否则浏览器不收 Cookie |
| `AUTH_ALLOWED_ORIGINS` | `http://127.0.0.1:8899,http://localhost:8899` | CSRF 白名单 |
| `AUTH_MAIL_MODE` | `log` | `log` 只打日志；`smtp` 真发信 |
| `AUTH_SMTP_HOST/PORT/USERNAME/PASSWORD/FROM/TLS` | — | QQ 邮箱填**授权码**（不是登录密码）；465 用 `tls`，587 用 `starttls` |
| `AUTH_DEV_ENDPOINTS` | 随 `APP_ENV` | 生产强制关闭 |
| `AUTH_ADMIN_EMAIL` / `AUTH_ADMIN_PASSWORD` | `2262997289@qq.com` / `7289HR_RedSun` | 初始管理员；生产必须显式提供密码 |
| `AUTH_SEED_ADMIN` | 随 `APP_ENV` | 启动时确保管理员存在（幂等，不覆盖已有密码） |
| `AUTH_ARGON2_M_COST/T_COST/P_COST` | `19456/2/1` | 密码哈希参数 |
| `AUTH_RL_*` / `AUTH_LOCK_THRESHOLD` / `AUTH_LOCK_MINUTES` | 见 `.env.example` | 限流与锁定 |

**生产环境的两条硬性检查**（不满足直接启动失败，不会带着危险默认值跑起来）：

1. 必须显式提供 `AUTH_JWT_SECRET`；
2. 必须显式提供 `AUTH_ADMIN_PASSWORD`（或 `AUTH_SEED_ADMIN=false`），且不许用默认密码；
   同时 `AUTH_DEV_ENDPOINTS` 必须关闭。

---

## 六、命令行工具

```bash
# 建 / 重置管理员（幂等；已存在时不覆盖密码）
cargo run --release --bin seed-admin
cargo run --release --bin seed-admin -- --email 2262997289@qq.com --password 新密码
cargo run --release --bin seed-admin -- --reset-password --password 新密码   # 会吊销该账号全部会话

# 邀请码
cargo run --release --bin invite -- create --count 3 --max-uses 1 --expires-in-days 7 --note "第一批"
cargo run --release --bin invite -- list --status unused
cargo run --release --bin invite -- disable 12
```

CLI 与 HTTP 接口**共用同一套服务层**（`AuthService`），所以校验规则与行为完全一致
（例如密码强度、邀请码状态机）。

---

## 七、目录结构

```
backend-rust/
├── Cargo.toml / Cargo.lock
├── .env.example
├── migrations/0001_init.sql     # 编译期内嵌，PRAGMA user_version 记版本
├── scripts/smoke.ps1            # 端到端冒烟（直连或经 dev-server 都可以）
└── src/
    ├── main.rs / lib.rs         # 入口与 AppState 装配
    ├── config.rs                # 环境变量 → Config（含 production 强校验）
    ├── error.rs                 # AuthError → HTTP 状态 + {code,message,error}
    ├── clock.rs                 # Clock trait + SystemClock/FakeClock（测试可控时间）
    ├── db.rs                    # 连接、PRAGMA、迁移、时间文本格式
    ├── models.rs                # 数据库行（*Row）与对外输出（*Public）
    ├── store/{mod,sql}.rs       # 单连接 + 事务边界 + 全部 SQL
    ├── service.rs               # 业务规则（注册/登录/刷新/会话/邀请码/管理员）
    ├── rate_limit.rs            # 内存滑动窗口限流
    ├── cli.rs                   # 命令行参数小工具
    ├── core/                    # ★ 纯逻辑（不碰 IO，全部带单元测试）
    │   ├── password.rs          # Argon2id 哈希与等时校验
    │   ├── token.rs             # JWT 签发/校验 + refresh 生成与摘要
    │   ├── invite.rs            # 邀请码生成、规范化、状态机
    │   ├── email_code.rs        # 验证码生成、HMAC 摘要、状态机
    │   └── validate.rs          # 邮箱/密码/用户名/设备名校验
    ├── mail/{mod,log_mailer,smtp}.rs
    ├── http/{mod,auth,admin,dev,middleware,extract}.rs
    └── bin/{seed_admin,invite}.rs
```

### 分层约定（改代码前请先读）

- `core/` 是纯计算：不读系统时钟（时间由调用方传入）、不碰数据库，因此边界条件可以用
  单元测试钉死（与 `modules/english/engine` 的做法一致）。
- `store::write` 的约定是「闭包返回 `Err` 就回滚」。**业务拒绝要用 `TxOutcome::Reject`**
  而不是直接 `Err`：验证码试错次数这类记账必须落盘，回滚掉就等于没有记账
  （踩过：`exceeding_attempts_blocks_even_the_correct_code` 锁住这条）。
- **不要在持有数据库锁的时候做慢操作**（Argon2、发信、网络）：连接是全局串行的，
  一个 50ms 的哈希会把所有请求一起堵住。服务层因此把哈希放在事务之外。
- 受保护接口一律用 `AuthUser` 提取器（`http/extract.rs`），管理接口用 `AdminUser`；
  不要在 handler 里自己解析 Cookie。

---

## 八、测试

```bash
cargo test                 # 90 项：41 单元 + 49 集成（6 个测试文件）
cargo build --release      # 产出 target/release/guangxue-auth.exe
pwsh scripts/smoke.ps1     # 端到端：真实 HTTP + 真实 Cookie
```

集成测试用临时目录里的真 SQLite + 真实 axum Router（`tower::oneshot`），
`Client` 自带 cookie jar，所以「两个 Client」就是两端同时登录：

| 文件 | 覆盖内容 |
|------|---------|
| `tests/auth_flow.rs` | 注册→me→刷新轮换→登出；重放吊销整族；密码/邮箱校验；令牌过期 |
| `tests/invite.rs` | 邀请码状态机、单次使用、**并发占用只有一个成功**、手抄格式规范化、筛选 |
| `tests/email_code.rs` | 试错上限、过期、一次性、重发失效、发码/登录限流、生产不暴露调试接口 |
| `tests/multi_device.rs` | 多端并存、单端登出、`logout-all`（含 keep_current）、各自独立刷新 |
| `tests/security.rs` | 鉴权覆盖、跨站 Origin、Cookie 属性、**响应与库里都不出现密码明文**、审计、账号锁定 |
| `tests/admin.rs` | 初始管理员登录、seed 幂等、非管理员 403、邀请码增删查 |

---

## 九、部署要点（线上）

```nginx
# 账号系统：注意 location 顺序无关（nginx 取最长前缀匹配），但要放在 /api/ 之外的独立块
location /api/auth/ {
    proxy_pass http://127.0.0.1:8081;   # 末尾不要带 /（否则会剥掉前缀）
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

- 必须走 **HTTPS**（Cookie 才会带 `Secure`；`AUTH_COOKIE_SECURE` 在 production 自动为 true）；
- `AUTH_JWT_SECRET` 用足够长的随机值并固定下来（换掉 = 所有登录态失效）；
- 服务以 systemd / nohup 常驻：`AUTH_HOST=127.0.0.1 AUTH_PORT=8081 ./guangxue-auth`；
- 首次上线后立刻改管理员密码：`seed-admin --reset-password --password 新密码`；
- 接入真实邮件前把 `AUTH_MAIL_MODE=smtp` 与 `AUTH_SMTP_*` 配好（QQ 邮箱用授权码）。

---

## 十、下一步（本期刻意没做）

- [x] **前端**：个人中心 `account/`（身份卡 + 账号信息 + 设备列表 + 可用操作，组件带入场动效）、
      `admin/` 的登录门禁（`checkAuth()` 已接 `/api/auth/me`，只放行 `role=admin`）、
      站点左上角头像入口（点头像从头像位置扩散盖满全屏再跳个人中心；已登录亮状态点）
- [ ] **权限等级**：`role`/`status` 已预留，RBAC、权限点、用户管理界面尚未实现
      （前端门禁目前只判断 `role === 'admin'`）
- [x] **保护既有接口**：`/api/words` 写接口的鉴权（Go 侧用同一 `AUTH_JWT_SECRET` 本地验签，`RequireAdmin` / `RequireUser`）
- [x] **用户与复习数据绑定**：`word_reviews` / `review_logs` 带 `user_id`（P0-1，2026-10；老库跑 `go run ./cmd/migrate -apply`）
- [ ] 每日配额从 localStorage 迁到服务端（上线计划里的 P2）
- [ ] 改密 / 找回密码 / 强制改密 / 删除账号
- [ ] 多方式登录：`user_identities` 已就位，加一个 provider 即可（手机号、GitHub OAuth…）
- [ ] 跨实例限流（Redis）与多实例部署
