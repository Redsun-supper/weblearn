// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 广学 · 账号系统（Rust 认证服务）
//!
//! 职责：注册（邮箱 + 管理员派发的邀请码 + 邮箱验证码）、多端同时登录、令牌轮换、
//! 会话管理、邀请码管理。由 Go 主后端之外的**独立服务**承载，线上经 Nginx 把
//! `/api/auth/` 分流过来（本地由 `dev-server.js` 做同样的事）。
//!
//! 分层（从内到外）：`core`（纯逻辑）→ `store`（SQLite）→ `service`（业务规则）→ `http`（axum 路由）。

pub mod cli;
pub mod clock;
pub mod config;
pub mod core;
pub mod db;
pub mod error;
pub mod http;
pub mod mail;
pub mod models;
pub mod rate_limit;
pub mod service;
pub mod store;

use std::sync::Arc;

use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::error::Result;
use crate::mail::Mailer;
use crate::rate_limit::RateLimiter;
use crate::service::AuthService;
use crate::store::SqliteStore;

/// 整个服务共享的状态（axum 的 `State`）
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub service: Arc<AuthService>,
    pub mailer: Arc<dyn Mailer>,
    pub limiter: Arc<RateLimiter>,
    pub clock: Arc<dyn Clock>,
}

impl AppState {
    /// 打开数据库、应用迁移、装配各层（tracing 等由调用方初始化）
    pub async fn init(cfg: Config) -> Result<Self> {
        let cfg = Arc::new(cfg);
        let store = Arc::new(SqliteStore::open(&cfg.db_path)?);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let mailer = mail::build(&cfg)?;
        let limiter = Arc::new(RateLimiter::new());
        let service = Arc::new(AuthService::new(
            cfg.clone(),
            store,
            clock.clone(),
            mailer.clone(),
            limiter.clone(),
        )?);
        Ok(Self { cfg, service, mailer, limiter, clock })
    }
}

/// 组装 HTTP 路由
pub fn build_app(state: AppState) -> axum::Router {
    http::router(state)
}

/// 初始化日志（`RUST_LOG` 优先，缺省给一个够用的级别）
pub fn init_tracing(default_level: &str) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level.to_string()));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(false).try_init();
}
