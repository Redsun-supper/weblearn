// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 数据模型：数据库行（`*Row`）与对外输出（`*Public`）
//!
//! 约定：`password_hash` / `refresh_hash` / `code_hash` 只存在于 `*Row`，
//! 绝不进入任何 `Serialize` 结构——这样「密码与令牌摘要不会出现在响应里」
//! 是类型层面的保证，而不是靠人工检查。

use serde::Serialize;
use time::OffsetDateTime;

use crate::db::ts;

// ---------------------------------------------------------------- 角色
//
// ⚠️ 这三个字符串是**跨服务的契约**：Rust 账号服务写进 `users.role`，
// Go 后端与前端拿它做准入判定（`backend-go/middleware/auth.go` 里的同名常量）。
// 任何一边改了字面量，另一边就会静默失配——症状是「后台能进、接口 403」这类灵异现象，
// 所以两侧都有测试把字面量钉死。

/// 普通用户：只能复习、看自己的数据
pub const ROLE_USER: &str = "user";
/// 管理员：内容 / 运营角色（管词条、看数据、看用户），**碰不到权限**
pub const ROLE_ADMIN: &str = "admin";
/// 超级管理员：治理角色（发码、调权限、封号、查审计）
pub const ROLE_SUPER_ADMIN: &str = "super_admin";

/// 这个角色能不能进后台（管理员与超管都可以）
pub fn can_enter_admin(role: &str) -> bool {
    role == ROLE_ADMIN || role == ROLE_SUPER_ADMIN
}

/// 这个角色能不能做治理动作（发码 / 改他人角色 / 封禁）
pub fn is_super_role(role: &str) -> bool {
    role == ROLE_SUPER_ADMIN
}

/// 角色是否合法（用于发码时校验 grant_role）
pub fn is_valid_grant_role(role: &str) -> bool {
    role == ROLE_USER || role == ROLE_ADMIN
}

/// 角色权重：用来判断「提升 / 降级」，以及多张码取最高等级
pub fn role_rank(role: &str) -> i32 {
    match role {
        ROLE_SUPER_ADMIN => 3,
        ROLE_ADMIN => 2,
        _ => 1,
    }
}

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
        can_enter_admin(&self.role)
    }

    pub fn is_super_admin(&self) -> bool {
        is_super_role(&self.role)
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
            last_login_at: self.last_login_at.map(ts),
            created_at: ts(self.created_at),
        }
    }
}

/// 对外的用户信息（无密码、无内部计数）
#[derive(Debug, Clone, Serialize)]
pub struct UserPublic {
    pub id: i64,
    /// ⚠️ 普通管理员在管理面板里看到的是**打码后**的邮箱（`22***@qq.com`）：
    /// 脱敏在 HTTP 层做（见 `http/admin.rs` 的 `list_users`），服务层始终返回真值。
    pub email: String,
    pub username: Option<String>,
    pub role: String,
    pub status: String,
    pub email_verified_at: Option<String>,
    /// 最后登录时间（P1 管理面板的用户列表要显示它；可空 = 从没登录过）
    pub last_login_at: Option<String>,
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
    /// 兑换后得到的角色（`user` / `admin`）——**这才是真正的判定**，
    /// 码上那个 `ADMIN-` 前缀只是给人看的
    pub grant_role: String,
    /// 一次生成的一批（同一批共享），便于按批回收与统计
    pub batch_id: String,
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
            grant_role: self.grant_role.clone(),
            batch_id: self.batch_id.clone(),
            // 兑换记录由服务层单独查（这里只保证字段存在，前端不必判 undefined）
            uses: Vec::new(),
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
///
/// `PartialEq/Eq` 是为了能塞进 `AuthError::InviteCodeTaken`（错误类型带 Eq 派生）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// 兑换后得到的角色（`user` / `admin`）
    pub grant_role: String,
    /// 生成批次（同一次生成的码共享）
    pub batch_id: String,
    /// 最近几条兑换记录（谁用了 / 邮箱），列表页显示用。
    /// [`InviteRow::public`] 只给空数组 —— 它不知道兑换记录，
    /// 由服务层查出来再挂上（见 `AuthService::list_invites`）。
    #[serde(default)]
    pub uses: Vec<InviteUsePublic>,
}

/// 邀请码兑换记录（一行 = 一次成功兑换）
#[derive(Debug, Clone)]
pub struct InviteUseRow {
    pub invite_code_id: i64,
    pub user_id: Option<i64>,
    pub email: String,
    pub ip: String,
    pub used_at: OffsetDateTime,
}

impl InviteUseRow {
    pub fn public(&self) -> InviteUsePublic {
        InviteUsePublic {
            user_id: self.user_id,
            email: self.email.clone(),
            ip: self.ip.clone(),
            used_at: ts(self.used_at),
        }
    }
}

/// 对外的兑换记录
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InviteUsePublic {
    pub user_id: Option<i64>,
    pub email: String,
    pub ip: String,
    pub used_at: String,
}

/// 审计日志行（查询用；写入用 [`NewAudit`]）
#[derive(Debug, Clone)]
pub struct AuditRow {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    /// 关联查出来的操作者邮箱（`LEFT JOIN users`）：账号没了或动作没有操作者时为空
    pub actor_email: Option<String>,
    pub action: String,
    pub target: String,
    pub ip: String,
    pub user_agent: String,
    pub detail: String,
    pub created_at: OffsetDateTime,
}

impl AuditRow {
    pub fn public(&self) -> AuditPublic {
        AuditPublic {
            id: self.id,
            actor_user_id: self.actor_user_id,
            actor_email: self.actor_email.clone(),
            action: self.action.clone(),
            target: self.target.clone(),
            ip: self.ip.clone(),
            user_agent: self.user_agent.clone(),
            detail: self.detail.clone(),
            created_at: ts(self.created_at),
        }
    }
}

/// 对外的审计日志
#[derive(Debug, Clone, Serialize)]
pub struct AuditPublic {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub actor_email: Option<String>,
    pub action: String,
    pub target: String,
    pub ip: String,
    pub user_agent: String,
    pub detail: String,
    pub created_at: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ 这三个字面量是**跨服务契约**（Go 侧 `middleware.RoleSuperAdmin` 必须逐字相同）：
    /// 改了这里而没改 Go，症状是「后台能进、接口 403」。这条测试的价值就是让改动卡在这里。
    #[test]
    fn role_strings_are_the_cross_service_contract() {
        assert_eq!(ROLE_USER, "user");
        assert_eq!(ROLE_ADMIN, "admin");
        assert_eq!(ROLE_SUPER_ADMIN, "super_admin");
    }

    #[test]
    fn admin_gate_accepts_both_admin_and_super_admin() {
        assert!(can_enter_admin(ROLE_ADMIN));
        assert!(can_enter_admin(ROLE_SUPER_ADMIN));
        assert!(!can_enter_admin(ROLE_USER));
        assert!(!can_enter_admin(""));
        assert!(!can_enter_admin("root"), "不认识的字符串一律不放行");
    }

    #[test]
    fn super_gate_is_strictly_narrower() {
        assert!(is_super_role(ROLE_SUPER_ADMIN));
        assert!(!is_super_role(ROLE_ADMIN), "管理员不能做治理动作（发码、调权限）");
        assert!(!is_super_role(ROLE_USER));
    }

    #[test]
    fn grant_role_only_allows_user_or_admin() {
        assert!(is_valid_grant_role(ROLE_USER));
        assert!(is_valid_grant_role(ROLE_ADMIN));
        // 本期刻意不支持超管码：一张码就能再造一个能封你号的人
        assert!(!is_valid_grant_role(ROLE_SUPER_ADMIN));
        assert!(!is_valid_grant_role("root"));
    }

    #[test]
    fn role_rank_orders_user_below_admin_below_super() {
        assert!(role_rank(ROLE_USER) < role_rank(ROLE_ADMIN));
        assert!(role_rank(ROLE_ADMIN) < role_rank(ROLE_SUPER_ADMIN));
        assert_eq!(role_rank("nonsense"), role_rank(ROLE_USER), "未知角色按最低算");
    }
}
