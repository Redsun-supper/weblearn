// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 随机器：可复现的种子随机（洗牌 / 无放回抽样）
//!
//! 用途：为"间隔式重复调用单词"提供随机性——
//! - 每天从到期词池中随机决定复习顺序（避免固定顺序）
//! - 从新词库中无放回随机抽取今日新词
//!
//! 使用内置 XorShift32 伪随机数生成器（纯 Rust、无外部随机源），
//! 同一 `seed` 必然产生同一结果，便于测试和复现；`random_seed()`
//! 提供来自系统的真正随机种子。

use wasm_bindgen::prelude::*;

/// 简单的 XorShift32 伪随机数生成器
#[derive(Debug, Clone)]
pub struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    /// 从任意 32 位种子构造（对种子做一次混淆，避免弱种子）
    pub fn new(seed: u32) -> Self {
        // MurmurHash3 收尾混淆：改善劣质种子（0、小整数）的分布
        let mut s = seed.wrapping_add(0x9E37_79B9);
        s = (s ^ (s >> 16)).wrapping_mul(0x21F0_AAAD);
        s = (s ^ (s >> 15)).wrapping_mul(0x735A_2D97);
        s = s ^ (s >> 15);
        if s == 0 {
            s = 0x9E37_79B9; // xorshift 状态不允许为 0
        }
        Self { state: s }
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    pub fn next_u64(&mut self) -> u64 {
        ((self.next_u32() as u64) << 32) | self.next_u32() as u64
    }
}

/// 返回 `0..len` 的一个随机排列（Fisher–Yates 洗牌后的索引）。
/// 适合：把到期卡池/词库索引顺序打乱。
#[wasm_bindgen]
pub fn random_indices(len: u32, seed: u32) -> Vec<u32> {
    let mut idx: Vec<u32> = (0..len).collect();
    if len > 1 {
        let mut rng = XorShift32::new(seed);
        for i in (1..len).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as u32;
            idx.swap(i as usize, j as usize);
        }
    }
    idx
}

/// 从 `0..len` 中**无放回**随机抽取 `count` 个索引（部分 Fisher–Yates）。
/// 当 `count >= len` 时返回整组洗牌后的索引。
/// 适合：从新词库中随机抽取今天要学的一批词。
#[wasm_bindgen]
pub fn random_sample(len: u32, count: u32, seed: u32) -> Vec<u32> {
    let take = count.min(len);
    let mut idx: Vec<u32> = (0..len).collect();
    if len > 1 && take > 0 {
        let mut rng = XorShift32::new(seed);
        for i in 0..take {
            let j = i + (rng.next_u64() % (len - i) as u64) as u32;
            idx.swap(i as usize, j as usize);
        }
    }
    idx.truncate(take as usize);
    idx
}

/// 生成一个系统随机（浏览器中来自 Web Crypto / getrandom）的 32 位种子。
/// 在 wasm 目标下编译需要 getrandom 的 `wasm_js` 特性（已在 Cargo.toml 开启）。
#[wasm_bindgen]
pub fn random_seed() -> Result<u32, JsValue> {
    getrandom::u32().map_err(|e| JsValue::from_str(&format!("随机源不可用: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_is_permutation() {
        let out = random_indices(9, 42);
        assert_eq!(out.len(), 9);
        let mut sorted = out.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_deterministic_per_seed() {
        assert_eq!(random_indices(20, 7), random_indices(20, 7));
        // 不同种子大概率不同（允许极低概率相同）
        assert_ne!(random_indices(1000, 1), random_indices(1000, 2));
    }

    #[test]
    fn sample_counts_and_unique() {
        let out = random_sample(50, 10, 99);
        assert_eq!(out.len(), 10);
        let mut sorted = out.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 10, "抽样不应有重复");
        assert!(sorted.iter().all(|&i| i < 50));
    }

    #[test]
    fn sample_over_cap_returns_full_shuffle() {
        let out = random_sample(8, 100, 3);
        assert_eq!(out.len(), 8);
        let mut sorted = out.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn zero_len_is_empty() {
        assert!(random_indices(0, 1).is_empty());
        assert!(random_sample(0, 5, 1).is_empty());
    }

    #[test]
    fn rng_never_zero_state() {
        let mut rng = XorShift32::new(0);
        for _ in 0..100 {
            assert_ne!(rng.next_u32(), 0);
        }
    }
}
