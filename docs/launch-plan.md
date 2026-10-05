# 广学 · 上线计划（实验性邀请制 · 仅英语模块）

> **状态**：待你确认（2026-10 起草）
> **决策来源**：2026-10-01 两轮问答，已定案的 12 条见 [附录 A](#附录-a已定案的决策)
> **已知环境事实**（来自 `README.md:754-835` 的「部署说明」，本文不重复造一遍）：
> - 线上域名 **`test.lovezmx.com`**（HTTPS），Nginx 反代；服务器目录 `/var/www/test.lovezmx.com/`（`frontend/` + `backend-go/` + `nginx.conf`）；
> - 两个后端的生产启动命令、Nginx 配置示例、环境变量清单都已在 `README.md` 里写过；
> - **写作假设**（若与实际不符，只需改第 9 节，不影响其他部分）：服务器是 Linux（x86_64）、systemd 托管进程、certbot 签证书；
> - 只有 **英语** 一个学科上线，其他学科暂不开放。
>
> 读完本文你只需要记住一件事：**第 2 节那五件事（P0）没做完，不要放人进来。**

---

## 0. 一页结论

| # | 要做的事 | 为什么必须做 | 预估工作量 |
|---|---------|------------|-----------|
| P0-1 | **复习进度按人隔离** ✅ 已完成（2026-10） | 现在 `word_reviews` 的唯一索引是 `WordID`，**两个用户共用一份进度会互相覆盖**——这是数据正确性问题，不是体验问题 | 1~2 天（含测试） |
| P0-2 | **强制邀请码注册** ✅ 服务端已完成（2026-10） | 你选的「暂时邀请制」；现在不填邀请码也能注册 | 半天 |
| P0-3 | **打通真发邮件** 🟡 代码侧已完成（2026-10），差发件邮箱的授权码 | 现在 `AUTH_MAIL_MODE=log`，验证码只打进日志，**真实用户收不到码就注册不了** | 半天（SMTP 代码早已写好；配置校验、自检与验收命令已补齐） |
| P0-4 | **公网部署** 🟡 代码侧与部署产物已完成（2026-10），差一台服务器 | HTTPS、Cookie Secure、Nginx 分流、生产环境开关、每日备份 | 1 天（照着 [`deploy-runbook.md`](deploy-runbook.md) 敲） |
| P0-5 | **三级角色（超管/管理员/用户）** ✅ 已完成（2026-10） | 你要的「超管发带等级的码」；改造前只有 `user`/`admin`，且**带邀请码注册会直接变管理员** | 1~2 天 |
| P1 | 管理面板四个模块（邀请码/用户/审计/看板） | ✅ **已完成（2026-10-05）**：四个面板挂在**个人中心**、按角色显示；接口契约与四批交付见 [5.0](#50-p1-施工计划2026-10-定案)，实测与交接见 [附录 C](#附录-c进度记录) | 四批（批 3 最重） |
| P2 | 每日配额服务端化、限流调参、监控 | ✅ **已完成（2026-10-05）**：配额其实早在单一循环池改造时就服务端化了（本轮补测试与口径）；限流已可配并按「严在账号、宽在 IP」重算、Nginx 补 `limit_req` 且修掉 XFF 伪造；新增深度健康检查 + `guangxue-monitor` 邮件告警。见 [14](#14-p2配额口径限流与监控2026-10-定案-已完成) | 1 天 |

**推荐顺序**：P0-1 → P0-5 → P0-2 → P0-3 → P0-4 → 放人 → P1 → P2。
理由：P0-1 是数据正确性（越早做，废数据越少）；P0-5/P0-2 是同一批代码（注册流程），一起改一次测一次；P0-3/P0-4 是外部依赖（等授权码/等证书），可以和开发并行推进。

**当前状态（2026-10-05）**：P0-1 / P0-2 / P0-5 / **P1** / **P2** 都已完成并过了测试
（三套测试 + 跨服务脚本 55 项 + 一键 `scripts/verify.ps1` 六步全绿）。
剩下的两步都卡在外部依赖：**P0-3** 只差发件邮箱的 SMTP 授权码（代码、自检与验收命令都齐了），
**P0-4** 只差一台服务器（照着 [`deploy-runbook.md`](deploy-runbook.md) 敲，含监控与告警那一节）。
**放人之前**：先把这两步收尾，再按 runbook「上线前的最后一份清单」逐条打勾。

---

## 1. 本期范围

**做**：
- 仅 **英语** 学科的复习（词库、复习调度、统计）；
- **邀请制注册**：没有有效邀请码，连邮箱验证码都发不出来；
- **三级权限**：超级管理员 / 管理员 / 普通用户；
- **管理面板**：邀请码管理、用户管理、审计日志、数据看板、词条管理入口；
- **公网部署**：HTTPS + Nginx + 进程守护 + 每日备份。

**不做**（明确推迟，避免范围蔓延）：
- 多学科（`main.js:38` 的学科分发改造）；
- 英语**多词书**（roadmap 第 4 节，等有内容产能再说）；
- 社交/分享/排行榜；
- 付费；
- 手机 App / 小程序。

**数据态度**（你已确认「实验性上线，未来删档重来」）：
- 现有的 `word_reviews` / `review_logs` 是本地开发数据，**P0-1 改造时直接清空，不写迁移脚本**；
- `auth.db` 里的账号（含管理员）**保留**；
- 上线后若真要删档重来，只需要清空 Go 侧的三张业务表 + 保留 `auth.db`。

---

## 2. 阻塞项（P0）：不做完不能上线

### P0-1 复习进度按人隔离

> **✅ 已完成（2026-10）**：实现清单与实测结果见文末 [附录 C](#附录-c进度记录)；
> `backend-go/handlers/review_isolation_test.go` 7 项、`scripts/verify-auth.ps1` **22 项**全 PASS
> （P0-1 时是 18 项，P0-2 又加了 4 项）。

**现状（P0-1 之前，留档以免重复踩）**：

| 位置 | 现状 | 问题 |
|---|---|---|
| `backend-go/models/models.go:114` | `WordID uint \`gorm:"uniqueIndex"\`` | 一个单词**全局只有一行**复习记录 |
| `backend-go/models/models.go:134` | `ReviewLog.WordID` 有索引、**没有 `user_id`** | 日志分不清是谁复习的 |
| handlers 全部查询 | 没有 `WHERE user_id = ?` | A 用户复习完，B 用户看到的是 A 的进度 |

结果：**第 2 个人一开始用，两个人的进度就会互相覆盖**——A 把词标成「已掌握」，B 那边也跟着变；B 再复习一次，A 的到期时间又被改掉。这不是「体验差」，是数据被写坏。

**改造方案（已定案，2026-10）**：

目标结构（索引名由 GORM 标签固定，实测旧库现状见下）：

```
word_reviews: 加 user_id 列 + UNIQUE(user_id, word_id) idx_word_reviews_user_word
                            + INDEX(user_id, due_at)    idx_word_reviews_user_due
review_logs:  加 user_id 列 + INDEX(user_id, reviewed_at) idx_review_logs_user_time
删除：idx_word_reviews_word_id（旧的 UNIQUE(word_id)，就是它挡住第二个用户）
      idx_word_reviews_due_at （已被复合索引取代，留着只会让每次写入多维护一个索引）
```

> 实测证据（`backend-go/guangxue.db`）：`idx_word_reviews_word_id` 确实是 `CREATE UNIQUE INDEX ... ON word_reviews(word_id)`；
> 现有数据 100 条进度 / 241 条日志 / 100 个词条。

| 决策点 | 定案 |
|---|---|
| 身份来源 | **只从 `gx_access` Cookie**（`middleware.CurrentUserID`）；**请求体不加 `user_id`**，前端无法替别人写进度 |
| 旧数据（100 条进度 / 241 条日志） | **清理**：迁移时删掉 `user_id = 0` 的历史行，不归属给任何人 |
| 词库 | **共享**，`Word` 表不加 `user_id`；`total_words` 统计保持全局 |
| 删词条语义 | **保持**「连带删掉所有人对该词的进度」；「软删除 / 只下架内容不动进度」**暂不考虑**（记在此处，避免以后重复讨论） |
| 浏览器 localStorage 配额 | P0-1 **不动**（今日 5 新词、抽查冷却仍按浏览器存），随 P2 配额服务端化一并解决 |
| 迁移怎么触发 | **显式命令**：`go run ./cmd/migrate`（默认只报告，dry-run），`go run ./cmd/migrate -apply` 先自动快照再执行；**启动时只做自检**——发现旧结构就直接拒绝启动并提示该跑哪条命令（避免「忘了跑」变成第二个用户复习时炸 UNIQUE 约束） |

**代码侧**（改动全在 Go 侧，前端与 WASM 引擎一行不用改）：

- `backend-go/models/models.go:112-145`：`WordReview` / `ReviewLog` 加 `UserID`（`not null;default:0` + 索引标签；SQLite 的 `ADD COLUMN` 加 `NOT NULL` 列必须带默认值）；
- `backend-go/handlers/review_handlers.go`：7 个函数、16 处查询/赋值按 user 收口 —— `DueReviews:424-435`、`NewWords:455-460`（**join 条件里就要带 user_id**）、`learnedQuery:472-483`、`QueueReviews:509` 计数、`SubmitReview:595/606/629`、`ReviewStats:665-700` 九条统计、`calcStreakDays:729`；`DeleteWord:307-318` 保持全用户级联；
- `backend-go/middleware/auth.go`：新增 `CurrentUserID(c) (uint, bool)`；handlers 侧加一个兜底助手（取不到 id 就 401，**绝不写 `user_id = 0`**）；
- `backend-go/database/database.go`：启动自检（只读检查，不改结构）；
- 新增 `backend-go/cmd/migrate`。

**验收标准**：

1. 新增 `backend-go/handlers/review_isolation_test.go`：两用户复习同一个词 → `word_reviews` **两行**、`due_at` 互不影响；A 的 stats 不含 B；B 的 `/reviews/new` 仍把该词当新词；A 的 `/reviews/queue` 不含 B 的词；删词条后两人的行都没了；
2. `pwsh scripts/verify.ps1` 全绿；
3. `scripts/verify-auth.ps1` 加两用户隔离检查：两人各 submit 同一个词后，各自的 `reviewed_words` 都是 **1**（而不是 2）。

---

### P0-2 强制邀请码注册

> **✅ 服务端已完成（2026-10）**：开关、发码前的强校验、两道门的测试与端到端检查都做完，
> 详见文末 [附录 C](#-p0-2-强制邀请码注册已完成2026-10)。
> **刻意没做的两件事**（都不是遗漏）：
> ① 前端邀请码框**没**改成必填（你选的「先不动前端」——服务端才是唯一安全边界）；
> ② 「按 `grant_role` 赋角色」仍留在 **P0-5**（那一列由 P0-5 的 `0002_launch.sql` 引入）。
> 也就是说：**现在把 `AUTH_REQUIRE_INVITE=true` 打上去，邀请制就已经生效**，
> 只是「带码注册即管理员」这个老行为要等 P0-5 才会被分级角色取代。

**现状**：`backend-rust/src/core/invite.rs:8-10` 明确写着「邀请码**不再是注册门槛**」，`POST /api/auth/email-code` 不带 `invite_code` 也能发码（`service.rs:199` 起），注册时邀请码是可选的（`service.rs:285`）。

**改造**：

- 新增配置开关 **`AUTH_REQUIRE_INVITE`**（默认 `false` 保持开发方便；**生产置 `true`**）。
- 开关打开时：
  - `POST /api/auth/email-code`：`invite_code` **必填**，校验不通过直接不下发验证码（错误沿用现有的 `invalid_invite` / `invite_expired` / `invite_exhausted`）；
  - `POST /api/auth/register`：`invite_code` **必填**，注册成功时按邀请码上的 `grant_role` 赋角色（见 P0-5）。
    > ⚠️ **实际只做了「必填」**：`grant_role` 那一列随 P0-5 的 `0002_launch.sql` 一起加，
    > 现在带码注册仍然走老逻辑（`service.rs` 的 `let role = if redeemed.is_empty() { "user" } else { "admin" }`）。
    > 不要以为 P0-2 做完就有三级角色了 —— 那是 P0-5。
- 前端 `account/account.js`：邀请码输入框改成**必填**（`regInvite`），文案改成「没有邀请码？暂时无法注册」；`:657-658` 那条「不要把空格规范化」的注释仍然有效，别动。
  > ⚠️ **实际未做**（2026-10 用户选择「先不动前端」）：输入框保持可选。原因是**服务端才是安全边界**——
  > 强制模式下不填码根本发不出验证码、注册也会被 400 挡住，前端标不标必填不影响安全性，
  > 只影响「用户提不提前知道要填码」。✅ **2026-10-05 已随 P1 一起改完**：注册表单会先问公开开关
  > `GET /api/auth/config`，强制邀请制下把标签改成「必填」、写清「没有码就注册不了」，并在发码/注册前先本地拦一次。
- **为什么用开关而不是写死**：你说过未来会「删档重来，注册不强制邀请码，填了才带权限」——那时把开关关掉即可，代码不用再改一遍。

**验收标准**：
- 不填邀请码请求 `email-code` → 400 且**数据库里没有新增 `email_codes` 行**（这是关键：不能「先发码后校验」）；
- 填过期码 / 已用尽的码 → 分别返回 `invite_expired` / `invite_exhausted`；
- 填有效码 → 正常收到验证码。

---

### P0-3 打通真发邮件

> **🟡 代码侧已完成（2026-10），就差一个授权码**：配置解析、TLS 模式、启动期自检、
> 验收命令与测试都补齐了；**唯一没做的是真发一封到收件箱** —— 那需要发件邮箱的 SMTP
> 授权码（本机上没人能替你做）。拿到之后按下面第 3 步填进 `.env`，跑一条命令即可验收。
>
> 已完成的代码侧改动（三条都是「配错了也看不出来」那类问题）：
> | 改动 | 为什么 |
> |---|---|
> | `SmtpTls` 枚举 + 未知值报错 | 原来 `AUTH_SMTP_TLS` 是字符串，非 `starttls/none` 的值**一律静默当隐式 TLS**。本意若是 587 + starttls，表现是「连接超时」，根本查不出配错了。现在 `tls`/`implicit`/`starttls`/`none` 显式收，其余启动即报错并列出可选项 |
> | `AUTH_SMTP_FROM` 启动期校验（`SmtpConfig::from_mailbox`） | `lettre` 要到**发第一封信**时才解析发件人，配错的表现是「服务正常启动、用户点发送才失败」。现在加载配置时就拦住 |
> | `mail::preflight` + 启动横幅一行自检 | 只连 TCP、不登录不发信，能查出主机/端口写错、防火墙、DNS 问题。⚠️ **失败只告警不阻止启动** —— SMTP 挂了不该让已登录用户也用不了站点 |
> | `cargo run --bin mail-test -- 邮箱` | 一条命令真发一封测试邮件，读的是与账号服务同一份配置；报错会把**具体 SMTP 错误**（认证失败 535 / 连不上）打到终端 |
> | `Config::from_lookup`（可注入的环境读取） | 让配置解析能写单测：`std::env::set_var` 在 Rust 2024 起是 unsafe 的，测试里塞一张表进来即可。P0-3 新增 12 条用例 |
>
> 实测（不带凭据）：`smtp.qq.com:465` 出网可达；用**故意写错的密码**完整走一遍，
> TLS 握手成功、服务器返回 `535 Login fail`，错误被正确翻译成「授权码不对」的提示。
>
> ⚠️ 踩到一个隐蔽的坑，**写配置时会遇到**：`AUTH_SMTP_FROM=广学 <no-reply@qq.com>`
> **不加引号**时 `dotenvy` 会把整行丢掉**且不报错**，发件人静默退化成登录账号。
> 带空格 / 尖括号的值**必须加双引号**：`AUTH_SMTP_FROM="广学 <no-reply@qq.com>"`
>（`config.rs` 里有一条用例把这条行为钉住了）。

**好消息**：SMTP 发信**代码早就写完了**（`backend-rust/src/mail/smtp.rs`，用 `lettre`），缺的只是配置：

1. 选一个发件通道（见 [第 10 节](#10-邮件通道选型)）；
2. 拿到 **SMTP 主机 / 端口 / 账号 / 授权码 / 发件人**；
3. 在服务器 `.env` 里配上（⚠️ 带空格或尖括号的值要加引号）：

```env
AUTH_MAIL_MODE=smtp
AUTH_SMTP_HOST=smtp.example.com
AUTH_SMTP_PORT=465
AUTH_SMTP_USERNAME=no-reply@test.lovezmx.com
AUTH_SMTP_PASSWORD=授权码
AUTH_SMTP_FROM="广学 <no-reply@test.lovezmx.com>"
AUTH_SMTP_TLS=implicit      # 465 用 implicit（= tls）；587 用 starttls；25 用 none（不填默认 implicit）
```

3b. **发一封验收**（不用等用户注册）：

```bash
cd backend-rust
cargo run --bin mail-test -- 你的邮箱@example.com
```

4. **做域名邮件的 SPF / DKIM / DMARC 解析记录**——不做这一步，验证码大概率进垃圾箱，用户收不到就注册不了（这是上线最常见的翻车点）；
5. 真机验收：用一个**非管理员、非本机邮箱**（比如 Gmail / QQ 邮箱）走完整注册流程。

**验收标准**：注册一封真实邮箱 → 1 分钟内收到中文验证码邮件 → 验证码可完成注册。**并在服务器上确认 `AUTH_DEV_ENDPOINTS` 是关闭的**（生产强制关闭，但要亲眼确认）。

---

### P0-4 公网部署

> **🟡 本机能做的都做完了（2026-10），剩下的必须在服务器上做**：完整的十步流程、
> 每步的验证命令、以及「本机做不到的部分由谁补」现在都在
> **[部署手册 `deploy-runbook.md`](deploy-runbook.md)** 里，照着敲即可。
>
> 已随本次改动落地的产物：
> | 文件 | 作用 |
> |---|---|
> | `deploy/nginx/guangxue.conf.template` | 站点配置模板（HTTP→HTTPS 跳转、`/api/auth/`→8081、`/api/`→8080、静态托管、HSTS、敏感文件兜底拒绝、缓存策略） |
> | `deploy/systemd/guangxue-auth.service` / `guangxue-api.service` | 两个后端进程守护（`Restart=always` + systemd 沙箱加固 + `EnvironmentFile` 注入密钥） |
> | `deploy/systemd/guangxue-backup.service` / `.timer` | 每天 03:00 用 `backend-go/cmd/backup`（`VACUUM INTO`）快照两个库，保留 30 份 |
> | `docs/deploy-runbook.md` | 十一步部署手册（含恢复演练、**监控与告警**与上线前清单） |
>
> 同时修掉的代码问题：
> - **`gin.ReleaseMode`（`routes.go`）**：原先生产也跑在 debug 模式，会打印路由表、每个请求一行
>   `[GIN]`，日志量翻好几倍且会进 journald 长期留着。口径与 Rust 侧 `is_production` 一致：
>   **只有小写 `production` 算生产**（大小写写错一律当开发，免得「本机调试日志突然消失」无从解释）。
> - **Go 侧启动告警（`config.StartupWarnings`）**：把「配错但不至于起不来」的三项点出来 ——
>   密钥为空（所有要登录的接口静默 401）、生产却监听 `0.0.0.0`（绕过 HTTPS 与限流）、
>   白名单还带着本机地址（**所有写请求 403**）。
> - **测试**：`backend-go/routes/routes_test.go` 新增 `ginMode`、生产模式仍能服务、以及
>   **路径前缀边界**用例（`/api/auth/*` 绝不能由 Go 处理 —— 反代配错时就是它最先红）；
>   `backend-go/config/config_test.go` 新增启动告警用例；`backend-rust/src/config.rs`
>   新增 6 条生产开关用例（强制 Cookie Secure、拒绝 dev 接口、拒绝默认管理员口令、
>   必须显式密钥、种子账号不默认开）。
>
> ⚠️ **两处 `.env` 的白名单与密钥都必须改**（`AUTH_ALLOWED_ORIGINS` / `AUTH_JWT_SECRET`）：
> 只改一个的现象分别是「能登录但一提交就 403」与「登录成功但复习接口一直 401」。
> 手册第 3 步把这两条写在显眼处。

**目标架构**：

```
                  ┌─ /              → 静态文件（仓库根的 html/js/css，Nginx 直接托管）
浏览器 ──HTTPS──→ │
         Nginx    ├─ /api/auth/*    → 127.0.0.1:8081   Rust 账号服务
         (443)    └─ /api/*         → 127.0.0.1:8080   Go 主后端
```

**要做的事**：

| 项 | 要点 |
|---|---|
| Nginx 反代 | ⚠️ `proxy_pass` 末尾**不要带 `/`**，否则 `/api/xxx` 会被改写成 `/xxx` 而 404（这是仓库 README 里记过的坑）；`/api/auth/` 必须**先于** `/api/` 匹配 |
| HTTPS | certbot 自动续期；开启 HSTS |
| Cookie | 生产 `APP_ENV=production` → `AUTH_COOKIE_SECURE` 自动为 true（本地 http 会因为 Secure 收不到 Cookie，别在本地开） |
| **CSRF 白名单** | `AUTH_ALLOWED_ORIGINS` 改成 `https://test.lovezmx.com`——**漏配的后果是所有写请求 403**（它同时是 CSRF 白名单和 CORS 依据），两处都要改：`backend-rust/.env` 与 `backend-go/.env` 的 `AUTH_ALLOWED_ORIGINS` |
| 生产开关 | `APP_ENV=production`：dev 接口强制关闭、默认管理员密码拒绝启动、Cookie 加 Secure（`backend-rust/src/config.rs` 已实现） |
| Gin 模式 | ⚠️ `backend-go/routes/routes.go:11` 现在**没按 `APP_ENV` 切 `gin.ReleaseMode`**，生产会一直刷调试日志——上线前补上 |
| 监听地址 | 两个后端都改成 `127.0.0.1`（只让 Nginx 访问，别暴露 8080/8081） |
| 进程守护 | 两个 systemd 服务（`guangxue-auth.service` / `guangxue-api.service`），`Restart=always` |
| 静态托管 | 生产**不再用 `dev-server.js`**（它是开发用的代理）；Nginx `root` 指向仓库目录，或 `rsync` 一份到 `/var/www/guangxue` |
| **每日备份** | ⚠️ `scripts/backup.ps1` 是 **PowerShell（Windows 专用）**，服务器上跑不了。改用 **`backend-go/cmd/backup` 编译出的 Linux 二进制**（它是跨平台的，走 `VACUUM INTO`），配 systemd timer 每天 03:00 跑，产物留在**另一块盘或对象存储**——不要和数据库放同一个目录 |
| 日志 | Nginx access/error log + 两个服务的 journald；确认日志里**不出现**验证码明文与密码 |
| 限流复核 | `AUTH_RL_*` 的默认值是按本机调优的，上线前按「上百人」重算一遍（见 P2） |

**验收标准**：
- `https://test.lovezmx.com/api/health` 与 `/api/auth/health` 都 200；
- 浏览器打开站点 → 注册 → 登录 → 复习 → 统计，全链路通；
- 刷新页面后登录态还在（Cookie Secure + SameSite 配置正确）；
- 手动执行一次备份，确认产物非空、`integrity_check` 为 ok；
- 重启服务器后两个服务自动起来。

---

### P0-5 三级角色与邀请码分级

> **✅ 已完成（2026-10）**：13 条决策、代码清单与实测结果见文末 [附录 C](#-p0-5-三级角色与邀请码分级已完成2026-10)。
> 一句话结果：**角色字符串 `user` / `admin` / `super_admin` 在 Rust、Go、前端三处同名同义**，
> 注册时的角色**只由邀请码上的 `grant_role` 决定**（默认 `user`），
> 那条「带码注册即管理员」的权限漏洞已经堵上。
>
> ✅ **邀请码管理面板已在 P1 补上（2026-10-05）**：发码 / 一次性明文 / 复制全部 / 导出 CSV / 邀请链接 /
> 整批发邮件 / 状态筛选分页 / 单张停用 / **按批停用** 都在个人中心的「邀请码」面板里（见 [5.0](#50-p1-施工计划2026-10-定案) 与
> [附录 C](#附录-c进度记录)）。**CLI 仍然可用**，而且批量脚本化时它比界面方便。

**现状（改造前，留档以免重复踩）**：只有 `user` / `admin` 两种字符串；`users.role` 是 TEXT 字段（`migrations/0001_init.sql:17`，注释里写了「预留：user / admin / ...」），所以**加角色不需要改表结构**。

**要改的地方**（⚠️ **角色字符串是跨服务契约**，Rust 与 Go 两边都要改，漏一边就会出现「后台能进、接口 403」这类灵异现象）：

| 位置 | 改造前 | 现在（已完成） |
|---|---|---|
| `backend-rust/src/http/extract.rs` | 只认 `role == "admin"` | 判定收口到 `models::can_enter_admin`：认 `admin` **和** `super_admin`；新增 `SuperAdminUser` 提取器（更窄，只认超管） |
| `backend-go/middleware/auth.go` | `RoleAdmin = "admin"` | 加 `RoleSuperAdmin = "super_admin"`；`RequireAdmin` 走 `CanEnterAdmin`（放行两者），新增 `RequireSuperAdmin` |
| `backend-rust/src/service.rs` 注册那一行 | `let role = if redeemed.is_empty() { "user" } else { "admin" }` | **按邀请码的 `grant_role` 赋值**（默认 `user`）——这条权限漏洞已堵上 |
| `admin/admin.js` 的门禁 | 要求 `role === 'admin'` | `canEnterAdmin(role)` 放行 `admin` 与 `super_admin`（`account.js`、`main.js` 里同样的判定也一并改了） |
| 角色常量 | 散落的字符串字面量 | 收口在 `models.rs` 的 `ROLE_USER` / `ROLE_ADMIN` / `ROLE_SUPER_ADMIN`，**两侧各有一条测试钉住字面量**（改错时测试红，而不是线上 403） |

**邀请码分级**：`invite_codes` 加了两列 —— `grant_role`（`user` / `admin`）与 `batch_id`（一次生成的一批共享，便于按批回收与统计），见 `backend-rust/migrations/0002_launch.sql`。

**给现有管理员升级为超管**：**不需要手写 SQL 了**。启动时自动把 `AUTH_ADMIN_EMAIL` 那个账号**确保为超管**（不存在就建、是 admin 就提权、已经是超管就不动），另有 `cargo run --bin seed-admin -- --role super_admin` 供以后给**别人**授权。
⚠️ 刻意**不做自动降级**：换了环境变量里的邮箱时，旧超管仍然是超管 —— 自动降权会把上一个超管悄悄锁死，而锁死超管无法自救。

---

## 3. 角色与权限矩阵

| 能力 | 普通用户 `user` | 管理员 `admin` | 超级管理员 `super_admin` |
|---|:---:|:---:|:---:|
| 复习 / 看词库 / 看自己的统计 | ✅ | ✅ | ✅ |
| 进 `/admin/` 后台 | ❌ | ✅ | ✅ |
| **词条**增删改查（内容管理） | ❌ | ✅ | ✅ |
| **数据看板**（今日新增、活跃、复习量） | ❌ | ✅ | ✅ |
| **用户列表** / 搜索 / 看某人复习进度 | ❌ | ✅（只读） | ✅ |
| 发**邀请码** | ❌ | ❌ | ✅ |
| 发**管理员码** | ❌ | ❌ | ✅ |
| 看/停用邀请码列表 | ❌ | ❌ | ✅ |
| 改别人的角色（提升/降级） | ❌ | ❌ | ✅ |
| 封禁 / 解封账号 | ❌ | ❌ | ✅ |
| 看审计日志 | ❌ | ❌ | ✅ |

**设计取舍**（你选了「只有超管能发码」+「管理员身份两条路都要」）：

- **管理员 = 内容/运营角色**：管词条、看数据、看用户，但**碰不到权限**；
- **超管 = 治理角色**：发码、调权限、封号、查审计；
- 管理员身份**两条产生路径**都要：
  1. **后台直接提升**——超管在用户列表里把某个已注册用户改成 `admin`（适合熟人）；
  2. **管理员码**——超管发一张 `grant_role='admin'` 的邀请码，对方注册即管理员（适合新人）。
- ⚠️ 目前唯一的超管应该是**你自己**。等有第二个可信的人，再考虑「超管能否封超管」这类问题（本期不设计，避免自锁）。

---

## 4. 邀请码体系

**已经有的**（不用重做）：`invite_codes` 表已有 `code / note / max_uses / used_count / expires_at / disabled / created_by / created_at`，**「可用几次」你现在就能用**；`invite_uses` 记录每一笔兑换（谁、哪个邮箱、什么 IP、什么时候）；状态机在 `backend-rust/src/core/invite.rs:98`（停用 > 过期 > 用尽 > 可用，顺序固定）。

**要加的**：

```sql
-- backend-rust/migrations/0002_launch.sql（在 db.rs 的 MIGRATIONS 里登记为版本 2）
ALTER TABLE invite_codes ADD COLUMN grant_role TEXT NOT NULL DEFAULT 'user';  -- user / admin
ALTER TABLE invite_codes ADD COLUMN batch_id   TEXT NOT NULL DEFAULT '';      -- 一次生成的一批，便于回收与统计
```

`db.rs:28` 是迁移表：`pub const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];`——加一行 `(2, include_str!("../migrations/0002_launch.sql"))` 即可，`PRAGMA user_version` 会自动跳过已应用的版本。

**码的格式**（`core/invite.rs:12-18` 已经预留了这个约定）：

| 类型 | 格式 | 谁能发 | 兑换结果 |
|---|---|---|---|
| 普通邀请码 | 16 位（字符表 **A-Z 与 0-9**，如 `7K3M9QRT2XWZ5BHD`） | 超管 | `role = 'user'` |
| 管理员码 | `ADMIN-` + 16 位（如 `ADMIN-7K3M9QRT2XWZ5BHD`） | 超管 | `role = 'admin'` |
| 自定义码 | 超管自己定的 16 位（同一套字符表） | 超管 | 按 `grant_role` |

> ⚠️ 2026-10-05 用户定案：**邀请码只要 A-Z 与 0-9**。此前随机生成用的是 Crockford Base32（排除 I/L/O/U 这套易混字符），已改掉 —— 生成与手填现在共用同一张 36 字符表。改表**不影响已发出的码**（兑换一律查库），老库里 Crockford 时代的码照样有效。

> ⚠️ 前缀是**给人看的**（方便你一眼分出手上这张码的分量），**不是安全边界**——真正的判定永远是查库读 `grant_role`。规范化时**绝不能抹掉 `-`**（`core/invite.rs:37` 有明确警告）。

**兑换流程**（改造后）：

```
用户填码 → 查库 → evaluate(停用/过期/用尽/可用) → 发行邮箱验证码
       → 邮箱验证码校验 → 占额度(consume_invite) → 建用户(role = grant_role)
       → 写 invite_uses 留痕 → 写 audit_logs(invite_use) → 注册即登录
```

**超管发码时能设的参数**：张数（1~100）、每张可用次数、有效期（天）、备注、**等级**（普通/管理员）。

**这个命令怎么发码**：

```powershell
cd backend-rust
# 普通邀请码（对方注册即普通用户）：默认 7 天有效、每张用 1 次
cargo run --release --bin invite -- create --count 5 --note "第一批"

# 管理员码（对方注册即管理员）：带 ADMIN- 前缀，一眼能看出分量
cargo run --release --bin invite -- create --count 1 --grant-role admin --note "给运营"

# 看列表（带等级列）/ 停用某张
cargo run --release --bin invite -- list --status unused
cargo run --release --bin invite -- disable 12
```

打印出来的码是**分组显示**（`7K3M-9QRT-2XWZ-5BHD`）方便你自己核对，**发给对方的仍然是紧凑形式**（两种写法查库时都认）。

---

## 5. 管理面板设计

### 5.0 P1 施工计划（2026-10 定案）

> **状态：四批全部完成（2026-10-05）** —— 代码清单、实测数字与交接事项见 [附录 C](#附录-c进度记录) 的 P1 段。
> 唯一没做的：**在真浏览器里人工点一遍**（面板已过 API 端到端 45 项 + 静态检查 + 桩 DOM 46 项断言三层验证）。

**定案（本轮与用户逐条敲定，取代本节原先的待确认项）**

| 问题 | 定案 |
|---|---|
| 界面放哪 | **全部搬进个人中心**（`account/`）——用户中心里直接就是管理界面，不取「入口卡片 + `/admin/`」的折中。`/admin/` 保留学科后台（英语词条管理）；代价是个人中心要补一套表格/徽标/分页样式（见批 3） |
| 看板跨库取数 | **Go 侧聚合，且求「请求尽量少」**：Go **只读**打开 `auth.db`（用户 / 邀请码漏斗）+ 自己的 `guangxue.db`（复习量 / 活跃 / 新学），**看板 1 个请求出全部数字**；用户列表用 **2 个并行请求**（Rust 出用户表——治理数据的唯一来源；Go 出当页批量进度），刻意不让 Go 也读用户表，否则会多出第二份「谁是超管」的判定 |
| 邀请码必填怎么告知前端 | 新增**公开**接口 `GET /api/auth/config` 返回 `{require_invite}`，注册表单据此标必填并写「没有邀请码？暂时无法注册」 |
| 审计保留期 | **180 天**（`AUTH_AUDIT_RETENTION_DAYS` 可配），启动时清理一次 + 每 24 小时清理一次 |
| 整批发邮件 | 代码本轮做完（含 log 模式验收），**真发等 SMTP 授权码**：拿到后只需改 `.env` 再跑一遍 `mail-test` |
| 顺带项 | 修 `admin/index.html` 那条过时的安全横幅 ✅、管理员看用户列表时**服务端脱敏邮箱** ✅、审计保留期 ✅ |

**四批交付（每批独立验收）**

1. **Rust 接口**：`GET /admin/audit`（按 action / 时间 / 分页筛选，同响应带 `actions` 去重列表，省掉前端第二个请求）、`POST /admin/invite-batches/{batch_id}/disable`、`POST /admin/invite-mail`、`POST /admin/users/{id}/logout-all`、邀请码列表附带兑换记录（谁用了 / 邮箱）、用户列表加关键词搜索 + 脱敏 + 放开给管理员只读、审计清理任务、`GET /api/auth/config`。
   > ⚠️ **路径与计划草案不同**：按批停用与整批发邮件**没有**写成 `invites/batch/...` 与 `invites/email` —— 那样静态段会和 `invites/{id}` 落在同一层，症状是一类解释不清的 404。最终用 `invite-batches/{batch_id}/disable` 与 `invite-mail`。
2. **Go 接口**：`AUTH_DB_PATH` + 只读打开 `auth.db`（**库缺失 / 读不到时降级**，不让看板整页 500）、`/api/admin/stats/overview`、`/api/admin/stats/trend?days=`、`/api/admin/users/progress?ids=`、`/api/admin/users/{id}/progress`；`/api/admin/*` 挂 `RequireAdmin`（放行 `admin` 与 `super_admin`）；`deploy/systemd/guangxue-api.service` 加 `ReadOnlyPaths=/opt/guangxue/backend-rust`（把「只读」从自觉变成 systemd 层面的**显式保证**）。
   > ⚠️ 更正一条我先前的说法：这行**不是**读权限的来源（`ProtectSystem=strict` 本来就允许读那个目录），
   > 线上看板账号侧**全是 0 的真正原因是 `AUTH_DB_PATH` 配错或账号服务没起来** —— 默认值 `../backend-rust/auth.db` 只在 `WorkingDirectory` 对得上时才成立，
   > 所以 Go 启动时会检查这个路径并打告警（`StartupWarnings`）。
3. **前端面板**：新增 `admin/panels/{invites,users,audit,dashboard}.js`（导出 `meta` + `mount(container, ctx)`，ES5，与 `modules/english/admin/english-admin.js` 同一套契约，`/admin/` 以后也能挂同一份）+ 共用样式 `admin/panels/panels.css`（前缀 `pn-`，个人中心引它）；`account/account.js` 改 ES module、「管理」导航组按角色渲染、`?invite=` 预填；`account/index.html` 修正注册文案（「填了可升级为管理员」在 P0-5 之后已不成立）。
4. **落档**：本文件附录 C 加 P1 段、[`admin-api.md`](admin-api.md)、[`backend-auth.md`](backend-auth.md)、[`admin/README.md`](../admin/README.md)、根 `README.md`、`backend-go/README.md`、`backend-rust/README.md`、`backend-go/.env.example`，以及 **`scripts/verify-auth.ps1` 补 18 项 P1 检查**。

**口径（先钉死，免得三个服务各算一套）**

- 「今日」一律按**北京时间**切天：`auth.db` 的 `created_at` 是 ISO8601 **UTC** 文本，`guangxue.db` 的 `reviewed_at` 是 `time.Time`，不显式钉时区两个库必然对不上；
- 今日新学 = 当天 `stability_before = 0` 的日志数、今日抽查 = 当天 `is_probe`、今日活跃 = 当天有 `review_logs` 的 distinct `user_id`（沿用既有口径）；
- 邮箱脱敏在**服务端**做（管理员看 `a***@qq.com`，超管看全），不是只做界面；
- 面板**零动效**，沿用现有 CSS 过渡（第 8 节红线不变）。

---

### 5.1 放在哪里（✅ 已定案：全部搬进个人中心，见 5.0）

> 下面这段是当初的取舍分析，留档说明「为什么后来还是搬了」：用户 2026-10 选择**全部搬进用户中心**，
> 于是第 8 节里「新增 `admin/panels/*.js`」的模块位置仍然成立（面板做成壳无关的通用模块），
> 但它们的**入口与样式留在 `account/`**，`/admin/` 只留学科后台。


你的原话是「**在用户中心**为管理员添加一些不一样的功能」。我建议的实现是：

- **用户中心（`account/`）**：给管理员/超管多出一块「**管理**」分区，里面是**入口卡片**（数据看板、邀请码、用户、审计、内容管理），点进去跳 `/admin/#/...`；
- **实际的管理界面仍然放 `/admin/`**（它是独立后台，已有 hash 路由、`ctx` 注入、toast/confirm/el/escapeHtml 工具、独立 CSS）。

**理由**：`admin/` 的骨架（`admin/admin.js:27` 的 `SUBJECT_ADMINS` 注册表 + `:196` 的 `renderNav` + 动态 `import()` 挂载）已经就绪，加面板是「登记一行 + 写一个模块」；而用户中心是**学生视角**的页面（`account/account.js` 623 行，三张卡：账号信息/登录设备/可用操作），把用户表格和邀请码列表塞进去，两边都会变差，而且管理逻辑要写两遍。

如果你坚持「全部都在用户中心」，代价大约是多花 1~2 天做 shell 与样式整合——**建议先按入口卡片做，用一阵子再决定要不要搬**。

### 5.2 四个面板的功能清单

**① 邀请码管理**（超管可见）

- 生成：张数 / 每张次数 / 有效期 / 备注 / 等级 → 生成后**一次性展示**明文，提供「复制全部」与「导出 CSV」；
  ⚠️ 明文只在这次响应里出现（`backend-rust/src/http/admin.rs:70` 的设计），**刷新后拿不回来**——界面上要写清楚这一点；
- 列表：按状态筛选（全部/未用/已用/过期/已停用 → `InviteFilter` 已实现）、分页、显示「谁发的、谁用了、什么时候、用了哪个邮箱」；
- 停用：单张停用（`POST /api/auth/admin/invites/{id}/disable`）；批量停用建议按 `batch_id`（新增能力）。

**② 用户管理**（管理员只读 / 超管可操作）

- 列表：搜索（邮箱/用户名）、注册时间、角色、状态、最后登录、复习进度摘要（词数/连续天数）；
- 超管操作：改角色（user ↔ admin，**不能自我降级**，防自锁）、封禁/解封（`users.status` → `disabled`，配套改登录与鉴权判定）、强制下线（吊销该用户全部会话 → `sessions` 表已有 `revoked_at` 字段，做「一键踢下线」成本很低）。

**③ 审计日志**（超管可见）

- `audit_logs` 表已齐备（`actor_user_id / action / target / ip / user_agent / detail / created_at`），已有 action：`register`、`login`、`invite_create`、`invite_use`、`invite_disable`；
- 界面：按 action 与时间筛选、分页、点开看 detail；
- ⚠️ 遵守既有红线：**密码、验证码、refresh 明文一律不写进审计**（`0001_init.sql:100`）。

**④ 数据看板**（管理员可见）

- 今日：新增用户、活跃用户（有复习记录）、复习次数、新学词数；
- 趋势：最近 7/30 天曲线；
- 邀请码漏斗：发出 N 张 / 兑换 M 张 / 转化率；
- 数据来源：`auth.db`（用户、邀请码）+ `guangxue.db`（复习日志）——**跨库**，需要 Go 后端出聚合接口（Rust 侧读不到业务库）。

**⑤ 内容管理入口**：直接复用现有 `/admin/#/english`（`modules/english/admin/english-admin.js` 608 行），不加新代码，只加入口卡片。

---

## 6. 接口清单（新增 / 改动）

**账号服务（Rust，前缀 `/api/auth`）**

| 方法 | 路径 | 权限 | 说明 |
|---|---|---|---|
| POST | `/email-code` | 公开 | **改**：`AUTH_REQUIRE_INVITE=true` 时邀请码必填 |
| POST | `/register` | 公开 | **改**：邀请码必填；角色按 `grant_role` 赋 |
| POST | `/admin/invites` | **超管** | **改**：请求体加 `grant_role`（user/admin）、`note` 可为批次名 |
| GET | `/admin/invites` | **超管** | 已有，支持 `status` 筛选与分页 |
| POST | `/admin/invites/{id}/disable` | **超管** | 已有 |
| POST | `/admin/invites/batch/{batch_id}/disable` | **超管** | 新增：按批停用 |
| GET | `/admin/users` | 管理员+ | 新增：列表 / 搜索 / 分页 |
| POST | `/admin/users/{id}/role` | **超管** | 新增：改角色（禁止改自己） |
| POST | `/admin/users/{id}/status` | **超管** | 新增：封禁 / 解封 |
| POST | `/admin/users/{id}/logout-all` | **超管** | 新增：吊销该用户全部会话 |
| GET | `/admin/audit` | **超管** | 新增：审计日志查询 |

**主后端（Go，前缀 `/api`）**

| 方法 | 路径 | 权限 | 说明 |
|---|---|---|---|
| GET | `/admin/stats/overview` | 管理员+ | 新增：今日新增/活跃/复习量 |
| GET | `/admin/stats/trend?days=30` | 管理员+ | 新增：趋势曲线 |
| GET | `/admin/users/{id}/progress` | 管理员+ | 新增：某个用户的复习进度摘要 |
| 全部 `/reviews/*` | — | 需登录 | **改**：查询加 `WHERE user_id = ?` |
| 全部 `/words` 写接口 | — | 管理员+ | **改**：`RequireAdmin` 放行 `super_admin` |

> ⚠️ Go 侧的管理员接口要**同时**接受 `admin` 与 `super_admin`；而「封禁」这类治理接口只该在 Rust 侧（账号库在那）。

---

## 7. 数据库变更汇总

| 库 | 表 | 变更 | 时机 |
|---|---|---|---|
| `guangxue.db` | `word_reviews` | 加 `user_id`；唯一键 → `(user_id, word_id)`；旧数据清空 | P0-1 |
| `guangxue.db` | `review_logs` | 加 `user_id` + 索引 `(user_id, reviewed_at)` | P0-1 |
| `auth.db` | `invite_codes` | 加 `grant_role`、`batch_id` | P0-5 |
| `auth.db` | `users` | 不改结构；把你自己那条的 `role` 改成 `super_admin` | P0-5 |
| `auth.db` | — | 迁移文件 `migrations/0002_launch.sql` + `db.rs:28` 登记 | P0-5 |

**关于「每日 5 新词 + 5 抽查」配额**：现在纯在浏览器 `localStorage`（`docs/roadmap.md` 阶段 1 已记录），多设备会翻倍、换浏览器就重置。服务端化不需要新表——「今日新学 = 当天 `stability_before = 0` 的日志数、今日抽查 = 当天 `is_probe` 的日志数」（口径见 `backend-go/handlers/review_handlers.go:690-692`）。**建议放 P2**：配额不准不影响数据正确性。

---

## 8. 前端改动点

| 文件 | 改动 | 风险 |
|---|---|---|
| `account/account.js`（623 行，ES5） | 邀请码必填文案；管理员多一块「管理」入口卡片 | 低 |
| `admin/admin.js`（432 行） | 登录门禁放行 `super_admin`；nav 增加「通用面板」区；新增 4 个面板模块 | 中（`:44` 那条注释警告过 DOMContentLoaded 里必须等 `checkAuth().then(...)` 再 `renderNav/route`） |
| `admin/admin.css`（425 行） | 面板样式（复用现有卡片/表格风格） | 低 |
| 新增 `admin/panels/*.js` | 邀请码 / 用户 / 审计 / 看板四个模块，导出 `mount(container, ctx)` | 中（新代码） |
| `modules/english/english.js` | 无（除非配额服务端化） | — |

> ⚠️ **红线**：仓库 `CLAUDE.md` 明确规定「改动画效果必须先商量」。以上改动**不含任何动效调整**，管理面板沿用现有 CSS 的过渡即可。若你想给面板加入场动画，先跟我说。

---

## 9. 部署与运维

### 9.1 Nginx：已有基线 + 上线前要补的差分

**不要从零写**：`README.md:754-835` 已经有服务器目录结构（`/var/www/test.lovezmx.com/{frontend,backend-go}`）、一份可直接用的
Nginx 配置（`server_name test.lovezmx.com`、`/api/auth/` → 8081、`/api/` → 8080、`try_files $uri $uri/ /index.html`）
与两个服务的生产启动命令。上线前只需要补这几项：

| 要补的 | 说明 |
|---|---|
| `AUTH_ALLOWED_ORIGINS=https://test.lovezmx.com` | **两个 `.env` 都要改**；漏配 = 所有写请求 403（它同时是 CSRF 白名单与 CORS 依据） |
| `AUTH_COOKIE_SECURE=true` | 由 `APP_ENV=production` 自动带上；本地 http 调试时别开，否则 Cookie 收不到 |
| HSTS + certbot 自动续期 | `add_header Strict-Transport-Security "max-age=31536000" always;` |
| `limit_req` | ✅ 已落地（P2）：模板里两条 `limit_req_zone`（`gx_auth` 30r/m、`gx_api` 300r/m，按 `$binary_remote_addr`）+ `limit_conn`，都回 429。**同时修掉 XFF 伪造**：`proxy_set_header X-Forwarded-For $remote_addr`（原来用 `$proxy_add_x_forwarded_for`，客户端伪造的值会留在最前面，而 `client_ip()` 取第一个 → 可绕过全部按 IP 的限流） |
| Gin 生产模式 | ✅ 已落地（P0-4）：`backend-go/routes/routes.go` 的 `ginMode(cfg.Env)` 只在**小写 `production`** 时切 `ReleaseMode`（刻意不把拼错的大小写当生产，否则「本机日志突然消失」会变成查不出来的问题） |
| 监听地址收回环 | ✅ 已落地（P2 文档）：`backend-go/.env.example` 与生产清单都写明 `SERVER_HOST=127.0.0.1`。这不只是洁癖 —— `client_ip()` 信任 `X-Forwarded-For` 的前提就是「外面只有 Nginx」，8080 直接暴露时伪造 XFF 即可绕过按 IP 的限流；Go 启动时对「生产却监听非回环」有告警 |
| 静态文件同步 | 生产不再用 `dev-server.js`：`rsync` 到 `/var/www/test.lovezmx.com/frontend`，或把 `root` 直接指到仓库目录 |
| systemd 两个 unit | README 里只有「怎么启动」，没有守护；补 `guangxue-auth.service` / `guangxue-api.service`（`Restart=always`） |

**README 里已经写明的两条坑，别踩回去**：`proxy_pass` 末尾**不要带 `/`**（否则 `/api/health` 被改写成 `/health` 而 404）；
`/api/auth/` 与 `/api/` 都是前缀匹配、Nginx 取最长者，所以两者的书写顺序不影响结果。

> CSRF 白名单读的是 `Origin` 头，Nginx **不要**改写或删除它（默认透传，别画蛇添足）。

### 9.2 生产环境变量（两个 `.env` 的关键项）

```env
# 公共
APP_ENV=production
AUTH_JWT_SECRET=<64 位随机串，两份文件必须完全一致>
AUTH_ALLOWED_ORIGINS=https://test.lovezmx.com

# backend-rust/.env
AUTH_HOST=127.0.0.1
AUTH_PORT=8081
AUTH_DB_PATH=/var/lib/guangxue/auth.db
AUTH_COOKIE_SECURE=true
AUTH_REQUIRE_INVITE=true
AUTH_MAIL_MODE=smtp
AUTH_SMTP_*=...
AUTH_DEV_ENDPOINTS=false
AUTH_ADMIN_PASSWORD=<改成强密码>
AUTH_AUDIT_RETENTION_DAYS=180
# 限流（P2，可省：不写就用默认值。格式「次数/窗口秒」，0/0 关闭这一条）
AUTH_RL_LOGIN_IP=200/900
AUTH_RL_CODE_IP=200/3600
AUTH_RL_REGISTER_IP=100/3600
AUTH_RL_CODE_EMAIL_MINUTE=1/60
AUTH_RL_CODE_EMAIL_HOUR=5/3600

# backend-go/.env
SERVER_HOST=127.0.0.1
SERVER_PORT=8080
DB_PATH=/var/lib/guangxue/guangxue.db
# ⚠️ 看板要读**同一个** auth.db：两边都叫 AUTH_DB_PATH，但含义不同 ——
#    Rust 那份是「写在哪」，Go 这份是「只读地从哪读」。写错的表现是「看板账号侧全是 0」。
AUTH_DB_PATH=/var/lib/guangxue/auth.db
```

> ⚠️ 改 `AUTH_JWT_SECRET` 会让所有已登录用户掉线——**上线后就别改了**（改的话两边一起改 + 重启两个服务）。
> 监控的 `MONITOR_*`（告警收件人、两个探活地址）写在 `deploy/systemd/guangxue-monitor.service` 里，不进 `.env`；
> 见第 14.3 节与 runbook 的「监控与告警」。

### 9.3 备份（每天 03:00）

- 用 `backend-go/cmd/backup` 编译出的 **Linux 二进制**（`GOOS=linux GOARCH=amd64 go build ./cmd/backup`），走 `VACUUM INTO`，**不用复制 `.db` 文件**——实测磁盘上 4 KB 的 `auth.db` 安全快照出来是 124 KB，直接拷贝丢 97%；
  - ⚠️ **也不要手工 `Copy-Item auth.db`**：这个库开着 WAL，最新变更可能还只在 `auth.db-wal` 里，而 `VACUUM INTO` 会把 WAL 一起并进去。手工拷贝 `.db`（尤其漏掉 `-wal`）拿到的是**旧快照**，恢复时会丢掉最近的用户与邀请码。
  - 这条是实测踩出来的（2026-10-03）：拷完发现 `sqlite` 头的 `user_version` 还是 1、新表结构「不见了」——因为那些变更全在 3.7 MB 的 WAL 里。
- systemd timer 每天跑一次，`-keep 14` 保留两周；
- ⚠️ 备份产物**不要和数据库放同一块盘**（盘坏了两个一起没），建议 `rsync` 到对象存储或另一台机器；
- 每月手动恢复演练一次（把备份文件拷到测试目录起一次服务，确认能登录）。

---

## 10. 邮件通道选型

你还没定，这里是三条路的对比（**默认推荐 A**）：

| 方案 | 成本 | 备案 | 到达率 | 接入工作量 | 适合 |
|---|---|---|---|---|---|
| **A. 阿里云邮件推送 / 腾讯云 SES** | 免费额度通常够（日均几百封），超出约 ¥0.001~0.002/封 | **需域名备案** | 高，有 IP 信誉与退信统计 | 半天（配 SMTP 或 API） | **上百人规模、要正式一点** |
| **B. QQ / 163 邮箱 SMTP** | 免费 | 不需要 | 中低，**容易进垃圾箱**、有每日发信上限、可能被判为可疑登录 | 1 小时（只要授权码） | 实验性上线、先跑通流程 |
| **C. 自建 SMTP（Postfix 等）** | 服务器成本 | 不需要（但要自己配 SPF/DKIM/PTR） | 低，新 IP 基本被拒 | 1~2 天且有持续运维负担 | **不推荐**，除非你要学这个 |

**我的建议**：**先 B 后 A**。
- 现在离上线近，先用 QQ/163 的授权码把「真发信」跑通（代码不用改，只改 `.env`），能立刻放人进来；
- 同时去准备 A 的备案与域名邮箱，等第一波反馈处理完再切过去——切换只是改 6 行 `.env` + 重启，**不改代码**。

⚠️ 无论哪条路，都要配 **SPF**（`v=spf1 include:...`）与 **DKIM**，否则「自己觉得发了、用户在垃圾箱里找不到」。另外建议：**验证码邮件正文里写明「10 分钟内有效」和「不是你操作的请忽略」**，减少误报。

---

## 11. 里程碑

| 里程碑 | 内容 | 产出验收（可执行） |
|---|---|---|
| **M0** | P0-1 进度按人隔离 | 新增多用户隔离测试通过；`pwsh scripts/verify.ps1` 全绿；`scripts/verify-auth.ps1` 加两条互不可见用例 |
| **M1** | P0-5 + P0-2 角色与邀请制 | Rust/Go 两侧角色判定一致（联调脚本覆盖 `super_admin` 发码、`admin` 发码被拒、无码注册被拒） |
| **M2** | P0-3 邮件 | 真实邮箱收到验证码并完成注册（🟡 现在可先用 `cargo run --bin mail-test -- 邮箱` 验收到「服务器已接收」这一步；还差最后一封到收件箱） |
| **M3** | P0-4 部署 | 公网 https 全链路可用；重启自愈；备份产物可恢复（🟡 代码侧与部署产物已就绪，见 [`deploy-runbook.md`](deploy-runbook.md) 第 10 步的上线清单） |
| **M4** | P1 管理面板 ✅ 已完成（2026-10-05） | 超管能在界面上发一批码、提升一个管理员、封一个号、查到对应审计。接口侧已由 `scripts/verify-auth.ps1` 的 18 项 P1 检查覆盖；**界面侧还差你在真浏览器里点一遍**（见附录 C 的交接） |
| **M5** | 放人（10 → 50 → 上百） | 每批观察 3 天：注册转化、复习留存、错误日志、备份是否正常 |
| **M6** | P2 配额服务端化 + 限流调参 + 监控告警 ✅ 已完成（2026-10-05） | 配额其实早在单一循环池改造时就服务端化了（本轮补测试与口径）；限流已可配并重算、Nginx 补 `limit_req`、修掉 XFF 伪造；新增深度健康检查与 `guangxue-monitor` 邮件告警（**告警通道要按 runbook 演练一次**）。见 [14](#14-p2配额口径限流与监控2026-10-定案-已完成) |

**M5 的放人节奏很重要**：先 10 个人（能一天内问遍每一个人的体验），再 50，再放开。上百人一次性涌入，出问题时你连「谁卡在哪一步」都不知道。

---

## 12. 风险与红线

| 风险 | 说明 | 对策 |
|---|---|---|
| 🔴 **进度串号** | 不做 P0-1 就放人 = 用户数据互相写坏 | P0-1 是上线**前置**，不是「以后再说」 |
| 🔴 **带码即管理员** | `service.rs:367` 现在的行为：你发给朋友的每张码都是管理员 | P0-5 必须与 P0-2 一起改，**改完先验证「带普通码注册 = user」** |
| 🟠 **邮件进垃圾箱** | 用户收不到码 = 注册不了 = 邀请码白发了 | SPF/DKIM + 用小号真机测 + 界面上给「收不到？看垃圾箱 / 重发」提示 |
| 🟠 **角色判定跨服务不一致** | Rust 认 `super_admin` 而 Go 不认（或反过来）→ 后台能进、接口 403 | 角色字符串当契约对待：改一处就同时改两处 + 加联调用例 |
| 🟠 **CSRF 白名单漏配** | `AUTH_ALLOWED_ORIGINS` 没改成真实域名 → 所有写操作 403 | 部署检查清单里单列一项 |
| 🟠 **备份不可用** | 备份跑没跑、能不能恢复，不测就不知道 | 每月恢复演练；`backup.ps1` 是 Windows 专用，服务器要用 Go 二进制 |
| 🟡 **无限流被刷** | 公网暴露的注册/发码接口会被脚本扫 | 复核 `AUTH_RL_*`；Nginx 层再加一层 `limit_req` |
| 🟡 **审计日志泄露隐私** | 记了 IP 与邮箱 | 遵守既有红线：不写密码/验证码/refresh 明文；日志保留期限定个规矩 |
| 🟡 **自锁** | 超管把自己降级 / 封禁，没人能救 | 代码层面禁止「改自己的角色」与「封自己」 |

**既有红线（`CLAUDE.md`）仍然有效**，其中与本次上线最相关的三条：
1. **不提交任何密钥**（`.env` 已在 `.gitignore` 里，别手滑 `git add -f`）；
2. **改动画必须先商量**；
3. **提交信息末尾保留 `Committed-by: DeepSeek Harness`**。

---

## 13. 待你确认的问题

1. **域名确认**：继续用 `test.lovezmx.com`（README 里已有配置），还是要换成正式域名？（换的话我要把 Nginx 与两份 `.env` 里的地址一起改）
2. **服务器系统与规格**？（Linux 发行版、内存、有没有独立数据盘——决定备份往哪写；我按 Linux + systemd 出部署脚本与 unit 文件）
3. **邮件通道**：先 B（QQ/163 授权码，1 小时能通）还是直接上 A（云邮件推送，要备案）？
4. **管理面板位置**：✅ **已定（2026-10）——全部搬进用户中心**，见 [5.0](#50-p1-施工计划2026-10-定案)；`/admin/` 只保留学科后台。
5. **要不要封禁功能**？（我列进了 P1，但如果你觉得用不上，可以从本期砍掉，少一块状态判定）
6. **第一个管理员给谁**？（除了你自己，还有没有要一起管内容的人——这决定 M1 要不要连「管理员码」一起做，还是先只做「后台提升」）
7. **放人节奏**：按 10 → 50 → 上百 三批走，还是你想一次放开？

---

## 14. P2：配额口径、限流与监控（2026-10 定案，✅ 已完成）

> **状态：三项全部完成（2026-10-05）。** 其中配额的**代码**其实是 2026-10-02 单一循环池改造时就落地的
> （[`review-pool-plan.md`](review-pool-plan.md) 的 D14/D15），本轮只补了测试与过时口径；
> 真正的新工作是「限流真的可配 + 按上百人重算」与「监控告警」。
> 实测数字与交接见 [附录 C](#附录-c进度记录) 的 P2 段。

**定案（2026-10 与用户逐条确认，三个问题都选了推荐项）**

| 问题 | 定案 |
|---|---|
| 「每日配额」要不要服务端**强制**？ | **不强制**。保持 D14/A2 的既有设计：今日 5 个是**排序权重**（置顶、按稳定哈希选），继续复习不受限。本轮只把过时文档改对 + 补一条「客户端什么都不传时服务端也算得对」的测试 |
| 限流按什么口径重算？ | **严在账号、宽在 IP**。IP 是共享资源（校园 / 公司 / CGNAT 出口后面站着几十上百人），账号（邮箱 + 密码错误次数）才是身份 |
| 监控告警怎么做？ | **深度健康端点 + 复用账号服务的 SMTP 发告警邮件**（systemd timer 每 5 分钟）。磁盘余量与备份新鲜度**本轮不做**（明确的不做项，见下） |

### 14.1 配额：口径更正 + 一条测试（无功能改动）

- 事实：前端已无配额账 —— `modules/english/english.js` 顶部注释即写明「单一循环池之后，这里不再有配额账」，
  旧的 `PLAN_KEY` / `loadTodayPlan` / `markPlanDone` / `remainingQuota` / `probedIdsInCooldown` 已无引用，
  `localStorage.reviewDailyPlan` 不再写入。今日进度 = 服务端 `stats.daily_done` + 本会话增量；
  「今日 5 个」与「今日完成几个」都由 `backend-go/handlers/review_handlers.go` 的稳定哈希算
  （`dailyWordCount = 5` / `dailyWordIDs` / `countTodayDaily`）。
- 剩下的 localStorage 只有两件**本该在本地**的东西：自动朗读开关、沉浸模式开关。
- 时区口径：`poolTZName = "Asia/Shanghai"`（`review_handlers.go`，注释明确「**不要**改成 Local」）。
  客户端可以传 `?today=YYYYMMDD` 指定自己的自然日 —— 它只影响「今天哪 5 个被置顶」；
  因为配额不强制，所以不构成「换个日期就多刷一轮」的漏洞。
- 本轮补的测试：`backend-go/handlers/review_stats_test.go` 证明**客户端什么都不传时** `daily_target` 恒为 5、
  提交一张 daily 卡后 `daily_done` 由 0 变 1。
- ⚠️ 与 P1 看板的口径差异：看板的「今日新学」只判 `stability_before = 0`，而 `/api/reviews/stats` 的 `today_new`
  还排除 `is_reset` —— 「重置重学」多的日子两个数字会不一样（两处注释互相指认，别当成 bug）。

### 14.2 限流：先让 `AUTH_RL_*` 真的可配，再按上百人重算

- ⚠️ **修掉一条「帮助文本在撒谎」**：`main.rs --help` 里列着 `AUTH_RL_*`，但 `config.rs` 里**一行解析都没有** ——
  「调参」此前只能改代码重编译。本轮把五条规则全部接成环境变量，格式 `次数/窗口秒`，`0/0` 关闭；
  格式写错**拒绝启动**并点名变量（静默退回默认值的症状是「明明放宽了却还被 429」，最难查）。
- 新旧默认值（口径：严在账号、宽在 IP）：

| 变量 | 旧 | 新 | 为什么 |
|---|---|---|---|
| `AUTH_RL_LOGIN_IP` | 20/900 | **200/900** | 早高峰上百人登录，20 次/15 分钟会被自己人打满 |
| `AUTH_RL_CODE_IP` | 20/3600 | **200/3600** | 同上；注册当天会集中发码 |
| `AUTH_RL_REGISTER_IP` | 10/3600 | **100/3600** | 校园 / 公司出口 IP 后面可能一次来几十个新生 |
| `AUTH_RL_CODE_EMAIL_MINUTE` | 1/60 | 1/60（不变） | **账号维度**，这才是要卡死的地方 |
| `AUTH_RL_CODE_EMAIL_HOUR` | 5/3600 | 5/3600（不变） | 同上 |
| `AUTH_LOCK_THRESHOLD` / `AUTH_LOCK_MINUTES` | 5 / 15 | 不变 | 密码连错锁定；落库，重启不失效 |

- 启动横幅现在会打印**实际生效**的限流值（「我明明改了啊」不该靠再读一遍配置文件来查）。
- ⚠️ **顺带修掉一个真漏洞**：Nginx 原来用 `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for`，
  它把**客户端自己发来的 XFF 留在最前面**；而 `client_ip()`（`backend-rust/src/http/middleware.rs`）
  取的是第一个值 → 伪造 XFF 即可绕开所有按 IP 的限流。改成 `$remote_addr`（单层代理场景的权威值），
  并在模板里写明「将来前面加了 CDN，要改用 `set_real_ip_from` + `real_ip_header`，别无脑信任 XFF」。
- Nginx 再加一层粗粒度限流（应用里的限流是**单进程内存**、重启清零、多实例各算一份）：
  `limit_req_zone` 两条（`gx_auth` 30r/m、`gx_api` 300r/m，按 `$binary_remote_addr`）+ `limit_conn` 兜底，
  都回 **429**（与应用一致）；两层分工写在模板注释里。
- ⚠️ 生产必须让两个服务只监听回环（Go 的 `SERVER_HOST=127.0.0.1`、Rust 的 `AUTH_HOST=127.0.0.1`）：
  `client_ip()` 信任 XFF 的前提就是「外面只有 Nginx」。Go 启动时对「生产却监听非回环」有告警。

### 14.3 监控：深度健康 + 邮件告警

- 两个服务新增**深度**健康检查（`?deep=1`）：`GET /api/health?deep=1`（Go）与 `GET /api/auth/health?deep=1`（Rust）。
  深检真查一次库（Rust：数一次 `users` 行 + 读 `PRAGMA user_version`；Go：`SELECT 1` + 账号库可用性 + 迁移版本），
  不健康回 **503**。浅检（不传参数）形状不变，继续供 Nginx / dev-server 探活。
  账号库读不到**不算**不健康（沿用 P1 的降级口径）。
- 新增 `backend-rust/src/bin/monitor.rs`（二进制 `guangxue-monitor`）：请求两个深检端点，**只看状态码**；
  本轮结果与上一轮状态文件比较，**只在状态变化时发信**（开始故障 / 已恢复），持续故障每 6 小时重发一次。
  退出码 **0 = 健康 / 1 = 不健康**，于是 `systemctl --failed` 也能看到。
  它**手写 HTTP 请求**（`std::net::TcpStream`）而不是引入 HTTP 客户端库：探活只要「发一个 GET、读第一行状态码」，
  为此拉进 reqwest / hyper 一整棵依赖树不划算；代价是只支持 `http://`、不跟随重定向
  （公网 HTTPS 入口交给外部 uptime 服务）。
- 发信复用账号服务那一套 SMTP 配置（`Mailer` trait 新增 `send_notice`）：于是「告警能发出去」与
  「注册能收到验证码」是同一个前提；`AUTH_MAIL_MODE=log` 时告警只写日志（命令会明确提示这一点）。
- 新增 `deploy/systemd/guangxue-monitor.{service,timer}`：`OnCalendar=*:0/5`，
  **刻意不设 `Persistent=true`**（探活不是备份；补跑只会先发一封假告警再发一封「已恢复」）。
  状态文件在 `/var/lib/guangxue-monitor/`（systemd 的 `StateDirectory=` 自动建好并授权给服务用户）。
- ⚠️ **告警通道必须演练**：runbook 的「监控与告警」一节写了「停掉账号服务 → 收到告警 → 起回来 → 收到恢复」的步骤。
  **没验过的告警通道等于没有监控。**
- ⚠️ **本轮不覆盖**（明确的不做项）：**磁盘余量**与**备份新鲜度**。它们同样属于「悄悄坏掉」，
  真要加就是在 monitor 里再加两个检查函数 + 两个阈值环境变量；在那之前，备份是否真的在跑仍靠人工核对（runbook 第 7 步）。
- 机器整体挂掉（或 Nginx 挂掉）时 monitor 自己也喊不出来 → runbook 建议再挂一个**外部 uptime 服务**打
  `https://<域名>/healthz`（Nginx 直接回 ok、不碰后端，能发现「机器 / 网络 / Nginx 死了」）。

---

## 附录 A：已定案的决策

| # | 问题 | 你的决定 |
|---|---|---|
| 1 | 上线范围 | **仅英语模块**，实验性上线 |
| 2 | 注册门槛 | **必须填邀请码才能注册**（现阶段）；未来删档重来后改为「不强制、填了带权限」 |
| 3 | 进度隔离 | **先做「进度按人隔离」再上线**（P0-1） |
| 4 | 角色层级 | **三级**：超级管理员 / 管理员 / 普通用户 |
| 5 | 谁能发码 | **只有超管** |
| 6 | 管理员怎么产生 | **两条路都要**：后台直接提升 + 超管发管理员码 |
| 7 | 管理功能 | 邀请码管理、用户管理（含封禁/改等级）、审计日志、数据看板、内容管理入口——**五项全要** |
| 8 | 开放注册 | **关闭**：没有有效邀请码连验证码都发不出来 |
| 9 | 部署 | 已有云服务器 + 域名，用 **Nginx 反代** |
| 10 | 规模预期 | **上百人或更多** |
| 11 | 邮件通道 | 未定，先听建议（本文第 10 节） |
| 12 | 数据 | 实验性上线，**未来可能删档重来**；现有复习数据可弃，账号保留 |

---

## 附录 B：相关文档

- [`roadmap.md`](roadmap.md) —— 长期规划与阶段 0/1/2 的进度记录（本计划是它的「上线特化版」）
- [`backend-auth.md`](backend-auth.md) —— 账号服务的接口与鉴权细节
- [`admin-api.md`](admin-api.md) —— 词条与复习接口、后台约定
- [`boundaries.md`](boundaries.md) —— 硬性边界、测试数字、备份与验证脚本
- [`frontend.md`](frontend.md) —— 前端结构与 ES5 约定

---

## 附录 C：进度记录

### ✅ P0-1 复习进度按人隔离（已完成，2026-10）

**模型**（`backend-go/models/models.go`）

- `WordReview.UserID` + `UNIQUE(user_id, word_id)`（`idx_word_reviews_user_word`）、`(user_id, due_at)`（`idx_word_reviews_user_due`）；
- `ReviewLog.UserID` + `(user_id, reviewed_at)`（`idx_review_logs_user_time`）；`Word` 表**不动**（词库共享）。

**代码**

- `middleware.CurrentUserID(c) (uint, bool)`：只从 `gx_access` 令牌的 `sub` 取用户，`id <= 0` 一律当未登录；
- `handlers.currentUser(c)` 兜底：取不到就 401，**绝不写 `user_id = 0`**（那种行对谁都不再可见）；
- `review_handlers.go` 7 个函数、16 处查询按 user 收口：`DueReviews`、`NewWords`（**user 条件写在 join 里**，否则新词列表会变空）、`learnedQuery`（队列 / 抽查共用）、`QueueReviews` 的计数、`SubmitReview` 的读取与两处写入、`ReviewStats` 九条统计、`calcStreakDays`；
- `DeleteWord` **保持全用户级联**（删词条 = 删掉所有人对该词的进度），并在注释里写明这是决策而非疏忽；
- 请求体里**没有** `user_id` 字段——身份只能来自 Cookie，前端无法替别人写进度。

**迁移**（用户选定「显式迁移」而不是启动自愈）

- 新增 `backend-go/cmd/migrate`：默认 dry-run 只报告；`-apply` 时**先 `VACUUM INTO` 快照**（`backups/<时间戳>/`）再执行；
- 迁移内容：清掉 `user_id = 0` 的历史行 → 删掉旧的 `UNIQUE(word_id)` 索引与已被取代的 `idx_word_reviews_due_at` → 复验；
- `database.Init` 加了**启动自检**：旧结构直接拒绝启动并打印该跑哪条命令（把「忘了跑迁移」从运行时数据事故变成启动期报错）；
- 快照逻辑从 `cmd/backup` 抽成 `backend-go/snapshot` 包，两个命令共用。

**实测**

- `backend-go` 测试 53 → **61 项**：新增 `handlers/review_isolation_test.go` 7 项（含一条「旧唯一索引必须不存在」的结构断言）
  与 `database/user_isolation_test.go` 1 项（**在手工造的旧结构上**跑完整迁移：补列 → 清孤儿行 → 删旧索引 → 复合唯一键生效 → 幂等 → 启动自检放行）；
- `pwsh scripts/verify.ps1`：6 步 0 失败、9.5 秒；
- `pwsh scripts/verify-auth.ps1`：13 → **18 项全 PASS**、5.6 秒（管理员与普通用户复习同一个词后，各自 `total_reviews=1`——改造前这里会是 2）；
- 开发库实迁记录（2026-10-01）：快照 `backups/20261001-123656/guangxue.db`（132 KB，integrity ok）→ 清掉 100 行进度 / 241 行日志、删掉两条旧索引；
  迁移后库结构为 `UNIQUE(user_id, word_id)` + `(user_id, due_at)` + `(user_id, reviewed_at)`，实测 6 条索引、`words` 仍 100 条；
- 自检实测：拿**迁移前**的快照副本启动新二进制 → 打印提示并 **exit 1**（不带着旧结构上线）。

**遗留（已记录，不在 P0-1 范围内）**

- 浏览器 localStorage 的每日配额与抽查冷却仍按浏览器存 → 随 **P2** 配额服务端化一起处理；
- `/admin/` 里「某个词被复习过多少次」仍是全站口径 → **P1** 做管理面板时再分人。

---

### ✅ P0-2 强制邀请码注册（服务端已完成，2026-10）

**开关**

- `backend-rust/src/config.rs` 新增 `require_invite: bool`，环境变量 **`AUTH_REQUIRE_INVITE`**（`env_parse_bool`，与 `AUTH_DEV_ENDPOINTS` 同一套）。
- 默认值 **`false`**（本地开发随手注册），**生产必须在 `.env` 里显式写 `true`**。
  刻意**不**做成「生产自动 true」：那样「忘了配」与「故意关掉」在配置里长得一模一样。
  `backend-rust/.env.example` 里已加这一段并写明这一点。
- 为什么用开关而不是写死：你说过未来会「删档重来、不强制邀请码、填了才带权限」，届时关掉开关即可，注册流程代码不用再改一遍。

**代码（两处，顺序是关键）**

- `service.rs::request_email_code`：邮箱格式 → `split_codes` → **`if self.cfg.require_invite && codes.is_empty() { return Err(AuthError::InvalidInvite) }`** → 逐个 `is_plausible` → 限流 → 查库 → 写 `email_codes`。
- `service.rs::register`：同一条判断加在 `split_codes` 之后、`is_plausible` 之前（**两道门都要有**——只拦发码那一步的话，拿着有效码换来的验证码仍能不带码注册）。
- ⚠️ **邀请码校验必须排在写 `email_codes` 之前**：「先发码后校验」等于没拦 —— 码已经落库，按现有流程还能被用掉。这条是本次改动的核心，测试直接查库锁住它。

**测试（9 项，全在 `backend-rust/tests/require_invite.rs`）**

`spawn_strict()` = `spawn_with(|cfg| cfg.require_invite = true)`，覆盖：没带码 400 + 0 行；空串/纯空白等价于没带；查不到的码 0 行；过期码 `invite_expired` 0 行；用尽码 `invite_exhausted` 0 行；停用码 `invalid_invite` 0 行；有效码正常写 1 行且额度被核销；**注册这一步自己也拦**；开关关掉时开放注册仍可用（反向对照）。

- 为了能断言「**一条都没写库**」，新增 `store/sql.rs::count_email_codes(conn, email, purpose)`（数**含已消费**的全部行）
  与测试脚手架 `TestApp::email_code_rows(email)`。只数「未消费」不够 —— 「写了一条又立刻标记已消费」也会让那种断言通过。

**实测**

- `cargo test --test require_invite`：**9 项全过**；账号服务合计 **90 → 99 项**、0 失败。
- `scripts/verify-auth.ps1`：18 → **22 项全 PASS**（11.5 秒）。新增的 4 项是**另起一个实例**（18082，`AUTH_REQUIRE_INVITE=true` + 独立临时库）跑的：
  不带码发码 → 400 `invalid_invite`、被拒后库里**没有留下验证码**、不带码注册 → 400 `invalid_invite`。
  > 为什么要另起实例：这个开关只认「环境变量 → 配置」这一条链路，而 Rust 集成测试是直接改 `cfg.require_invite` 字段的，
  > 恰好**测不到「环境变量有没有真的接上」**；同时主实例必须保持开放注册（第 5) 段要注册普通用户）。

**刻意没做（不是遗漏）**

1. **前端邀请码框没改成必填**：你选的「先不动前端」。服务端是唯一安全边界，界面只影响用户是否提前知道要填码 → **✅ 2026-10-05 已随 P1 改完**（`GET /api/auth/config` 决定标签与提示，发码与注册前各本地拦一次）。
2. **按 `grant_role` 赋角色留在 P0-5**：那一列由 P0-5 的 `0002_launch.sql` 引入。
   ✅ **2026-10 已由 P0-5 补齐**（见下一条记录）。

---

### ✅ P0-5 三级角色与邀请码分级（已完成，2026-10）

**13 条与你逐条敲定的决策**（前 8 条是格式/发放/生成，后 5 条是边界行为）：

| # | 问题 | 定案 |
|---|---|---|
| 1 | 码长与显示 | **16 位不变**，只在界面/列表里分组显示成 `7K3M-9QRT-2XWZ-5BHD`；给用户的仍是紧凑形式 |
| 2 | 一张能用几次 | **默认 1 次**；要批量就发 N 张，要「一码多用」也可以（表结构与 CLI 本来就支持） |
| 3 | 有效期默认 | **默认 7 天**，界面给 1/7/30/90 天选项 |
| 4 | 怎么发出去 | 面板一键复制（P1）+ 邀请链接（P1）+ 整批发邮件（等 P0-3 SMTP）+ 导出 CSV/文本（P1） |
| 5 | 在哪里生成 | **CLI 先上，面板紧随** → CLI 与 HTTP 接口这次都做了，**面板界面留在 P1** |
| 6 | 等级与发放权限 | **只做普通码 + 管理员码**（不设超管码），且**只有超管能发** |
| 7 | 已有用户改角色 | **超管在用户列表里改**（管理员码只给新人） |
| 8 | 超管怎么产生 | **环境变量 + CLI 双保险**：启动期确保 `AUTH_ADMIN_EMAIL` 是超管，另有 `--role super_admin` |
| 9 | 存量邀请码怎么办 | **全部停用**（它们没有等级信息，留着就是一批身份不明的凭证）→ 迁移里 `UPDATE invite_codes SET disabled = 1` |
| 10 | 一次填多张码 | **拒绝**（400 `invalid_params`「一次只能使用一张邀请码」）——两张码等级可能冲突，「取最高」很难解释 |
| 11 | 超管自锁保护 | **禁止降级/封禁最后一个可用超管**（只数 `status='active'` 的） |
| 12 | 管理员码形态 | `ADMIN-` 前缀 + 库里 `grant_role`，**两种都保留**：前缀给人看，判定永远查库 |
| 13 | 权限矩阵 | 严格按第 3 节：管理员进后台管内容，**碰不到权限**；发码/调权限/封号/审计只有超管 |

**代码清单**

- `backend-rust/migrations/0002_launch.sql`（新）：加 `grant_role` / `batch_id` 两列 + 两个索引 + **停用全部存量码**；`db.rs` 的 `MIGRATIONS` 登记为版本 2。
- `models.rs`：`ROLE_USER` / `ROLE_ADMIN` / `ROLE_SUPER_ADMIN` 三个常量 + `can_enter_admin` / `is_super_role` / `is_valid_grant_role` / `role_rank` 四个判定，**所有角色判定都从这里走**（不再有散落的字符串比较）。
- `service.rs`：`create_invites(.., grant_role)` 生成带前缀的码并共享 `batch_id`；`register` 里角色改为
  `let grant_role = redeemed.first().map(|inv| inv.grant_role.clone()).unwrap_or(ROLE_USER)`（默认 `user`）；
  `ensure_super_admin`（启动期确保超管，**不降权**）；`change_user_role` / `set_user_status` / `list_users`（都带自锁保护与审计）。
- `http/extract.rs`：`AdminUser` 改用 `can_enter_admin`（放行两者），新增 `SuperAdminUser`。
- `http/admin.rs`：发码接口加 `grant_role` 出参与入参；新增 `GET /admin/users`、`POST /admin/users/{id}/role`、`POST /admin/users/{id}/status`；**邀请码与治理接口全部改成超管准入**。
- `bin/invite.rs`：`--grant-role` + 分组显示；`bin/seed_admin.rs`：`--role`（默认 `super_admin`）+ 明确「只在新账号时生效」。
- `backend-go/middleware/auth.go`：`RoleSuperAdmin` + `CanEnterAdmin` + `RequireSuperAdmin`。
- 前端：`admin/admin.js` 的 `canEnterAdmin`、`account/account.js` 的角色显示与后台入口、`main.js` 的标题标记。

**实测**

- 账号服务测试 **99 → 124 项**、0 失败。其中 P0-5 新增 **25 项**：
  `tests/roles.rs` 15 项（三级准入、超管提权幂等且不降权、发码分级、非法 `grant_role`、管理员发不了码、
  封禁顺带吊销会话、**最后超管不能降级/封禁**、被封的超管不算「可用」、用户列表过滤、管理员码端到端），
  `core/invite.rs` 3 项（前缀生成、前缀不是安全边界），`models.rs` 5 项（角色字面量契约、三道判定），
  `db.rs` 1 项（**迁移停用存量码** + 新列默认值），Go 侧 2 项（`RequireSuperAdmin` 门禁 + 字面量契约）。
- `scripts/verify-auth.ps1`：22 → **26 项全 PASS**（新增 4 项真实 HTTP：超管发管理员码带前缀、
  普通用户看邀请码/用户列表都 403、降级最后一个超管 400）。
- `scripts/verify.ps1` 6 步 PASS、`go test ./...` 5 包 ok。

**开发库实迁记录（2026-10-03）**

- 迁移日志：`已应用数据库迁移 version=2` + `已有账号已提升为超级管理员 email=2262997289@qq.com`。
- 实测：登录后 `role = super_admin`；发码得到 `ADMIN-2XRBKGKY2FPQKG6V`（`grant_role=admin`，带 `batch_id`）；
  邀请码列表 **17 张里 16 张 disabled**（存量码全停用，只有刚发的那张 unused）。
- ⚠️ **踩坑（值得记住）**：`cargo build`（不加 `--release`）只更新 `target/debug/`，而**开发服务跑的是
  `target/release/guangxue-auth.exe`** —— 我一度以为「改好的代码没生效、迁移没跑」，
  实际是那个 release 二进制还是三周前的。`scripts/verify-auth.ps1` 用的是 debug 二进制（它自己会 `cargo build`），
  所以它一直测的是新代码；**只有手动起的开发服务踩到了这个坑**。改完服务端后请 `cargo build --release`。
- ⚠️ 另一个坑：**不要用 `Copy-Item auth.db` 手工备份**。这个库开着 WAL，
  当时磁盘上 `auth.db` 是 126 KB 而 `auth.db-wal` 有 3.7 MB —— 变更全在 WAL 里，
  手工拷贝（尤其漏掉 `-wal`）拿到的是旧快照。备份请用 `VACUUM INTO`（`cmd/backup`）。

**刻意没做（不是遗漏）**

1. **邀请码管理面板**：发码/列表/一键复制/导出 CSV/邀请链接/邮件发放都要界面，属 P1 的四个面板之一。
   P0-5 交付的是能力（CLI + HTTP 接口都通了），**现在发码用 CLI**。
2. **超管码**：一张码就能再造一个能封你号的人，且本期没有「超管能否封超管」的设计 —— 按第 3 节的取舍不做。
3. **管理员的只读用户列表**：权限矩阵里写了「管理员 ✅（只读）」，但这次 `GET /admin/users` 只放给超管。
   理由是「只读用户列表」属于 P1 面板的功能，等面板做的时候再按矩阵放开（接口已经就绪，改准入即可）。
   **所以做完 P0-2 并不等于有了三级角色**，别把这两件事混起来。

---

### ✅ P1 管理面板（已完成，2026-10-05）

**你逐条选定的五条**（见 [5.0](#50-p1-施工计划2026-10-定案)）：

1. 面板位置 = **四个面板全部搬进个人中心**（`account/`），`/admin/` 从此只做内容管理；
2. 跨库取数 = **尽可能少的前端请求** → 看板 **1 个请求**出全部数字、用户列表 **2 个请求**（Rust 出用户表 + Go 出当页批量进度），刻意**不让 Go 也读用户表**（否则会多出第二份「谁是超管」的判定）；
3. 邀请码面板 = 全选（生成 + 一次性明文 + 复制全部 / 列表筛选分页 + 单张停用 + 按批停用 / 导出 CSV / 邀请链接 / **整批发邮件**）；
4. 用户管理 = 全选（改角色 + 封禁解封 / 强制下线 / 点开看复习进度 / 管理员**只读**用户列表）；
5. 顺带项 = 全选（修 `admin/` 过时横幅 / 管理员看用户列表时**服务端脱敏邮箱** / 审计保留期）。

**口径（钉死，改之前先回来看这段）**：「今日 / 按天」一律按**北京时间（UTC+8）**切天；今日新学 = `stability_before = 0`、今日抽查 = `is_probe`、今日活跃 = 当天有 `review_logs` 的 distinct `user_id`；邮箱脱敏在**服务端**做；面板**零动效**（第 8 节红线）；Go **只读**打开 `auth.db`，读不到时降级显示而不是报错。

**批 1：Rust 侧接口（✅ 已完成）**

- 模型（`models.rs`）：`UserPublic.last_login_at`、`InvitePublic.uses`（每张码最多回 3 条兑换记录，`INVITE_USES_SHOWN=3`）、`AuditRow`/`AuditPublic`（多带 `actor_email`）、`InviteUseRow`/`InviteUsePublic`。
- SQL（`store/sql.rs`）：`AuditFilter{action,actor_user_id,from,to}`（`WHERE_CLAUSE` 四个参数位始终绑定，时间列是定宽 UTC 文本 → 字符串比较就是时间序）、`count_audit`/`list_audit`（`LEFT JOIN users` 取操作者邮箱）/`distinct_audit_actions`/`prune_audit_before`、`disable_invites_by_batch`（`disabled = 0` 幂等）、`count_invites_in_batch`、`list_invite_uses`、`list_users` 加 `keyword`（拼 LIKE 前先剔掉 `%` 与 `_`）。
- 服务（`service.rs`）：`list_audit`（连动作清单一起回）、`prune_audit`、`disable_invites_by_batch`（审计 `invite_disable_batch`）、`revoke_user_sessions_admin`（`revoked_reason="admin_revoke"`，审计 `user_logout_all`）、`send_invites_by_email`（**一对一配对**、≤50 条、逐封独立、只发未使用的码、审计 `invite_email` **只记 id 与统计**）。
- 邮件：`Mailer::send_invite`（`expires_hint` 由服务层排版）；`LogMailer` 也能取回明文口令码，联调不必真发信。
- 配置：`AUTH_AUDIT_RETENTION_DAYS` 默认 **180**（`0` = 永不清理）；`main.rs` 启动时清一次 + 每 24 小时一次（`interval` 的第一次 tick 立即返回，正好当启动清理用）。
- 路由：**公开** `GET /api/auth/config`；超管 `GET /api/auth/admin/audit`、`POST /api/auth/admin/invite-batches/{batch_id}/disable`、`POST /api/auth/admin/invite-mail`、`POST /api/auth/admin/users/{id}/logout-all`；`GET /api/auth/admin/users` 放开给**管理员**（只读 + 非超管邮箱脱敏 + `email_masked`）。
- ⚠️ **路由命名**：按批停用没有写成 `invites/batch/...`、发邮件没有写成 `invites/email` —— 那样静态段会和 `invites/{id}` 落在同一层，症状是一类解释不清的 404。
- 实测：`cargo test --all-targets` **198 项全过**（P1 新增 `tests/audit.rs` **15 项**：公开开关、审计列表/筛选/时间范围、超管专属、保留期清理、按批停用幂等、脱敏、强制下线、逐封发信与坏输入；`tests/invite.rs` 里另加**自定义码 6 项**，见下面那条补记）。
- 踩坑：① `crate::db::parse_ts` 返回的是 `Option` 不是 `Result`；② 审计动作名是 **`login_ok` / `login_fail`**（没有 `login`）—— 面板的下拉直接吃这个清单，测试里就栽过一次。

**批 2：Go 侧统计（✅ 已完成）**

- 新增 `backend-go/database/authdb.go`：只读账号库访问层，DSN 是 `file:<绝对路径>?mode=ro&_pragma=busy_timeout(5000)`。
  - ⚠️ **必须带 `mode=ro`**：实测不带它时，打开一个**不存在的路径会凭空创建空库**（于是看板「读得到、但账号侧全是 0 且没有表」）；带上之后，不存在的路径直接报错并且**不创建文件**。
  - 惰性打开、**失败不缓存**（账号服务重启后自愈）、任何失败只返回 `*AuthDBUnavailableError` 交给 handler 降级 —— 绝不 panic、不影响 Go 服务启动。
- 新增 `backend-go/handlers/admin_stats.go`（四个 handler）与 `routes.go` 的 `/api/admin` 分组（整组 `RequireAdmin`）。
- `config.go` 新增 `AuthDBPath`（`AUTH_DB_PATH`，默认 `../backend-rust/auth.db`），并在 `StartupWarnings` 里加一条：这个路径读不到就在启动时喊一声。
- `deploy/systemd/guangxue-api.service` 加 `ReadOnlyPaths=/opt/guangxue/backend-rust`（**显式只读保证**，不是读权限来源），`deploy-runbook.md` 的 `.env` 清单补 `AUTH_DB_PATH`。
- **路由冲突是实测过的**：gin 1.9.1 下 `/users/progress` 与 `/users/:id/progress` 两种注册顺序都**不 panic**，两条各走各的处理器（`tree.go` 里 `skippedNodes` 回溯那套），所以保留了计划里的路径形状，并用 `TestRouterAdminProgressPathsCoexist` 钉住（批量看响应里有没有 `items`、单看有没有 `last7`）。
- ⚠️ **跨库时间的字典序陷阱**：`guangxue.db` 的时间是带自身偏移的文本（形如 `2026-10-03 17:27:36.1460602+08:00`），拿 UTC 边界直接比字典序会**排错先后** → SQL 只做**放宽两天**的粗筛（覆盖 −12:00~+14:00 的文本差，只多不漏），精确判定在 Go 侧按真实时间点做；另外 `MAX(reviewed_at)` 会被驱动当**文本**返回，必须扫成 `*string` 再解析（扫进 `*time.Time` 会得到零值而不报错）。
- 实测：`go test ./... -count=1` 各包 ok（新增 25 个 Test 函数）、`go vet ./...` 干净。
- 口径提醒：`today.new_words`（按定案只判 `stability_before = 0`）与 `/api/reviews/stats` 的 `today_new`（还排除 `is_reset`）**在「重置重学」多的日子会不一样**，两处注释互相指认。

**批 4：验证与落档（✅ 已完成）**

- `scripts/verify-auth.ps1` 新增第 9 段 **18 项** P1 检查；脚本总计 **45 项、0 失败、7.2 秒**（P2 又加了第 10 段 10 项；再往后 P1 补了「重置已用过的码」与「自定义码」两组、末尾又补了 3 条样式断言，**现在总计 71 项、0 失败**）。覆盖：公开开关、审计（匿名 401 / 普通用户 403 / 超管 200 且 `actions` 含 `login_ok` / `from=YYYY-MM-DD`）、一次生成 2 张且同批次、按批停用与幂等、整批发邮件（`sent=2 failed=0 mail_mode=log`）、用户列表 keyword 搜索 + `email_masked=false` + `last_login_at`、批量与单人进度、匿名读看板 401、看板聚合（账号侧 2 个用户 / 今天 2 条复习）、趋势 7 个点、强制下线、重置已用过的码（`cleared` 三段口径）、自定义码（位数报错 / 转大写抹横杠 / 永不过期 / 撞码 409 / 确认沿用 / 沿用后再用掉）。
- ⚠️ 两条断言是**按实际行为**（而不是我原先的想当然）改写的，值得记住：
  1. **注册不会更新 `last_login_at`**（注册时的自动登录不走 `login` 那条路）→ 断言改成「字段在」，另外用「刚登录过的管理员那行有值」来证明这个字段真会被写；
  2. **强制下线只能立刻踢掉账号侧**：Go 是本地验签、不查库，所以同一个 Cookie 打 Go **仍然 200**，直到 access 令牌过期（默认 900 秒）。这是阶段 2 就写进文档的已知限制，现在把它**显式测出来**（而不是留一句「应该会 401」），免得以后误以为「点了强制下线 = 全站立刻失效」。
- 文档同步：`docs/backend-auth.md`（权限矩阵、管理接口表、审计保留期、**样式共用口径的更正**）、`docs/admin-api.md`（管理面板已搬到个人中心 + 面板契约）、`admin/README.md`、根 `README.md`、`backend-go/README.md`、`backend-rust/README.md`、`backend-go/.env.example`。
- ⚠️ **更正一条我先前的说法**：`ReadOnlyPaths` **不是**读权限的来源（`ProtectSystem=strict` 本来就允许读那个目录），线上「看板账号侧全是 0」的真正原因是 **`AUTH_DB_PATH` 配错**或账号服务没起来。

**还没做的（交接给下一次）**

- **在真浏览器里人工点一遍**：面板的取数与渲染已经过三层验证 —— ① API 端到端（当时脚本 45 项 P1+跨服务，P2 后总计 55 项，加了「重置」「自定义码」与样式断言后 **71 项**）、② 静态检查（`pn-` 类名全部有定义 / 调用的接口全对得上路由表 / 0 处箭头函数·let·const·模板串 / 0 处动效 API）、③ 桩 DOM 里真跑 `mount()`（46 项断言，含降级与按钮 URL）——**但没有在真浏览器里人工点过**。请打开 `/account/#/invites` 走一遍：自定义一串 16 位码（勾「永不过期」、每张 1 次）→ 生成 → 复制 → 拿同一个串再点一次（应弹「已经用过，是否沿用」）→ 停用一张 → 整批停用 → 发邮件（log 模式只写日志）。
- 面板遇到 401 只提示、**不主动刷新登录态**（侧栏仍停在已登录布局）；会话中被改角色，按钮显隐要刷新页面才变（服务端仍然会拦）。

**补记：自己指定邀请码（2026-10-05，用户原话「不是我希望可以定制邀请码 就是我自己想一个16位的邀请码并设计时间等和触发条件等 不如说不过期但是只能使用一次」）**

> 起因：「邀请码添加可以固定设置功能 再加一个判断 若邀请码已经用过是否无视分险」这句被**误读**成「让生成表单记住上次填的那套值」，
> 连问两次都没问出来（用户当时没答），于是先按误读做了一版。用户随后把意思说清楚了，才有了这一段。
> 那次误读的教训写在最后，值得留档。

- **用户要的其实是三件事**：① 生成时能**自己指定一串码**（16 位）；② 有效期要能选**不过期**；③ 已被用过的码要能「无视风险继续用」，但要**先提醒**。
- **后端本来就支持一半**：`service.rs` 里 `expires_in_days <= 0 → None`（= 永不过期）早就写着，只是**界面没给这个入口**；「只能用一次」= `max_uses: 1`，本来就是默认值。真正缺的只有「自己指定码」。
- **码的形状**（`core/invite.rs` 新增 `normalize_custom_code` / `validate_custom_code`）：**正好 16 位**（与系统码同长同形）、只允许 `A-Z` 与 `0-9`；小写自动转大写、手写的 `-` 自动抹掉（`abcd-efgh-jklm-npqt` → `ABCDEFGHJKLMNPQT`）。
  - ⚠️ **随机生成的码也一起放开了**（用户 2026-10-05 追加：「邀请码要 A-Z 和 0-9」）：`core/invite.rs` 的 `ALPHABET` 从 Crockford Base32（32 个字符、排除 I/L/O/U）换成 **36 个字符的 `0-9` + `A-Z`**。理由是不一致本身就是 bug —— 手填的码能用 `O`，系统发的却永远见不到 `O`，看起来像坏了。改表不影响已发出的码（兑换一律查库）；信息量从 80 bit 升到约 82.7 bit。代价与对策：随机码现在会出现 `O`/`I`，手抄容易看错，所以**不做**「O→0」这类自动纠正（那会把用户真写对的码改成另一个码，更难查），界面上一律用「复制」按钮。
  - 锁这个字符表的测试是 `core/invite.rs` 的 `alphabet_covers_every_letter_and_digit`：生成 2000 张（32000 个字符位），断言 36 个字符**每个都出现过**且总共正好 36 种 —— 换回 32 字符表会立刻红（漏掉 I/L/O/U 的概率 ≈ 10^-386，不是靠运气过的）。
  - 字母表与系统生成**完全一致**（2026-10-05 起两边都是 A-Z + 0-9）：`MYCODE1234567890` 这种含 O/0 的写法必须放行，随机码里同样会出现这些字符 —— 「我自己写的码能用 O，系统发的却永远见不到 O」这种不一致本身就是 bug。
  - ⚠️ 两道检查**分开**且**长度在前**：`中文码1234567890` 只有 13 位，报的是「位数不够」而不是「字符不合法」；想测字符集必须拿一个长度正好 16 的串。测试里就栽过一次。
- **撞码（这是用户那句话里「若邀请码已经用过是否无视风险」的落点）**：自定义的串在库里**已经存在**时 ——
  - **不覆盖、也不新建**，回 **409 `invite_code_taken`**，`data` 里带 `{code, existing}`（那张码的状态 / 已用次数 / 谁用过）。这样前端面板能直接弹确认框，**不用多发一次查询**；
  - 管理员点「继续」后带 `allow_existing: true` 重发才**沿用**：只改 `max_uses` 与 `expires_at`，**`used_count` / `disabled` / `grant_role` / 兑换记录一个字都不动**（`sql::update_invite_terms` 的注释里写了为什么：那些是这张码的身份，顺手改掉等于悄悄放宽已发出的凭据）；
  - 响应带 `{custom, reused}`，面板据此说「已沿用这张已经存在的码」而不是「已生成」；
  - ⚠️ **不能静默沿用**：手滑把一张已经发出去的码又「建」一遍，最坏的后果是以为新发了一张而把旧码重复给了人。默认拒绝、要人点头，才拦得住。
  - ⚠️ **只对随机码重试撞码**：随机码撞了换一个再来（极小概率），自定义码撞了必须**如实报冲突** —— 重试只会把「你写的码已被占用」变成一句莫名其妙的「连续撞码」。
- **前端**（`admin/panels/invites.js`）：生成表单加「自定义码」框（留空 = 随机生成一批；填了 = 只出 1 张，边打边转大写并抹掉 `-`，下方实时提示「将用这一串建 1 张：…」）+「**永不过期**」勾选框（勾上天数框禁用并置灰划掉标签，按 `expires_in_days=0` 提交）。
  - ⚠️ 自定义码**不会**被加上 `ADMIN-` 前缀（前缀只在随机生成管理员码时加），所以界面上要靠列表里的「等级」列分辨 —— 提示文案里点明了这一点。
  - 壳（`admin/admin.js` 的 `apiFetch` 与 `account/account.js` 的 `panelApi`）现在会把**服务端信封**挂在抛出的 error 上（`err.error` / `err.data` / `err.status`）：光有一句中文文案分不出「码被占用了」和「参数写错了」，而 409 那条路径要读 `existing`。
- **命令行**（`cargo run --release --bin invite -- create --code <16位> [--allow-existing]`）：与面板走同一个 `InviteSpec` / 同一套校验，不在第二个入口重写一遍规则。
- **实测**：`tests/invite.rs` 新增 6 项（形态 / 转大写抹横杠 / 永不过期真能注册 / 坏形状不落库 / 409 带现状 / 确认沿用只改额度 + `allow_existing` 的 HTTP 形状）；`verify-auth.ps1` 新增 6 项跨服务检查；端到端另跑过一遍完整场景（建 → 注册用掉 → 429 冲突 → 沿用 → 再注册）。
- **教训（误读是怎么发生的）**：用户说「添加可以固定设置功能」时，把它读成了「表单要记住设置」，而用户指的是「**我要能固定下来一串自己定的码**」。两次追问都选了「用选项让我选」的形式，用户没答，于是按错的理解往下做了 —— 更稳的做法是：**把两种理解都写出来并列成选项**（「A：记住你上次填的值 / B：让你自己指定码本身」），而不是只给「存在哪儿」这种同一理解之下的细分选项。另外，用户对某个问题**不回答**本身就是信号：多半意味着选项没覆盖他真正要的东西。

**批 3：前端面板（✅ 已完成）**

- 新增 `admin/panels/`：`panels.css`（只允许 `pn-` 前缀、浅色卡片风、**零动效**）、`util.js`（时间按本地时区显示、复制、CSV、角色/状态中文名）、四个面板 `invites.js` / `dashboard.js` / `users.js` / `audit.js`。
- **面板是壳无关模块**：`export var meta = {id,title,desc,roles}` + `export function mount(container, ctx)`（可选 `unmount()`），能力全靠 ctx 注入（`api` / `el` / `toast` / `confirm` / `escapeHtml` / `setTitle` / `user`）—— 个人中心挂一份，以后 `/admin/` 也能挂同一份。
- `account/index.html`：脚本改成 `<script type="module">`（动态 import 的前提）、内容区加四个空容器 `#panelDashboard/#panelInvites/#panelUsers/#panelAudit`、右下角提示条 `#accToasts`、引 `../admin/panels/panels.css`；顺手把注册页那两句 P0-5 之后就不对的邀请码文案改了。
- `account/account.js`：`ADMIN_GROUPS` + `visibleGroups()`（「管理」这一组按角色出现，普通用户一个都看不到）+ `mountPanel()`（动态 `import()`、同 id 不重复挂、加载失败显示一条 `.pn-msg.is-error`）+ `AUTH_CONFIG`（读 `GET /api/auth/config`，强制邀请制下改标签/占位/required 并提前拦一次）+ `prefillInvite()`（邀请链接 `?invite=` 自动填码并停在注册页）。
- 静态校验：四个面板 **0 处**箭头函数 / `let` / `const` / 模板字符串 / `transition|animation|@keyframes`；用到的 **37 个 `pn-` 类名全部在 `panels.css` 里有定义**；调用的 11 条接口路径与两侧真实路由表逐条对得上。
- 三个面板各带一个自检用的极简 DOM 桩（在临时目录跑，不进仓库）：46 项断言覆盖取数、降级、按钮 URL 与请求体、配色映射、展开行。

### ✅ P2 配额口径 / 限流 / 监控（已完成，2026-10-05）

> 详细定案见第 14 节；这里只记「这轮做了什么、实测多少、还剩什么」。

**批 1：盘点（先查清事实，再决定做什么）**

- ⚠️ **三条与计划描述不符的事实**（都在动手前查清）：
  1. **配额早就服务端化了** —— 2026-10-02 的单一循环池改造（`docs/review-pool-plan.md` 的 D14/D15）已经删掉前端那本配额账，`localStorage.reviewDailyPlan` 不再写入；剩下的 localStorage 只有「自动朗读」「沉浸模式」两个 UI 偏好。**所以 P2 的「服务端化」实际只剩：把口径写进文档 + 补一条测试**。
  2. **`AUTH_RL_*` 根本没接进配置** —— `backend-rust/src/main.rs` 的 `--help` 一直列着它，但 `config.rs` 的 `from_lookup` 一行解析都没有；「调参」只能改代码重编译。旧默认值按 IP 卡得很紧（登录 20 次/15 分、发码 20 次/时、注册 10 次/时），上百人共用校园/公司/CGNAT 出口 IP 时会被自己人打成 429。
  3. **监控是从零开始** —— 只有三个浅探活（Nginx `/healthz`、Go `/api/health`、Rust `/api/auth/health`），没有任何主动检查与告警。
- **顺带查出一个真漏洞**：Nginx 模板用 `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for`，它把**客户端伪造的 XFF 留在最前面**，而 `backend-rust/src/http/middleware.rs` 的 `client_ip()` 取第一个值 → 伪造 XFF 即可绕过账号服务全部按 IP 的限流。正确写法是 `$remote_addr`。
- **用户定案（三问都选推荐项）**：① 配额**只改口径 + 补测试**；② 限流口径 = **严在账号、宽在 IP**；③ 监控 = **深度健康端点 + 复用账号服务的 SMTP 发告警邮件**（**磁盘余量、备份新鲜度明确不做**，写进第 14.3 节与 runbook）。

**批 2：Rust 账号服务（限流可配 + 深度健康 + 监控命令）**

- `config.rs`：新增 `env_rate_rule`（`次数/窗口秒`；任一侧为 0 = 关闭；**格式错拒绝启动并点名变量**）+ 五条 `AUTH_RL_*`，默认值 `200/900`、`1/60`、`5/3600`、`200/3600`、`100/3600`（4 个新测试）。
- `lib.rs`：`AppState` 加 `started_at: Instant` + `uptime_seconds()`（`tests/common/mod.rs` 的 `AppState` 字面量同步补字段）。
- `mail`：`Mailer` trait 加 `send_notice(to, subject, body)`；`log` 模式打进日志、`smtp` 模式走 `deliver`（正文尾巴注明是自动告警）。
- `service.rs`：`health_details()` 用 `SELECT COUNT(*) FROM users` + `PRAGMA user_version` 拼出 `{"database":"ok","users":N,"migration_version":N}`（**不用 `SELECT 1`**：那条查不出「进程活着但表没了」）。
- `http/auth.rs`：`/health?deep=1` → 深检，失败回 **503**；`wants_deep()` 只认 `1` 与大小写不敏感的 `true`（**与 Go 侧逐字对齐**）。
- `src/bin/monitor.rs`（新，约 480 行 + 6 测试）+ `[[bin]] guangxue-monitor`：**手写 HTTP**（`TcpStream`，不引 reqwest）；状态机 `Nothing/Started/StillFailing/Recovered`（`repeat_hours<=0` 只在状态变化时发信，默认 6 小时重发一次）；状态文件 `{failing,since,notified_at}`（坏文件当没有历史）；**先落状态再发信**（SMTP 挂了不会变成每 5 分钟一封）；退出码 0/1 供 systemd 判定；`MONITOR_*` 六个环境变量。
- `deploy/systemd/guangxue-monitor.service` + `.timer`（`OnCalendar=*:0/5`，刻意**不设** `Persistent=true`：错过的轮次没有补跑的意义）。
- 启动横幅现在打印**实际生效**的限流值（改完 `AUTH_RL_*` 看那里核对）；`--help` 里五条限流、`mail-test`、`guangxue-monitor` 齐了。
- 踩坑：给 `parse_target` 写测试时先写成 `unwrap_or_else(|_| panic!(...))` —— 闭包返回值类型是 Ok 分支的 `Target`，`err.is_empty()` 直接编译不过（E0599），改成 `match` 取 `Err`。

**批 3：Go 后端（深检 + 配额口径测试）**

- `handlers/health.go`（新）：`/api/health?deep=1` 追加 `deep` 对象（`database` 真查一次库、`migration_version`、`auth_db.available/error`、`uptime_seconds`、`version`）；**只有业务库查询失败才 503 + `status:"degraded"`**，账号库读不到仍 200（P1 口径：账号服务重启不该让监控每 5 分钟报警）；浅检形状**一个字段都不变**。
- `handlers.go` 删掉旧 `HealthCheck`（不留第二份实现），`const apiVersion = "1.0.0"`；`admin_stats.go` 抽出 `authDBUnavailableStatus`（看板与深检共用同一套判定与文案）；`database.MigrationVersion` 读 `PRAGMA user_version`（业务库没人写过 → 恒 0，纯信息项，读失败也不降级）。
- `review_stats_test.go` 新增 `TestStatsDailyQuotaServerSide`（**生产代码零改动**：非 daily 卡提交后 `daily_done` 仍 0、daily 卡提交后 0→1、`daily_target` 恒 5）。

**批 4：部署与文档**

- `deploy/nginx/guangxue.conf.template`：http 上下文加 `limit_req_zone`（`gx_auth` 30r/m、`gx_api` 300r/m）+ `limit_conn_zone`，两处 location 挂 `limit_req`/`limit_conn` 并统一 429；**两处 XFF 都改成 `$remote_addr`**（附 CDN 升级路径 `set_real_ip_from`）；`proxy_pass` 与 location 顺序未动。
- `backend-rust/.env.example`：⚠️ 旧限流段写的是**不存在的** `AUTH_RL_CODE_EMAIL_PER_MINUTE/_PER_HOUR`（`config.rs` 根本不认）、默认值还是旧的 20/10 —— 已按真实变量名与真默认值重写，并新增 `MONITOR_*` 段。
- `backend-go/.env.example`：`SERVER_HOST=127.0.0.1` + 理由（绕开 HTTPS、Nginx 限流、「后端只信 Nginx」的前提）。
- `deploy/systemd/guangxue-monitor.service` 里那行空的 `Environment=MONITOR_ALERT_TO=` 改成**注释**并写明「systemd 的 `EnvironmentFile` 覆盖 `Environment=`，与书写先后无关」。
- 文档同步：根 `README.md`、`backend-rust/README.md`、`backend-go/README.md`、`docs/README.md`、`docs/boundaries.md`、`docs/backend-auth.md`、`docs/review-engine.md`、`docs/roadmap.md`、`docs/deploy-runbook.md`（新的第 9 节「监控与告警」，含**告警通道演练**脚本：停机 → 退出码 1 → 收到「服务异常」→ 起回来 → 收到「已恢复」；后面两节顺延成第 10/11 节）。三处示例 Nginx 配置的 XFF 也一并改掉（不然会被照抄）。

**实测（2026-10-05）**

- `cargo test --all-targets` = **169 项全过**（73 单元 + 6 `guangxue-monitor` + 90 集成）。
- `go test ./... -count=1` 六个包全 `ok`；`-list` 数出 **114 个测试函数**（config 6 / database 7 / handlers 67 / stablehash 6 / middleware 12 / routes 16）—— ⚠️ 这是**函数个数**，带子测试的包实际 PASS 条数更多，别和别的数字混着引用。
- `pwsh scripts/verify-auth.ps1` = **55 项、0 失败、31.3 秒**（P2 段 10 项全 PASS，关键实测：`migration_version=2`、Go `auth_db.available=true`、`AUTH_RL_LOGIN_IP=2/60` 时三次登录 `401,401,429`、坏格式 `exit=1` 且日志点名变量、监控命令四轮 `exit=0/1/1/0`）。
  - ⚠️ 2026-10-05 之后这个数字变过两次（P1 又补了「重置已用过的码」「自定义码」两组与 3 条样式断言），**以脚本自己打印的合计为准**；最新一次是 **71 项、0 失败**。
- 静态复核：`node --check` 全过；Go 侧 `gofmt -l` 对改动文件为空、`go vet ./...` 无输出。

**交接（还没做的）**

- **告警通道要用真 SMTP 演练一次**（本机只有 `log` 模式）：runbook 第 9.3 节，配好 `MONITOR_ALERT_TO` 后停一次 `guangxue-auth`，确认真的收到「服务异常」与「已恢复」两封。
- 上线时 `backend-go/.env` 必须写 `AUTH_DB_PATH`（指到账号库），否则看板账号侧全是 0。
- 仍未做（本轮明确不做）：**磁盘余量与备份新鲜度**的监控；细粒度 RBAC；图形验证码。
