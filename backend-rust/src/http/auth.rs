// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! HTTP 处理器：注册 / 登录 / 刷新 / 登出 / 会话

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
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

/// 健康检查的查询参数（P2）
#[derive(Debug, Default, Deserialize)]
pub struct HealthQuery {
    /// `?deep=1`：**真的查一次库**。监控命令（`guangxue-monitor`）走深检，
    /// Nginx / dev-server 的探活走浅检。写成 `Option<String>` 而不是 bool：
    /// 监控是用手写请求发的（`?deep=1`），`?deep=true` 也一并认。
    /// ⚠️ 只认 `1` 与 `true`（大小写不敏感）—— 与 Go 侧 `?deep=1` 的口径**逐字一致**，
    /// 两边多认一个写法就会多一类「明明传了却没生效」的排查题。
    #[serde(default)]
    pub deep: Option<String>,
}

impl HealthQuery {
    fn wants_deep(&self) -> bool {
        // 大小写不敏感地认 `1` / `true`（与 Go 侧同一口径）；其余一律浅检
        match self.deep.as_deref().map(str::trim) {
            Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
            None => false,
        }
    }
}

/// 服务健康检查（dev-server.js / 运维探活用）
///
/// 浅检只证明「进程活着、路由通」；`?deep=1` 多查一次数据库 ——
/// **进程活着但库坏了**（迁移没跑、文件被换、盘满了写不进去）是最常见的「悄悄坏掉」，
/// 浅检永远发现不了。深检不健康时回 **503**：这样监控只需要看状态码。
pub async fn health(State(state): State<AppState>, Query(q): Query<HealthQuery>) -> Response {
    let mut data = json!({
        "service": "guangxue-auth",
        "version": env!("CARGO_PKG_VERSION"),
        "env": state.cfg.env,
        "mail_mode": state.mailer.mode(),
        "dev_endpoints": state.cfg.dev_endpoints,
        "uptime_seconds": state.uptime_seconds(),
    });

    if !q.wants_deep() {
        return ok(data).into_response();
    }

    match state.service.health_details().await {
        Ok(details) => {
            data["deep"] = details;
            ok(data).into_response()
        }
        Err(e) => {
            // 具体错误（SQLite 的原文）留给日志与响应体，别只回一句「503」——
            // 半夜收到告警的人需要一眼看出是「表不见了」还是「盘满了」
            tracing::error!(error = %e, "深度健康检查失败");
            data["deep"] = json!({ "database": format!("error: {e}") });
            data["status"] = json!("degraded");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "code": 503,
                    "message": format!("数据库不可用：{e}"),
                    "error": "unavailable",
                    "data": data,
                })),
            )
                .into_response()
        }
    }
}

/// GET /api/auth/config —— **公开**的「前端需要知道的服务端开关」（不需要登录）
///
/// 目前只有一项：注册是否强制邀请码。注册表单据此把邀请码标成必填并写清提示，
/// 用户就不会填完邮箱、点了「发验证码」才发现自己手里根本没有码。
///
/// ⚠️ 这是匿名可调的接口：**只放布尔开关**，不要往里加任何带隐私或安全含义的字段
/// （账号数、管理员邮箱、SMTP 主机名之类一律不要）。
pub async fn config(State(state): State<AppState>) -> Json<Value> {
    ok(json!({ "require_invite": state.cfg.require_invite }))
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
