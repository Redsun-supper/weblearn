// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 时钟：核心逻辑一律通过 `Clock` 取时间，测试注入 `FakeClock` 就能精确控制
//! 「验证码过期 / 令牌过期 / 账号锁定」这些与时间强相关的分支。

use std::sync::Mutex;
use std::time::Duration;

use time::OffsetDateTime;

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> OffsetDateTime;
}

/// 真实系统时钟（UTC）
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// 可控时钟（测试用）
#[derive(Debug)]
pub struct FakeClock {
    now: Mutex<OffsetDateTime>,
}

impl FakeClock {
    pub fn new(start: OffsetDateTime) -> Self {
        Self { now: Mutex::new(start) }
    }

    /// 从一个 Unix 秒构造（测试里写起来最短）
    pub fn from_unix(secs: i64) -> Self {
        Self::new(OffsetDateTime::from_unix_timestamp(secs).expect("合法的时间戳"))
    }

    pub fn set(&self, t: OffsetDateTime) {
        *self.now.lock().unwrap() = t;
    }

    pub fn advance(&self, d: Duration) {
        let mut guard = self.now.lock().unwrap();
        *guard += time::Duration::seconds(d.as_secs() as i64);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> OffsetDateTime {
        *self.now.lock().unwrap()
    }
}
