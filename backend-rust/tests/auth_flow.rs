// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 注册 / 登录 / 刷新 / 登出 的主流程

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

#[tokio::test]
async fn register_returns_user_and_session_cookies() {
    let app = spawn().await;
    let mut client = Client::new(&app);

    // 走开放注册（不带邀请码）→ 普通用户；带邀请码会升级成管理员，见 tests/invite.rs
    let res = register_open(&app, &mut client, "alice@example.com", "abc12345").await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.field("/data/user/email").unwrap(), "alice@example.com");
    assert_eq!(res.field("/data/user/role").unwrap(), "user");
    assert!(res.field("/data/user/password").is_none(), "响应里不能有密码字段");
    assert!(client.access_cookie().is_some(), "注册后应拿到 access cookie");
    assert!(client.refresh_cookie().is_some(), "注册后应拿到 refresh cookie");

    let me = client.get("/api/auth/me").await;
    me.assert_status(StatusCode::OK);
    assert_eq!(me.field("/data/user/email").unwrap(), "alice@example.com");
}

#[tokio::test]
async fn login_works_after_logout_and_me_requires_session() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "bob@example.com", "abc12345").await;

    client.get("/api/auth/me").await.assert_status(StatusCode::OK);

    let out = client.post("/api/auth/logout", json!({})).await;
    out.assert_status(StatusCode::OK);
    assert!(out.raw_cookie("gx_access").unwrap().contains("Max-Age=0"));
    assert!(out.raw_cookie("gx_refresh").unwrap().contains("Max-Age=0"));

    client.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);

    let again = client
        .post("/api/auth/login", json!({"email": "bob@example.com", "password": "abc12345"}))
        .await;
    again.assert_status(StatusCode::OK);
    client.get("/api/auth/me").await.assert_status(StatusCode::OK);
}

#[tokio::test]
async fn wrong_password_and_unknown_email_share_one_error() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "carol@example.com", "abc12345").await;

    let mut other = Client::new(&app);
    let wrong = other
        .post("/api/auth/login", json!({"email": "carol@example.com", "password": "wrong12345"}))
        .await;
    wrong.assert_status(StatusCode::UNAUTHORIZED).assert_error("bad_credentials");

    let unknown = other
        .post("/api/auth/login", json!({"email": "nobody@example.com", "password": "wrong12345"}))
        .await;
    unknown.assert_status(StatusCode::UNAUTHORIZED).assert_error("bad_credentials");
    assert_eq!(wrong.message(), unknown.message(), "两种失败必须文案一致，避免枚举邮箱");
}

#[tokio::test]
async fn register_validates_email_and_password() {
    let app = spawn().await;
    let invite = app.new_invite(5, 7).await;
    let mut client = Client::new(&app);

    let bad_email = client
        .post(
            "/api/auth/register",
            json!({"email": "not-an-email", "email_code": "123456", "invite_code": invite, "password": "abc12345"}),
        )
        .await;
    bad_email.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_params");

    let weak = client
        .post(
            "/api/auth/register",
            json!({"email": "dave@example.com", "email_code": "123456", "invite_code": invite, "password": "abcd"}),
        )
        .await;
    weak.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_params");

    let no_digit = client
        .post(
            "/api/auth/register",
            json!({"email": "dave@example.com", "email_code": "123456", "invite_code": invite, "password": "abcdefgh"}),
        )
        .await;
    no_digit.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_params");
}

#[tokio::test]
async fn register_requires_a_plausible_invite_code() {
    let app = spawn().await;
    let mut client = Client::new(&app);

    let short = client
        .post(
            "/api/auth/register",
            json!({"email": "eve@example.com", "email_code": "123456", "invite_code": "SHORT", "password": "abc12345"}),
        )
        .await;
    short.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");

    let unknown = client
        .post(
            "/api/auth/register",
            json!({"email": "eve@example.com", "email_code": "123456", "invite_code": "ZZZZZZZZZZZZZZZZ", "password": "abc12345"}),
        )
        .await;
    unknown.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");
}

#[tokio::test]
async fn duplicate_email_is_rejected() {
    let app = spawn().await;
    let mut first = Client::new(&app);
    register_new(&app, &mut first, "frank@example.com", "abc12345").await;

    // 第二个邀请码 + 新的验证码，但邮箱重复
    let invite = app.new_invite(1, 7).await;
    let mut second = Client::new(&app);
    let sent = second
        .post("/api/auth/email-code", json!({"email": "frank@example.com", "invite_code": invite}))
        .await;
    sent.assert_status(StatusCode::CONFLICT).assert_error("email_taken");
}

#[tokio::test]
async fn refresh_rotates_tokens_and_rejects_replay() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "gina@example.com", "abc12345").await;

    let first_refresh = client.refresh_cookie().cloned().expect("有 refresh cookie");
    let rotated = client.post("/api/auth/refresh", json!({})).await;
    rotated.assert_status(StatusCode::OK);
    let second_refresh = client.refresh_cookie().cloned().expect("刷新后有新的 refresh cookie");
    assert_ne!(first_refresh, second_refresh, "refresh token 必须轮换");

    // 用旧令牌再刷一次 = 重放：整条轮换链被吊销
    let mut attacker = Client::new(&app);
    attacker.set_cookie("gx_refresh", &first_refresh);
    let replay = attacker.post("/api/auth/refresh", json!({})).await;
    replay.assert_status(StatusCode::UNAUTHORIZED).assert_error("unauthenticated");

    // 当前这个端也一起失效（这正是重放检测的意义）
    client.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
    let after = client.post("/api/auth/refresh", json!({})).await;
    after.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_access_token_is_rejected() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "hank@example.com", "abc12345").await;
    client.get("/api/auth/me").await.assert_status(StatusCode::OK);

    // access 15 分钟 + 60 秒 leeway
    app.advance(900 + 61);
    client.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_refresh_token_cannot_refresh() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "ivy@example.com", "abc12345").await;

    app.advance(30 * 24 * 3600 + 10);
    let res = client.post("/api/auth/refresh", json!({})).await;
    res.assert_status(StatusCode::UNAUTHORIZED);
}
