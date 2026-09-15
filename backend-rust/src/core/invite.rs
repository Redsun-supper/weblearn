//! 邀请码：生成、规范化与状态机
//!
//! 字符集用 Crockford Base32（去掉了容易看错的 I / L / O / U），默认 16 位 =
//! 80 bit 随机量，足够抵抗猜测；`-` 与空格在规范化时会被去掉，因此管理员转发时
//! 手抄成 `XXXX-XXXX-...` 也能用。

use rand::Rng;
use time::OffsetDateTime;

/// 无易混字符的 Base32 字母表
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 默认邀请码长度
pub const CODE_LEN: usize = 16;

/// 生成一个邀请码（系统随机源）
pub fn generate_code() -> String {
    let mut rng = rand::rngs::OsRng;
    (0..CODE_LEN).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect()
}

/// 规范化：去空白与 `-`、统一大写（大小写不敏感、允许手抄分隔符）
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// 形如「长度合理且只用字母表内字符」——用它在查库前先挡掉明显无效的输入
pub fn is_plausible(raw: &str) -> bool {
    let code = normalize_code(raw);
    (8..=32).contains(&code.len()) && code.bytes().all(|b| ALPHABET.contains(&b))
}

/// 邀请码状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteState {
    /// 可用
    Usable,
    /// 已被管理员停用
    Disabled,
    /// 已过期
    Expired,
    /// 使用次数已用尽
    Exhausted,
}

/// 状态机：停用 > 过期 > 用尽 > 可用
///
/// 判断顺序刻意固定——同时满足多个条件时（例如过期的码还被停用），
/// 返回的原因必须稳定，否则错误提示会随实现细节漂移。
pub fn evaluate(
    disabled: bool,
    used_count: i64,
    max_uses: i64,
    expires_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> InviteState {
    if disabled {
        return InviteState::Disabled;
    }
    if matches!(expires_at, Some(e) if e <= now) {
        return InviteState::Expired;
    }
    if used_count >= max_uses {
        return InviteState::Exhausted;
    }
    InviteState::Usable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(secs).unwrap()
    }

    #[test]
    fn generated_code_uses_safe_alphabet_and_length() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)), "含字母表外的字符：{code}");
        for bad in ['I', 'L', 'O', 'U'] {
            assert!(!code.contains(bad), "不应出现易混字符 {bad}");
        }
    }

    #[test]
    fn generated_codes_differ() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            assert!(seen.insert(generate_code()), "邀请码出现重复");
        }
    }

    #[test]
    fn normalization_accepts_handwritten_forms() {
        let code = generate_code();
        let spaced = code
            .chars()
            .collect::<Vec<_>>()
            .chunks(4)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("-");
        assert_eq!(normalize_code(&spaced), code);
        assert_eq!(normalize_code(&code.to_lowercase()), code);
        assert_eq!(normalize_code(" ab cd 12 "), "ABCD12");
    }

    #[test]
    fn plausibility_rejects_short_or_foreign_input() {
        assert!(is_plausible(&generate_code()));
        assert!(!is_plausible("SHORT"));
        assert!(!is_plausible("IIIIIIIIIIIIIIII"), "I 不在字母表内");
        assert!(!is_plausible(""), "空串");
        assert!(!is_plausible(&"A".repeat(40)), "过长");
    }

    #[test]
    fn state_machine_prefers_disabled_then_expired_then_exhausted() {
        let now = at(1_000_000);
        assert_eq!(evaluate(false, 0, 1, None, now), InviteState::Usable);
        assert_eq!(evaluate(false, 0, 1, Some(at(1_000_001)), now), InviteState::Usable);
        // 过期：边界上「到期时刻 == 现在」即算过期
        assert_eq!(evaluate(false, 0, 1, Some(at(1_000_000)), now), InviteState::Expired);
        // 用尽
        assert_eq!(evaluate(false, 1, 1, None, now), InviteState::Exhausted);
        // 停用优先于其他条件
        assert_eq!(evaluate(true, 5, 1, Some(at(1)), now), InviteState::Disabled);
    }
}
