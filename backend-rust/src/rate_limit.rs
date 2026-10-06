// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 内存滑动窗口限流
//!
//! 为什么不做成数据库表：这些计数是**短命且高频**的，落库只会白白增加写压力。
//! 代价必须写清楚（README 里也有）：
//!   - 仅对**单个进程**有效：多开实例时每个实例各算一份；
//!   - 进程重启即清零。
//! 真正要跨实例限流时再换 Redis / 网关层限流。
//!
//! 账号维度的「连续失败锁定」不在这里——那个必须落库（`users.failed_attempts`
//! / `locked_until`），否则重启就能绕过。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::RateRule;
use crate::error::{AuthError, Result};

/// 单个键最多保留的窗口条数上限，防止内存无限增长
const MAX_KEYS: usize = 4096;

#[derive(Debug)]
pub struct RateLimiter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
    enabled: bool,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self { hits: Mutex::new(HashMap::new()), enabled: true }
    }

    /// 测试用：整体关掉限流（除了专门测限流的用例）
    pub fn disabled() -> Self {
        Self { hits: Mutex::new(HashMap::new()), enabled: false }
    }

    /// 记录一次命中；任一规则超限则返回 `RateLimited`
    ///
    /// 规则里 `limit == 0` 或窗口为 0 表示该条不启用。
    pub fn check(&self, key: &str, rules: &[RateRule]) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let now = Instant::now();
        let mut map = self.hits.lock().unwrap();

        let longest = rules
            .iter()
            .filter(|r| r.limit > 0 && r.window > Duration::ZERO)
            .map(|r| r.window)
            .max();
        let Some(longest) = longest else {
            return Ok(());
        };

        if map.len() > MAX_KEYS {
            // 简单清理：丢掉已经过期的键；仍然过多就整体清空（限流宁可短暂放松，也不能泄漏内存）
            map.retain(|_, list| {
                list.retain(|t| now.duration_since(*t) <= longest);
                !list.is_empty()
            });
            if map.len() > MAX_KEYS {
                map.clear();
            }
        }

        let entry = map.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) <= longest);

        for rule in rules {
            if rule.limit == 0 || rule.window == Duration::ZERO {
                continue;
            }
            let within = entry.iter().filter(|t| now.duration_since(**t) <= rule.window).count();
            if within >= rule.limit {
                return Err(AuthError::RateLimited);
            }
        }
        entry.push(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(limit: usize, secs: u64) -> RateRule {
        RateRule::new(limit, Duration::from_secs(secs))
    }

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let limiter = RateLimiter::new();
        let rules = [rule(2, 60)];
        assert!(limiter.check("a", &rules).is_ok());
        assert!(limiter.check("a", &rules).is_ok());
        assert!(matches!(limiter.check("a", &rules), Err(AuthError::RateLimited)));
        assert!(limiter.check("b", &rules).is_ok());
    }

    #[test]
    fn zero_limit_rule_is_disabled() {
        let limiter = RateLimiter::new();
        let rules = [rule(0, 60)];
        for _ in 0..100 {
            assert!(limiter.check("a", &rules).is_ok());
        }
    }

    #[test]
    fn hits_survive_the_shorter_window_for_the_longer_rule() {
        let limiter = RateLimiter::new();
        // 短窗口 2 次/100 毫秒 + 长窗口 3 次/小时：
        // 短窗口过期后计数必须仍然保留在长窗口里（清理只能按最长窗口来）
        let rules = [
            RateRule::new(2, Duration::from_millis(100)),
            RateRule::new(3, Duration::from_secs(3600)),
        ];
        assert!(limiter.check("a", &rules).is_ok());
        assert!(limiter.check("a", &rules).is_ok());
        std::thread::sleep(Duration::from_millis(120));
        assert!(limiter.check("a", &rules).is_ok(), "短窗口已过期，应当放行");
        assert!(
            matches!(limiter.check("a", &rules), Err(AuthError::RateLimited)),
            "长窗口的 3 次额度应当已被前面的请求用掉"
        );
    }

    #[test]
    fn disabled_limiter_never_blocks() {
        let limiter = RateLimiter::disabled();
        let rules = [rule(1, 60)];
        assert!(limiter.check("a", &rules).is_ok());
        assert!(limiter.check("a", &rules).is_ok());
    }
}
