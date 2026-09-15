//! 管理接口：邀请码的创建 / 列表 / 停用
//!
//! ⚠️ 准入用的是 `AdminUser`（`role == "admin"`）这一条最小检查；
//! 完整权限系统（RBAC / 权限点 / 用户管理）本期不做，只预留了 `users.role` 字段。

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AuthError, Result};
use crate::http::ok_msg;
use crate::http::extract::AdminUser;
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
}

fn default_one() -> i64 {
    1
}

fn default_seven() -> i64 {
    7
}

/// GET /api/auth/admin/invites
pub async fn list_invites(
    State(state): State<AppState>,
    _admin: AdminUser,
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
    admin: AdminUser,
    Json(req): Json<CreateReq>,
) -> Result<Json<Value>> {
    let created = state
        .service
        .create_invites(Some(admin.0.user_id), req.count, req.max_uses, req.expires_in_days, &req.note)
        .await?;
    Ok(ok_msg(
        "邀请码已生成，请立即记录（明文只显示这一次）",
        json!({ "items": created.codes, "codes": created.codes.iter().map(|c| c.code.clone()).collect::<Vec<_>>() }),
    ))
}

/// POST /api/auth/admin/invites/{id}/disable
pub async fn disable_invite(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    state.service.disable_invite(id).await?;
    Ok(ok_msg("已停用", json!({ "id": id })))
}
