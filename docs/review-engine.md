# 复习引擎（词汇间隔重复）

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `modules/english/engine/`（Rust/WASM）前请通读一遍。
> 界面与动画部分在 [`english-ui.md`](english-ui.md)；接口清单在 [`admin-api.md`](admin-api.md)。

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
- **每日配额记在浏览器 localStorage**（`reviewDailyPlan`：日期 / 新词数 / 抽查数 / 各词的抽查时间）。⚠️ 原因：**复习侧尚未接入登录**（服务端只有一份共享词库）——若在服务端按「每天 5 个」算，等于全站每天共放 5 个新词，你先学了别人就没得学。代价是换设备/清缓存会重置；**等复习接口接入账号（多用户化）后应迁到服务端**，改法见 [`roadmap.md`](roadmap.md) 第 3 节。
- **流程**：取 `/api/reviews/queue`（整库紧迫度序，分页）+ `/api/reviews/new`（新词候选）+ `/api/reviews/probes`（到期最远的候选）+ `/api/reviews/stats` 的 **JSON 原文** → `new ReviewSession(queueText, newText, probeText, planJson)`（`planJson` = `{new_limit, probe_limit, probed_ids, now_ms}`，两个 limit 是**今天还剩多少额度**）→ 渲染时 `current_json()` → 评分时 `rate(rating, Date.now())` 返回**可直接 POST 的请求体**（同时把卡按新到期时间插回池子）→ 后端写 `word_reviews`（due_at = now + 间隔）并记 `review_logs`。分页用 `session.append(queueText, planJson)`（**只追加复习区**）+ `pending_count()/seen_count()/rounds()` 判断时机。
- wasm 侧方法一览：`done()` 本轮已评分数 · `pending_count()` 池中待抽 · `seen_count()` 本轮已评过的不同卡数 · `rounds()` 过完的轮数 · `total()` 池中 + 手上 · `plan_json()` 今日计划规模 · `current_json(now_ms)` · `rate(rating, now_ms)` · `append(queueJson, planJson)`。
- 记忆状态 `{stability, difficulty}` 与 SQLite `word_reviews` 字段一一对应；间隔最短 10 分钟。
- 随机器：`random_sample(len, count, seed)` 抽新词 / 抽卡 · `random_seed()` 取系统种子；抽卡时用 `next_seed()`（LCG）逐次推进种子，保证同一种子下整轮可复现。
- `days_elapsed = floor((now - last) / 86400000)`（取 `last_review_at`，缺失回退 `due_at`，负值归零），在**评分那一刻**换算；时间串用 `js_sys::Date::parse` 解析（与改动前 `new Date(x).getTime()` 同一解析器）。纯计算层的排序只认毫秒（`QueueInput.due_ms` / `PlannedCard.due_ms`），这样宿主测试才能覆盖。
- ⚠️ **接口空结果返回 `"items": null`**（Go 的 nil 切片）：解析层必须容忍 null，否则「今天没有到期卡」这种正常状态会直接报错（已踩过一次）。
- ⚠️ `fsrs` 6.6.2 只公开 `next_states()`，内部一次算四个分支、**无单评分入口**，所以 `rate()` 是「算四个取一个」。
- 引擎是确定性算术、不依赖系统时钟；`fsrs` 的 rayon/getrandom 已通过 getrandom `wasm_js` 特性适配 wasm32。
- **一词多义**：`words.senses` 是**一列 JSON 文本**（`models.WordSenses`，实现了 `Value`/`Scan`/`MarshalJSON`），不单开子表——释义永远跟着词条走，省一次 join、少传 `id`/`word_id`。空值必须序列化成 `[]` 而非 `null`（`Scan` 里先重置为非 nil 空切片）；Rust 侧对应字段仍用 `Option<Vec<ApiSense>>` 兜一层。填了 `senses` 就用它，没填则引擎按 `meaning` 里的词性标签自动分块（`card_view::split_senses`，历史数据不用改）。

## 构建与测试

- **测试**：`modules/english/engine` 有 **118 项**宿主测试（`cargo test`，秒级）；账号服务那份「90 项」是另一个 crate，**别混用这两个数字**。
- **一键验证**：仓库根跑 `pwsh scripts/verify.ps1`（Go 构建/vet/测试 + 账号服务测试 + 引擎宿主测试 + wasm32 目标检查，全绿约 45 秒，只读不改仓库）。
- **`engine/pkg/` 由 wasm-bindgen 生成（已 gitignore）**，缺失时需在 `modules/english/engine/` 下重新构建，三段命令见
  [`../modules/english/engine/README.md`](../modules/english/engine/README.md)：`cargo check --target wasm32-unknown-unknown`
  → `cargo build --target wasm32-unknown-unknown --release` → `wasm-bindgen --target web --out-dir pkg --out-name guangxue_wasm
  target/wasm32-unknown-unknown/release/guangxue_wasm.wasm`。
  ⚠️ 本机 `wasm-bindgen` **不在 PATH**，实际在 `C:\Users\22629\.local\bin\wasm-bindgen-0.2.128-x86_64-pc-windows-msvc\wasm-bindgen.exe`；版本必须与 `Cargo.toml` 的 `wasm-bindgen = 0.2.128` 一致。
- 完整工具链与验证路径见 [`boundaries.md`](boundaries.md)。
