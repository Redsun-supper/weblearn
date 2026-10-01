# 广学 · 未来规划（roadmap）

> **这份文件就是项目的「未来规划」**：要做什么、按什么顺序做、为什么是这个顺序。
> 用法：先看第 0 节的**一页结论 + 阶段表** → 要落地细节时翻对应章节（第 2 节鉴权、第 3 节多用户化的具体做法）；
> **第 8 节**记着已定案的决策与进度记录，做完一个阶段就回来更新它。
> 2026-09 写成，依据是对代码的实地侦察（每条结论都带 `路径:行号`，便于核对）。
> ⚠️ 侦察是**静态阅读**，没有启动服务实测；文中已标出不确定项。
> 本文是活文档：决策落地后回来更新，**不要在 `CLAUDE.md` 里复述**（见其保护条款）。

---

## 0. 一页结论

**事实基线（五条，决定了所有判断）**

1. **账号服务里几乎没有数据**：`backend-rust/auth.db` 只有 4 KB（WAL 另占 3.7 MB）→ **现在做数据模型改造最便宜**，晚了要写迁移脚本。
2. **业务侧完全没有身份概念**：`backend-go/routes/routes.go:10-53` 无任何中间件；`WordReview`/`ReviewLog`（`backend-go/models/models.go:134-167`）没有 user 维度；英语页从不问 `/api/auth/me`。
3. **Go 侧 0 个测试**（`backend-go/` 全目录无 `_test.go`），而 681 行的 `backend-go/handlers/review_handlers.go` 正是改造要动的地方。（✅ **阶段 1 已解决**：补了 5 个测试文件 / 40 项测试；阶段 2 又补了 middleware 与路由回归，共 53 项，见第 6 节风险 2）
4. **前端分发写死了英语**（`main.js:38` 的 `if (pageName !== ENGLISH_PAGE) return;`）——「框架已支持多学科」只对了一半。
5. **备份方式不可靠**：现在靠整目录拷贝到 `备份3`/`备份4`，而 `auth.db` 有 3.7 MB 未 checkpoint 的 WAL → **直接拷 `.db` 可能备份出一个空库**。

**建议顺序**（每阶段都能独立交付、可随时停）

| 阶段 | 内容 | 预估 | 风险 |
|------|------|------|------|
| ✅ 0 | `scripts/verify.ps1`（一键验证）+ `scripts/backup.ps1`（安全快照）+ 文档口径修正 | 0.5–1 天 | 无（纯增益） |
| ✅ 1 | Go 关键路径测试基线（submit 的到期计算、stats 口径） | 1–2 天 | 无 |
| 2 | 服务端鉴权中间件 + 写接口门槛 + 删掉 Go 侧重复表与占位接口 | 1–2 天 | 中（配错密钥会全站 401） |
| 3 | 多用户化（两张表加 user 维度、handlers 加过滤、配额迁服务端） | 2–3 天 | 中（动数据模型） |
| 4 | 英语做深：统计/热力图 → 词书筛选 → 会话续上 | 按项估 | 低 |
| 5 | 可选：扩科框架改造 / 引擎卡片模型泛化 / CI | — | 低 |

---

## 1. 定位：三条路线的成本 × 解锁力

| 路线 | 技术前置 | 成本 | 解锁了什么 |
|------|---------|------|-----------|
| **A. 自用 / 熟人**（当前状态） | 无 | 0 | 只有体验提升，没有外部反馈；但仍要承担 5 条事实基线里的风险（尤其备份） |
| **B. 小范围邀请制**（几十人） | 鉴权 + 进度按人隔离 | ≈ 2–4 天 | 真实用户反馈、可展示的作品、可持续积累的数据；**风险可控**（邀请码已经是现成的门禁） |
| **C. 公开运营**（陌生人注册） | B 的全部 + 备份恢复 + 限流加固 + 内容产能 + 邮件/合规 | 持续投入 | 规模化；但单人维护 + 开放注册 = 必然被刷（邮件轰炸、垃圾账号） |

**推荐 B**，理由三条：

1. **门已经装好了**：`invite_codes` / `invite_uses` / `audit_logs` 表齐全（`backend-rust/migrations/0001_init.sql:62-112`），验证码注册、多端登录、令牌轮换、限流都已实现且 smoke 覆盖 24 项。B 缺的**只有业务侧的身份接线**。
2. **窗口期最好**：`auth.db` 无真实用户（事实基线 1）→ 加列、改索引、清测试数据都不用写迁移。
3. **C 的瓶颈不是技术**，是内容与运营。技术投入在 B 阶段就足以支撑，C 需要的是另一个人力结构。

> 换句话说：**B 的成本是「一次数据模型改造」，A 的成本是「永远不能给别人用」，C 的成本是「持续运营」。** 现在选 B 最划算。

---

## 2. 上线与安全：服务端鉴权怎么做

现状：Rust 侧令牌机制完整（access = HS256 JWT，`backend-rust/src/core/token.rs:25-36/57-89`），但 **Go 侧完全不认它**。

### 三条可选路径

| 路径 | 做法 | 优点 | 代价 |
|------|------|------|------|
| **(a) 共享密钥本地验签** | Go 读 `AUTH_JWT_SECRET`，自己验 HS256 并取 `sub/sid/role/exp` | 零 DB 访问、零跨进程、最快 | 无状态 → 登出/封号最长要等 access TTL（900s）才生效 |
| (b) HTTP 调 `/api/auth/me` | 中间件把浏览器 Cookie 转发给 Rust | Rust 零改动、状态实时 | 每请求一次跨进程调用；Rust 每次都会 `touch_session` 写库（放大 WAL）；**账号服务挂了，复习也停** |
| (c) 直读 `auth.db` | Go 另开只读连接查 `sessions`（`migrations/0001_init.sql:42-57`） | 能查吊销、无跨进程 | 跨服务 schema 耦合；每请求一次查询；⚠️ `sessions.expires_at` 是 **refresh 的 30 天 TTL**（`config.rs:134`），**不是** access 的 900 秒——照它判会错得离谱 |

### 推荐做法：以 (a) 为主 + 角色门槛

1. 新增 `backend-go/middleware/auth.go`，挂载点是现成的 `routes.go:14` 的 `api := r.Group("/api")`。
2. 验签必须**钉死 `alg=HS256`**、拒绝 `none`、校验 `exp`。⚠️ Rust 侧有 60 秒 leeway（`core/token.rs:39`），Go 侧要对齐同一口径，否则临界时间两边不一致。
3. 两级门槛：
   - `RequireUser` → `POST /api/reviews/submit`（`routes.go:49`）；
   - `RequireAdmin` → 词库写接口 `POST /api/words`(`:38`)、`PUT /api/words/:id`(`:40`)、`DELETE /api/words/:id`(`:41`)。
4. **CSRF**：Cookie 鉴权 + 写接口必须防跨站。Rust 侧已有 Origin 校验（`src/http/middleware.rs:104`，只拦非 GET），Go 侧照抄一个同源校验；`SameSite=Lax` 已挡掉大部分跨站 POST。
5. **依赖**：`go.mod` 里没有任何 JWT 库（`backend-go/go.mod:5-9`）。建议引 `github.com/golang-jwt/jwt/v5` 并用 `go mod tidy` 同步（符合 CLAUDE.md 红线 6）。**不建议手写 JWT 解析**——`alg` 混淆这类坑不值得自己踩。
6. **密钥分发（最容易踩的坑）**：Rust 在开发模式缺 `AUTH_JWT_SECRET` 时会**每次重启随机生成**（`config.rs:210-213`）→ 一旦没显式设，Go 侧验签全部失败且现象随机（重启就好/重启就坏）。必须在文档与 `.env.example` 里写死：**两侧共用同一个显式密钥**。
7. **顺手减熵**：建议删掉 Go 自己的 `users` / `data_items` 表（`models/models.go:12-17`，AutoMigrate 会建，`database/database.go:38-44`）与 `/api/user/*`、`/api/data/*` 占位接口——它们与 `auth.db` 的 users **同名不同源**，留着必然误用。现在删零成本。

---

## 3. 数据架构：多用户化最小路径

**核心判断：词库共享，进度私有。**

- `words` 表**保持全局**（admin 维护）→ 内容产能压力不变，不用给每个人导词。
- ✅ **已完成（P0-1，2026-10）** `word_reviews` 加 `user_id`，唯一索引从 `WordID` 改为 `(UserID, WordID)`。这是**语义变更**：从「这张卡在全站的状态」→「我的卡的状态」。
- ✅ **已完成（P0-1）** `review_logs` 加 `user_id`（便于按人统计 / 导出 / 将来的参数优化）。
- ✅ **已完成（P0-1）** handlers 全量加 `WHERE user_id = ?`：`queue` / `new` / `probes` / `due` / `submit` / `stats`（`backend-go/handlers/review_handlers.go`）。
- **每日配额从 localStorage 迁到服务端**，而且**不需要新建表**：
  - 今日新学 = 该用户当天 `stability_before = 0` 的日志数（现口径 `review_handlers.go:690-692`）；
  - 今日抽查 = 当天 `is_probe = 1` 的日志数。
  - 副作用是好的：换设备 / 清缓存不再重置。`docs/review-engine.md` 里那条「因为没有登录系统所以记在浏览器」的取舍**就此作废**，届时同步更新该文档。
- **迁移策略**：直接 AutoMigrate 加列，把现有复习数据当**本地开发数据**处理（无真实用户、词库仅 143 KB）——加列后置 NULL 或清空重建，不写迁移脚本。
- **前端要改**：`modules/english/english.js` 增加一次 `/api/auth/me`（现在完全不问），并据此决定未登录行为（见第 8 节待决策 2）。

---

## 4. 英语模块做深（按成本 × 收益排序）

| 功能 | 成本 | 收益 | 关键事实 |
|------|------|------|---------|
| **统计 / 热力图** | 中 | **高** | 数据已齐（`review_logs` 有 `ReviewedAt` + `Rating`，`models.go:154-167`）。需新接口（按天聚合）+ 前端视图。现在顶栏只有三格（`modules/english/english.html:31-33`），**全模块没有任何图表/历史页** |
| **词书 / 单元筛选进复习** | 低-中 | 中高 | `Word.Book/Unit` 字段已有（`models.go:121-122`），`/api/words` 已支持 subject/book/unit/search 筛选（`review_handlers.go:118-134`）；但复习的三个取数接口**没有**这些参数，要加上 + 前端加选择器 |
| **会话续上** | 中 | 中 | `modules/english/FUTURE.md` 已列 (a)/(b)/(c)。**推荐 (b)**：引擎加「只推进游标、不落库」的接口 + `sessionStorage` 存游标（要动 `modules/english/engine/src/session.rs` 并加测试，测试范式现成） |
| **FSRS 参数优化** | 高 | **现在低** | 数据量不够（词库 143 KB、单用户）；且日志缺 `difficulty_before`、`elapsed_days`（上次间隔）、`desired_retention`。**建议只做低成本的投资动作：先补这三列**，等积累到几千条再谈 |
| 朗读 / 键位 / 动效打磨 | 低 | 递减 | 已相当完善；且动效受 CLAUDE.md 工作流程第 5 条约束（先商量） |

---

## 5. 扩科：真正的瓶颈与一个必须先做的小改造

**事实修正**：`main.js:38` 是 `if (pageName !== ENGLISH_PAGE) return;`——**分发写死了英语**。加一门学科要动：`index.html:39`（导航项）、`main.js:14`（路径常量）、`main.js:38`（分发）、`admin/admin.js:27-29`（后台注册表），还要注意 `main.js:65` 的 `CACHE_VERSION` 要 +1（路径变更必须整体失效）。

**建议先做一次框架小改造**：把 `main.js:14` 的单一常量换成学科注册表（`{ english: './modules/english/english.html', ... }`），`initSubjectModule` 查表分发。成本低，把「加学科」从改 3 处变成加 1 行配置——而且**不违反**「学科逻辑不得回流 main.js」的约定（表是数据，不是逻辑）。

**真瓶颈有两层**：

1. **内容产能**——谁写词表/题库。导入链路已通（后台批量导入 → `parse_word_list` → 每 200 条 POST），但注意 CLI `backend-go/cmd/seed/main.go:40-48` 的词表结构**不含 book/unit**，脚本导入填不了词书。
2. **卡片模型**——现有引擎是「单词卡」模型（列序＝单词/音标/释义/例句/例句翻译，`modules/english/engine/src/wordlist.rs:209-216`）。历史/政治要的是问答卡，要改引擎的卡片模型（成本高，且要重做渲染与动效）。

**我的建议：先不扩科**。用英语内部的**多词书**（四级 / 六级 / 考研 / 雅思）替代「多学科」来扩张内容面——数据结构已支持、导入已支持、复习只需加筛选参数（第 4 节第 2 项）。除非你有明确的「某门学科有人要学」的证据，否则扩科是投入产出最低的方向。

---

## 6. 工程质量与运维（现存风险 + 最小动作）

按严重度排序：

1. **备份不可靠（现存风险，最高优先）— ✅ 阶段 0 已解决**
   `auth.db` 只有 4 KB 而 `auth.db-wal` 有 3.7 MB——数据滞留在 WAL（未 checkpoint，或写服务当时在跑）。
   **整目录拷贝不是 SQLite 的安全备份方式**。已加 `scripts/backup.ps1`（→ `backend-go/cmd/backup`）：对两个库执行 `VACUUM INTO '<带时间戳的目标>'`。
   实测佐证：磁盘上 `auth.db` **4096 B** → 安全快照 **126976 B**（差约 97%）。
2. **Go 零测试 — ✅ 阶段 1 已解决**
   原先 `backend-go/` 无任何 `_test.go`，而 `review_handlers.go`（681 行）承载全部复习调度与落库，多用户化要改它 → 先补测试。现已补 **40 项测试 / 5 个文件**（handlers 36 + routes 4，`go test ./...` 约 6 秒）：submit 的到期计算与落库、`stats` 今日口径（`today_new` = 当天 `stability_before = 0` 的日志数）、队列取数（new/due/queue/probes 的过滤与 limit/offset 边界）、`SetupRouter` 真实路由表（18 条 method+path）。
   做法：内存 SQLite（`mode=memory&cache=shared`）+ 生产同款 `AutoMigrate` + `httptest` 走 HTTP 层；**未改任何生产代码**、不 mock 时钟（跨天靠构造历史数据）。✅ `user` 维度的过滤测试已在 P0-1 补上（`handlers/review_isolation_test.go`，7 项：两用户同词各行、stats / new / due / queue / probes 互不可见、删词条清所有人、旧索引必须不存在）。
3. **无一键验证 — ✅ 阶段 0 已解决**
   原来没有 Makefile / npm scripts / CI；`scripts/` 只有 `clean-build-cache.ps1`（87 行，带白名单安全闸）；`backend-rust/scripts/smoke.ps1`（225 行 / 24 项）只管账号服务。
   已加 `scripts/verify.ps1`：Go 构建/vet/测试 + 账号服务 `cargo test` + 引擎 `cargo test` + `cargo check --target wasm32-unknown-unknown`，一条命令出 PASS/FAIL 表（实测 6 步全绿、45.3 秒），加 `-IncludeSmoke` 可带上账号服务 smoke。
4. **无 CI / 无 hook**：仓库无 CI 配置、`.git/hooks` 无自定义 hook。⚠️ GitCode 是否支持 CI 我**未确认**，需要时先查。
5. **部署全靠文档**：仓库里没有 nginx 配置 / systemd 单元 / Dockerfile，部署步骤只存在于 `README.md:692-801` 的代码块里。→ 建议入库一个 `deploy/` 目录（nginx 站配置 + systemd 单元模板），让环境可重建。
6. **文档口径漂移 — ✅ 阶段 0 已修**
   - 测试数已统一并实测：`backend-rust` **90 项**（41 单元 + 49 集成）、引擎 **118 项**。改的是 `docs/boundaries.md`（原先误记 118）与 `backend-rust/README.md`（原先过期记 83）；`docs/backend-auth.md` 的 90 本来是对的。
   - `DB_PATH` 已补进 `docs/backend-auth.md`（`config/config.go:22`）。
   - `docs/review-engine.md` 里「因为现在没有登录系统」的措辞已改为「复习侧尚未接入登录」，并链到本文第 3 节。

---

## 7. 非技术面：定位与投入产出

- **摊得太开**：9 门学科 + 双后端 + WASM 引擎 + 完整账号系统——技术栈的宽度已经超过一个人的维护带宽。理性的做法是**砍**：要么砍学科数（专注英语），要么砍账号系统的复杂度。
- **最有辨识度的资产不是「学科平台」**，而是 `modules/english/engine/` 那套间隔重复引擎：`session.rs` 2074 行 52 个测试，FSRS 调度 + 无限复习 + 抽卡与轮次规则，全部有测试锁住。学科平台这条路拼的是内容，引擎这条路拼的是算法与体验——**后者更适合一个人做**。
- 爱发电 + MIT 的组合适合「作品展示 + 少量捐赠」，**不适合承载运营成本**。若要长期跑，得接受它是个作品而不是生意。
- 因此建议把目标写成：**邀请制、几十人、留存导向，不追求规模**。

---

## 8. 待决策与建议节奏

**四件事已定案**（用户拍板：暂时 B，未来再考虑 C）

| # | 当时的问题 | 定案 | 影响 |
|---|-----------|------|------|
| 1 | 定位 A/B/C | **暂时 B（小范围邀请制），未来再考虑 C** | 技术前置 = 第 2 节的鉴权 + 第 3 节的进度按人隔离 |
| 2 | 未登录能不能复习 | **(i) 必须登录才能进入复习** | `user_id` 永远有值，模型最干净；(ii) 游客池与 (iii) 双取数路径**作废** |
| 3 | 先补 Go 测试再改数据模型 | **同意这个顺序** | 阶段 1 必须排在阶段 2/3 之前，**不可颠倒** |
| 4 | 扩科 | **先做英语多词书** | 不走「开新学科」路线；`main.js` 学科注册表改造暂缓（第 5 节） |

**进度记录**

- **阶段 0 已完成（2026-09）**
  - `scripts/verify.ps1` 落地并实跑：6 步全 **PASS**、用时 **45.3 秒**（Go 构建/vet/测试 + 账号服务 90 项 + 引擎宿主 118 项 + wasm32 目标检查）。
  - `scripts/backup.ps1` + `backend-go/cmd/backup/main.go` 落地并实跑：走 SQLite `VACUUM INTO`，**未新增依赖**。
  - **备份风险的实证**：磁盘上 `backend-rust/auth.db` 只有 **4096 B**，安全快照出来是 **126976 B** ——照旧「复制 `.db` 文件」会丢掉约 **97%** 的数据（第 6 节风险 1 由此从推测变成实测）。
  - 文档口径修正：测试数统一为「账号服务 90 项 / 引擎 118 项，别混用」（`docs/boundaries.md`、`docs/backend-auth.md`、`backend-rust/README.md` 三处）；`DB_PATH` 补进 `docs/backend-auth.md`；wasm 构建三段命令与 wasm-bindgen 绝对路径补进 `docs/review-engine.md`。
- **阶段 1 已完成（2026-09）**
  - `backend-go` 关键路径测试基线落地：`handlers/setup_test.go`（公用装置，内存库 + 生产 models + httptest）+ `review_submit_test.go`（14）+ `review_stats_test.go`（10）+ `review_queue_test.go`（12）+ `routes/routes_test.go`（4）= **40 项**，`go test ./...` 与 `go vet ./...` 全绿。
  - 顺带查出的「宽松口径」已写成**护栏测试**（行为未改）：submit 不校验 `Content-Type`、`json.Decoder` 忽略尾随多余字符、数值字段 `null` → 0；`/api/reviews/queue` 的 `?now=` 只回显、不参与取数；「每日 5 新词 + 5 抽查」配额纯在客户端 localStorage，服务端不记账（阶段 3 一并迁到服务端）。
- **阶段 2 已完成（2026-09）**
  - 服务端鉴权落地（第 2 节路径 (a)：共享密钥本地验签）：
    - `backend-go/middleware/auth.go`：Go 读**同一把** `AUTH_JWT_SECRET` 自验 HS256（`jwt.WithValidMethods` 只放 HS256、`WithExpirationRequired`、`WithLeeway(60s)` 与账号服务对齐），两级门槛 `RequireUser` / `RequireAdmin`（`role == "admin"`），外加 `CSRFGuard`（Origin 白名单，无 Origin 时要求 `Content-Type: application/json`）。
    - `routes.SetupRouter(db, cfg)` 改签名：`/api/reviews/*` 全部要登录、`/api/words` 的写接口要管理员、其余公开。
    - 删掉 Go 侧 `users` / `data_items` 两张表与 `/api/user/*`、`/api/data/*` 四条占位路由（与账号服务同名不同源，留着必被误用）；`models.User` / `DataItem` 与四个占位处理器一并移除。
    - 失败口径与账号服务逐字对齐：401 `{"code":401,"message":"请先登录","error":"unauthenticated"}`、403 `{"code":403,"message":"没有权限","error":"forbidden"}`。
  - 配置：`backend-go/config/config.go` 新增零依赖 dotenv 读取（`$GX_ENV_FILE` → `./.env` → `./backend-go/.env`，真实环境变量优先）＋ `backend-go/.env.example`；本机 `backend-go/.env` 与 `backend-rust/.env` 写入同一把 64 位随机密钥（**改密钥要同时改两份并重启两个服务**）。
  - 前端最小处理（未动任何动画）：`modules/english/english.js` 的 `fetchText`/`fetchJson`/submit 与 `admin/admin.js` 的 `apiFetch` 遇到 401 → 提示后跳 `account/?next=<当前页>`，403 只报错不跳。
  - 测试与验证：`backend-go` 从 40 项涨到 **53 项**（handlers 36 + middleware 10 + routes 7，新增鉴权/CSRF/路由回归）；`pwsh scripts/verify.ps1` 6 步全 PASS（5.7 秒）。
  - **新增 `scripts/verify-auth.ps1`（跨服务联调）**：用临时库起两个真服务（18080/18081，不碰真实库），实跑 13 项检查全 PASS、用时 7.9 秒 —— 账号服务发的 `gx_access` 在 Go 侧 200 通过、匿名 401 `unauthenticated`、普通用户写词条 403 `forbidden`、外站 Origin 403。**这是「共享密钥」这条设计唯一的端到端证据**（两侧单元测试各自 mock 密钥，发现不了不一致）。
- **上线计划单独成文（2026-10，见 [`launch-plan.md`](launch-plan.md)）**：用户确定「最近要上线、只上英语模块、暂时邀请制」，于是把「上线」拆成一份可执行的计划：P0 五件事 = 进度按人隔离 / 强制邀请码 / 真发邮件 / 公网部署 / 三级角色；管理面板（邀请码、用户、审计、看板）排 P1。
  - 对本文的影响：**第 3 节的「进度按人隔离」被提为上线前置**（计划里的 P0-1），其余（每日配额服务端化）仍在计划内但排后（P2）。
  - 顺带查清两条上线必改项：`backend-rust/src/service.rs:367` **带邀请码注册会直接变管理员**（要改成按码的等级赋值）；Rust 侧 SMTP 发信**已经实现**，只差 `AUTH_MAIL_MODE=smtp` 与 `AUTH_SMTP_*` 配置。
- **阶段 3 已完成一半（2026-10）**：**进度按人隔离**（上线计划里的 P0-1）已落地 —— `word_reviews` 唯一键改为 `(user_id, word_id)`、`review_logs` 加 `user_id`、handlers 16 处查询按 user 收口、新增 `cmd/migrate` 迁移工具（默认 dry-run，`-apply` 先自动快照）与启动自检（旧结构拒绝启动）；`backend-go` 测试 53 → **61 项**，`verify-auth.ps1` 13 → **18 项**（两用户复习同一个词后各自 `total_reviews=1`）。
  **剩下的是「每日配额从 localStorage 迁到服务端」**（上线计划里排在 P2；口径已定：今日新学 = 当天 `stability_before = 0` 的日志数、今日抽查 = 当天 `is_probe` 数）。
  ⚠️ 红线 5 只解除一半：**多用户隔离做了，但会话撤销仍未做**（登出 / 踢端之后，那张 access 令牌在有效期内仍能通过验签，见 `backend-go/middleware/auth.go` 的包注释），`/admin/` 与站点仍不要挂公网。

**落地后请回来更新本文**：把已完成阶段移到上面的进度记录，把新增风险补进第 6 节。
