//! 广学 · 英语模块引擎（Rust / WebAssembly）
//!
//! 编译目标：`wasm32-unknown-unknown`，通过 `wasm-bindgen` 导出给浏览器调用。
//!
//! 模块划分：
//! - [`randomizer`]：随机器（复习顺序洗牌 / 新词无放回抽样 / 系统随机种子）
//! - [`fsrs_engine`]：FSRS 记忆调度计算（单卡状态推进、可提取率）
//! - [`session`]：复习会话编排（队列构建 / 评分决策 / 进度统计），
//!   由 `ReviewSession` 持有会话状态，JS 侧只负责渲染与取数
//!
//! 宿主（非 wasm）环境下，[`session`] 与 [`fsrs_engine`] 的纯计算部分可直接单元测试：
//!
//! ```text
//! cargo test                                  # 需要可用的宿主链接工具链（见 README）
//! cargo check --target wasm32-unknown-unknown # 类型检查
//! ```

pub mod fsrs_engine;
pub mod randomizer;
pub mod session;
