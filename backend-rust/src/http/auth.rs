//! HTTP 处理器：注册 / 登录 / 刷新 / 登出 / 会话

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AuthError, Result};
use crate::http::middleware::{attach_auth_cookies, clear_auth_cookies, client_ip, cookie, user_agent};
use crate::http::{ok, ok_msg, ACCESS_COOKIE, REFRESH_COOKIE};
use crate::service::{AuthOutcome, AuthUser, LoginInput, RegisterInput};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct EmailCodeReq {
    pub email: String,
    /// 邀请码（**可选**，多个用空格分隔）。填了就先校验，用于「带码注册升级管理员」
    #[serde(default)]
    pub invite_code: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterReq {
    pub email: String,
    pub email_code: String,
    /// 邀请码（**可选**，多个用空格分隔）：不填=普通用户，填了就升级为管理员
    #[serde(default)]
    pub invite_code: String,
    pub password: String,
    #[serde(default)]
    pub username: Option<String>,
    /// 设备名（可空，空则按 User-Agent 推断）
    #[serde(default)]
    pub device_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub device_label: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LogoutAllReq {
    /// 只登出其他端，保留当前这个端
    #[serde(default)]
    pub keep_current: bool,
}

/// 服务健康检查（dev-server.js / 运维探活用）
pub async fn health(State(state): State<AppState>) -> Json<Value> {
    ok(json!({
        "service": "guangxue-auth",
        "version": env!("CARGO_PKG_VERSION"),
        "env": state.cfg.env,
        "mail_mode": state.mailer.mode(),
        "dev_endpoints": state.cfg.dev_endpoints,
    }))
}

/// POST /api/auth/email-code —— 发注册验证码（填了邀请码就先校验，再发信）
pub async fn email_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EmailCodeReq>,
) -> Result<Json<Value>> {
    let ip = client_ip(&headers);
    let sent = state.service.request_email_code(&req.email, &req.invite_code, &ip).await?;
    Ok(ok_msg(
        "验证码已发送",
        json!({ "email": sent.email, "expires_in": sent.expires_in }),
    ))
}

/// POST /api/auth/register —— 邮箱验证码注册，成功即登录；带邀请码则升级为管理员
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Result<Response> {
    let ip = client_ip(&headers);
    let ua = user_agent(&headers);
    let outcome = state
        .service
        .register(
            RegisterInput {
                email: req.email,
                email_code: req.email_code,
                invite_code: req.invite_code,
                password: req.password,
                username: req.username,
                device_label: req.device_label,
            },
            &ip,
            &ua,
        )
        .await?;
    Ok(auth_response(&state, outcome, "注册成功"))
}

/// POST /api/auth/login —— 每次登录都新建一个会话（多端并存）
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Result<Response> {
    let ip = client_ip(&headers);
    let ua = user_agent(&headers);
    let outcome = state
        .service
        .login(
            LoginInput { email: req.email, password: req.password, device_label: req.device_label },
            &ip,
            &ua,
        )
        .await?;
    Ok(auth_response(&state, outcome, "登录成功"))
}

/// POST /api/auth/refresh —— 轮换 refresh token 并换发 access token
pub async fn refresh(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let refresh_token = cookie(&headers, REFRESH_COOKIE);
    let Some(refresh_token) = refresh_token else {
        let mut res = AuthError::Unauthenticated.into_response();
        clear_auth_cookies(&mut res, &state);
        return Ok(res);
    };
    let ip = client_ip(&headers);
    let ua = user_agent(&headers);
    match state.service.refresh(&refresh_token, &ip, &ua).await {
        Ok(outcome) => Ok(auth_response(&state, outcome, "已刷新登录态")),
        Err(e) => {
            // 刷新失败一律把两个 Cookie 清掉，避免浏览器里留着一个没用的会话
            let mut res = e.into_response();
            clear_auth_cookies(&mut res, &state);
            Ok(res)
        }
    }
}

/// POST /api/auth/logout —— 只登出当前这个端（幂等）
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let access = cookie(&headers, ACCESS_COOKIE);
    let refresh = cookie(&headers, REFRESH_COOKIE);
    state.service.logout(access.as_deref(), refresh.as_deref()).await?;
    let mut res = ok_msg("已退出登录", json!({})).into_response();
    clear_auth_cookies(&mut res, &state);
    Ok(res)
}

/// GET /api/auth/me —— 当前登录用户
pub async fn me(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>> {
    let public = state.service.user_public(user.user_id).await?;
    Ok(ok(json!({ "user": public })))
}

/// GET /api/auth/sessions —— 我的全部活跃会话（多端管理的基础）
pub async fn sessions(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>> {
    let items = state.service.sessions(user.user_id, user.session_id).await?;
    Ok(ok(json!({ "items": items, "total": items.len() })))
}

/// POST /api/auth/logout-all —— 登出全部端（可保留当前端）
pub async fn logout_all(
    State(state): State<AppState>,
    user: AuthUser,
    body: Option<Json<LogoutAllReq>>,
) -> Result<Response> {
    let keep_current = body.map(|Json(b)| b.keep_current).unwrap_or(false);
    let revoked = state
        .service
        .logout_all(user.user_id, user.session_id, keep_current)
        .await?;
    let mut res = ok_msg("已登出", json!({ "revoked": revoked, "keep_current": keep_current })).into_response();
    if !keep_current {
        clear_auth_cookies(&mut res, &state);
    }
    Ok(res)
}

/// 把令牌写进 HttpOnly Cookie 并返回用户信息（令牌本身**不出现在响应体里**）
fn auth_response(state: &AppState, outcome: AuthOutcome, message: &str) -> Response {
    let mut res = ok_msg(
        message,
        json!({
            "user": outcome.user,
            "device_label": outcome.device_label,
            "access_expires_in": outcome.access_expires_in,
            "refresh_expires_in": outcome.refresh_expires_in,
        }),
    )
    .into_response();
    attach_auth_cookies(
        &mut res,
        state,
        &outcome.access_token,
        &outcome.refresh_token,
        outcome.access_expires_in,
        outcome.refresh_expires_in,
    );
    res
}
