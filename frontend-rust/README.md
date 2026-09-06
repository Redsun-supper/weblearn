# guangxue_wasm — 广学前端 Rust / WebAssembly 模块

## 功能

1. **FSRS 间隔复习调度引擎**（`fsrs_engine.rs`，基于 `fsrs` v6）
   给单词卡做间隔重复调度：输入「记忆状态 + 期望保持率 + 已过天数」，输出四种评分
   （Again / Hard / Good / Easy）的下一记忆状态与间隔天数。纯算术、不依赖系统时钟，可在浏览器 WASM 中运行。
2. **随机器**（`randomizer.rs`）
   - 到期词池/词库的随机洗牌（`random_indices`）
   - 新词无放回随机抽样（`random_sample`）
   - 系统随机种子（`random_seed`）
3. 示例函数：`greet` / `add` / `is_even` / `subject_list`

## 导出 API（wasm-bindgen）

```js
// ---- FSRS 引擎 ----
// state_json：{"stability":..,"difficulty":..}；空串/null = 新卡
// desired_retention：期望记忆保持率（如 0.9）；days_elapsed：距上次复习天数
fsrs_next_states(state_json, desired_retention, days_elapsed)
// → {"again":{"memory":{...},"interval_days":..},"hard":{...},"good":{...},"easy":{...}}

// 当前记忆可提取率（0~1）
fsrs_retrievability(state_json, days_elapsed)

// ---- 随机器 ----
random_indices(len, seed)   // 0..len 的随机排列（复习顺序）
random_sample(len, count, seed) // 无放回随机抽 count 个索引（新词批次）
random_seed()               // 系统随机 32 位种子
```

### 引擎使用示例

```js
import { fsrs_next_states } from './pkg/guangxue_wasm.js';

// 新卡评分 Good（rating=3）
const next = JSON.parse(fsrs_next_states(null, 0.9, 0));
// 选"Good"分支 → 记忆状态 + interval_days，然后调用后端 POST /api/reviews/submit
const chosen = next.good;
const stateJson = JSON.stringify(chosen.memory); // 持久化到 SQL
```

## 结构

```
frontend-rust/
├── Cargo.toml          # cdylib + wasm-bindgen + fsrs + serde
├── src/
│   ├── lib.rs          # 模块导出
│   ├── fsrs_engine.rs  # FSRS 调度引擎（CardState / NextStatesOut）
│   └── randomizer.rs   # 随机器（XorShift32 + 洗牌/抽样）
└── README.md
```

## 构建

```powershell
rustup target add wasm32-unknown-unknown

# 类型检查
cargo check --target wasm32-unknown-unknown

# 生成 .wasm
cargo build --target wasm32-unknown-unknown --release
# 产物：target/wasm32-unknown-unknown/release/guangxue_wasm.wasm

# 生成 JS 胶水（wasm-bindgen-cli，版本 0.2.128 与 crate 一致）
wasm-bindgen --target web --out-dir pkg --out-name guangxue_wasm `
  target/wasm32-unknown-unknown/release/guangxue_wasm.wasm
# 产物：pkg/guangxue_wasm.js + pkg/guangxue_wasm_bg.wasm
```

> `pkg/` 目录由以上命令生成（当前已生成）。前端 `main.js` 通过
> `import('frontend-rust/pkg/guangxue_wasm.js')` 加载引擎，用于英语页单词复习。
> 注意：发布/部署前端时需保证 `frontend-rust/pkg/` 与静态文件一起被服务器提供。
>
> 说明：`fsrs` 间接依赖 `rayon`/`getrandom`，已通过 `getrandom` 的 `wasm_js`
> 特性解决 wasm32 编译问题；调度路径本身是确定性算术，不使用随机源。

## 宿主单元测试

`cargo test` 会在宿主目标上运行 `fsrs_engine` / `randomizer` 的单元测试。
注意：Windows 上宿主链接需要完整的 mingw（`as`）或 MSVC SDK，
若 `cargo test` 报 `dlltool` / `kernel32.lib` 错误，请安装对应工具链后运行。
