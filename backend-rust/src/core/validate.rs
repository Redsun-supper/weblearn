//! 入参校验与文本规范化（不引正则，规则写死在代码里并逐条测试）

/// 截断到 `max` 个字符（按字符而不是字节，避免截断出半个汉字）
pub fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// 邮箱规范化：去空白、转小写；不合法返回 `None`
///
/// 刻意不接受 `a@b`（顶级域至少两位字母）与任何带空格/控制字符的写法：
/// 邮箱是账号主键，宽松校验会让「同一邮箱的两种写法」变成两个账号。
pub fn normalize_email(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 254 {
        return None;
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (local, domain) = s.split_once('@')?;
    if local.is_empty() || local.len() > 64 {
        return None;
    }
    if domain.is_empty() || !domain.contains('.') {
        return None;
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return None;
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return None;
    }
    let domain_ok = domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let local_ok = local.chars().all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c));
    if !domain_ok || !local_ok {
        return None;
    }
    let tld = domain.rsplit('.').next().unwrap_or("");
    if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(s.to_ascii_lowercase())
}

/// 邮箱打码（日志用）：`2262997289@qq.com` → `22***@qq.com`
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        None => "***".to_string(),
        Some((local, domain)) => {
            let head: String = local.chars().take(2).collect();
            format!("{head}***@{domain}")
        }
    }
}

/// 口令强度：8~128 个字符，且同时包含字母与数字
pub fn validate_password(password: &str) -> Result<(), String> {
    let len = password.chars().count();
    if len < 8 {
        return Err("口令至少 8 个字符".to_string());
    }
    if len > 128 {
        return Err("口令最长 128 个字符".to_string());
    }
    if !password.chars().any(|c| c.is_ascii_alphabetic()) {
        return Err("口令需要包含字母".to_string());
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err("口令需要包含数字".to_string());
    }
    Ok(())
}

/// 用户名规范化：可空，最多 50 字符
pub fn normalize_username(raw: Option<&str>) -> Option<String> {
    let s = raw.unwrap_or("").trim();
    if s.is_empty() {
        return None;
    }
    Some(truncate(s, 50))
}

/// 设备名：优先用客户端传的，其次从 User-Agent 猜一个（多端会话列表要能区分）
pub fn device_label(raw: Option<&str>, user_agent: &str) -> String {
    if let Some(label) = raw {
        let label = label.trim();
        if !label.is_empty() {
            return truncate(label, 40);
        }
    }
    let ua = user_agent.to_ascii_lowercase();
    let os = if ua.contains("iphone") {
        "iPhone"
    } else if ua.contains("ipad") {
        "iPad"
    } else if ua.contains("android") {
        "Android"
    } else if ua.contains("windows") {
        "Windows"
    } else if ua.contains("mac os") || ua.contains("macintosh") {
        "macOS"
    } else if ua.contains("linux") {
        "Linux"
    } else {
        "未知设备"
    };
    let browser = if ua.contains("edg/") {
        "Edge"
    } else if ua.contains("chrome/") {
        "Chrome"
    } else if ua.contains("firefox/") {
        "Firefox"
    } else if ua.contains("safari/") {
        "Safari"
    } else if ua.contains("curl") {
        "curl"
    } else if ua.contains("powershell") {
        "PowerShell"
    } else {
        "浏览器"
    };
    format!("{os} · {browser}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization_accepts_common_forms() {
        assert_eq!(normalize_email("  2262997289@QQ.com ").as_deref(), Some("2262997289@qq.com"));
        assert_eq!(normalize_email("a.b+c@sub.example.com.cn").as_deref(), Some("a.b+c@sub.example.com.cn"));
    }

    #[test]
    fn email_normalization_rejects_bad_forms() {
        for bad in [
            "",
            "   ",
            "no-at-sign",
            "@example.com",
            "a@",
            "a@b",
            "a@b.c",
            "a@@example.com",
            "a b@example.com",
            "a@exam ple.com",
            "a@.example.com",
            "a@example..com",
            "a@example.1com",
            ".a@example.com",
            "a.@example.com",
        ] {
            assert!(normalize_email(bad).is_none(), "本应拒绝：{bad}");
        }
    }

    #[test]
    fn email_too_long_is_rejected() {
        let long = format!("{}@example.com", "a".repeat(250));
        assert!(normalize_email(&long).is_none());
    }

    #[test]
    fn mask_email_hides_local_part() {
        assert_eq!(mask_email("2262997289@qq.com"), "22***@qq.com");
        assert_eq!(mask_email("a@b.com"), "a***@b.com");
        assert_eq!(mask_email("weird"), "***");
    }

    #[test]
    fn password_policy_requires_letter_and_digit() {
        assert!(validate_password("7289HR_RedSun").is_ok());
        assert!(validate_password("abcdefgh").is_err(), "纯字母不通过");
        assert!(validate_password("12345678").is_err(), "纯数字不通过");
        assert!(validate_password("abc1234").is_err(), "太短不通过");
        assert!(validate_password(&"a1".repeat(100)).is_err(), "太长不通过");
    }

    #[test]
    fn username_and_device_label_are_trimmed_and_capped() {
        assert_eq!(normalize_username(Some("  小明  ")).as_deref(), Some("小明"));
        assert_eq!(normalize_username(Some("   ")), None);
        assert_eq!(normalize_username(Some(&"字".repeat(80))).unwrap().chars().count(), 50);

        assert_eq!(device_label(Some(" 我的手机 "), "xxx"), "我的手机");
        assert_eq!(
            device_label(None, "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) AppleWebKit/605.1.15 Safari/604.1"),
            "iPhone · Safari"
        );
        assert_eq!(device_label(None, "curl/8.4.0"), "未知设备 · curl");
    }
}
