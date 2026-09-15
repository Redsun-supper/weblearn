//! 数据模型：数据库行（`*Row`）与对外输出（`*Public`）
//!
//! 约定：`password_hash` / `refresh_hash` / `code_hash` 只存在于 `*Row`，
//! 绝不进入任何 `Serialize` 结构——这样「密码与令牌摘要不会出现在响应里」
//! 是类型层面的保证，而不是靠人工检查。

use serde::Serialize;
use time::OffsetDateTime;

use crate::db::ts;

/// 用户行（含密码哈希，仅服务端使用）
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: i64,
    pub email: String,
    pub username: Option<String>,
    pub password_hash: String,
    /// 权限等级：**预留字段**，本期不写判定逻辑
    pub role: String,
    /// 账号状态：**预留字段**（active / disabled）
    pub status: String,
    pub email_verified_at: Option<OffsetDateTime>,
    pub failed_attempts: i64,
    pub locked_until: Option<OffsetDateTime>,
    pub last_login_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl UserRow {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    /// 锁是否仍然有效
    pub fn is_locked(&self, now: OffsetDateTime) -> bool {
        matches!(self.locked_until, Some(until) if until > now)
    }

    pub fn public(&self) -> UserPublic {
        UserPublic {
            id: self.id,
            email: self.email.clone(),
            username: self.username.clone(),
            role: self.role.clone(),
            status: self.status.clone(),
            email_verified_at: self.email_verified_at.map(ts),
            created_at: ts(self.created_at),
        }
    }
}

/// 对外的用户信息（无密码、无内部计数）
#[derive(Debug, Clone, Serialize)]
pub struct UserPublic {
    pub id: i64,
    pub email: String,
    pub username: Option<String>,
    pub role: String,
    pub status: String,
    pub email_verified_at: Option<String>,
    pub created_at: String,
}

/// 会话行（一行 = 一个端的登录）
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub user_id: i64,
    pub family_id: String,
    pub refresh_hash: String,
    pub device_label: String,
    pub user_agent: String,
    pub ip: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub expires_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_reason: Option<String>,
    pub replaced_by: Option<i64>,
}

impl SessionRow {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    /// 是否仍然可用（未吊销、未过期）
    pub fn is_usable(&self, now: OffsetDateTime) -> bool {
        !self.is_revoked() && self.expires_at > now
    }

    pub fn public(&self, current_session_id: i64) -> SessionPublic {
        SessionPublic {
            id: self.id,
            device_label: self.device_label.clone(),
            ip: self.ip.clone(),
            user_agent: self.user_agent.clone(),
            created_at: ts(self.created_at),
            last_used_at: self.last_used_at.map(ts),
            expires_at: ts(self.expires_at),
            current: self.id == current_session_id,
        }
    }
}

/// 对外的会话信息
#[derive(Debug, Clone, Serialize)]
pub struct SessionPublic {
    pub id: i64,
    pub device_label: String,
    pub ip: String,
    pub user_agent: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    pub current: bool,
}

/// 邀请码行
#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: i64,
    pub code: String,
    pub note: String,
    pub max_uses: i64,
    pub used_count: i64,
    pub expires_at: Option<OffsetDateTime>,
    pub disabled: bool,
    pub created_by: Option<i64>,
    pub created_at: OffsetDateTime,
}

impl InviteRow {
    pub fn public(&self, now: OffsetDateTime) -> InvitePublic {
        InvitePublic {
            id: self.id,
            code: self.code.clone(),
            note: self.note.clone(),
            max_uses: self.max_uses,
            used_count: self.used_count,
            expires_at: self.expires_at.map(ts),
            disabled: self.disabled,
            created_at: ts(self.created_at),
            status: self.status(now).to_string(),
        }
    }

    pub fn status(&self, now: OffsetDateTime) -> &'static str {
        if self.disabled {
            "disabled"
        } else if matches!(self.expires_at, Some(e) if e <= now) {
            "expired"
        } else if self.used_count >= self.max_uses {
            "used"
        } else {
            "unused"
        }
    }
}

/// 对外的邀请码信息
#[derive(Debug, Clone, Serialize)]
pub struct InvitePublic {
    pub id: i64,
    pub code: String,
    pub note: String,
    pub max_uses: i64,
    pub used_count: i64,
    pub expires_at: Option<String>,
    pub disabled: bool,
    pub created_at: String,
    /// unused / used / expired / disabled
    pub status: String,
}

/// 邮箱验证码行（只存摘要）
#[derive(Debug, Clone)]
pub struct EmailCodeRow {
    pub id: i64,
    pub email: String,
    pub purpose: String,
    pub code_hash: String,
    pub attempts: i64,
    pub expires_at: OffsetDateTime,
    pub consumed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub ip: String,
}

/// 审计日志条目（写入用）
#[derive(Debug, Clone)]
pub struct NewAudit {
    pub actor_user_id: Option<i64>,
    pub action: String,
    pub target: String,
    pub ip: String,
    pub user_agent: String,
    pub detail: String,
    pub created_at: OffsetDateTime,
}

impl NewAudit {
    pub fn new(action: &str, now: OffsetDateTime) -> Self {
        Self {
            actor_user_id: None,
            action: action.to_string(),
            target: String::new(),
            ip: String::new(),
            user_agent: String::new(),
            detail: String::new(),
            created_at: now,
        }
    }

    pub fn actor(mut self, user_id: Option<i64>) -> Self {
        self.actor_user_id = user_id;
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    pub fn client(mut self, ip: &str, user_agent: &str) -> Self {
        self.ip = ip.to_string();
        self.user_agent = crate::core::validate::truncate(user_agent, 300);
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

/// 验证码用途（本期只有注册；找回密码等留待后续）
pub const PURPOSE_REGISTER: &str = "register";
