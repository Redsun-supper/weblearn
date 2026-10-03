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
use crate::http::extract::SuperAdminUser;
use crate::http::middleware::{client_ip, user_agent};
use crate::http::ok_msg;
use crate::models::{self, ROLE_ADMIN, ROLE_SUPER_ADMIN, ROLE_USER};
use crate::store::sql::InviteFilter;
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
    /// 生成几个（1~50）
    #[serde(default = "default_one")]
    pub count: i64,
    /// 每个可用几次（1~1000）
    #[serde(default = "default_one")]
    pub max_uses: i64,
    /// 有效期天数，0 表示不过期（上限 365）
    #[serde(default = "default_seven")]
    pub expires_in_days: i64,
    #[serde(default)]
    pub note: String,
    /// 兑换后授予的角色：`user`（默认）/ `admin`
    #[serde(default = "default_role_user")]
    pub grant_role: String,
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
pub async fn create_invites(
    State(state): State<AppState>,
    admin: SuperAdminUser,
    Json(req): Json<CreateReq>,
) -> Result<Json<Value>> {
    let created = state
        .service
        .create_invites(
            Some(admin.0.user_id),
            req.count,
            req.max_uses,
            req.expires_in_days,
            &req.note,
            &req.grant_role,
        )
        .await?;
    Ok(ok_msg(
        "邀请码已生成，请立即记录（明文只显示这一次）",
        json!({
            "items": created.codes,
            "codes": created.codes.iter().map(|c| c.code.clone()).collect::<Vec<_>>(),
            "grant_role": req.grant_role,
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

// ---------------------------------------------------------------- 用户治理

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
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

/// GET /api/auth/admin/users
pub async fn list_users(
    State(state): State<AppState>,
    _admin: SuperAdminUser,
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
    let (items, total) = state
        .service
        .list_users(query.role.clone(), query.status.clone(), page, size)
        .await?;
    Ok(ok_msg(
        "ok",
        json!({ "items": items, "total": total, "page": page.max(1), "size": size.clamp(1, 100) }),
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
