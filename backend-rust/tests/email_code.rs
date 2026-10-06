// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 邮箱验证码：错误次数、过期、一次性、发送限流、生产环境不暴露调试接口

mod common;

use axum::http::StatusCode;
use common::*;
use guangxue_auth::config::{RateConfig, RateRule};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn wrong_code_can_be_retried_until_limit() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;
    let mut client = Client::new(&app);
    let email = "retry@example.com";

    client
        .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);
    let correct = fetch_dev_code(&mut client, email).await;
    let wrong = if correct == "000000" { "111111" } else { "000000" };

    for _ in 0..3 {
        let res = client
            .post(
                "/api/auth/register",
                json!({"email": email, "email_code": wrong, "invite_code": &invite, "password": "abc12345"}),
            )
            .await;
        res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_code");
    }

    // 试错三次之后，正确的码依然可用（上限是 5 次）
    client
        .post(
            "/api/auth/register",
            json!({"email": email, "email_code": correct, "invite_code": &invite, "password": "abc12345"}),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn exceeding_attempts_blocks_even_the_correct_code() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;
    let mut client = Client::new(&app);
    let email = "brute@example.com";

    client
        .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);
    let correct = fetch_dev_code(&mut client, email).await;
    let wrong = if correct == "000000" { "111111" } else { "000000" };

    for _ in 0..5 {
        client
            .post(
                "/api/auth/register",
                json!({"email": email, "email_code": wrong, "invite_code": &invite, "password": "abc12345"}),
            )
            .await
            .assert_error("invalid_code");
    }

    let blocked = client
        .post(
            "/api/auth/register",
            json!({"email": email, "email_code": correct, "invite_code": &invite, "password": "abc12345"}),
        )
        .await;
    blocked.assert_status(StatusCode::BAD_REQUEST).assert_error("code_attempts_exceeded");
}

#[tokio::test]
async fn expired_code_is_rejected() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;
    let mut client = Client::new(&app);
    let email = "expire@example.com";

    client
        .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);
    let code = fetch_dev_code(&mut client, email).await;

    app.advance(601); // 10 分钟有效期之外

    let res = client
        .post(
            "/api/auth/register",
            json!({"email": email, "email_code": code, "invite_code": &invite, "password": "abc12345"}),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("code_expired");
}

#[tokio::test]
async fn code_cannot_be_reused_after_success() {
    let app = spawn().await;
    let invite_a = app.new_invite(1, 7).await;
    let invite_b = app.new_invite(1, 7).await;
    let mut client = Client::new(&app);
    let email = "once@example.com";

    client
        .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite_a}))
        .await
        .assert_status(StatusCode::OK);
    let code = fetch_dev_code(&mut client, email).await;

    client
        .post(
            "/api/auth/register",
            json!({"email": email, "email_code": &code, "invite_code": &invite_a, "password": "abc12345"}),
        )
        .await
        .assert_status(StatusCode::OK);

    // 换邮箱、换邀请码，但复用同一枚验证码 —— 它已经被消费掉了
    let mut other = Client::new(&app);
    let res = other
        .post(
            "/api/auth/register",
            json!({"email": "once2@example.com", "email_code": &code, "invite_code": &invite_b, "password": "abc12345"}),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_code");
}

#[tokio::test]
async fn resending_a_code_invalidates_the_previous_one() {
    let app = spawn().await;
    let invite = app.new_invite(5, 7).await;
    let mut client = Client::new(&app);
    let email = "resend@example.com";

    client
        .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);
    let old_code = fetch_dev_code(&mut client, email).await;

    client
        .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);
    let new_code = fetch_dev_code(&mut client, email).await;
    assert_ne!(old_code, new_code, "重发应当生成新码");

    let stale = client
        .post(
            "/api/auth/register",
            json!({"email": email, "email_code": old_code, "invite_code": &invite, "password": "abc12345"}),
        )
        .await;
    stale.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_code");

    client
        .post(
            "/api/auth/register",
            json!({"email": email, "email_code": new_code, "invite_code": &invite, "password": "abc12345"}),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn email_code_sending_is_rate_limited_per_mailbox() {
    let app = spawn_with(|cfg| {
        cfg.rate = RateConfig {
            login_ip: RateRule::off(),
            code_email_minute: RateRule::new(1, Duration::from_secs(60)),
            code_email_hour: RateRule::off(),
            code_ip: RateRule::off(),
            register_ip: RateRule::off(),
        };
    })
    .await;
    let invite = app.new_invite(5, 7).await;
    let mut client = Client::new(&app);

    client
        .post("/api/auth/email-code", json!({"email": "limit@example.com", "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);

    let second = client
        .post("/api/auth/email-code", json!({"email": "limit@example.com", "invite_code": &invite}))
        .await;
    second.assert_status(StatusCode::TOO_MANY_REQUESTS).assert_error("rate_limited");

    // 换个邮箱不受影响（限流是按邮箱维度的）
    client
        .post("/api/auth/email-code", json!({"email": "other@example.com", "invite_code": &invite}))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn login_is_rate_limited_per_ip() {
    let app = spawn_with(|cfg| {
        cfg.rate = RateConfig {
            login_ip: RateRule::new(2, Duration::from_secs(900)),
            code_email_minute: RateRule::off(),
            code_email_hour: RateRule::off(),
            code_ip: RateRule::off(),
            register_ip: RateRule::off(),
        };
    })
    .await;

    let mut client = Client::new(&app).with_ip("198.51.100.5");
    for _ in 0..2 {
        client
            .post("/api/auth/login", json!({"email": "nobody@example.com", "password": "abc12345"}))
            .await
            .assert_error("bad_credentials");
    }
    client
        .post("/api/auth/login", json!({"email": "nobody@example.com", "password": "abc12345"}))
        .await
        .assert_status(StatusCode::TOO_MANY_REQUESTS)
        .assert_error("rate_limited");

    let mut other = Client::new(&app).with_ip("198.51.100.6");
    other
        .post("/api/auth/login", json!({"email": "nobody@example.com", "password": "abc12345"}))
        .await
        .assert_error("bad_credentials");
}

#[tokio::test]
async fn dev_endpoint_is_not_registered_when_disabled() {
    let app = spawn_with(|cfg| cfg.dev_endpoints = false).await;
    let mut client = Client::new(&app);
    let res = client.get("/api/auth/dev/codes?email=someone@example.com").await;
    res.assert_status(StatusCode::NOT_FOUND);
}
