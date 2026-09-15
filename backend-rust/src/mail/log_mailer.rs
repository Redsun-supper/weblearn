//! 开发模式的「邮件发送器」：不发信，只把验证码写进服务端日志与内存

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::core::validate::mask_email;
use crate::error::Result;
use crate::mail::Mailer;

/// 内存里保留多少条最近验证码（防止长期运行无限增长）
const MAX_REMEMBERED: usize = 200;

#[derive(Debug, Default)]
pub struct LogMailer {
    codes: Mutex<HashMap<String, String>>,
}

impl LogMailer {
    pub fn new() -> Self {
        Self { codes: Mutex::new(HashMap::new()) }
    }
}

#[async_trait]
impl Mailer for LogMailer {
    async fn send_code(&self, to: &str, code: &str, purpose: &str, ttl_seconds: i64) -> Result<()> {
        {
            let mut codes = self.codes.lock().unwrap();
            if codes.len() >= MAX_REMEMBERED && !codes.contains_key(to) {
                codes.clear();
            }
            codes.insert(to.to_string(), code.to_string());
        }
        // 邮箱打码：日志里不需要完整邮箱
        tracing::info!(
            target: "mail",
            email = %mask_email(to),
            code = %code,
            purpose = %purpose,
            ttl_seconds,
            "【开发模式】验证码不会真的发信，请从这条日志或 /api/auth/dev/codes 获取"
        );
        Ok(())
    }

    fn last_dev_code(&self, to: &str) -> Option<String> {
        self.codes.lock().unwrap().get(to).cloned()
    }

    fn mode(&self) -> &'static str {
        "log"
    }
}
