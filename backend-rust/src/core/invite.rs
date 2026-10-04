// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 邀请码：生成、规范化与状态机
//!
//! 字符集用 Crockford Base32（去掉了容易看错的 I / L / O / U），默认 16 位 =
//! 80 bit 随机量，足够抵抗猜测。
//!
//! ## 邀请码的定位（2026-09 改）
//!
//! 邀请码**不再是注册门槛**（邮箱验证码是门槛，普通用户可以直接注册），
//! 而是「兑换券」：注册时带上它就把账号升级成管理员，将来还可以承载
//! 积分 / 礼物之类的兑换入口。
//!
//! ## 分隔符约定
//!
//! - **多个邀请码用空白分隔**（见 [`split_codes`]），可以一次填好几张券；
//! - **`-` 属于邀请码自身的格式**，后台将来用它区分用途（如 `ADMIN-xxxx` / `POINTS-xxxx`），
//!   所以规范化时**绝不能把它抹掉**——抹掉就区分不了了。
//!   （管理员手抄成 `XXXX-XXXX-XXXX-XXXX` 的老习惯靠查库时的兜底处理兼容，
//!   见 `service::find_invite`。）

use rand::Rng;
use time::OffsetDateTime;

/// 无易混字符的 Base32 字母表
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 默认邀请码长度
pub const CODE_LEN: usize = 16;

/// 管理员码的前缀（**只是给人看的**：方便一眼分出手上这张码的分量）
pub const ADMIN_PREFIX: &str = "ADMIN-";

/// 生成一个邀请码（系统随机源）
pub fn generate_code() -> String {
    let mut rng = rand::rngs::OsRng;
    (0..CODE_LEN).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect()
}

/// 按要授予的角色生成邀请码：管理员码带 `ADMIN-` 前缀。
///
/// ⚠️ 前缀**不是安全边界**：兑换时一律查库读 `grant_role`。
/// 作用只是让人扫一眼就知道这张码能不能造管理员（避免把管理员码当普通码随手转出去）。
pub fn generate_code_for(grant_role: &str) -> String {
    if grant_role == crate::models::ROLE_ADMIN {
        format!("{ADMIN_PREFIX}{}", generate_code())
    } else {
        generate_code()
    }
}

/// 规范化**单个**邀请码：去空白、统一大写（大小写不敏感）。
///
/// ⚠️ 刻意**不去掉 `-`**：它是邀请码自身的格式，后台要靠它区分用途。
/// 多个码的切分由 [`split_codes`] 负责，这里只管一个码。
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// 把用户输入拆成多个邀请码：**按空白分隔**，各自规范化，保持原顺序并去重。
///
/// - 用空白而不是 `-` 分隔：`-` 留给邀请码自身的格式；
/// - 去重是为了防止把同一个码手抖写两遍时被扣掉两次额度。
pub fn split_codes(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split_whitespace() {
        let code = normalize_code(part);
        if code.is_empty() {
            continue;
        }
        if !out.iter().any(|c| c == &code) {
            out.push(code);
        }
    }
    out
}

/// 形如「长度合理且只用字母数字与 `-`」——用它在查库前先挡掉明显无效的输入。
///
/// 长度按**去掉 `-` 之后的字母数字部分**算（8~32）。
///
/// ⚠️ 这里刻意**不套用生成时那套 Crockford 字母表**：用途前缀会是
/// `ADMIN` / `POINTS` 这类英文单词，而它们含 I / L / O / U（正是生成时被排除的
/// 易混字符）。真正的判定是查库，这里只是一道廉价的预筛，放宽不会漏掉什么。
pub fn is_plausible(raw: &str) -> bool {
    let code = normalize_code(raw);
    let core: String = code.chars().filter(|c| *c != '-').collect();
    if core.is_empty() {
        return false;
    }
    (8..=32).contains(&core.len())
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
    fn admin_codes_carry_the_prefix_but_user_codes_do_not() {
        let admin = generate_code_for(crate::models::ROLE_ADMIN);
        assert!(admin.starts_with(ADMIN_PREFIX), "管理员码应当带前缀：{admin}");
        assert_eq!(admin.len(), ADMIN_PREFIX.len() + CODE_LEN);
        assert!(is_plausible(&admin), "带前缀的码必须能通过预筛");

        let user = generate_code_for(crate::models::ROLE_USER);
        assert!(!user.starts_with(ADMIN_PREFIX));
        assert_eq!(user.len(), CODE_LEN);

        // 未知角色按普通码处理（不能因为传了个奇怪的字符串就生成管理员码）
        let odd = generate_code_for("root");
        assert!(!odd.starts_with(ADMIN_PREFIX));
    }

    #[test]
    fn prefix_is_not_a_security_boundary() {
        // 前缀只是给人看的：兑换判定永远查库读 grant_role。
        // 这条测试锁的是「规范化不会把前缀抹掉」——抹掉了就没法在日志/列表里看出分量，
        // 但也**不能**靠它判定权限（`service.rs` 只读 grant_role）。
        let admin = generate_code_for(crate::models::ROLE_ADMIN);
        assert_eq!(normalize_code(&admin), admin, "前缀必须原样保留");
        assert!(!admin.is_empty());
        // 反过来：手里拿一个不带前缀的码，也可能是 admin —— 判定在库里，不在字符串上
        assert!(!generate_code_for(crate::models::ROLE_USER).starts_with(ADMIN_PREFIX));
    }

    #[test]
    fn normalization_keeps_hyphen_but_drops_whitespace() {
        let code = generate_code();
        // 手抄成 XXXX-XXXX-XXXX-XXXX 的样子：`-` 会被保留（查库时再兜底去掉）
        let spaced = code
            .chars()
            .collect::<Vec<_>>()
            .chunks(4)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("-");
        assert_eq!(normalize_code(&spaced), spaced);
        assert_eq!(normalize_code(&code.to_lowercase()), code);
        assert_eq!(normalize_code(" ab cd 12 "), "ABCD12");
    }

    #[test]
    fn split_codes_uses_whitespace_and_dedups() {
        assert_eq!(split_codes("aaaa bbbb"), vec!["AAAA".to_string(), "BBBB".to_string()]);
        assert_eq!(split_codes("  aa\n\tbb   cc "), vec!["AA", "BB", "CC"]);
        // 手抖写两遍同一个码 → 只算一次（否则会扣两次额度）
        assert_eq!(split_codes("CODE1 code1"), vec!["CODE1".to_string()]);
        // `-` 不是分隔符：整串就是一个码
        assert_eq!(split_codes("ADMIN-ABCD1234"), vec!["ADMIN-ABCD1234".to_string()]);
        assert!(split_codes("").is_empty());
        assert!(split_codes("   \n ").is_empty());
    }

    #[test]
    fn plausibility_rejects_short_or_foreign_input() {
        assert!(is_plausible(&generate_code()));
        assert!(!is_plausible("SHORT"), "太短");
        assert!(!is_plausible(""), "空串");
        assert!(!is_plausible(&"A".repeat(40)), "过长");
        assert!(!is_plausible("---"), "只有分隔符");
        // 单个码里的空白会被规范化去掉（切分多个码是 split_codes 的事，它先按空白切）
        assert!(is_plausible("ABCD 1234"), "空白会被去掉，剩下 8 位");
        assert!(!is_plausible("ABCD_1234"), "下划线不是合法字符");
    }

    #[test]
    fn plausibility_accepts_purpose_prefixed_codes() {
        // 将来后台靠 `-` 前缀区分用途，这类前缀含 I/O 等易混字母，必须放行
        assert!(is_plausible("ADMIN-ABCD1234"));
        assert!(is_plausible("points-abcd1234"), "大小写不敏感");
        assert!(is_plausible("ABCD-EFGH-IJKL-MNOP"), "手抄分组");
        assert!(!is_plausible("ADMIN-AB"), "去掉 - 后不足 8 位");
        // 生成出来的码仍然只用安全字母表（不会出现 I/L/O/U）
        for bad in ['I', 'L', 'O', 'U'] {
            assert!(!generate_code().contains(bad));
        }
    }

    #[test]
    fn state_machine_prefers_disabled_then_expired_then_exhausted() {
        let now = at(1_000_000);
        assert_eq!(evaluate(false, 0, 1, None, now), InviteState::Usable);
        assert_eq!(evaluate(false, 0, 1, Some(at(1_000_001)), now), InviteState::Usable);
        // 过期：边界上「到期时刻 == 现在」即算过期
        assert_eq!(evaluate(false, 0, 1, Some(at(1_000_000)), now), InviteState::Expired);
        assert_eq!(evaluate(false, 1, 1, None, now), InviteState::Exhausted);
        assert_eq!(evaluate(true, 5, 1, Some(at(1)), now), InviteState::Disabled);
    }
}
