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

// 直接用接口返回的 JSON 原文建会话（内部完成洗牌、抽新词、时间解析）
// 第三个参数是**本次抽取的新词批量**（默认 20），不是每日上限
const session = new ReviewSession(dueJsonText, newJsonText, 20);

session.total();             // 队列总张数（会随 append 增长）
session.done();              // 已完成张数
session.is_finished();       // 当前队列是否已走完
session.progress_percent();  // 0~100
session.current_word_id();   // 当前词条 id（无卡时 0）
session.current_json(Date.now());
// 当前卡片展示数据：
//   word / phonetic / source
//   example_parts  主例句按目标词切分的片段（空 = 该词条没有例句，前端整块不渲染）
//   example_translation  主例句的中文翻译（空则不输出该字段）
//   senses         释义块数组 [{pos, meaning, example_parts?, translation?}]，有释义时至少一块
//   meta           记忆元信息（难度/稳定性/复习次数/距上次天数/预计记住）
// 说明：卡片 JSON 刻意不带冗余字段（meaning/example 原文、下标等），多释义本来就比单词条重，
//       JS 那边也不再需要它们。
//
// senses 的来源有两条：
//   1. 词条填了结构化多释义（后端 words.senses）→ 直接用，每块可带自己的例句与译文；
//   2. 没填 → 把 meaning 按词性标签自动拆开（"n. 好处；益处 v. 有益于" → 名词、动词两块），
//      此时例句只有词条级那一条，显示在顶部。
// 两件事都在 card_view::split_senses / build_senses 里，纯计算、有单元测试兜住。

// 评分：1=Again 2=Hard 3=Good 4=Easy；now_ms 传 Date.now()
const body = session.rate(3, Date.now());
// body 就是 POST /api/reviews/submit 的请求体：
// {"word_id":42,"rating":3,"stability":2.3065,"difficulty":2.1181,"interval_days":2.3065}

// 队列走完后继续抽：追加一批（游标不动），返回实际追加数量；0 表示没得抽了
const added = session.append(dueJsonText, newJsonText, 20);
// 「不限制每日新词」就是靠它实现：前端在 is_finished() 时再取一批交进来即可

session.free();  // 离开英语页时释放（wasm-bindgen 生成）
```

`append` 的去重规则：跳过 id 已经在**待办区**（尚未评分的部分）里的卡片；
游标之前已评完的卡不算重复——到期后再次抽到属于正常复习。

另有 `ReviewSession.with_seed(dueJson, newJson, newLimit, seed)` 与
`append_with_seed(..., seed)`：显式指定随机种子，便于复现与测试。

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
cargo test            # 94 个测试：会话编排 / 天数换算 / 进度 / 解析 / FSRS / 随机器 / 多释义拆分 / 词形匹配
cargo test -- --nocapture
```

测试全部跑在**纯计算层**，不依赖浏览器：

- `plan_session` / `days_elapsed` / `progress_percent` / `pick_branch`
- `ReviewSession::build` / `ReviewSession::try_rate`（错误类型是 `String`）
- `parse_items`（含 Go 空结果返回 `"items": null`、`"senses": null` 的场景）
- `split_senses`（一行多个义项、连续词性标签、`num.` 不被 `n.` 抢、括号里的标签不误判）
- `split_example` / `word_forms`（词边界优先、变形匹配 `applied` ↔ `apply`、原形优先）
- `parse_word_list_core`（制表符/竖线/逗号/空格、注释、重复、列错位、第 5 列例句翻译）

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
