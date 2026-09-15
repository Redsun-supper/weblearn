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
        .route("/api/auth/email-code", post(auth::email_code))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/sessions", get(auth::sessions))
        .route("/api/auth/logout-all", post(auth::logout_all))
        .route("/api/auth/admin/invites", get(admin::list_invites).post(admin::create_invites))
        .route("/api/auth/admin/invites/{id}/disable", post(admin::disable_invite));

    // 调试接口只在 development 注册——生产环境是「路由不存在」，而不是「存在但 403」
    if state.cfg.dev_endpoints {
        app = app.route("/api/auth/dev/codes", get(dev::codes));
    }

    app.layer(axum_mw::from_fn_with_state(state.clone(), middleware::csrf_guard))
        .with_state(state)
}
