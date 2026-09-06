//! 广学 前端 WebAssembly 模块（Rust 部分）
//!
//! 编译目标：`wasm32-unknown-unknown`。
//! 通过 `wasm-bindgen` 把函数导出给浏览器中的 JavaScript 调用。
//!
//! 模块划分：
//! - [`fsrs_engine`]：FSRS 间隔复习调度引擎（单词卡调度）
//! - [`randomizer`]：随机器（复习顺序洗牌 / 新词无放回抽样）
//!
//! 宿主（非 wasm）环境下也可用于 `cargo test` / 单元测试。

pub mod fsrs_engine;
pub mod randomizer;

use wasm_bindgen::prelude::*;

/// 向用户打招呼，演示字符串从 JS 传入并返回。
#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    format!("你好，{}！这是来自 Rust / WebAssembly 的问候。", name)
}

/// 两个整数相加，演示基础数值计算。
#[wasm_bindgen]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// 判断一个整数是否为偶数，演示布尔运算。
#[wasm_bindgen]
pub fn is_even(n: i32) -> bool {
    n % 2 == 0
}

/// 返回项目中的学科列表（JSON 字符串），演示返回结构化数据。
#[wasm_bindgen]
pub fn subject_list() -> String {
    r#"{"subjects":["语文","数学","英语","物理","化学","生物","历史","政治","地理"],"count":9}"#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_works() {
        assert_eq!(add(2, 3), 5);
    }

    #[test]
    fn is_even_works() {
        assert!(is_even(4));
        assert!(!is_even(7));
    }
}
