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
| 通用后台（`admin/`） | 独立入口页（`/admin/`）：布局、侧栏导航、hash 路由、通用组件、**登录门禁（只放行 `role=admin`）** |
| 个人中心（`account/`） | 独立入口页（`/account/`）：身份卡 + 账号信息 + 登录中的设备 + 可用操作；未登录时显示登录 / 注册。**入口是站点左上角的头像**（点头像播一段扩散过场再进来），登录态是 httpOnly Cookie，页面靠 `GET /api/auth/me` 判断 |
| 后端（`backend-go/`） | Go 1.21 + Gin + **GORM/SQLite**（`database/`、`handlers/review_handlers.go`），入口 `main.go` |
| 账号系统（`backend-rust/`） | 独立 Rust 认证服务 `guangxue-auth`（axum + rusqlite，库 `auth.db`，端口 8081）：邮箱验证码**开放注册**（邀请码可选，**带邀请码注册即升级为 `admin`**）、多端登录、令牌轮换、会话与邀请码管理；**只处理 `/api/auth/*`** |
| 复习引擎（`modules/english/engine/`） | Rust/WASM crate `guangxue_wasm`：`session.rs`（会话编排）+ `fsrs_engine.rs`（FSRS 调度）+ `randomizer.rs`（随机器）+ `wordlist.rs`（词表解析）+ `card_view.rs`（词性拆分 / 例句高亮切分），浏览器内运行 |
| 部署 | Nginx 反代：`/` → 前端静态文件，`/api/auth/` → `localhost:8081`（Rust），`/api/` → `localhost:8080`（Go） |

### 复习引擎（词汇间隔重复）
- **职责边界（重要）**：`modules/english/english.js` 只做 DOM 渲染 / `fetch` / `localStorage` / 语音；**队列编排、游标、日期换算、FSRS 计算、进度统计全部在 `engine/`（Rust）里**，JS 不再保存卡片数组。
- **每日队列的编排口径**：一天的开局不是「把所有到期的抓过来」，而是三段拼接（`session.rs::plan_day`）：
  1. **新词**：每天固定 5 个（无放回随机抽；没有未学词时这段为空）
  2. **抽查**：每天固定 5 个，从「`due_at` 最远」的已学词里挑，最近 7 天抽过的不再抽。作用是防止只按到期时间出题时**间隔长的词永远轮不到**（长期不露面就是盲区）
  3. **复习区**：其余已学词，按 `due_at` **升序**（服务端 `ORDER BY due_at ASC` 保证）
- **无限复习（重要）**：三段的初始池子建好之后，**评完的卡会带着「now + 引擎算出的间隔」插回池子**（`insert_sorted`，池子始终按到期时间有序），所以池子永远不会空、**没有结束页**。想一直复习就一直复习。
  - `due_ms = now + max(interval_days*86400000, 600000)`：**最短 10 分钟的下限必须与服务端 `review_handlers.go` 的 `dueSeconds` 保持一致**，否则插回池里的排序位置会和服务端实际 `due_at` 有偏差。
  - 状态行显示「本轮已复习 N 张」（没有「剩余」这个概念了）；`showFinishMessage` 只在池子真的空时出现（词库没词）。
- **抽卡规则**（`ReviewSession::pick_index`）：① 池首还是「已过期 / 今日计划」的卡 → **严格最旧优先**，不打乱；② 池首全是未到期的 → 在**最靠前的 10 张**里随机抽一张；③ **本轮已经评过的卡不再抽**（它们的到期时间往往就落在最前面，不挡的话会来回循环），窗口里全是评过的就逐步放大窗口，保证本轮每张都能轮到；④ 池子比窗口还小（如只有 3 张）→ 放行。
- **一轮 = 池里每张卡都评过一次**：`check_round()` 在**评完一张之后**判定（不是抽到下一张时，否则最后一张刚出现就提示「过完一遍」），过完一轮时左下角小字闪一句「本轮已过一遍 · 可以继续」，2.6 秒后切回计划文案。
  - ⚠️ **顺序铁律**：`try_rate` 里必须**先 `check_round()` 再 `advance()`**。反过来的话，新一轮的第一张是在「整池都评过」的状态下抽的，候选被挡光后退化到兜底分支，刚评过的卡会隔 8 张就重复（踩过，`repeat_gap_holds_across_round_boundaries` 锁住）。
  - ⚠️ 「前 10 张随机抽」的窗口如果永远不往后放宽，**排在后面的卡会被饿死、轮次永远凑不齐**（踩过，`round_completes_even_when_some_cards_are_far_in_the_future` 锁住）。
- **分页取数**：池子永不空，所以「空了再取下一页」的旧条件再也触发不了。改成按**本轮进度**预取：本轮已评分数逼近池子总量（差 ≤ `PREFETCH_MARGIN`=5）时取下一页（`maybePrefetch`）。追加只进复习区，**新词与抽查不因翻页变多**。
- **抽查卡评分时按「新卡」重算**（丢掉原 stability/difficulty，天数按 0 算），等于重新体检：它的间隔会被压缩回几天，从而很快回来重新标定。请求体里带 `is_probe: true`，后端记进 `review_logs.is_probe`（**`stability_before` 仍是库里的真实旧值**，所以「今日新学」统计不会被污染）。日后做 FSRS 参数优化时要排除这批记录。
- **每日配额记在浏览器 localStorage**（`reviewDailyPlan`：日期 / 新词数 / 抽查数 / 各词的抽查时间）。⚠️ 因为现在**没有登录系统**，服务端只有一份共享词库：若在服务端按「每天 5 个」算，等于全站每天共放 5 个新词，你先学了别人就没得学。代价是换设备/清缓存会重置——等有登录再迁到服务端。
- **流程**：取 `/api/reviews/queue`（整库紧迫度序，分页）+ `/api/reviews/new`（新词候选）+ `/api/reviews/probes`（到期最远的候选）+ `/api/reviews/stats` 的 **JSON 原文** → `new ReviewSession(queueText, newText, probeText, planJson)`（`planJson` = `{new_limit, probe_limit, probed_ids, now_ms}`，两个 limit 是**今天还剩多少额度**）→ 渲染时 `current_json()` → 评分时 `rate(rating, Date.now())` 返回**可直接 POST 的请求体**（同时把卡按新到期时间插回池子）→ 后端写 `word_reviews`（due_at = now + 间隔）并记 `review_logs`。分页用 `session.append(queueText, planJson)`（**只追加复习区**）+ `pending_count()/seen_count()/rounds()` 判断时机。
- wasm 侧方法一览：`done()` 本轮已评分数 · `pending_count()` 池中待抽 · `seen_count()` 本轮已评过的不同卡数 · `rounds()` 过完的轮数 · `total()` 池中 + 手上 · `plan_json()` 今日计划规模 · `current_json(now_ms)` · `rate(rating, now_ms)` · `append(queueJson, planJson)`。
- 记忆状态 `{stability, difficulty}` 与 SQLite `word_reviews` 字段一一对应；间隔最短 10 分钟。
- 随机器：`random_sample(len, count, seed)` 抽新词 / 抽卡 · `random_seed()` 取系统种子；抽卡时用 `next_seed()`（LCG）逐次推进种子，保证同一种子下整轮可复现。
- `days_elapsed = floor((now - last) / 86400000)`（取 `last_review_at`，缺失回退 `due_at`，负值归零），在**评分那一刻**换算；时间串用 `js_sys::Date::parse` 解析（与改动前 `new Date(x).getTime()` 同一解析器）。纯计算层的排序只认毫秒（`QueueInput.due_ms` / `PlannedCard.due_ms`），这样宿主测试才能覆盖。
- ⚠️ **接口空结果返回 `"items": null`**（Go 的 nil 切片）：解析层必须容忍 null，否则「今天没有到期卡」这种正常状态会直接报错（已踩过一次）。
- ⚠️ `fsrs` 6.6.2 只公开 `next_states()`，内部一次算四个分支、**无单评分入口**，所以 `rate()` 是「算四个取一个」。
- 引擎是确定性算术、不依赖系统时钟；`fsrs` 的 rayon/getrandom 已通过 getrandom `wasm_js` 特性适配 wasm32。
- **复习 UI**：`modules/english/english.html` 含 `#reviewApp`，由 `english.js` 的 `initReviewApp()` 驱动（`main.js` 的 `initSubjectModule()` 动态 import）。界面为**极简全屏**风格：顶栏（沉浸模式开关 / 今日新学 / 今日复习 / 剩余待学 + 记忆元信息）、大字号单词 + 音标胶囊、底部操作区、**左下角计划小字**；揭晓后主例句目标词高亮 + 中文翻译，下面是**一条条释义块**（一个词性一块，可各带例句与译文）。键位：空格揭晓，`Q/W/E/R`（或 `1~4`）评分，`P` 读单词，`L` 读例句（`E` 被「一般」占用），`Esc` 切换沉浸模式。⚠️ `engine/pkg/` 由 wasm-bindgen 生成（已 gitignore），缺失时需在 `modules/english/engine/` 下重新构建，命令见 `modules/english/README.md`。
- **左下角计划小字**（`#studyPlan` / `#studyPlanText`，绝对定位在 `.study-foot` 左下、11px 极淡色、`pointer-events: none`）：计划阶段显示「新词 3/5 · 抽查 2/5」，计划区走完的那一刻**带动效切换**成「计划完成 · 进入复习阶段」（`revealUp` 同族的 `planOut` / `planIn`）。
  - ⚠️ 两个坑：① 分母（`state.planTotals`）**建会话时算一次后固定**，不能每次渲染重算，否则分子涨分母也涨（会出现「新词 1/4、2/5」）；② `setPlanText` 在文案未变时**直接返回、不要清理动画类**，`markPlanDone` 也**不刷界面**——否则换卡时那次渲染会把刚起步的切换动画掐断（踩过，表现为「动画没播」）。
- **沉浸模式**：进复习页默认给 `body` 挂 `is-immersive`（隐藏站点导航栏、内容区占满整屏），样式在 `main.css` 的通用规则 + `english.css` 自己的留白里，偏好存 `reviewImmersive`。**切学科时必须摘掉**，由 `main.js` 的 `teardownSubjectModule()` 调用模块导出的 `unmount()` 完成（框架级收口点，别把清理逻辑写回 `main.js` 各学科判断里）。
- **揭晓动效**：不是整块淡入，而是「显示答案」按钮缩小淡出 → 主例句 → 译文 → 各释义块依次错开浮现（块内高亮再用 `background-size` 从左往右扫出来）→ 按钮退场动画播完的**同一刻**四个评分按钮从原位置依次顶上来（浮起 + 由小变大）。**节拍时刻**在 `english.js` 的 `playRevealAnimation()`（`REVEAL_*` 常量，靠内联 `animation-delay` 下达；`REVEAL_OUT_MS` 是换人时刻），**动作定义**在 `english.css` 的 `revealUp` / `revealOut` / `ratingIn` / `hitSweep`；`renderCardNow()` 与 `unmount()` 会调 `cancelRevealAnimation()` 把换人定时器与收尾定时器都清掉。⚠️ 两条铁律：① **单词不参与任何动画**，揭晓时必须留在原位（`.study-stage` 用 `justify-content: flex-start` 而非 `center`，否则答案变高会把单词顶上去，实测 1 条释义 53px、3 条 159px）；② **必须先藏揭晓按钮再放评分按钮**，两者是底部操作区相邻的两个块，同时显示会让操作区变高、把单词顶上去。
- **一词多义**：`words.senses` 是**一列 JSON 文本**（`models.WordSenses`，实现了 `Value`/`Scan`/`MarshalJSON`），不单开子表——释义永远跟着词条走，省一次 join、少传 `id`/`word_id`。空值必须序列化成 `[]` 而非 `null`（`Scan` 里先重置为非 nil 空切片）；Rust 侧对应字段仍用 `Option<Vec<ApiSense>>` 兜一层。填了 `senses` 就用它，没填则引擎按 `meaning` 里的词性标签自动分块（`card_view::split_senses`，历史数据不用改）。

### 前端要点
- 导航栏 `rectangle` 内含头像 + 9 个导航项，字段 `data-page="<学科页路径>"`。
- `main.js` 用 **localStorage 缓存**（键前缀 `pageCache_`，30 天过期、自动清理；`CACHE_VERSION` 在结构或路径变更时整体失效），点击导航用 `fetch` 加载并缓存，默认展示英语。
- **学科页路径**：已有独立模块的学科写成 `modules/<学科>/<学科>.html`（当前仅英语）；其余 8 门仍是 `pages/<学科>.html` 占位（`<p>敬请期待</p>`）。
- **学科模块约定**：放在 `modules/<学科>/` 下并导出初始化函数，`main.js` 的 `initSubjectModule()` 按需动态 `import()`；**学科逻辑不得回流到 `main.js`**。
- 本地起站点用仓库根目录的 `dev-server.js`（Node 内置模块实现，静态文件 + `/api` 同源代理，等价线上 Nginx 形态）；不能直接双击 `index.html`（`file://` 下 `/api` 与 WASM 模块都会失败）。
- **头像 = 个人中心入口**（`index.html` 的 `#navAvatar`，样式在 `main.css` 的 `.rounded-square`）：入口位置已从右上角的文字链接改成左上角头像（**暂时方案**；原来的 `#navAccount` 文字入口与导航上的「退出」按钮都已撤掉，退出改在个人中心里做）。`main.js` 的 `renderAvatarState()` 问一次 `GET /api/auth/me`：已登录给头像挂 `is-signed` 点亮右下角绿点并把昵称写进 `title`，未登录只把文案改成「登录 / 注册 · 个人中心」；账号服务没起来时静默降级，头像照样能点。⚠️ 它是 `.rectangle` 的子元素，所以**沉浸模式下随导航栏一起隐藏**——复习页默认就是沉浸，那时要先点顶栏的「显示导航栏」才能看到头像。`.nav-items` 的 `right` 已从 190px 改回 30px，与头像的左侧留白对称。
- **点头像的过场**（`main.js` 的 `playAvatarZoom()` + `main.css` 的 `.avatar-zoom`）：以头像中心为圆心，用 `clip-path: circle()` 把一个圆放大到盖住四角（半径按窗口与头像位置实时算），然后跳 `account/?from=avatar`。⚠️ 两个坑：① 必须分两帧写**行内** `clip-path`，且第一帧要**临时把 transition 关掉**再强制重排，否则过渡起点会落在样式表的兜底值（圆心在屏幕正中），圆就从屏幕中心长出来了（实测踩过）；② 遮罩的渐变背景必须与 `account/account.css` 里 `body` 的背景**逐字一致**，跳到个人中心才看不出接缝。系统开启「减少动态效果」或不支持 `clip-path`（`CSS.supports` 探测）时直接跳转，不播过场。

### 管理后台
- **入口**：`admin/index.html`（本地 `/admin/`）。与学生站**完全独立**：不走 `main.js`、不使用学科页的 localStorage 缓存。
- **约定**：学科后台放 `modules/<学科>/admin/`，导出 `mount(container, ctx)`（可选 `unmount()`），再到 `admin/admin.js` 的 `SUBJECT_ADMINS` 登记一行；框架用动态 `import()` 按需加载。通用能力通过 `ctx` 注入：`api` / `toast` / `confirm` / `el` / `escapeHtml` / `setTitle`。
- **登录门禁已接入界面层**：`admin.js` 的 `checkAuth()` 已改为**异步**（调 `GET /api/auth/me`），`role=admin` 才渲染后台，否则只显示 `#adminAuthGate` 里的登录表单；顶栏 `#adminUser` 显示当前账号 + 退出登录。⚠️ 改这块要注意：`DOMContentLoaded` 里必须等 `checkAuth().then(...)` 再 `renderNav/route`。
- ⚠️ **服务端鉴权仍未补**：Go 侧 `/api/words` 写接口任何人都能直接调（`curl -X PUT /api/words/1` 就能改数据），**在补中间件之前不要把 `/admin/` 或站点部署到公网**（页面上常驻提示条写的就是这件事）。下一期用同一 `AUTH_JWT_SECRET` 验签 + 查 `auth.db` 会话。
- **英语后台**（`modules/english/admin/english-admin.js`）：词条列表（搜索 / 词书 / 单元 / 分页）、增删改查、**多释义编辑**（一个词性一块，每块可带自己的例句与译文；全空的行提交前会被丢掉，后端 `normalizeSenses` 再清一遍）、批量导入（粘贴 → `engine` 的 `parse_word_list` 解析 → 预览 → 前端每 200 条分批 POST）。导入字段按位置对应 **单词 / 音标 / 释义 / 例句 / 例句翻译**，多出的忽略；导入只填单条释义，多释义在编辑页补。
- **词条接口**：`GET/POST /api/words`、`GET/PUT/DELETE /api/words/:id`、`GET /api/word-options`。
  - `PUT` 是全量更新；改名撞车返回 409。
  - `DELETE` **会连带删除该词的 `word_reviews` 与 `review_logs`**（日志留着会让 stats 虚高）。改错别字用 `PUT`，别删了重建。
  - `POST /api/words` 查重**大小写不敏感**。
  - `word-options` 刻意不在 `/api/words/options`，避免与 `/api/words/:id` 通配路由冲突。
  - 词条带 `example_translation`（词条级例句翻译）与 `senses`（多释义数组）两个字段；两者都可空。`/api/reviews/due` 的 `dueCard` 里 `senses` 用 `models.WordSenses` 直接扫列，GORM 认 `sql.Scanner`，不需要额外 join。
- **复习调度接口**：`/api/reviews/due`（只给已到期的，仍在）、`/api/reviews/new`（未学词候选）、**`/api/reviews/queue`**（整库按 `due_at` 升序，含未到期，分页带 `total`）、**`/api/reviews/probes`**（`due_at` **倒序**，即「到期最远」的抽查候选）、`/api/reviews/submit`、`/api/reviews/stats`。
  - `submit` 请求体多一个 `is_probe`（默认 false），后端记进 `review_logs.is_probe`；`stability_before` 取库里真实旧值，**不要**改成引擎的输入状态，否则抽查会被统计成「今日新学」。

### 后端要点
- API 前缀 `/api`：`/health`、`/hello`、`/user/*`、`/data/*`。
- 用户/数据相关 handler 目前多为 TODO 占位（返回固定 JSON）。⚠️ `/api/user/*` 与账号系统无关（真正的账号接口是 Rust 侧的 `/api/auth/me`），且**Go 侧目前没有任何鉴权中间件**，`/api/words` 的写接口是公开的。
- 环境变量：`SERVER_HOST`（默认 `0.0.0.0`）、`SERVER_PORT`（默认 `8080`）、`APP_ENV`（默认 `development`）。

### 账号系统要点（`backend-rust/`）
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
- **前端入口**：个人中心 `account/`（身份卡 + 账号信息 + 设备列表 + 操作；本地开发会自动从 `/api/auth/dev/codes` 回填验证码；带 `?from=avatar` 进来时跳过身份卡的入场动效，与首页的扩散过场衔接；**登录 / 注册标签栏的选中高亮是会滑动的滑块** `.acc-tabs-thumb`——宽度用 `calc((100% - 16px) / 2)` 算、靠 `translateX(calc(100% + 8px))` 换位，不需要 JS 量像素；切标签时表单按点击方向用 `is-enter-right` / `is-enter-left` 从侧边滑入，并且**白卡高度会平滑延伸 / 回缩**（`animateCardHeight()`：量旧高 → 换内容 → 量新高 → 过渡到新高 → 收尾**必须清掉行内 height/overflow** 还原成自动高度，否则报错文案或窄屏换行撑高的内容会被裁掉；量新高前也要先清掉上一轮的行内高度，否则量到的是被 `overflow:hidden` 裁过的值；卡片还是 `hidden` 时不要量、不要播，交给入场动效））、后台门禁 `admin/`、站点左上角头像（点它进个人中心）。三处都只做界面层门禁，服务端判定仍是权威。
- 📌 **待处理问题统一记在根目录 `TODO.md`**：沉浸模式下的「返回」反向动画（暂缓，含三个候选方案）、头像 2.6MB 的缩略图（等用户点头，涉及素材）、登录后画面是临时的、转场时长的人工对齐。**接手账号/转场相关改动前先读它一遍**，别把已决定暂缓的事又当成漏掉的 bug。
- ⚠️ 本地没设 `AUTH_JWT_SECRET` 时每次重启都会随机生成密钥（旧令牌全失效）；`APP_ENV=production` 下必须显式提供它和管理员密码，并关闭 `AUTH_DEV_ENDPOINTS`，否则启动失败。

---

## 二、硬性边界（必须遵守）

1. **备份目录只读**：`备份/`（含 `备份1/`、`备份2/`）是存档副本，**永不修改、永不删除、永不作为建设对象**。读取/列出项目文件时一律跳过 `备份` 目录。
2. **`pages.zip`** 为二进制备份包，不作为文本读取、不修改、不展开，除非用户明确要求检查。
3. **`console.log('FAIL'`** 是一个**空文件**（0 字节），系误用重定向产生的残留：**不是合法代码，不要把它当代码，也不要试图“修复”它**；除非用户确认，不要删除，也不要修改。
4. **`go.sum` 已生成**：后端已有 `go.sum`（GORM + glebarez/sqlite 等依赖已通过 `go mod tidy` 固化）；新增依赖时用 `go mod tidy` 同步即可。
5. **`image/avatar.png`**：导航栏左上角的头像素材，**可以读、可以看**（助手已具备读图能力，`read_image` 直接读这个文件即可），但**不要擅自替换**——主题/素材更换一律先问用户。
   - **本体是真 PNG**（`89 50 4E 47` 文件头，颜色类型 6），**1330 × 1146**，约 **2.56 MB**（此前文档记的「WebP、199 KB」是读图工具生成的**归一化副本**格式，不是源文件，别照那个改文件扩展名）。
   - 内容：五人合影（特朗普 / 马斯克 / 中间戴墨镜穿灰夹克的男性 / 黄仁勋 / 库克，一起竖大拇指）。
   - 页面上的呈现：`index.html` 的 `.rounded-square`（`left: 30px`，垂直居中）内，**显示尺寸仅 64 × 64 px**，圆角 12px、3px 白边、`object-fit: cover`（源图左右各裁掉约 7%，五个人的脸都在框内）。`main.css` 的 `.avatar` 只负责填满容器。
   - ⚠️ **为什么它偏重**：64×64 的显示尺寸在用 1330×1146 的原图。`dev-server.js` 已经给图片发 `public, max-age=86400`（不再每次重下），但线上仍是 2.6MB 的首屏负担。优化方向是生成 128×128 缩略图给导航与个人中心用（原图保留），**涉及视觉素材，动手前必须先问用户** —— 详见根目录 `TODO.md` 第 2 条。
6. **本机工具链**：Go 已装为便携版 `C:\Users\22629\go-portable\go\bin\go.exe`（go1.27.1，已 `go env -w GOPROXY=https://goproxy.cn,direct GOSUMDB=off`，直接 `go build` 即可）；Rust `cargo 1.97` 且 `wasm32-unknown-unknown` target 已装；wasm-bindgen CLI 在 `C:\Users\22629\.local\bin\wasm-bindgen-0.2.128-*\wasm-bindgen.exe`（须与 Cargo.toml 的 wasm-bindgen 版本一致 0.2.128）。
   ✅ **原生（宿主）Rust 也能编译链接**：host 目标为 `x86_64-pc-windows-gnu`，mingw gcc/ar 已在 PATH 上，所以 `backend-rust/` 用 `rusqlite` 的 `bundled` 特性（现场编译 sqlite3.c）可以正常 `cargo test` / `cargo build --release`。链接时的 `corrupt .drectve at end of def file` 是 mingw 的无害告警。
   ✅ **宿主 `cargo test` 现在可以运行**（2026-09-13 实测 118 个测试通过；本文档此前记录的「缺 mingw `as`/MSVC SDK 无法链接」已不再成立）。完整验证路径：`cargo test` → `cargo check --target wasm32-unknown-unknown` → `cargo build --target wasm32-unknown-unknown --release` → `wasm-bindgen` 生成 `pkg/` → 浏览器端到端。
   ⚠️ 但 `JsValue` 在非 wasm32 目标上未实现（调用即 `panic: function not implemented on non-wasm32 targets`，无法 unwinding 会直接 abort）：**纯计算层不要碰 `JsValue`**，把它留在 wasm 导出方法的边界上。
7. **`word_reviews` 行是懒创建**：单词由 `POST /api/words` 写入 `words` 表；首次提交复习时才创建对应 `word_reviews` 行。`/api/reviews/new` = 无复习行的词。

---

## 三、代码风格约定

- 注释与面向用户文案使用**中文**。
- **注释按「代码块」写，不要逐句注释**：一个函数 / 组件 / 配置段在开头用一小段说明「这块做什么、为什么这么写、有什么坑」；块内不逐行解说，复述代码的注释（「// 获取元素」「// 返回结果」）与解释语言语法的注释一律不要。⚠️ 但**原因类说明必须留**（踩过的坑、跨模块口径、不变量、安全提醒）——压缩可以，丢信息不行，那是本项目注释的主要价值。
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
5. **Git 提交标注（重要）**：凡是由 **DeepSeek Harness**（AI 助手）执行的 `git commit`，提交信息末尾必须带一行归属尾注：

   ```
   <类型>: <一句话简述>

   <正文，可选>

   Committed-by: DeepSeek Harness
   ```

   - 位置：**空一行之后、作为提交信息的最后一行**（Git trailer 形式，便于 `git log --grep` 检索）。
   - **不改动仓库的 `user.name` / `user.email`**（本仓库为 `HR_RedSun <2262997289@qq.com>`），也**不要**臆造 `Co-authored-by` 的邮箱；署名归属只用上面这一行。
   - 多行提交信息用 `-m "标题" -m "正文" -m "Committed-by: DeepSeek Harness"` 或 heredoc 传入，不要用会破坏编码的管道写法。
   - 该约定只约束**由助手发起的提交**；用户手工提交不受影响。
   - 执行提交/推送后，在回复里**明确说明已提交并推送**，并给出分支名与提交号。

---

## 五、支持与许可

项目由作者在爱发电维护；许可证 MIT。涉及对外发布或引用时保留作者信息。
