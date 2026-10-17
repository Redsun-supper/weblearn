# 复习引擎（词汇间隔重复）

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `modules/english/engine/`（Rust/WASM）前请通读一遍。
> 界面与动画部分在 [`english-ui.md`](english-ui.md)；接口清单在 [`admin-api.md`](admin-api.md)。

- **职责边界（重要）**：`modules/english/english.js` 只做 DOM 渲染 / `fetch` / `localStorage`（仅界面偏好）/ 语音；**池子排序、抽卡、日期换算、FSRS 计算、进度统计全部在 `engine/`（Rust）里**，服务端负责「整池怎么排 + 今日置顶哪 5 个」，JS 不再保存卡片数组、也不再记账。
- **单一循环池（2026-10-02 起，替代原来的「三段编排」）**：全库只有**一个循环池**，池子 = 整个词表。
  - 没有「已学 / 未学」之分：一个词的进度就是 `word_reviews` 里的一行（`stability`/`difficulty`/`due_at`/`reps`），没有行就是「从未复习」。
  - 服务端 `QueueReviews` 按**五桶**排好整池并分页返回，桶内规则见 [`review-pool-plan.md`](review-pool-plan.md)：桶 0 = 今日置顶抽样（按 (user, 当地日期, word) 的稳定哈希从**全池**抽，可能命中已学过的词；数量按用户复习量自适应）→ 桶 1 = 已过期 → 桶 2 = 顽固词（`lapses` 达阈值）→ 桶 3 = 从未复习 → 桶 4 = 未到期。
  - 决策与 20 条定案见 [`review-pool-plan.md`](review-pool-plan.md)；`/api/reviews/new` 已废弃（返回空列表 + `legacy: true`），`/api/reviews/probes` 降级成「池尾诊断」（只读，不参与编排）。
- ⚠️ **引擎侧的三段编排已彻底删除（2026-10-02 第二步清理，与上面「单一循环池」是同一次改造的两半）**：`plan_day` 以前会按 `new_limit` / `probe_limit` 把 `new` / `probes` 两个入参里的卡拼到池子最前、并按 `probed_ids` 跳过刚抽过的。前端一直传空数组 + 0，所以**界面上看不出问题，但代码路径是活的** —— 往里塞内容会凭空多出一批「既不在池子里、也不受四桶约束」的卡，破坏池子的排序承诺。
  - 现在 `plan_day` 只做一件事：**照抄队列顺序 + 标出来源**（`daily` → `Daily`，其余 → `Due`），`plan_len` / `new_count` / `probe_count` 一律为 0。
  - `new` / `probes` 形参与 `PlanOptions.new_limit` / `probe_limit` / `probed_ids` 仍然留在签名里（接口冻结，前端还在传），但**完全不参与编排**（形参已改名带下划线）。
  - `legacy_new_and_probe_inputs_are_completely_ignored` 与 `plan_is_only_the_queue_order` 把这一点钉死：塞满废弃输入后，池子张数、`plan_json()`、首张卡都必须与「空数组 + 0」的干净调用完全一致。
  - `CardSource::New` / `Probe` 两个变体保留（历史响应的 `source` 值仍要能表达），但引擎**不再产出**它们。
- **无限复习（重要）**：池子建好之后，**评完的卡会带着「now + 引擎算出的间隔」插回池子**（`insert_sorted`，池子始终按到期时间有序），所以池子永远不会空、**没有结束页**。想一直复习就一直复习。
  - `due_ms = now + clamp(interval_days*86400000, 600000, 365*86400000)`：**下限 10 分钟、上限 365 天两个口径都必须与服务端 `review_handlers.go` 保持一致**（`MIN_INTERVAL_MS` / `MAX_INTERVAL_MS` ↔ `dueSeconds` / `maxIntervalDays`），否则插回池里的排序位置会和服务端实际 `due_at` 有偏差。
  - 上限 365 天是 2026-10-02 新增的（用户决策 B8）：FSRS 连续 Easy 会把间隔推到几年，那正是「记得牢的词再也不出现、无法无限学下去」的根源。
  - 状态行显示「本轮已复习 N 张」（没有「剩余」这个概念了）；`showFinishMessage` 只在池子真的空时出现（词库没词）。
- **抽卡规则**（`ReviewSession::pick_index`）：① 池首还是「已过期」的卡 → **严格最旧优先**，不打乱；①b **还没评过的今日置顶卡一张不落地先出完**（见下条）；② 池首全是未到期的 → 在**最靠前的 10 张**里随机抽一张；③ **本轮已经评过的卡不再抽**（它们的到期时间往往就落在最前面，不挡的话会来回循环），窗口里全是评过的就逐步放大窗口，保证本轮每张都能轮到；④ 池子比窗口还小（如只有 3 张）→ 放行。
  - ⚠️ ①b 是 2026-10-02 端到端抓到的缺陷修补：只靠 ② 的窗口时，10 张里混着大量「从未复习」的普通卡，实测连评 7 张只碰到 2 张置顶卡，「今日置顶 N/5」停在 4/5 —— 用户以为今天的 5 个没给全。修法是**不给置顶卡随机**：服务端已经用稳定哈希在它们之间排好了随机顺序（`dailyWordIDs`），引擎按池子里的相对顺序取第一张即可。`all_daily_cards_come_out_before_the_window_mixes_them` 锁住这条。
  - 只对 `current.is_none()`（开局抽第一张）时生效：之后置顶卡会一直待在池首，每抽一张自然就取走一张，不需要每个回合都做这段扫描。
- **一轮 = 池里每张卡都评过一次**：`check_round()` 在**评完一张之后**判定（不是抽到下一张时，否则最后一张刚出现就提示「过完一遍」），过完一轮时左下角小字闪一句「整池已过一遍，继续复习不受限」，2.6 秒后切回进度文案。
  - ⚠️ **顺序铁律**：`try_rate` 里必须**先 `check_round()` 再 `advance()`**。反过来的话，新一轮的第一张是在「整池都评过」的状态下抽的，候选被挡光后退化到兜底分支，刚评过的卡会隔 8 张就重复（踩过，`repeat_gap_holds_across_round_boundaries` 锁住）。
  - ⚠️ 「前 10 张随机抽」的窗口如果永远不往后放宽，**排在后面的卡会被饿死、轮次永远凑不齐**（踩过，`round_completes_even_when_some_cards_are_far_in_the_future` 锁住）。
- **分页取数**：池子永不空，所以「空了再取下一页」的旧条件再也触发不了。改成按**本轮进度**预取：本轮已评分数逼近池子总量（差 ≤ `PREFETCH_MARGIN`=5）时取下一页（`maybePrefetch`）。追加只进池子（`append` 只吃一个 queue 参数）。
- **置顶卡走正常复习（2026-10 起，不再重置）**：它和别的卡一样用自己真实的 `stability` / `difficulty` 累积记忆状态，提交时带 `is_daily`（只用于日志留痕）。
  - 早先的做法是「置顶卡丢掉状态、按新卡重算」（用户决策 C11），目的是不让循环池里的重学把「今日新学」冲高 —— 但那等于用摧毁记忆状态去凑一个统计口径。现在改成：日志照记真实旧值，统计只看 `stability_before = 0`（该用户此前从没复习过这个词就是 0），口径天然正确，日志也能用于将来的参数拟合。
  - ⚠️ **卡片必须拿到自己真实的 `stability` / `difficulty`**：服务端 `poolSelectSQL` 从 2026-10 起下发这两列（`wr.stability, wr.difficulty`）。它们曾经被删掉（用户决策 E18），后果是**每一张卡**都退回「新卡」路径 —— 四个评分恒为 Again 0.2120 / Hard 1.2931 / Good 2.3065 / Easy 8.2956 天、换会话逐位相同，B8 的 365 天封顶与右上角元信息面板一起变成死代码。
  - 旧字段 `is_probe` / `is_reset` 仍留在请求体与日志里（兼容旧客户端与历史数据），但引擎不再据此重置状态；`is_daily` 才是新口径的标记。
  - 顽固词（`lapses >= 8`）由服务端算好下发（池卡带 `leech`），引擎把它透传到卡片元信息，界面打「顽固词」标。
- **今日置顶进度**：分母来自池子响应的 `daily`（服务端算的），分子 = **建会话时的服务端值（基线）+ 本会话本地计数**（`state.dailyBaseline` + `state.dailyRatedSinceBaseline`，评分成功时 +1）。
  - ⚠️ 不要退化成「一个计数器 + 与服务端取大」：两者不是同一个量，取大必然**虚高** —— 实测连评 6 张时界面报 5/5「整池已过一遍」而服务端才 3/5，用户被提前告知完成。增量式写法没有这个缝：基线只在建会话 / 每次 stats 刷新时重设（`syncDailyFromStats`），本地只在评分成功时 +1。
  - ⚠️ **每次 stats 拿到手都要走 `syncDailyFromStats`**（它顺手把 `state.stats` 也更新了）。早先 `fetchDay()` 把新统计直接返回给调用方、自己没存，于是建会话时那次同步读到的仍是**上一次会话**的旧快照：实测界面从「今日置顶 2/5」起步而服务端其实是 0/5。
  - ⚠️ `markDailyRated` 必须在 `session.rate()` **之后**、`advanceCard()` **之前**：换卡过渡里会 `renderPlanProgress()`，晚一步记就会让左下角停在旧分子上（实测「评完还是 2/5 不涨」）。
  - ⚠️ 判「这张是不是置顶卡」要用**引擎返回的请求体里的 `source` / `is_daily`**，不要读界面上的 `state.card.source`：换卡过渡期间那可能已经指向下一张（或首张卡还没赋值），计数会漏一张（实测界面 5/5 而服务端 4/5）。`SubmitPayload.source` 就是为此加的，服务端解析请求体时忽略这个未知字段。
- **配额已全部迁到服务端**（用户决策 D15）：本地 `reviewDailyPlan` 这个键不再读写（旧值留着无害，可手动清）。改法是服务端 `dailyWordIDs(userID, seed)` + `countTodayDaily(...)`，种子 = (user_id, 当地日期, word_id) 的稳定哈希。
- **流程**：取 `/api/reviews/queue?limit=100&offset=N`（整池五桶序，分页）+ `/api/reviews/stats` 的 **JSON 原文** → 先 `parseQueueMeta` 拿到服务端下发的 `daily` 与 `desired_retention`，再 `new ReviewSession(queueText, '{"data":{"items":[]}}', '{"data":{"items":[]}}', planJson)`（后两个位置参数是已废弃的新词 / 抽查候选，位置不能省；`planJson` = `{new_limit: 0, probe_limit: 0, probed_ids: [], now_ms, desired_retention}`）→ 渲染时 `current_json()` → 评分时 `rate(rating, Date.now())` 返回**可直接 POST 的请求体**（同时把卡按新到期时间插回池子）→ 后端写 `word_reviews`（due_at = now + 间隔）并记 `review_logs`。分页用 `session.append(queueText, planJson)` + `pending_count()/seen_count()/rounds()` 判断时机。
  - ⚠️ **「今天」由服务端定**（2026-10 起）：前端**不再传** `?today`，服务端按 `Asia/Shanghai` 算「今日置顶抽样」的哈希种子。`?today=YYYYMMDD` 降级为**显式覆盖**（只给测试 / 排障用，例如 Go 单测固定 `poolTestDay`）。早先前端总带浏览器的本地日期，浏览器与服务端不在同一时区时两边会算出不同的「今天」，跨零点前后出现两套置顶批次。
  - ⚠️ **`desired_retention`（期望记忆保持率）由服务端下发**：来自 `GX_DESIRED_RETENTION`（缺省 0.9），随队列响应返回，前端透传进 `planJson`，引擎在 `build` 时采用（缺省 / 非法值回落 0.9）。评分时引擎把它一并写进提交体，后端记进 `word_reviews.desired_retention`（供将来参数拟合对样本）。⚠️ 因此**必须先 `parseQueueMeta` 再建会话**：早先是「先建会话、后解析元信息」，引擎只能拿到前端写死的 0.9。
  - ⚠️ 改 `fetchDay` 的请求清单时，**`results` 的下标必须同步改**（现在是 `[池子, 统计]`；旧版四个请求时统计在 `results[3]`）。漏改的表现是「统计永远是 0，但页面不报错」（踩过）。

- wasm 侧方法一览：`done()` 本轮已评分数 · `pending_count()` 池中待抽 · `seen_count()` 本轮已评过的不同卡数 · `rounds()` 过完的轮数 · `total()` 池中 + 手上 · `plan_json()` 池子规模 · `current_json(now_ms)` · `rate(rating, now_ms)` · `append(queueJson, planJson)`。
- 记忆状态 `{stability, difficulty}` 与 SQLite `word_reviews` 字段一一对应；间隔 ∈ [10 分钟, 365 天]，由稳定度与 `desired_retention`（期望保持率）共同决定 —— 期望保持率越高，间隔越短。
- `CardSource` 四个变体：`Due`（普通池卡）· `Daily`（今日置顶抽样，服务端带 `is_daily`）· `New`/`Probe`（旧口径遗留，新模型下服务端不再下发；`New` 只在旧队列响应里出现）。置顶卡与普通卡一样走正常复习，区别只在「来源」与日志里的 `is_daily` 标记。
- 随机器：`random_sample(len, count, seed)` 抽新词 / 抽卡 · `random_seed()` 取系统种子；抽卡时用 `next_seed()`（LCG）逐次推进种子，保证同一种子下整轮可复现。
- `days_elapsed = floor((now - last) / 86400000)`（取 `last_review_at`，缺失回退 `due_at`，负值归零），在**评分那一刻**换算；时间串用 `js_sys::Date::parse` 解析（与改动前 `new Date(x).getTime()` 同一解析器）。纯计算层的排序只认毫秒（`QueueInput.due_ms` / `PlannedCard.due_ms`），这样宿主测试才能覆盖。
- ⚠️ **接口空结果返回 `"items": null`**（Go 的 nil 切片）：解析层必须容忍 null，否则「今天没有到期卡」这种正常状态会直接报错（已踩过一次）。
- ⚠️ `fsrs` 6.6.2 只公开 `next_states()`，内部一次算四个分支、**无单评分入口**，所以 `rate()` 是「算四个取一个」。
- 引擎是确定性算术、不依赖系统时钟；`fsrs` 的 rayon/getrandom 已通过 getrandom `wasm_js` 特性适配 wasm32。
- **一词多义**：`words.senses` 是**一列 JSON 文本**（`models.WordSenses`，实现了 `Value`/`Scan`/`MarshalJSON`），不单开子表——释义永远跟着词条走，省一次 join、少传 `id`/`word_id`。空值必须序列化成 `[]` 而非 `null`（`Scan` 里先重置为非 nil 空切片）；Rust 侧对应字段仍用 `Option<Vec<ApiSense>>` 兜一层。填了 `senses` 就用它，没填则引擎按 `meaning` 里的词性标签自动分块（`card_view::split_senses`，历史数据不用改）。

## 构建与测试

- **测试**：`modules/english/engine` 有 **123 项**宿主测试（`cargo test`，秒级）；账号服务那份「90 项」是另一个 crate，**别混用这两个数字**。
- **一键验证**：仓库根跑 `pwsh scripts/verify.ps1`（Go 构建/vet/测试 + 账号服务测试 + 引擎宿主测试 + wasm32 目标检查，全绿约 35 秒，只读不改仓库）。
- **单一循环池验收**：`pwsh scripts/verify-pool.ps1 -Browser`（**45 项**，走真实 HTTP + 真实开发库 + Edge 无头浏览器端到端，**会清空进度并自动快照**，所以可重复运行）；账号与权限回归是 `pwsh scripts/verify-auth.ps1`（**55 项**，起临时库与临时端口，不碰开发库）。
  - `-Browser` 那段跑的是 `scripts/browser-e2e.mjs`（CDP 驱动 Edge 无头），它**必须**走 `/index.html` 这个 SPA 外壳：直接开 `/modules/english/english.html` 只有 HTML 片段、没有 `main.js`，按钮点了没反应。
  - 反复跑会撞上**登录限流**（`backend-rust/src/config.rs` 的 `AUTH_RL_LOGIN_IP`；P2 起默认已放宽到 **200 次 / 15 分钟**，此前是 20 次 —— 上百人共用出口 IP 时那个值会被自己人打满），表现为整片 429；计数器在 Rust 进程内存里，**重启账号服务即清零**（见 [`boundaries.md`](boundaries.md) 第 10 节）。
- **`engine/pkg/` 由 wasm-bindgen 生成（已 gitignore）**，缺失时用一条命令重建：`pwsh scripts/build-engine.ps1`
  （cargo build → wasm-bindgen → 写版本号，见下一条）。要手工跑三段命令也行，见
  [`../modules/english/engine/README.md`](../modules/english/engine/README.md)：`cargo check --target wasm32-unknown-unknown`
  → `cargo build --target wasm32-unknown-unknown --release` → `wasm-bindgen --target web --out-dir pkg --out-name guangxue_wasm
  target/wasm32-unknown-unknown/release/guangxue_wasm.wasm`。
  ⚠️ 本机 `wasm-bindgen` **不在 PATH**，实际在 `C:\Users\22629\.local\bin\wasm-bindgen-0.2.128-x86_64-pc-windows-msvc\wasm-bindgen.exe`；版本必须与 `Cargo.toml` 的 `wasm-bindgen = 0.2.128` 一致（构建脚本会替你校验，不一致直接报错）。
- ✅ **引擎版本号不再手写**（2026-10 起）：`pwsh scripts/build-engine.ps1` 会把产出 wasm 的 SHA-256 前 12 位
  写进 `pkg/engine-version.js`，`english.js` 读它、拼进 `.js` 与 `_bg.wasm` 两个 URL。
  此前是人工把 `modules/english/english.js` 的 `ENGINE_VERSION` 加一，忘了的症状是「改了引擎在页面上毫无反应、
  且不报任何错」（`dev-server.js` 给 `.wasm` 发 `public, max-age=86400`，一天强缓存）。
  `pwsh scripts/verify.ps1` 里有一道一致性检查专门抓「绕过脚本重建 pkg/」。细节见 [`boundaries.md`](boundaries.md) 第 8 节。
- 完整工具链与验证路径见 [`boundaries.md`](boundaries.md)。
