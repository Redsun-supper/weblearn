//! FSRS 间隔复习调度引擎（WASM 导出层）
//!
//! 基于 [fsrs]（open-spaced-repetition/fsrs-rs，v6）。引擎本身是纯算术：
//! 输入「当前记忆状态 + 期望记忆保持率 + 距上次复习天数」，输出四种评分
//! （Again / Hard / Good / Easy）对应的下一记忆状态与间隔天数。
//! 不依赖系统时钟，适合在浏览器 WASM 中运行。
//!
//! 数据结构与 SQL 后端持久化的字段一一对应：
//! - 记忆状态 `CardState { stability, difficulty }`
//! - 评分结果 `ItemStateOut { memory, interval_days }`

use fsrs::{MemoryState, FSRS, FSRS6_DEFAULT_DECAY};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// 一张单词卡的 FSRS 记忆状态（对应后端 SQL 表字段）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CardState {
    /// 记忆稳定度（天）：预测多久后记忆仍可提取
    pub stability: f32,
    /// 记忆难度：1~10，越小越易
    pub difficulty: f32,
}

/// 一次评分后的结果
#[derive(Debug, Clone, Serialize)]
pub struct ItemStateOut {
    /// 评分后的新记忆状态
    pub memory: CardState,
    /// 距下次复习的间隔（天）
    pub interval_days: f32,
}

/// 四种评分对应的下一状态
#[derive(Debug, Clone, Serialize)]
pub struct NextStatesOut {
    pub again: ItemStateOut,
    pub hard: ItemStateOut,
    pub good: ItemStateOut,
    pub easy: ItemStateOut,
}

/// 核心计算：由可选记忆状态 + 期望保持率 + 已过天数，计算四种评分的下一状态。
///
/// - `state = None`：新卡（首次复习）
/// - `days_elapsed`：距上次复习的天数（新卡传 0）
pub fn compute_next_states(
    state: Option<CardState>,
    desired_retention: f32,
    days_elapsed: u32,
) -> Result<NextStatesOut, String> {
    if !desired_retention.is_finite() || desired_retention <= 0.0 || desired_retention >= 1.0 {
        return Err("desired_retention 必须位于 (0, 1) 区间内".to_string());
    }
    let fsrs = FSRS::default();
    let prev = state.map(|s| MemoryState {
        stability: s.stability,
        difficulty: s.difficulty,
    });
    let next = fsrs
        .next_states(prev, desired_retention, days_elapsed)
        .map_err(|e| format!("FSRS 计算失败: {e:?}"))?;
    let pick = |it: fsrs::ItemState| ItemStateOut {
        memory: CardState {
            stability: it.memory.stability,
            difficulty: it.memory.difficulty,
        },
        interval_days: it.interval,
    };
    Ok(NextStatesOut {
        again: pick(next.again),
        hard: pick(next.hard),
        good: pick(next.good),
        easy: pick(next.easy),
    })
}

/// 核心计算：当前记忆的可提取率（回忆概率），基于 FSRS-6 默认衰减参数。
pub fn compute_retrievability(state: &CardState, days_elapsed: f32) -> f32 {
    fsrs::current_retrievability(
        MemoryState {
            stability: state.stability,
            difficulty: state.difficulty,
        },
        days_elapsed,
        FSRS6_DEFAULT_DECAY,
    )
}

/// WASM 导出：计算四种评分的下一状态。
///
/// - `state_json`：记忆状态 JSON（`{"stability":..,"difficulty":..}`），空串/`null` 表示新卡
/// - `desired_retention`：期望记忆保持率，例如 0.9
/// - `days_elapsed`：距上次复习的天数（u32）
///
/// 返回 JSON：
/// `{"again":{...},"hard":{...},"good":{...},"easy":{...}}`
#[wasm_bindgen]
pub fn fsrs_next_states(
    state_json: Option<String>,
    desired_retention: f64,
    days_elapsed: u32,
) -> Result<String, JsValue> {
    let state = match state_json {
        Some(json) if !json.trim().is_empty() => Some(
            serde_json::from_str::<CardState>(&json)
                .map_err(|e| JsValue::from_str(&format!("state_json 解析失败: {e}")))?,
        ),
        _ => None,
    };
    let out = compute_next_states(state, desired_retention as f32, days_elapsed)
        .map_err(|e| JsValue::from_str(&e))?;
    serde_json::to_string(&out).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// WASM 导出：当前记忆的可提取率（0~1）。
#[wasm_bindgen]
pub fn fsrs_retrievability(state_json: &str, days_elapsed: f64) -> Result<f64, JsValue> {
    let state: CardState = serde_json::from_str(state_json)
        .map_err(|e| JsValue::from_str(&format!("state_json 解析失败: {e}")))?;
    Ok(compute_retrievability(&state, days_elapsed as f32) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "expected {b}, got {a}");
    }

    #[test]
    fn new_card_uses_initial_stability() {
        // 新卡 + 一天未过（days_elapsed=0）：good 分支的 interval/stability 等于默认参数 w[2]
        let out = compute_next_states(None, 0.9, 0).unwrap();
        assert_close(out.again.memory.stability, 0.212);
        assert_close(out.good.memory.stability, 2.3065);
        assert_close(out.easy.memory.stability, 8.2956);
        // 新卡 interval == stability
        assert_close(out.good.interval_days, out.good.memory.stability);
        // 排序：easy > good > hard > again
        assert!(out.easy.interval_days > out.good.interval_days);
        assert!(out.good.interval_days > out.hard.interval_days);
        assert!(out.hard.interval_days > out.again.interval_days);
    }

    #[test]
    fn review_increases_stability_on_success() {
        let state = CardState {
            stability: 2.3065,
            difficulty: 2.118104,
        };
        let out = compute_next_states(Some(state), 0.9, 7).unwrap();
        // 好评后稳定度提升，间隔至少 1 天
        assert!(out.good.memory.stability > state.stability);
        assert!(out.good.interval_days > 1.0);
        // 评分影响：easy 的新稳定度 > good 的新稳定度 > hard
        assert!(out.easy.memory.stability > out.good.memory.stability);
        assert!(out.good.memory.stability > out.hard.memory.stability);
    }

    #[test]
    fn again_reduces_stability_or_keeps_small() {
        let state = CardState {
            stability: 10.0,
            difficulty: 5.0,
        };
        let out = compute_next_states(Some(state), 0.9, 3).unwrap();
        // Again 应显著压低稳定度（低于 good 分支）
        assert!(out.again.memory.stability < out.good.memory.stability);
    }

    #[test]
    fn invalid_retention_rejected() {
        assert!(compute_next_states(None, 1.0, 0).is_err());
        assert!(compute_next_states(None, 0.0, 0).is_err());
        assert!(compute_next_states(None, 0.9, 0).is_ok());
    }

    #[test]
    fn retrievability_in_range() {
        let state = CardState {
            stability: 10.0,
            difficulty: 5.0,
        };
        let r0 = compute_retrievability(&state, 0.0);
        let r10 = compute_retrievability(&state, 10.0);
        assert!(r0 > r10);
        assert!((0.0..=1.0).contains(&r0));
        assert!((0.0..=1.0).contains(&r10));
    }

    #[test]
    fn json_roundtrip_via_serde() {
        let json = r#"{"stability":2.31,"difficulty":3.5}"#;
        let s: CardState = serde_json::from_str(json).unwrap();
        assert_close(s.stability, 2.31);
        let out = compute_next_states(Some(s), 0.9, 1).unwrap();
        let encoded = serde_json::to_string(&out).unwrap();
        assert!(encoded.contains("interval_days"));
    }
}
