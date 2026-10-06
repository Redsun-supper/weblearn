// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 邀请码：生成、规范化与状态机
//!
//! 字符集是 **A-Z 与 0-9 共 36 个字符**（用户 2026-10-05 定：邀请码就只要这个范围），
//! 默认 16 位 ≈ 82.7 bit 随机量（log2(36) × 16），足够抵抗猜测。
//!
//! ⚠️ 曾经用过 Crockford Base32（排除 I/L/O/U 这套易混字符），**2026-10-05 按用户要求改掉**：
//! 生成与手填现在用同一套字符表，邮件的码和超管自己定的码长得一样。
//! 代价是随机码里会出现 `O`/`I`/`l` 这类手抄容易看错的字符 ——
//! 兑换时不做「O→0」这类自动纠正（那会把用户真写对的码改成另一个码，反而更难查），
//! 所以抄码要连字符一起复制（面板上有「复制」按钮）。
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

/// 邀请码的字符表：**A-Z 与 0-9**（36 个）。
///
/// ⚠️ 别自作聪明换回「无易混字符」的 32 字符表（Crockford 那套）：2026-10-05 用户明确要
/// 「邀请码要 A-Z 和 0-9」，而且**手填的自定义码本来就是这个范围** —— 两处口径不一致时，
/// 表现是「我自己写的码里能用 O，系统发的码里却永远见不到 O」，看起来像 bug。
/// 改这张表**不影响已经发出去的码**：兑换一律查库，旧码照样有效。
const ALPHABET: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

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
/// ⚠️ 这里刻意比生成/自定义码**宽**：用途前缀会是 `ADMIN` / `POINTS` 这类英文单词，
/// 而且老库里可能还有 Crockford 时代（排除 I/L/O/U）发的码。真正的判定是查库，
/// 这里只是一道廉价的预筛，放宽不会漏掉什么。
pub fn is_plausible(raw: &str) -> bool {
    let code = normalize_code(raw);
    let core: String = code.chars().filter(|c| *c != '-').collect();
    if core.is_empty() {
        return false;
    }
    (8..=32).contains(&core.len())
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 管理员**自己指定**的邀请码：规范化（去空白、统一大写、抹掉所有 `-`）。
///
/// ⚠️ 这里比其他地方**多抹一层 `-`**（[`normalize_code`] 是刻意保留的，用途前缀靠它区分）：
/// 自定义码的长度口径是「16 个字符」= 系统生成那种 16 位的样子，而人手写时很自然会把
/// 它抄成分组的 `ABCD-EFGH-IJKL-MNOP`。不抹掉的话，这串去掉横杠明明就是 16 位，
/// 却会被判成「21 位、太长」，管理员只会觉得「我明明写的就是 16 位」。
///
/// 用它的地方只有一处：超管在管理面板里手填码（`service::create_invites`），
/// 兑换侧一律走 [`normalize_code`]（那里绝不能抹横杠，否则 `ADMIN-xxx` 会退化）。
pub fn normalize_custom_code(raw: &str) -> String {
    normalize_code(raw).replace('-', "")
}

/// 自定义邀请码的格式校验（与系统生成同长同形）。
///
/// 规则三条，都是**故意**的：
/// ① **正好 16 位**（与 `CODE_LEN` 对齐）：管理员的码和系统码在列表 / 邮件 / 手抄里长得一样，
///    出了事也好一眼分辨「这是人定的码」；
/// ② **只用 A-Z 与 0-9**：不放行 `-`（会被 [`normalize_custom_code`] 抹掉）与其它符号，
///    免得出现连自己都打不出来的码；
/// ③ 字母表与系统生成**完全一致**（2026-10-05 起两边都是 A-Z + 0-9）：`MYCODE1234567890`
///    这种含 O/0 混杂的写法必须放行，随机码里同样会出现这些字符 ——
///    「我自己写的码能用 O，系统发的却永远见不到 O」这种不一致本身就是 bug。
pub fn validate_custom_code(raw: &str) -> Result<String, String> {
    let code = normalize_custom_code(raw);
    if code.is_empty() {
        return Err("自定义码不能为空".to_string());
    }
    if code.chars().count() != CODE_LEN {
        return Err(format!("自定义码要正好 {CODE_LEN} 位字符（现在 {} 位）", code.chars().count()));
    }
    if !code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
        return Err("自定义码只能用 0-9 与 A-Z（大小写不敏感，会统一转成大写）".to_string());
    }
    Ok(code)
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
    fn generated_code_uses_full_alphanumeric_alphabet_and_length() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)), "含字母表外的字符：{code}");
        // ⚠️ 这里**曾经**断言「不能出现 I/L/O/U」（Crockford 那套无易混字符的 Base32）。
        // 2026-10-05 用户要求「邀请码要 A-Z 和 0-9」，断言随之反过来：只要在大写字母与
        // 数字里，出现什么都正常 —— 别再把它当成 bug 修回去。
        assert!(code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
    }

    /// 锁住「字符表真的是 A-Z + 0-9 这 36 个」，而不是靠 `ALPHABET` 自己证明自己。
    ///
    /// 生成 2000 张（32000 个字符位）时，36 个字符每个的期望出现次数 ≈ 889 次，
    /// 漏掉任何一个的概率低到可以忽略（(35/36)^32000 ≈ 10^-386）。所以「全都见过一次」
    /// 是这张表完整的充分证据，也就顺带证明了它**没有**被换回 32 字符那套。
    #[test]
    fn alphabet_covers_every_letter_and_digit() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            seen.extend(generate_code().bytes());
        }
        let want: Vec<u8> = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_vec();
        let missing: Vec<char> = want.iter().filter(|b| !seen.contains(b)).map(|b| *b as char).collect();
        assert!(missing.is_empty(), "这些字符永远生成不出来：{missing:?}");
        assert_eq!(seen.len(), 36, "字母表应当正好 36 个字符，实际生成了 {} 种", seen.len());
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
        // 将来后台靠 `-` 前缀区分用途，这类前缀含 I/O 等字母，必须放行
        assert!(is_plausible("ADMIN-ABCD1234"));
        assert!(is_plausible("points-abcd1234"), "大小写不敏感");
        assert!(is_plausible("ABCD-EFGH-IJKL-MNOP"), "手抄分组");
        assert!(!is_plausible("ADMIN-AB"), "去掉 - 后不足 8 位");
        // 生成出来的码必须能通过预筛（2026-10-05 前这里断言的是「不会出现 I/L/O/U」；
        // 字符表放开成 A-Z + 0-9 之后，该断言换成了「生成的码本身就能过预筛」）
        assert!(is_plausible(&generate_code()));
        assert!(is_plausible(&generate_code_for(crate::models::ROLE_ADMIN)));
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

    #[test]
    fn custom_code_normalization_strips_hyphens_and_uppercases() {
        // 手写成 4 位一组的写法要能收（去掉横杠正好 16 位）
        assert_eq!(normalize_custom_code("abcd-efgh-jklm-npqt"), "ABCDEFGHJKLMNPQT");
        assert_eq!(normalize_custom_code(" my code 1234 5678 "), "MYCODE12345678");
        // ⚠️ 兑换侧那一套**不能**跟着变：用途前缀 `ADMIN-` 必须原样保留
        assert_eq!(normalize_code("ADMIN-ABCD1234"), "ADMIN-ABCD1234");
    }

    #[test]
    fn custom_code_validation_is_exactly_sixteen_alphanumeric() {
        // 小写要自动转大写
        assert_eq!(validate_custom_code("myCode1234567890").unwrap(), "MYCODE1234567890");
        // 含易混字符（O/0/I/1）照样放行 —— 2026-10-05 起随机生成用的也是同一套字符表，
        // 两边口径一致：**A-Z 与 0-9 都合法**，谁也不比谁宽
        assert_eq!(validate_custom_code("ADMIN-12345678901").unwrap(), "ADMIN12345678901");
    }

    #[test]
    fn custom_code_validation_rejects_bad_shapes() {
        for bad in ["", "   ", "SHORT", "ADMIN-123456789", "TOOLONG1234567890"] {
            assert!(validate_custom_code(bad).is_err(), "本应拒绝：{bad}");
        }
        // ⚠️ 长度与字符集是两道**分开**的检查，且长度在前：
        // `中文码1234567890` 只有 13 位，报的是「位数不够」而不是「字符不合法」——
        // 想验字符集就必须拿一个**长度正好 16** 的串（下面那条）。
        let err = validate_custom_code("中文码1234567890").unwrap_err();
        assert!(err.contains("16") && err.contains("13 位"), "13 个字符的串该报长度：{err}");
        // 这两条都是「长度正好 16、但字符不合法」：下划线 / 中文
        let err = validate_custom_code("MYCODE_123456789").unwrap_err();
        assert!(err.contains("0-9 与 A-Z"), "下划线不合法：{err}");
        let err = validate_custom_code("中文码4567890123456").unwrap_err();
        assert!(err.contains("0-9 与 A-Z"), "长度恰好 16 但不是 ASCII：{err}");
        // 长度不对时要报出实际位数，管理员才知道差几位
        let err = validate_custom_code("ABC123").unwrap_err();
        assert!(err.contains("16") && err.contains("6 位"), "报错要带位数：{err}");
    }
}
