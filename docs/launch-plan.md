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
| P0-3 | **打通真发邮件** | 现在 `AUTH_MAIL_MODE=log`，验证码只打进日志，**真实用户收不到码就注册不了** | 半天（SMTP 代码已写好，只差配置） |
| P0-4 | **公网部署** | HTTPS、Cookie Secure、Nginx 分流、生产环境开关、每日备份 | 1 天 |
| P0-5 | **三级角色（超管/管理员/用户）** ✅ 已完成（2026-10） | 你要的「超管发带等级的码」；改造前只有 `user`/`admin`，且**带邀请码注册会直接变管理员** | 1~2 天 |
| P1 | 管理面板四个模块（邀请码/用户/审计/看板） | 现在邀请码只能跑 CLI，`/admin/` 里没有任何界面 | 2~3 天 |
| P2 | 每日配额服务端化、限流调参、监控 | 配额现在在浏览器 localStorage 里，多设备会翻倍；不影响数据正确性 | 1 天 |

**推荐顺序**：P0-1 → P0-5 → P0-2 → P0-3 → P0-4 → 放人 → P1 → P2。
理由：P0-1 是数据正确性（越早做，废数据越少）；P0-5/P0-2 是同一批代码（注册流程），一起改一次测一次；P0-3/P0-4 是外部依赖（等授权码/等证书），可以和开发并行推进。

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
  > 只影响「用户提不提前知道要填码」。这条等 P1 管理面板一起改文案与表单。
- **为什么用开关而不是写死**：你说过未来会「删档重来，注册不强制邀请码，填了才带权限」——那时把开关关掉即可，代码不用再改一遍。

**验收标准**：
- 不填邀请码请求 `email-code` → 400 且**数据库里没有新增 `email_codes` 行**（这是关键：不能「先发码后校验」）；
- 填过期码 / 已用尽的码 → 分别返回 `invite_expired` / `invite_exhausted`；
- 填有效码 → 正常收到验证码。

---

### P0-3 打通真发邮件

**好消息**：SMTP 发信**代码已经写完了**（`backend-rust/src/mail/smtp.rs`，用 `lettre`），现在是「有实现、没配置」的状态。缺的只是：

1. 选一个发件通道（见 [第 10 节](#10-邮件通道选型)）；
2. 拿到 **SMTP 主机 / 端口 / 账号 / 授权码 / 发件人**；
3. 在服务器 `.env` 里配上：

```env
AUTH_MAIL_MODE=smtp
AUTH_SMTP_HOST=smtp.example.com
AUTH_SMTP_PORT=465
AUTH_SMTP_USERNAME=no-reply@test.lovezmx.com
AUTH_SMTP_PASSWORD=授权码
AUTH_SMTP_FROM=广学 <no-reply@test.lovezmx.com>
AUTH_SMTP_TLS=implicit      # 465 用 implicit；587 用 starttls；25 用 none
```

4. **做域名邮件的 SPF / DKIM / DMARC 解析记录**——不做这一步，验证码大概率进垃圾箱，用户收不到就注册不了（这是上线最常见的翻车点）；
5. 真机验收：用一个**非管理员、非本机邮箱**（比如 Gmail / QQ 邮箱）走完整注册流程。

**验收标准**：注册一封真实邮箱 → 1 分钟内收到中文验证码邮件 → 验证码可完成注册。**并在服务器上确认 `AUTH_DEV_ENDPOINTS` 是关闭的**（生产强制关闭，但要亲眼确认）。

---

### P0-4 公网部署

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
> ⚠️ **邀请码管理面板（发码 / 复制 / 导出 / 邀请链接 / 邮件发放）仍在 P1**。
> P0-5 交付的是「能力」：CLI 与 HTTP 接口都能发码、带等级、带批次，
> 界面那一层等 P1 的四个面板一起做。**你现在发码用 CLI**（见附录 C 的命令示例）。

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
| 普通邀请码 | 16 位 Base32（如 `7K3M9QRT2XWZ5BHD`） | 超管 | `role = 'user'` |
| 管理员码 | `ADMIN-` + 16 位（如 `ADMIN-7K3M9QRT2XWZ5BHD`） | 超管 | `role = 'admin'` |

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

### 5.1 放在哪里（一个需要你拍板的取舍）

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
| `limit_req` | 在 Nginx 层再挡一道注册/发码接口的脚本扫描 |
| Gin 生产模式 | ⚠️ `backend-go/routes/routes.go:11` 目前没按 `APP_ENV` 切 `gin.ReleaseMode`，会一直刷调试日志，上线前补 |
| 监听地址收回环 | Go 默认 `SERVER_HOST=0.0.0.0`（Rust 已是 `127.0.0.1`），改成 `127.0.0.1` 只让 Nginx 访问 |
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

# backend-go/.env
SERVER_HOST=127.0.0.1
SERVER_PORT=8080
DB_PATH=/var/lib/guangxue/guangxue.db
```

> ⚠️ 改 `AUTH_JWT_SECRET` 会让所有已登录用户掉线——**上线后就别改了**（改的话两边一起改 + 重启两个服务）。

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
| **M2** | P0-3 邮件 | 真实邮箱收到验证码并完成注册 |
| **M3** | P0-4 部署 | 公网 https 全链路可用；重启自愈；备份产物可恢复 |
| **M4** | P1 管理面板 | 超管能在界面上发一批码、提升一个管理员、封一个号、查到对应审计 |
| **M5** | 放人（10 → 50 → 上百） | 每批观察 3 天：注册转化、复习留存、错误日志、备份是否正常 |
| **M6** | P2 配额服务端化 + 限流调参 + 监控告警 | — |

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
4. **管理面板位置**：接受「用户中心放入口卡片 + 界面留在 `/admin/`」这个折中吗？还是坚持全搬进用户中心（多 1~2 天）？
5. **要不要封禁功能**？（我列进了 P1，但如果你觉得用不上，可以从本期砍掉，少一块状态判定）
6. **第一个管理员给谁**？（除了你自己，还有没有要一起管内容的人——这决定 M1 要不要连「管理员码」一起做，还是先只做「后台提升」）
7. **放人节奏**：按 10 → 50 → 上百 三批走，还是你想一次放开？

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

1. **前端邀请码框没改成必填**：你选的「先不动前端」。服务端是唯一安全边界，界面只影响用户是否提前知道要填码 → 跟 P1 一起改。
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
