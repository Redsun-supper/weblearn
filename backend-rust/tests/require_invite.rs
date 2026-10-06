// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! P0-2：强制邀请码注册（`AUTH_REQUIRE_INVITE`）
//!
//! 这组用例只测**开关打开**时的行为（关掉时的开放注册路径在 `invite.rs` 里已有覆盖）。
//!
//! ⚠️ 最关键的一条不是「返回 400」，而是**数据库里一条验证码都没写**：
//! 邀请码校验必须排在写 `email_codes` 之前。「先发码后校验」等于没拦 ——
//! 码已经落库，按现有流程还能被用掉。`email_code_step_does_not_touch_db_*` 直接查库锁住它。

mod common;

use axum::http::StatusCode;
use common::*;
use guangxue_auth::store::sql::InviteFilter;
use serde_json::json;

/// 开关打开 + 常规测试配置（低 Argon2、关限流）
async fn spawn_strict() -> TestApp {
    spawn_with(|cfg| cfg.require_invite = true).await
}

#[tokio::test]
async fn email_code_without_invite_is_rejected_and_writes_nothing() {
    let app = spawn_strict().await;
    let mut client = Client::new(&app);

    let res = client
        .post("/api/auth/email-code", json!({ "email": "noinvite@example.com" }))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");

    assert_eq!(
        app.email_code_rows("noinvite@example.com").await,
        0,
        "没带邀请码时不能写 email_codes（先发码后校验等于没拦）"
    );
}

#[tokio::test]
async fn email_code_with_blank_invite_is_rejected_and_writes_nothing() {
    // 前端在「可选」模式下会把空串照样发上来，所以空串/纯空白必须与「没带」等价
    let app = spawn_strict().await;

    for raw in ["", "   ", "\t\n"] {
        let email = format!("blank{}@example.com", raw.len());
        let mut client = Client::new(&app);
        let res = client
            .post("/api/auth/email-code", json!({ "email": &email, "invite_code": raw }))
            .await;
        res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");
        assert_eq!(app.email_code_rows(&email).await, 0, "空白邀请码（{raw:?}）不能写库");
    }
}

#[tokio::test]
async fn email_code_with_unknown_invite_writes_nothing() {
    let app = spawn_strict().await;
    let mut client = Client::new(&app);
    let email = "ghost@example.com";

    let res = client
        .post(
            "/api/auth/email-code",
            json!({ "email": email, "invite_code": "ZZZZZZZZZZZZZZZZ" }),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");
    assert_eq!(app.email_code_rows(email).await, 0, "查不到的邀请码不能写库");
}

#[tokio::test]
async fn email_code_with_expired_invite_reports_expired_and_writes_nothing() {
    let app = spawn_strict().await;
    let invite = app.new_invite(5, 1).await;
    app.advance(48 * 3600); // 两天后

    let mut client = Client::new(&app);
    let email = "late@example.com";
    let res = client
        .post("/api/auth/email-code", json!({ "email": email, "invite_code": &invite }))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invite_expired");
    assert_eq!(app.email_code_rows(email).await, 0, "过期的码不能写库");
}

#[tokio::test]
async fn email_code_with_exhausted_invite_reports_exhausted_and_writes_nothing() {
    let app = spawn_strict().await;
    let invite = app.new_invite(1, 7).await;

    // 先用掉唯一一次额度
    let mut first = Client::new(&app);
    register(&app, &mut first, "used@example.com", "abc12345", &invite)
        .await
        .assert_status(StatusCode::OK);

    let mut second = Client::new(&app);
    let email = "second@example.com";
    let res = second
        .post("/api/auth/email-code", json!({ "email": email, "invite_code": &invite }))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invite_exhausted");
    assert_eq!(app.email_code_rows(email).await, 0, "用尽的码不能写库");
}

#[tokio::test]
async fn email_code_with_disabled_invite_is_rejected_and_writes_nothing() {
    let app = spawn_strict().await;
    let invite = app.new_invite(5, 7).await;
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    let id = items.iter().find(|i| i.code == invite).unwrap().id;
    app.state.service.disable_invite(id).await.unwrap();

    let mut client = Client::new(&app);
    let email = "disabled@example.com";
    let res = client
        .post("/api/auth/email-code", json!({ "email": email, "invite_code": &invite }))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");
    assert_eq!(app.email_code_rows(email).await, 0, "停用的码不能写库");
}

#[tokio::test]
async fn valid_invite_still_sends_the_code() {
    let app = spawn_strict().await;
    let invite = app.new_invite(1, 7).await;
    let mut client = Client::new(&app);

    register(&app, &mut client, "vip@example.com", "abc12345", &invite)
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(app.email_code_rows("vip@example.com").await, 1, "有效邀请码应当正常写一条验证码");

    // 注册成功后额度被核销
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    let row = items.iter().find(|i| i.code == invite).expect("邀请码在列表里");
    assert_eq!(row.used_count, 1);
}

#[tokio::test]
async fn register_without_invite_is_rejected_even_with_a_valid_email_code() {
    let app = spawn_strict().await;
    let invite = app.new_invite(5, 7).await;

    // 先用有效邀请码拿到一个真实验证码（绕过 email-code 的强制校验，
    // 专门测「注册这一步自己也必须拦」——两道门都要有）
    let mut client = Client::new(&app);
    client
        .post("/api/auth/email-code", json!({ "email": "sneaky@example.com", "invite_code": &invite }))
        .await
        .assert_status(StatusCode::OK);
    let code = fetch_dev_code(&mut client, "sneaky@example.com").await;

    let res = client
        .post(
            "/api/auth/register",
            json!({ "email": "sneaky@example.com", "email_code": code, "password": "abc12345" }),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");
}

#[tokio::test]
async fn open_registration_still_works_when_the_switch_is_off() {
    // 反向对照：开关关掉时，不填邀请码照样能注册（本地开发的行为不能被这次改动破坏）
    let app = spawn().await;
    let mut client = Client::new(&app);

    register_open(&app, &mut client, "open@example.com", "abc12345")
        .await
        .assert_status(StatusCode::OK);

    let me = client.me().await;
    me.assert_status(StatusCode::OK);
    assert_eq!(me.data("user")["role"], "user");
}
