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
