// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 管理接口：邀请码（超管）+ 用户治理（超管）
//!
//! 权限矩阵（见 `docs/launch-plan.md` 第 3 节）：
//!
//! | 接口 | 准入 |
//! |---|---|
//! | 邀请码：创建 / 列表 / 停用 | **超管**（`SuperAdminUser`）——管理员碰不到权限 |
//! | 用户列表 | 超管（一期先只给超管；管理员只读用户列表是 P1 面板的事） |
//! | 改他人角色 / 封禁解封 | **超管** |
//!
//! ⚠️ 这里的每个接口都会写 `audit_logs`（治理动作必须留痕，见各 service 方法）。

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AuthError, Result};
use crate::http::extract::{AdminUser, SuperAdminUser};
use crate::http::middleware::{client_ip, user_agent};
use crate::http::ok_msg;
use crate::models::{self, ROLE_ADMIN, ROLE_SUPER_ADMIN, ROLE_USER};
use crate::service::InviteSpec;
use crate::store::sql::{AuditFilter, InviteFilter};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReq {
    /// 生成几个（1~50）；填了 `custom_code` 时忽略它（一次只出 1 张）
    #[serde(default = "default_one")]
    pub count: i64,
    /// 每个可用几次（1~1000）
    #[serde(default = "default_one")]
    pub max_uses: i64,
    /// 有效期天数，**0 或负数表示不过期**（面板上的「永不过期」勾选框），上限 365
    #[serde(default = "default_seven")]
    pub expires_in_days: i64,
    #[serde(default)]
    pub note: String,
    /// 兑换后授予的角色：`user`（默认）/ `admin`
    #[serde(default = "default_role_user")]
    pub grant_role: String,
    /// 超管**自己指定**的码（可空 = 照旧随机生成）：
    /// 正好 16 位、A-Z 与 0-9，自动转大写、自动抹掉手写的 `-`（见 `invite::validate_custom_code`）
    #[serde(default)]
    pub custom_code: Option<String>,
    /// 自定义码**已经存在**时是否照用不误。
    ///
    /// ⚠️ 默认 `false`：第一次请求会拿到 409 `invite_code_taken`（带那张码的状态与谁用过），
    /// 面板弹完确认框、用户点「继续」之后才把这一项置 true 重发。这条路径的存在就是为了
    /// **不让人在不知情的情况下把一张已经发出去的码当成新码又发一遍**。
    #[serde(default)]
    pub allow_existing: bool,
}

fn default_one() -> i64 {
    1
}

fn default_seven() -> i64 {
    7
}

fn default_role_user() -> String {
    ROLE_USER.to_string()
}

/// GET /api/auth/admin/invites
pub async fn list_invites(
    State(state): State<AppState>,
    _admin: SuperAdminUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>> {
    let filter = match query.status.as_deref() {
        None => InviteFilter::All,
        Some(raw) => InviteFilter::parse(raw)
            .ok_or_else(|| AuthError::InvalidParams("status 只能是 unused/used/expired/disabled/all".into()))?,
    };
    let page = query.page.unwrap_or(1);
    let size = query.size.unwrap_or(20);
    let (items, total) = state.service.list_invites(filter, page, size).await?;
    Ok(ok_msg(
        "ok",
        json!({ "items": items, "total": total, "page": page.max(1), "size": size.clamp(1, 100) }),
    ))
}

/// POST /api/auth/admin/invites —— 明文邀请码只在这次响应里出现
///
/// 两种用法：不传 `custom_code` = 随机生成 `count` 张；传了 = 用超管指定的那串码出 1 张
/// （那串码已存在时先回 409 `invite_code_taken`，带 `allow_existing: true` 重发才照用）。
pub async fn create_invites(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    Json(req): Json<CreateReq>,
) -> Result<Json<Value>> {
    let custom = req.custom_code.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let created = state
        .service
        .create_invites(
            Some(admin.0.user_id),
            InviteSpec {
                count: req.count,
                max_uses: req.max_uses,
                expires_in_days: req.expires_in_days,
                note: req.note.clone(),
                grant_role: req.grant_role.clone(),
                custom_code: custom.map(str::to_string),
                allow_existing: req.allow_existing,
            },
        )
        .await?;
    // 自定义码成功时要让面板知道「这张码是本来就有的、还是刚建出来的」
    let message = if created.reused_existing {
        "已沿用这张已经存在的码（额度/有效期按这次填的改，兑换记录与人都不受影响）"
    } else if custom.is_some() {
        "邀请码已就绪（明文只显示这一次）"
    } else {
        "邀请码已生成，请立即记录（明文只显示这一次）"
    };
    Ok(ok_msg(
        message,
        json!({
            "items": created.codes,
            "codes": created.codes.iter().map(|c| c.code.clone()).collect::<Vec<_>>(),
            "grant_role": req.grant_role,
            "custom": created.custom,
            "reused": created.reused_existing,
        }),
    ))
}

/// POST /api/auth/admin/invites/{id}/disable
pub async fn disable_invite(
    State(state): State<AppState>,
    _admin: SuperAdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    state.service.disable_invite(id).await?;
    Ok(ok_msg("已停用", json!({ "id": id })))
}

/// POST /api/auth/admin/invites/{id}/reset —— 重新启用一张**已用过**的码（超管）
///
/// 面板上带风险确认才允许点（`admin/panels/invites.js`）：它把 `used_count` 清零，
/// 让这张已经流出去的码重新可用。兑换记录不删，只写 `invite_reset` 审计。
pub async fn reset_invite(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    let cleared = state
        .service
        .reset_invite(Some(admin.0.user_id), id, &client_ip(&headers), &user_agent(&headers))
        .await?;
    let msg = if cleared > 0 {
        format!("已重新启用（清掉 {cleared} 次使用记录，之前的兑换记录仍然保留）")
    } else {
        "这张码本来就没被用过（没有需要清零的记录）".to_string()
    };
    Ok(ok_msg(&msg, json!({ "id": id, "cleared": cleared })))
}

// ---------------------------------------------------------------- 用户治理

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// 搜索：同时匹配邮箱与用户名（子串）
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RoleReq {
    /// `user` / `admin` / `super_admin`
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct StatusReq {
    /// `active` / `disabled`
    pub status: String,
}

/// GET /api/auth/admin/users —— 用户列表
///
/// 准入是**管理员**（不是超管）：权限矩阵里管理员本来就能只读用户列表（P1 定案）。
/// 但邮箱要脱敏 —— 管理员没有看别人邮箱的理由，而超管要能按邮箱定位账号。
/// 响应里带 `email_masked` 说明「你看到的邮箱是打过码的」，免得前端把它当完整邮箱用
/// （例如拿去做「复制邮箱」按钮，复制到一串 `22***@qq.com`）。
pub async fn list_users(
    State(state): State<AppState>,
    admin: AdminUser,
    Query(query): Query<UserListQuery>,
) -> Result<Json<Value>> {
    if let Some(role) = query.role.as_deref() {
        if !models::is_valid_grant_role(role) && role != ROLE_SUPER_ADMIN {
            return Err(AuthError::InvalidParams(
                "role 只能是 user / admin / super_admin".into(),
            ));
        }
    }
    let page = query.page.unwrap_or(1);
    let size = query.size.unwrap_or(20);
    let viewer_is_super = models::is_super_role(&admin.0.role);
    let (items, total) = state
        .service
        .list_users(query.role.clone(), query.status.clone(), query.keyword.clone(), page, size)
        .await?;
    let mut items = items;
    if !viewer_is_super {
        for item in items.iter_mut() {
            item.email = crate::core::validate::mask_email(&item.email);
        }
    }
    Ok(ok_msg(
        "ok",
        json!({
            "items": items,
            "total": total,
            "page": page.max(1),
            "size": size.clamp(1, 100),
            "email_masked": !viewer_is_super,
        }),
    ))
}

/// POST /api/auth/admin/users/{id}/role —— 改角色（含自锁保护）
pub async fn change_user_role(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<RoleReq>,
) -> Result<Json<Value>> {
    let user = state
        .service
        .change_user_role(admin.0.user_id, id, &req.role, &client_ip(&headers), &user_agent(&headers))
        .await?;
    Ok(ok_msg("角色已更新", json!({ "user": user })))
}

/// POST /api/auth/admin/users/{id}/status —— 封禁 / 解封（含自锁保护）
pub async fn change_user_status(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<StatusReq>,
) -> Result<Json<Value>> {
    let user = state
        .service
        .set_user_status(admin.0.user_id, id, &req.status, &client_ip(&headers), &user_agent(&headers))
        .await?;
    let action = if req.status == "disabled" { "已封禁" } else { "已解封" };
    Ok(ok_msg(action, json!({ "user": user })))
}

/// 供前端显示「可选角色」时对齐（避免前端硬编码字符串）
pub const ASSIGNABLE_ROLES: [&str; 3] = [ROLE_USER, ROLE_ADMIN, ROLE_SUPER_ADMIN];

// ---------------------------------------------------------------- 审计日志（超管）

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// 动作名（取值见响应里的 `actions`，别在前端硬编码）
    #[serde(default)]
    pub action: Option<String>,
    /// 只看某个操作者（用户 id）
    #[serde(default)]
    pub actor: Option<i64>,
    /// 起始时间（含）
    #[serde(default)]
    pub from: Option<String>,
    /// 结束时间（含）
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub size: Option<i64>,
}

/// GET /api/auth/admin/audit —— 审计日志（超管）
///
/// 响应里带 `actions`（库里出现过的全部动作）：前端筛选下拉直接用它，
/// 既不硬编码、也不用为了一个下拉再发一次请求。
pub async fn list_audit(
    State(state): State<AppState>,
    _admin: SuperAdminUser,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Value>> {
    let filter = AuditFilter {
        action: query.action.clone().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()),
        actor_user_id: query.actor,
        from: parse_query_time(query.from.as_deref())?,
        to: parse_query_time(query.to.as_deref())?,
    };
    let page = query.page.unwrap_or(1);
    let size = query.size.unwrap_or(20);
    let (items, total, actions) = state.service.list_audit(filter, page, size).await?;
    Ok(ok_msg(
        "ok",
        json!({
            "items": items,
            "total": total,
            "page": page.max(1),
            "size": size.clamp(1, 100),
            "actions": actions,
        }),
    ))
}

/// 解析审计查询里的时间参数：接受 `2026-10-01`（= 当天 00:00:00 UTC）、
/// `2026-10-01T12:30:00Z`（RFC3339）与库内格式 `2026-10-01T12:30:00Z`。
///
/// ⚠️ 只给日期时**不做**「to 自动补到当天 23:59:59」的猜测：猜错了会静默少查一整天数据，
/// 那种错误看不出来。要含整天就显式写到 `T23:59:59Z`。
fn parse_query_time(raw: Option<&str>) -> Result<Option<time::OffsetDateTime>> {
    let Some(raw) = raw.map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if let Ok(parsed) = time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339)
    {
        return Ok(Some(parsed));
    }
    if let Some(parsed) = crate::db::parse_ts(raw) {
        return Ok(Some(parsed));
    }
    if let Ok(date) = time::Date::parse(raw, &time::macros::format_description!("[year]-[month]-[day]")) {
        return Ok(Some(date.midnight().assume_utc()));
    }
    Err(AuthError::InvalidParams(format!(
        "时间格式不对：{raw}（可用 2026-10-01 或 2026-10-01T12:30:00Z）"
    )))
}

// ---------------------------------------------------------------- 邀请码：按批停用 / 整批发邮件

/// POST /api/auth/admin/invite-batches/{batch_id}/disable —— 整批停用（超管）
pub async fn disable_invites_by_batch(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    headers: HeaderMap,
    Path(batch_id): Path<String>,
) -> Result<Json<Value>> {
    let n = state
        .service
        .disable_invites_by_batch(
            Some(admin.0.user_id),
            &batch_id,
            &client_ip(&headers),
            &user_agent(&headers),
        )
        .await?;
    Ok(ok_msg(
        &format!("已停用 {n} 张"),
        json!({ "batch_id": batch_id, "disabled": n }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct InviteMailReq {
    /// **一对一**配对：第 i 张码发给第 i 个邮箱
    pub pairs: Vec<InviteMailPair>,
}

#[derive(Debug, Deserialize)]
pub struct InviteMailPair {
    pub invite_id: i64,
    pub email: String,
}

/// POST /api/auth/admin/invite-mail —— 把指定邀请码发给指定邮箱（超管）
///
/// ⚠️ `AUTH_MAIL_MODE=log`（开发默认）时**不会真的发信**，只把码写进服务端日志：
/// 响应里回一个 `mail_mode`，界面据此提示操作者「本机没配 SMTP，码在日志里」。
pub async fn send_invites_email(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    headers: HeaderMap,
    Json(req): Json<InviteMailReq>,
) -> Result<Json<Value>> {
    let pairs: Vec<(i64, String)> = req.pairs.into_iter().map(|p| (p.invite_id, p.email)).collect();
    let results = state
        .service
        .send_invites_by_email(admin.0.user_id, pairs, &client_ip(&headers), &user_agent(&headers))
        .await?;
    let sent = results.iter().filter(|r| r.ok).count();
    let failed = results.len() - sent;
    Ok(ok_msg(
        &format!("成功 {sent} 封，失败 {failed} 封"),
        json!({
            "items": results,
            "sent": sent,
            "failed": failed,
            "mail_mode": state.mailer.mode(),
        }),
    ))
}

// ---------------------------------------------------------------- 用户治理

/// POST /api/auth/admin/users/{id}/logout-all —— 踢下线（吊销该用户全部会话，超管）
pub async fn revoke_user_sessions(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    let n = state
        .service
        .revoke_user_sessions_admin(admin.0.user_id, id, &client_ip(&headers), &user_agent(&headers))
        .await?;
    Ok(ok_msg(
        &format!("已吊销 {n} 个会话"),
        json!({ "user_id": id, "revoked": n }),
    ))
}
