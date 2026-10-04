// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 邮箱验证码：生成、摘要与状态判定
//!
//! 为什么摘要要带上邮箱与 pepper：
//!   - 加邮箱：同一时刻不同邮箱的码不会因为撞码而互相通过；
//!   - 加 pepper（服务端密钥）：库被拖走也无法用彩虹表反查那 6 位数字。
//! 注意 6 位数字只有 10^6 种组合，**摘要本身不是防线**，真正的防线是
//! 「10 分钟有效期 + 最多 5 次尝试 + 发送限流」，三者都在服务层强制执行。

use hmac::{Hmac, Mac};
use rand::Rng;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

/// 验证码位数
pub const CODE_LEN: usize = 6;

/// 生成 6 位数字验证码（系统随机源，`gen_range` 已处理取模偏置）
pub fn generate_code() -> String {
    let mut rng = rand::rngs::OsRng;
    format!("{:0width$}", rng.gen_range(0..1_000_000u32), width = CODE_LEN)
}

/// 验证码格式：恰为 6 位 ASCII 数字
pub fn is_well_formed(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| b.is_ascii_digit())
}

/// 计算验证码摘要（HMAC-SHA256，十六进制小写）
pub fn hash_code(pepper: &[u8], email: &str, code: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(pepper).expect("HMAC 接受任意长度的密钥");
    mac.update(email.as_bytes());
    mac.update(b":");
    mac.update(code.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// 恒定时间比对（长度不同直接判否；长度不是秘密）
pub fn verify_hash(pepper: &[u8], email: &str, code: &str, expected_hex: &str) -> bool {
    let computed = hash_code(pepper, email, code);
    let a = computed.as_bytes();
    let b = expected_hex.trim().to_ascii_lowercase().into_bytes();
    a.len() == b.len() && bool::from(a.ct_eq(&b))
}

/// 校验结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeCheck {
    /// 通过
    Ok,
    /// 码不对
    Mismatch,
    /// 已过期
    Expired,
    /// 尝试次数过多
    AttemptsExceeded,
}

/// 状态判定：过期 > 超次数 > 不匹配 > 通过
///
/// 过期排在最前是刻意的：过期之后无论码对不对，用户唯一该做的事都是「重新获取」，
/// 提示语也要一致，避免把「码对不对」这条信息漏给攻击者。
pub fn evaluate(
    attempts: i64,
    max_attempts: i64,
    expires_at: OffsetDateTime,
    now: OffsetDateTime,
    matched: bool,
) -> CodeCheck {
    if now >= expires_at {
        return CodeCheck::Expired;
    }
    if attempts >= max_attempts {
        return CodeCheck::AttemptsExceeded;
    }
    if !matched {
        return CodeCheck::Mismatch;
    }
    CodeCheck::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(secs).unwrap()
    }

    const PEPPER: &[u8] = b"unit-test-pepper";

    #[test]
    fn generated_code_is_six_digits() {
        for _ in 0..200 {
            let code = generate_code();
            assert_eq!(code.len(), CODE_LEN);
            assert!(is_well_formed(&code), "格式不对：{code}");
        }
        assert!(!is_well_formed("12345"));
        assert!(!is_well_formed("1234567"));
        assert!(!is_well_formed("12345a"));
        assert!(!is_well_formed(""));
    }

    #[test]
    fn hash_matches_only_for_same_email_and_code() {
        let hash = hash_code(PEPPER, "a@example.com", "123456");
        assert!(verify_hash(PEPPER, "a@example.com", "123456", &hash));
        assert!(!verify_hash(PEPPER, "a@example.com", "123457", &hash));
        assert!(!verify_hash(PEPPER, "b@example.com", "123456", &hash));
        assert!(!verify_hash(b"other-pepper", "a@example.com", "123456", &hash));
        assert!(!verify_hash(PEPPER, "a@example.com", "123456", "not-hex"));
    }

    #[test]
    fn hash_is_stable_and_case_insensitive_on_stored_value() {
        let hash = hash_code(PEPPER, "a@example.com", "000123");
        assert!(verify_hash(PEPPER, "a@example.com", "000123", &hash.to_uppercase()));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn state_machine_priority() {
        let now = at(1_000_000);
        let future = at(1_000_600);
        assert_eq!(evaluate(0, 5, future, now, true), CodeCheck::Ok);
        assert_eq!(evaluate(0, 5, future, now, false), CodeCheck::Mismatch);
        assert_eq!(evaluate(5, 5, future, now, true), CodeCheck::AttemptsExceeded);
        assert_eq!(evaluate(5, 5, now, now, true), CodeCheck::Expired);
        assert_eq!(evaluate(5, 5, at(999_999), now, true), CodeCheck::Expired);
    }
}
