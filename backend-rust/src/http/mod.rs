// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! HTTP 层：路由、Cookie、CSRF、限流与鉴权提取

pub mod admin;
pub mod auth;
pub mod dev;
pub mod extract;
pub mod middleware;

use axum::routing::{get, post};
use axum::{middleware as axum_mw, Json, Router};
use serde_json::{json, Value};

use crate::AppState;

/// access token 的 Cookie 名（`Path=/`，将来 Go 侧也能读它做鉴权）
pub const ACCESS_COOKIE: &str = "gx_access";
/// refresh token 的 Cookie 名（`Path=/api/auth`，只发给认证接口）
pub const REFRESH_COOKIE: &str = "gx_refresh";

/// 统一成功信封：`{code:200, message, data}`
pub(crate) fn ok(data: Value) -> Json<Value> {
    Json(json!({ "code": 200, "message": "ok", "data": data }))
}

pub(crate) fn ok_msg(message: &str, data: Value) -> Json<Value> {
    Json(json!({ "code": 200, "message": message, "data": data }))
}

pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/api/auth/health", get(auth::health))
        .route("/api/auth/config", get(auth::config))
        .route("/api/auth/email-code", post(auth::email_code))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/sessions", get(auth::sessions))
        .route("/api/auth/logout-all", post(auth::logout_all))
        // 邀请码：生成 / 列表 / 单张停用
        .route("/api/auth/admin/invites", get(admin::list_invites).post(admin::create_invites))
        .route("/api/auth/admin/invites/{id}/disable", post(admin::disable_invite))
        // 重新启用一张已用过的码（超管 + 面板上的风险确认）：只清零 used_count，
        // 兑换记录与审计都留着，见 service::reset_invite 的文档注释
        .route("/api/auth/admin/invites/{id}/reset", post(admin::reset_invite))
        // ⚠️ 批次与「整批发邮件」刻意各占一段独立路径（`invite-batches` / `invite-mail`），
        //    而不是写成 `invites/batch/...`、`invites/email`：那样会让静态段（batch/email）
        //    与上一条的 `{id}` 落在同一层，把「谁的优先级高」交给路由库的内部规则去决定。
        //    多打几个字符换掉一类解释不清的 404，值得。
        .route(
            "/api/auth/admin/invite-batches/{batch_id}/disable",
            post(admin::disable_invites_by_batch),
        )
        .route("/api/auth/admin/invite-mail", post(admin::send_invites_email))
        // 审计日志（超管）：谁在什么时候动了什么
        .route("/api/auth/admin/audit", get(admin::list_audit))
        // 用户治理：列表（管理员只读）/ 改角色 / 封禁解封 / 踢下线（后三个仅超管）
        .route("/api/auth/admin/users", get(admin::list_users))
        .route("/api/auth/admin/users/{id}/role", post(admin::change_user_role))
        .route("/api/auth/admin/users/{id}/status", post(admin::change_user_status))
        .route("/api/auth/admin/users/{id}/logout-all", post(admin::revoke_user_sessions));

    // 调试接口只在 development 注册——生产环境是「路由不存在」，而不是「存在但 403」
    if state.cfg.dev_endpoints {
        app = app.route("/api/auth/dev/codes", get(dev::codes));
    }

    app.layer(axum_mw::from_fn_with_state(state.clone(), middleware::csrf_guard))
        .with_state(state)
}
