// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
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
    /// 最近一次发给某个邮箱的**邀请码**（「整批发邮件」在开发模式下走这里；
    /// 集成测试也用 `last_dev_invite` 断言「信到底发出去没有、发的是哪张码」）
    invites: Mutex<HashMap<String, String>>,
}

impl LogMailer {
    pub fn new() -> Self {
        Self { codes: Mutex::new(HashMap::new()), invites: Mutex::new(HashMap::new()) }
    }

    /// 记住一条「邮箱 → 码」；超过上限且是新邮箱时整体清空
    /// （与验证码同样的策略：这里只是开发期的便利，不做真正的 LRU）
    fn remember(map: &Mutex<HashMap<String, String>>, to: &str, code: &str) {
        let mut guard = map.lock().unwrap();
        if guard.len() >= MAX_REMEMBERED && !guard.contains_key(to) {
            guard.clear();
        }
        guard.insert(to.to_string(), code.to_string());
    }
}

#[async_trait]
impl Mailer for LogMailer {
    async fn send_code(&self, to: &str, code: &str, purpose: &str, ttl_seconds: i64) -> Result<()> {
        Self::remember(&self.codes, to, code);
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

    async fn send_invite(&self, to: &str, code: &str, note: &str, expires_hint: &str) -> Result<()> {
        Self::remember(&self.invites, to, code);
        tracing::info!(
            target: "mail",
            email = %mask_email(to),
            code = %code,
            note = %note,
            expires_hint = %expires_hint,
            "【开发模式】邀请码邮件不会真的发信（AUTH_MAIL_MODE=smtp 才发），请从这条日志取码"
        );
        Ok(())
    }

    async fn send_notice(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        // 告警邮件不进内存表：没有任何地方会「把告警取回来对一下」。
        // 但正文要**完整**打进日志 —— log 模式下这是唯一的告警出口，
        // 而 AUTH_MAIL_MODE 还没切 smtp 时正好也就是这个模式（本地与首次上线都会遇到）。
        tracing::warn!(
            target: "mail",
            email = %mask_email(to),
            subject = %subject,
            body = %body,
            "【开发模式】告警邮件不会真的发信（AUTH_MAIL_MODE=smtp 才发），内容见本条日志"
        );
        Ok(())
    }

    fn last_dev_code(&self, to: &str) -> Option<String> {
        self.codes.lock().unwrap().get(to).cloned()
    }
    fn last_dev_invite(&self, to: &str) -> Option<String> {
        self.invites.lock().unwrap().get(to).cloned()
    }

    fn mode(&self) -> &'static str {
        "log"
    }
}
