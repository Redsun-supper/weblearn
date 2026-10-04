// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 仅开发模式的调试接口
//!
//! 存在的意义：本期不接 SMTP，验证码只打到服务端日志里，本地端到端测试需要
//! 一个可编程的取码入口。安全上做了三重约束：
//!   1. **路由只在 `dev_endpoints` 为真时注册**（`APP_ENV=production` 下根本不存在）；
//!   2. 生产环境启动时如果 `AUTH_DEV_ENDPOINTS=true` 会直接报错退出（见 `config.rs`）；
//!   3. 验证码明文只存在于 `LogMailer` 的内存里，库中永远只有 HMAC 摘要。

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AuthError, Result};
use crate::http::ok_msg;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct DevQuery {
    pub email: String,
}

/// GET /api/auth/dev/codes?email=xxx
pub async fn codes(State(state): State<AppState>, Query(query): Query<DevQuery>) -> Result<Json<Value>> {
    match state.service.dev_code(&query.email) {
        Some(code) => Ok(ok_msg(
            "仅开发模式：这是该邮箱最近一次收到的验证码",
            json!({ "email": query.email, "code": code, "dev_only": true }),
        )),
        None => Err(AuthError::NotFound),
    }
}
