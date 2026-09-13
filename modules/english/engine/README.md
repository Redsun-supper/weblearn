# guangxue_wasm — 广学英语模块引擎（Rust → WebAssembly）

位置：`modules/english/engine/`。编译目标 `wasm32-unknown-unknown`，通过 `wasm-bindgen`
导出给浏览器中的 `english.js` 调用。

## 职责

英语模块里**所有非 DOM 的计算**都在这里：

| 文件 | 职责 |
|------|------|
| `session.rs` | **会话编排（主入口）**：队列构建、记忆上下文换算、评分决策、进度统计、卡片输出 |
| `fsrs_engine.rs` | FSRS 记忆调度计算：单卡状态推进（`compute_next_states`）、可提取率 |
| `randomizer.rs` | 随机器：XorShift32 + Fisher–Yates 洗牌 / 无放回抽样 / 系统种子 |
| `wordlist.rs` | 词表文本解析（后台批量导入：制表符 / 竖线 / 逗号 / 空格，容错规则可测） |
| `card_view.rs` | 卡片文本：多释义拆分（`split_senses`，一个词性一块）与例句高亮切分（`split_example`） |

JS 侧只负责 DOM 渲染、事件绑定、`fetch`、`localStorage` 与语音合成——
队列与游标由 Rust 持有，**JS 不再保存卡片数组**。

## 导出 API（wasm-bindgen）

### 主入口：`ReviewSession`

```js
import init, { ReviewSession } from './engine/pkg/guangxue_wasm.js';
await init(); // wasm-bindgen --target web 产物必须先实例化

// 用四份接口返回的 JSON 原文建会话（内部完成编排、抽样、时间解析）
//   queueText：/api/reviews/queue  —— 整库按紧迫度升序（含未到期）
//   newText  ：/api/reviews/new    —— 新词候选
//   probeText：/api/reviews/probes —— 到期最远的候选（每日抽查用）
//   planText ：{new_limit, probe_limit, probed_ids, now_ms}
//              ⚠️ 两个 limit 是「今天还剩多少额度」，不是每日上限本身；
//              额度记在浏览器 localStorage 里（没有登录系统，服务端分不清是谁的）
const session = new ReviewSession(queueText, newText, probeText, planText);

// 队列总张数（池中待抽 + 手上这一张）；无限复习下它不会归零
session.total();
session.done();              // 本轮已评分张数（只增）
session.pending_count();     // 池中还有多少张没抽
session.seen_count();        // 本轮已经评过多少张**不同**的卡（前端用它判断该不该取下一页）
session.rounds();            // 已经过完整轮的次数（每张卡都评过一次 = 一轮）
session.is_finished();       // 池子空了才会是 true（正常词库下不会）
session.progress_percent();  // 0~100（无限复习下意义有限，保留给调试）
session.current_word_id();   // 当前词条 id（无卡时 0）
session.plan_json();         // {"new_target":5,"probe_target":5,"plan_len":10}
// 计划区长度：done() >= plan_len 就说明今日计划做完了、进入复习阶段（前端据此切左下角文案）

session.current_json(Date.now());
// 当前卡片展示数据：
//   word / phonetic / source（"new" | "probe" | "due"）
//   example_parts  主例句按目标词切分的片段（空 = 该词条没有例句，前端整块不渲染）
//   example_translation  主例句的中文翻译（空则不输出该字段）
//   senses         释义块数组 [{pos, meaning, example_parts?, translation?}]，有释义时至少一块
//   meta           记忆元信息（难度/稳定性/复习次数/距上次天数/预计记住；抽查卡的 status 是 "probe"）
// 说明：卡片 JSON 刻意不带冗余字段（meaning/example 原文、下标等），多释义本来就比单词条重，
//       JS 那边也不再需要它们。
//
// senses 的来源有两条：
//   1. 词条填了结构化多释义（后端 words.senses）→ 直接用，每块可带自己的例句与译文；
//   2. 没填 → 把 meaning 按词性标签自动拆开（"n. 好处；有益于 v. 有益于" → 两块），
//      此时例句只有词条级那一条，显示在顶部。
// 两件事都在 card_view::split_senses / build_senses 里，纯计算、有单元测试兜住。

// 评分：1=Again 2=Hard 3=Good 4=Easy；now_ms 传 Date.now()
const body = session.rate(3, Date.now());
// body 就是 POST /api/reviews/submit 的请求体：
// {"word_id":42,"rating":3,"stability":2.3065,"difficulty":2.1181,"interval_days":2.3065,"is_probe":false}
// ⚠️ 抽查卡（source === "probe"）在引擎里**按新卡重算**：丢掉原 stability/difficulty、天数按 0 算，
//    请求体里 is_probe 为 true。所以一张稳定到 60 天的卡被抽查时，Good 也只给 2.3065 天。
// ⚠️ 评分不只是算状态：它还会把这张卡**按新的到期时间插回池子**（见下面的「无限复习」）。

// 取复习区的下一页塞进池子（按到期时间插好），返回实际新增数量
const added = session.append(queueText, planText);
// ⚠️ 只追加**复习区**：新词与抽查受每日配额限制，不会因为翻页而变多

session.free();  // 离开英语页时释放（wasm-bindgen 生成）
```

`append` 的去重规则：跳过 id 已经**在池子里或手上**的卡片（防竞态导致同一张卡进池两次）。

另有 `ReviewSession.with_seed(queueJson, newJson, probeJson, planJson, seed)` 与
`append_with_seed(..., seed)`：显式指定随机种子，便于复现与测试。

### 无限复习：池子 + 抽卡规则

会话内部不再是「一次性队列 + 游标」，而是**一个始终按到期时间升序的池子**：

```text
评完一张 ──► 用「now + 引擎算出的间隔」算出新到期时间
          ──► insert_sorted() 按顺序插回池子（池子始终有序）
          ──► 抽下一张
```

池子因此永远不会空，也就没有「结束页」。抽卡规则（`ReviewSession::pick_index`）：

1. **池首是「已过期 / 今日计划」的卡** → 严格按最旧优先，不打乱
2. **池首全是未到期的** → 在**最靠前的 `CHUNK_SIZE`（10）张**里随机抽一张
3. **本轮已经评过的卡不再抽**（它们的到期时间往往就落在最前面，不挡的话会来回循环）；
   窗口里全是评过的就把窗口逐步放大，保证本轮每张都能轮到
4. 池子比保护窗口还小（例如只有 3 张）→ 放行，保证永远有卡可出

「同一张卡至少隔多久重复」由第 3 条保证：至少要等到下一轮（词库 ≥ 10 张时一轮至少那么长）。
**一轮 = 池里每张卡都评过一次**，过完一轮 `rounds()` +1，前端据此提示「本轮已过一遍」。

> ⚠️ 两个踩过的坑，各有一条单元测试锁住：
> - `round_completes_even_when_some_cards_are_far_in_the_future`：
>   第 2 步的窗口如果永远不往后放宽，排在后面的卡会被**饿死**，轮次永远凑不齐。
> - `repeat_gap_holds_across_round_boundaries`：
>   `try_rate` 里必须**先结算轮次再抽下一张**；反过来的话，新一轮的第一张是在
>   「整池都评过」的状态下抽的，候选被挡光后退化到兜底分支，刚评过的卡会隔 8 张就重复。

⚠️ **间隔下限要与服务端一致**：插回池子的到期时间是 `now + max(interval_days*86400000, 600000)`，
其中 600000（10 分钟）对应 `backend-go/handlers/review_handlers.go` 的 `dueSeconds`。
两处口径不一致会让池子里的排序位置与服务端实际 `due_at` 出现偏差。

### 纯计算层：`plan_day` / `insert_sorted`

```rust
// 编排今日的初始池子：新词 → 抽查 → 复习区（按到期时间升序，不打乱）
pub fn plan_day(queue: Vec<QueueInput>, new: Vec<ApiCard>, probes: Vec<ApiCard>,
                opts: &PlanOptions, seed: u32) -> DayPlan;

// 按到期时间插回池子（池保持升序；相同到期时间保持相对顺序）
fn insert_sorted(pool: &mut Vec<PlannedCard>, card: PlannedCard);

// 复习区翻页追加（只做复习区，不碰新词与抽查）
impl ReviewSession { pub fn append_page(&mut self, queue: Vec<QueueInput>, now_ms: f64, seed: u32) -> u32; }
```

`QueueInput` / `PlannedCard` 都带 `last_ms`（评分时算天数）与 `due_ms`（排序），
两者都是**已解析好的毫秒**——纯计算层不碰时间字符串，宿主测试才能覆盖排序与抽卡逻辑。
`PlannedCard.due_ms` 为 `None` 表示「今日计划卡」（新词 / 抽查），排序时按 −∞ 处理，排在最前面。

### 低层 API（仍在导出，供引擎复用与调试）

```js
// FSRS：一次性返回四种评分的下一状态
// state_json：{"stability":..,"difficulty":..}；空串/null = 新卡
fsrs_next_states(state_json, desired_retention, days_elapsed)
// → {"again":{"memory":{...},"interval_days":..},"hard":{...},"good":{...},"easy":{...}}

fsrs_retrievability(state_json, days_elapsed) // 当前记忆可提取率（0~1）

random_indices(len, seed)          // 0..len 的随机排列（复习顺序）
random_sample(len, count, seed)    // 无放回随机抽 count 个索引（新词批次）
random_seed()                      // 系统随机 32 位种子
```

## 结构

```
modules/english/engine/
├── Cargo.toml          # cdylib + rlib；fsrs + wasm-bindgen + js-sys + serde
├── Cargo.lock
├── src/
│   ├── lib.rs          # 模块导出
│   ├── session.rs      # 会话编排（ReviewSession / plan_session / days_elapsed / build_senses）
│   ├── fsrs_engine.rs  # FSRS 调度计算（CardState / NextStatesOut）
│   ├── randomizer.rs   # 随机器
│   ├── wordlist.rs     # 词表文本解析（parse_word_list）
│   └── card_view.rs    # 多释义拆分 + 例句高亮切分
├── pkg/                # wasm-bindgen 产物（已 gitignore，需自行生成）
└── target/             # cargo 构建缓存（已 gitignore）
```

## 构建

```powershell
cd modules/english/engine

# 类型检查
cargo check --target wasm32-unknown-unknown

# 生成 .wasm
cargo build --target wasm32-unknown-unknown --release
# 产物：target/wasm32-unknown-unknown/release/guangxue_wasm.wasm

# 生成 JS 胶水（wasm-bindgen-cli，版本须与 Cargo.toml 的 wasm-bindgen 一致：0.2.128）
wasm-bindgen --target web --out-dir pkg --out-name guangxue_wasm `
  target/wasm32-unknown-unknown/release/guangxue_wasm.wasm
# 产物：pkg/guangxue_wasm.js + pkg/guangxue_wasm_bg.wasm
```

> ⚠️ `pkg/` 与 `target/` 都被 gitignore，**克隆仓库后必须自己构建一次**，
> 否则英语页会提示「复习功能加载失败」。
>
> 部署时 `pkg/` 必须随静态文件一起提供。

## 单元测试

```powershell
cargo test            # 118 个测试：每日计划 / 抽卡规则 / 插回池子 / 轮次 / 天数换算 / 解析 / FSRS / 多释义 / 词形匹配
cargo test -- --nocapture
```

测试全部跑在**纯计算层**，不依赖浏览器：

- `plan_day`（新词→抽查→复习区的顺序、额度受限、抽查跳过刚抽过的、池子严格有序、计划卡排最前）
- `pick_*`（过期严格最旧优先、未到期只在前 10 张里抽、10 张内不重复、池子太小则放行）
- `rate_reinserts_*`（评完插回池子、按新到期时间排序、间隔下限 10 分钟、记忆状态同步更新）
- `rounds_*` / `repeat_gap_*`（轮次判定、跨轮边界的重复保护、窗口放宽防止饿死）
- `append_*`（插页、去重、只追加复习区、空池子追加后能出题）
- `days_elapsed` / `progress_percent` / `pick_branch` / `parse_items`（含 `"items": null` / `"senses": null`）
- `split_senses` / `split_example` / `word_forms` / `parse_word_list_core`

> **为什么要有「纯计算层」这一层**：`JsValue` 在非 wasm32 目标上并未实现
> （调用即 `panic: function not implemented on non-wasm32 targets`，且无法 unwinding，
> 会直接 abort 测试进程）。所以 wasm 导出方法只做两件事——把 ISO 时间字符串解析成毫秒、
> 把 `String` 错误转成 `JsValue`；业务逻辑一律放在不碰 `JsValue` 的纯方法里。

## 两点说明

1. **`fsrs` 没有单评分入口**。`fsrs` 6.6.2 只公开 `next_states()`，内部用
   `(1..=4).map(..)` 一次算完 Again/Hard/Good/Easy 四个分支。因此 `rate()` 仍是
   「算四个取一个」；每个分支只是几十次浮点运算，开销可忽略，不值得为省这点计算
   去重写 FSRS 公式（会造成算法重复与版本漂移风险）。

2. **时间解析用 JS 引擎的 `Date.parse`**（经 `js_sys::Date`）。这与改动前
   `new Date(x).getTime()` 是同一个解析器，因此对 Go 输出的 RFC3339 纳秒精度时间戳
   （如 `2026-09-13T11:47:59.1139268+08:00`）行为完全一致：解析成功并截断到毫秒。
   用 `chrono` 也能做，但会引入新依赖、增大 wasm 体积，且存在与原口径产生细微差异的风险。
