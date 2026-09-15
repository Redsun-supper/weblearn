//! SMTP 发送器（`lettre`）
//!
//! 本期默认不启用；配好 `AUTH_MAIL_MODE=smtp` 与 `AUTH_SMTP_*` 后即生效。
//! QQ 邮箱要填的是**设置里生成的 SMTP 授权码**，不是登录密码；
//! 465 端口用 `tls`（隐式 TLS），587 端口用 `starttls`。

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use crate::config::SmtpConfig;
use crate::error::{AuthError, Result};
use crate::mail::Mailer;

pub struct SmtpMailer {
    transport: SmtpTransport,
    from: String,
}

impl SmtpMailer {
    pub fn new(cfg: SmtpConfig) -> Result<Self> {
        let builder = match cfg.tls.as_str() {
            "none" | "plain" | "insecure" => SmtpTransport::builder_dangerous(cfg.host.clone()),
            "starttls" => SmtpTransport::starttls_relay(&cfg.host)
                .map_err(|e| AuthError::Internal(format!("SMTP 配置有误: {e}")))?,
            _ => SmtpTransport::relay(&cfg.host)
                .map_err(|e| AuthError::Internal(format!("SMTP 配置有误: {e}")))?,
        };
        let transport = builder
            .port(cfg.port)
            .timeout(Some(cfg.timeout))
            .credentials(Credentials::new(cfg.username.clone(), cfg.password.clone()))
            .build();
        Ok(Self { transport, from: cfg.from.clone() })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send_code(&self, to: &str, code: &str, purpose: &str, ttl_seconds: i64) -> Result<()> {
        let subject = match purpose {
            "register" => "广学 · 注册验证码",
            _ => "广学 · 邮箱验证码",
        };
        let minutes = (ttl_seconds / 60).max(1);
        let body = format!(
            "你的验证码是：{code}\n\n{minutes} 分钟内有效，请勿转发给他人。\n如果不是你本人操作，忽略本邮件即可。\n\n—— 广学"
        );
        let message = Message::builder()
            .from(
                self.from
                    .parse()
                    .map_err(|e| AuthError::Internal(format!("AUTH_SMTP_FROM 不是合法邮箱: {e}")))?,
            )
            .to(to.parse().map_err(|e| AuthError::Internal(format!("收件邮箱不合法: {e}")))?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .map_err(|e| AuthError::Internal(format!("构造邮件失败: {e}")))?;

        // lettre 的阻塞式 transport：放到阻塞线程池里发，别卡住异步运行时
        let transport = self.transport.clone();
        let result = tokio::task::spawn_blocking(move || transport.send(&message))
            .await
            .map_err(|e| AuthError::Internal(format!("邮件任务失败: {e}")))?;
        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                // 具体 SMTP 错误只进日志（可能含服务器信息），对外统一提示
                tracing::error!(error = %e, "SMTP 发送失败");
                Err(AuthError::MailFailed)
            }
        }
    }

    fn mode(&self) -> &'static str {
        "smtp"
    }
}
