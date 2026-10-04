//! 邮件发送：trait + 两种实现
//!
//! 本期默认走 `LogMailer`（只打日志、不发信），SMTP 实现已经写好，
//! 拿到邮箱授权码后把 `AUTH_MAIL_MODE=smtp` 与 `AUTH_SMTP_*` 配上即可启用。
//!
//! `last_dev_code` 是刻意的开发便利：验证码在库里只有 HMAC 摘要，
//! 无法反查，所以由**发送器自己**把最近一次明文留在内存里，
//! 仅供 `development` 的调试接口读取；SMTP 模式下一律返回 `None`。

pub mod log_mailer;
pub mod smtp;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::config::{Config, MailMode};
use crate::error::{AuthError, Result};

#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    /// 发送验证码邮件
    async fn send_code(&self, to: &str, code: &str, purpose: &str, ttl_seconds: i64) -> Result<()>;

    /// 取最近一次发给该邮箱的验证码明文（仅开发模式的 log 发送器实现）
    fn last_dev_code(&self, _to: &str) -> Option<String> {
        None
    }

    /// 当前模式（启动横幅与调试接口会展示）
    fn mode(&self) -> &'static str;
}

/// 按配置构造发送器
pub fn build(cfg: &Config) -> Result<Arc<dyn Mailer>> {
    match cfg.mail.mode {
        MailMode::Log => Ok(Arc::new(log_mailer::LogMailer::new())),
        MailMode::Smtp => {
            let smtp_cfg = cfg
                .mail
                .smtp
                .clone()
                .ok_or_else(|| AuthError::Internal("AUTH_MAIL_MODE=smtp 但缺少 SMTP 配置".into()))?;
            Ok(Arc::new(smtp::SmtpMailer::new(smtp_cfg)?))
        }
    }
}

/// 启动期的发邮件自检：**只连 TCP**，不登录、不发信、不带凭据。
///
/// 能查出来的：主机名写错、端口写错（比如 465 配了 STARTTLS 常用的 587）、出口被防火墙挡了、
/// DNS 解析不了、网络不通。查不出来的：授权码对不对（那要真登录一次才算数）。
///
/// ⚠️ **失败只告警，不阻止启动**：SMTP 挂了不该让已经登录的用户也没法用站点 ——
/// 能登录、能复习，只是注册收不到码而已。
pub async fn preflight(cfg: &Config) -> Option<String> {
    let smtp = cfg.mail.smtp.as_ref()?;
    let host = smtp.host.clone();
    let port = smtp.port;
    let timeout = smtp.timeout.min(Duration::from_secs(5));
    let target = format!("{host}:{port}");

    let attempt = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&target)).await;
    let message = match attempt {
        Ok(Ok(_)) => format!("能连上 {target}（TLS 模式 {}）", smtp.tls.as_str()),
        Ok(Err(e)) => format!("连不上 {target}：{e} —— 验证码发不出去，检查 AUTH_SMTP_HOST / AUTH_SMTP_PORT"),
        Err(_) => format!(
            "连不上 {target}：{} 秒超时 —— 检查 AUTH_SMTP_HOST / AUTH_SMTP_PORT，或被防火墙挡了",
            timeout.as_secs()
        ),
    };
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SmtpConfig;
    use std::time::Duration;

    fn cfg_with(host: &str, port: u16, timeout_secs: u64) -> Config {
        let mut cfg = Config::development();
        cfg.mail = crate::config::MailConfig {
            mode: MailMode::Smtp,
            smtp: Some(SmtpConfig {
                host: host.to_string(),
                port,
                username: "u@example.com".to_string(),
                password: "授权码".to_string(),
                from: "u@example.com".to_string(),
                tls: crate::config::SmtpTls::Implicit,
                timeout: Duration::from_secs(timeout_secs),
            }),
        };
        cfg
    }

    #[tokio::test]
    async fn preflight_is_silent_in_log_mode() {
        let cfg = Config::development();
        assert!(preflight(&cfg).await.is_none(), "log 模式不该做任何网络检查");
    }

    #[tokio::test]
    async fn preflight_reports_unreachable_host_without_failing() {
        // 保留地址段 + 没人监听的端口：一定连不上，调用方必须拿到一句能读的告警而不是 panic
        let cfg = cfg_with("127.0.0.1", 9, 1);
        let msg = preflight(&cfg).await.expect("smtp 模式必须有自检结果");
        assert!(msg.contains("连不上"), "告警里要说清「连不上」：{msg}");
        assert!(msg.contains("127.0.0.1:9"), "要带上目标地址，否则没法排查：{msg}");
    }

    #[tokio::test]
    async fn preflight_treats_a_listening_port_as_ok() {
        // 起一个本地监听当假 SMTP：自检只看 TCP 是否连得上（不看对方说什么）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("起监听");
        let port = listener.local_addr().unwrap().port();
        let cfg = cfg_with("127.0.0.1", port, 2);
        let msg = preflight(&cfg).await.expect("smtp 模式必须有自检结果");
        assert!(msg.starts_with("能连上"), "应当报能连上：{msg}");
        assert!(msg.contains("implicit"), "要带上 TLS 模式，方便核对配置：{msg}");
    }
}
