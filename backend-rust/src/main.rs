// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 广学账号系统 · 认证服务入口
//!
//! 用法（在 `backend-rust/` 目录下执行）：
//!
//! ```text
//! cargo run --release              # 按环境变量启动，默认 127.0.0.1:8081
//! cargo run --release -- --help    # 查看可用环境变量
//! ```
//!
//! 启动顺序：读配置 → 打开数据库并迁移 → 建各层 → （可选）确保管理员存在 → 起 HTTP。

use std::process::ExitCode;

use guangxue_auth::config::{Config, MailMode};
use guangxue_auth::{build_app, init_tracing, AppState};

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing("info");
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("启动失败：{message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    let cfg = Config::from_env()?;
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let db_path = cfg.db_path.clone();
    let mail_mode = match cfg.mail.mode {
        MailMode::Log => "log（只打日志，不发信）",
        MailMode::Smtp => "smtp（真实发信）",
    };
    let cookie_secure = cfg.cookie_secure;
    let dev_endpoints = cfg.dev_endpoints;
    let seed_admin = cfg.seed_admin;
    let admin_email = cfg.admin_email.clone();
    let admin_password = cfg.admin_password.clone();
    let default_secret = cfg.jwt_secret_is_default;
    let default_admin_password = cfg.admin_password_is_default;
    let is_production = cfg.is_production();
    let access_ttl = cfg.access_ttl.as_secs();
    let refresh_days = cfg.refresh_ttl_days();

    let state = AppState::init(cfg).await.map_err(|e| e.to_string())?;

    // 发邮件的启动期自检：只连 TCP，不登录不发信。⚠️ 失败只告警 —— SMTP 挂了不该让
    // 已经登录的用户也用不了站点（他们只是注册收不到码而已）。
    let mail_check = match &state.cfg.mail.mode {
        MailMode::Smtp => guangxue_auth::mail::preflight(&state.cfg).await,
        MailMode::Log => None,
    };

    // 初始管理员：幂等。
    // ⚠️ 用的是 `ensure_super_admin`：把 `AUTH_ADMIN_EMAIL` 那个账号**确保为超级管理员**
    // （账号不存在就建、是 admin 就提权、已经是超管就什么都不做）。
    // 这是「超管怎么产生」那条决策（环境变量 + CLI 双保险）里环境变量那一半 —— 没有它，
    // 全新部署出来的库只有普通管理员，而发码 / 调权限都要求超管，等于谁也开不了张。
    if seed_admin {
        if let Some(password) = admin_password {
            match state.service.ensure_super_admin(&admin_email, &password).await {
                Ok((true, _)) => {
                    tracing::info!(email = %admin_email, "已创建初始超级管理员账号")
                }
                Ok((false, true)) => {
                    tracing::info!(email = %admin_email, "已有账号已提升为超级管理员")
                }
                Ok((false, false)) => {
                    tracing::info!(email = %admin_email, "超级管理员账号已存在，跳过")
                }
                Err(e) => tracing::error!(error = %e, "初始化超级管理员失败"),
            }
        }
    }

    // 审计日志保留期（P1）：启动时清一次，之后每 24 小时一次。
    // `interval` 的第一次 tick 立刻返回，所以「启动时清一次」不用另写一遍。
    // 清理失败只记日志、不影响服务（下一轮再试）——审计表只在写入时才被用到。
    if state.cfg.audit_retention_days > 0 {
        let sweep = state.clone();
        let days = state.cfg.audit_retention_days;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
            loop {
                ticker.tick().await;
                match sweep.service.prune_audit(days).await {
                    Ok(0) => {}
                    Ok(removed) => tracing::info!(removed, days, "已清理过期审计日志"),
                    Err(e) => tracing::warn!(error = %e, "清理审计日志失败（下一轮再试）"),
                }
            }
        });
    }

    println!("广学 · 账号系统（Rust 认证服务）已启动");    println!("  监听地址    : http://{addr}");
    println!("  数据库      : {db_path}");
    println!("  邮件模式    : {mail_mode}");
    println!(
        "  审计保留    : {}",
        if state.cfg.audit_retention_days > 0 {
            format!("{} 天（启动时与每 24 小时各清一次）", state.cfg.audit_retention_days)
        } else {
            "永不清理".to_string()
        }
    );
    match &mail_check {
        // 连不上只是告警：服务照常起，用户照样能登录（发不出码另说）
        Some(msg) if msg.starts_with("能连上") => println!("  邮件自检    : {msg}"),
        Some(msg) => {
            println!("  邮件自检    : ⚠️ {msg}");
            tracing::warn!(detail = %msg, "SMTP 启动自检未通过");
        }
        None => {}
    }
    println!("  会话有效期  : access {access_ttl} 秒 / refresh {refresh_days} 天");
    // P2：把**实际生效**的限流打出来。这几个数字现在能用 AUTH_RL_* 改，
    // 写错格式会在启动时报错退出，写对了就在这几行里能核对 —— 「我明明改了啊」
    // 这类问题不该靠再读一遍配置文件来查。
    let rate = state.cfg.rate;
    println!(
        "  限流（同 IP）: 登录 {}/{} 秒 · 发码 {}/{} 秒 · 注册 {}/{} 秒",
        rate.login_ip.limit,
        rate.login_ip.window.as_secs(),
        rate.code_ip.limit,
        rate.code_ip.window.as_secs(),
        rate.register_ip.limit,
        rate.register_ip.window.as_secs()
    );
    println!(
        "  限流（账号）: 验证码每邮箱 {}/{} 秒 与 {}/{} 秒 · 密码连错 {} 次锁 {} 分钟",
        rate.code_email_minute.limit,
        rate.code_email_minute.window.as_secs(),
        rate.code_email_hour.limit,
        rate.code_email_hour.window.as_secs(),
        state.cfg.lock_threshold,
        state.cfg.lock_minutes
    );
    println!("  Cookie      : HttpOnly + SameSite=Lax{}", if cookie_secure { " + Secure" } else { "" });
    println!("  允许来源    : {}", state.cfg.allowed_origins.join(", "));
    println!(
        "  调试接口    : {}",
        if dev_endpoints { "已开启 /api/auth/dev/codes（仅开发）" } else { "未开启" }
    );
    println!("  接口前缀    : /api/auth/*（线上由 Nginx 分流，本地由 dev-server.js 分流）");

    if default_secret && !is_production {
        println!("  ⚠️ 未设置 AUTH_JWT_SECRET，本次已随机生成：重启后旧令牌会失效（正式部署请固定它）");
    }
    if default_admin_password {
        println!("  ⚠️ 正在使用默认管理员密码，请首次登录后立即更换（生产环境会拒绝启动）");
    }
    if !cookie_secure && !is_production {
        println!("  ℹ️ 本地 http 调试：Cookie 未加 Secure（正是为了让浏览器收下它）");
    }

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("无法监听 {addr}：{e}"))?;
    axum::serve(listener, build_app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("HTTP 服务异常退出：{e}"))
}

/// Ctrl+C 优雅退出
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\n收到退出信号，正在关闭……");
}

fn print_help() {
    println!(
        r#"广学 · 账号系统（Rust 认证服务）

用法：
  cargo run --release            # 启动服务（默认 127.0.0.1:8081）
  cargo run --release -- --help  # 显示本帮助

环境变量（详见 .env.example）：
  APP_ENV                      development（默认）/ production
  AUTH_HOST / AUTH_PORT        监听地址与端口，默认 127.0.0.1:8081
  AUTH_DB_PATH                 SQLite 文件，默认 auth.db
  AUTH_JWT_SECRET              access token 签名密钥（production 必填）
  AUTH_ACCESS_TTL_SECONDS      默认 900
  AUTH_REFRESH_TTL_DAYS        默认 30
  AUTH_COOKIE_SECURE           默认随 APP_ENV（本地 http 必须是 false）
  AUTH_ALLOWED_ORIGINS         CSRF 白名单，默认 http://127.0.0.1:8899,http://localhost:8899
  AUTH_MAIL_MODE               log（默认）/ smtp
  AUTH_SMTP_HOST/PORT/USERNAME/PASSWORD/FROM/TLS
  AUTH_DEV_ENDPOINTS           默认随 APP_ENV（生产强制关闭）
  AUTH_REQUIRE_INVITE          是否强制邀请码注册，默认 false
  AUTH_AUDIT_RETENTION_DAYS    审计日志保留天数，默认 180（0 = 永不清理）
  AUTH_ADMIN_EMAIL             默认 2262997289@qq.com
  AUTH_ADMIN_PASSWORD          仅 development 允许用默认值
  AUTH_SEED_ADMIN              默认随 APP_ENV
  AUTH_ARGON2_M_COST/T_COST/P_COST
  AUTH_RL_LOGIN_IP             登录：同 IP，默认 200/900
  AUTH_RL_CODE_EMAIL_MINUTE    发码：同邮箱每分钟，默认 1/60
  AUTH_RL_CODE_EMAIL_HOUR      发码：同邮箱每小时，默认 5/3600
  AUTH_RL_CODE_IP              发码：同 IP，默认 200/3600
  AUTH_RL_REGISTER_IP          注册：同 IP，默认 100/3600
                               （格式「次数/窗口秒」；0/0 关闭这一条。口径：严在账号、宽在 IP）
  AUTH_LOCK_THRESHOLD / AUTH_LOCK_MINUTES
                               密码连错几次锁多久，默认 5 次 / 15 分钟

配套命令：
  cargo run --bin seed-admin -- --help          # 建/重置管理员
  cargo run --bin invite -- --help              # 邀请码 CLI
  cargo run --bin mail-test -- 邮箱@example.com # 用同一份配置真发一封测试邮件（P0-3）
  cargo run --release --bin guangxue-monitor    # 探活两个服务（P2；退出码 1 = 有不健康项）
"#
    );
}
