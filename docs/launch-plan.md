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
| P0-1 | **复习进度按人隔离** | 现在 `word_reviews` 的唯一索引是 `WordID`，**两个用户共用一份进度会互相覆盖**——这是数据正确性问题，不是体验问题 | 1~2 天（含测试） |
| P0-2 | **强制邀请码注册** | 你选的「暂时邀请制」；现在不填邀请码也能注册 | 半天 |
| P0-3 | **打通真发邮件** | 现在 `AUTH_MAIL_MODE=log`，验证码只打进日志，**真实用户收不到码就注册不了** | 半天（SMTP 代码已写好，只差配置） |
| P0-4 | **公网部署** | HTTPS、Cookie Secure、Nginx 分流、生产环境开关、每日备份 | 1 天 |
| P0-5 | **三级角色（超管/管理员/用户）** | 你要的「超管发带等级的码」；现在只有 `user`/`admin`，且**带邀请码注册会直接变管理员**（`backend-rust/src/service.rs:367`） | 1~2 天 |
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

**现状**（这是全项目最危险的一处）：

| 位置 | 现状 | 问题 |
|---|---|---|
| `backend-go/models/models.go:114` | `WordID uint \`gorm:"uniqueIndex"\`` | 一个单词**全局只有一行**复习记录 |
| `backend-go/models/models.go:134` | `ReviewLog.WordID` 有索引、**没有 `user_id`** | 日志分不清是谁复习的 |
| handlers 全部查询 | 没有 `WHERE user_id = ?` | A 用户复习完，B 用户看到的是 A 的进度 |

结果：**第 2 个人一开始用，两个人的进度就会互相覆盖**——A 把词标成「已掌握」，B 那边也跟着变；B 再复习一次，A 的到期时间又被改掉。这不是「体验差」，是数据被写坏。

**改造方案**：

```sql
-- 1) 复习记录：加 user_id，唯一键从 (WordID) 改成 (UserID, WordID)
ALTER TABLE word_reviews ADD COLUMN user_id INTEGER NOT NULL DEFAULT 0;
DROP INDEX IF EXISTS idx_word_reviews_word_id;          -- GORM 自动建的那个
CREATE UNIQUE INDEX idx_word_reviews_user_word ON word_reviews(user_id, word_id);
CREATE INDEX idx_word_reviews_due ON word_reviews(user_id, due_at);

-- 2) 复习日志：加 user_id
ALTER TABLE review_logs ADD COLUMN user_id INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_review_logs_user_time ON review_logs(user_id, reviewed_at);
```

> ⚠️ SQLite 的 `ALTER TABLE` 不能改列类型、不能加带约束的复合唯一键，所以要么用「建新表 → 拷数据 → 换名」的标准流程，要么**直接 DROP 三张业务表让 GORM `AutoMigrate` 重建**（我们选了清空数据，这条路更简单，推荐）。

**代码侧**：

- 所有 handler 从 gin Context 取 `middleware.CtxUserID`（**阶段 2 已经把它注入好了**，这正是那次改造的价值），查询全部加 `WHERE user_id = ?`；
- `Word` 表**不加 `user_id`**——词库是**共享**的（roadmap 第 3 节的核心判断：**词库共享、进度私有**）；
- `WordReview`/`ReviewLog` 的 `UserID` 字段加 `gorm:"uniqueIndex:idx_word_reviews_user_word"` 这类显式索引标签。

**验收标准**：

1. 新增测试：两个用户各自复习同一个词，`word_reviews` 里出现**两行**、`due_at` 互不影响；
2. 新增测试：A 用户的 `/api/reviews/stats` 不含 B 用户的数据；
3. `pwsh scripts/verify.ps1` 全绿；
4. `scripts/verify-auth.ps1` 加两条用例：管理员与普通用户各自复习后，互相看不到对方的 `reviewed_words`。

---

### P0-2 强制邀请码注册

**现状**：`backend-rust/src/core/invite.rs:8-10` 明确写着「邀请码**不再是注册门槛**」，`POST /api/auth/email-code` 不带 `invite_code` 也能发码（`service.rs:199` 起），注册时邀请码是可选的（`service.rs:285`）。

**改造**：

- 新增配置开关 **`AUTH_REQUIRE_INVITE`**（默认 `false` 保持开发方便；**生产置 `true`**）。
- 开关打开时：
  - `POST /api/auth/email-code`：`invite_code` **必填**，校验不通过直接不下发验证码（错误沿用现有的 `invalid_invite` / `invite_expired` / `invite_exhausted`）；
  - `POST /api/auth/register`：`invite_code` **必填**，注册成功时按邀请码上的 `grant_role` 赋角色（见 P0-5）。
- 前端 `account/account.js`：邀请码输入框改成**必填**（`regInvite`），文案改成「没有邀请码？暂时无法注册」；`:657-658` 那条「不要把空格规范化」的注释仍然有效，别动。
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

**现状**：只有 `user` / `admin` 两种字符串；`users.role` 是 TEXT 字段（`migrations/0001_init.sql:17`，注释里写了「预留：user / admin / ...」），所以**加角色不需要改表结构**。

**要改的地方**（⚠️ **角色字符串是跨服务契约**，Rust 与 Go 两边都要改，漏一边就会出现「后台能进、接口 403」这类灵异现象）：

| 位置 | 现在 | 改成 |
|---|---|---|
| `backend-rust/src/http/extract.rs`（`role == "admin"` 的判定） | 只认 `admin` | 认 `admin` **和** `super_admin` |
| 新增 `RequireSuperAdmin` 等价物 | 无 | 只有 `super_admin` 能过（发码、改他人角色、封禁） |
| `backend-go/middleware/auth.go` | `RoleAdmin = "admin"` | 加 `RoleSuperAdmin`；`RequireAdmin` 放行两者，新增 `RequireSuperAdmin` |
| `backend-rust/src/service.rs:367` | `let role = if redeemed.is_empty() { "user" } else { "admin" }` | **按邀请码的 `grant_role` 赋值**（默认 `user`）——这一行是当前最大的权限漏洞 |
| `admin/admin.js` 的登录门禁 | 要求 `role === 'admin'` | 放行 `admin` 与 `super_admin` |

**邀请码分级**：`invite_codes` 表加一列（见 P0-5 的 SQL），超管发码时选「普通用户码 / 管理员码」。

**给现有管理员升级为超管**：一条 SQL 或 CLI 命令即可（`UPDATE users SET role='super_admin' WHERE email='2262997289@qq.com'`），建议顺手补一个 `cargo run --bin seed-admin -- --role super_admin` 的能力。

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
