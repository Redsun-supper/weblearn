//! 邀请码：状态机（有效/用尽/过期/停用）、单次使用、并发占用、管理接口

mod common;

use axum::http::StatusCode;
use common::*;
use guangxue_auth::store::sql::InviteFilter;
use serde_json::json;

#[tokio::test]
async fn single_use_invite_is_consumed_after_one_registration() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;

    let mut first = Client::new(&app);
    register(&app, &mut first, "one@example.com", "abc12345", &invite)
        .await
        .assert_status(StatusCode::OK);

    let mut second = Client::new(&app);
    let res = second
        .post(
            "/api/auth/register",
            json!({"email": "two@example.com", "email_code": "123456", "invite_code": &invite, "password": "abc12345"}),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invite_exhausted");
}

#[tokio::test]
async fn multi_use_invite_counts_each_registration() {
    let app = spawn().await;
    let invite = app.new_invite(2, 7).await;

    for email in ["m1@example.com", "m2@example.com"] {
        let mut client = Client::new(&app);
        register(&app, &mut client, email, "abc12345", &invite)
            .await
            .assert_status(StatusCode::OK);
    }

    let mut third = Client::new(&app);
    let res = third
        .post(
            "/api/auth/register",
            json!({"email": "m3@example.com", "email_code": "123456", "invite_code": &invite, "password": "abc12345"}),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invite_exhausted");

    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    let row = items.iter().find(|i| i.code == invite).expect("邀请码在列表里");
    assert_eq!(row.used_count, 2);
    assert_eq!(row.status, "used");
}

#[tokio::test]
async fn expired_invite_is_rejected_at_email_code_step_too() {
    let app = spawn().await;
    let invite = app.new_invite(5, 1).await;

    app.advance(48 * 3600); // 两天后

    let mut client = Client::new(&app);
    let res = client
        .post("/api/auth/email-code", json!({"email": "late@example.com", "invite_code": &invite}))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invite_expired");

    let reg = client
        .post(
            "/api/auth/register",
            json!({"email": "late@example.com", "email_code": "123456", "invite_code": &invite, "password": "abc12345"}),
        )
        .await;
    reg.assert_status(StatusCode::BAD_REQUEST).assert_error("invite_expired");
}

#[tokio::test]
async fn disabled_invite_is_rejected() {
    let app = spawn().await;
    let invite = app.new_invite(5, 7).await;
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    let id = items.iter().find(|i| i.code == invite).unwrap().id;

    app.state.service.disable_invite(id).await.unwrap();

    let mut client = Client::new(&app);
    let res = client
        .post("/api/auth/email-code", json!({"email": "disabled@example.com", "invite_code": &invite}))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");

    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    assert_eq!(items.iter().find(|i| i.code == invite).unwrap().status, "disabled");
}

#[tokio::test]
async fn invite_code_is_normalized_before_lookup() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;
    // 管理员手抄成小写 + 分隔符的写法也要能用
    let handwritten = invite
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();

    let mut client = Client::new(&app);
    register(&app, &mut client, "hand@example.com", "abc12345", &handwritten)
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn concurrent_registrations_with_one_invite_only_one_wins() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;

    // 两个邮箱各取码（获取验证码本身允许），然后用同一个邀请码同时注册
    let mut a = Client::new(&app);
    let mut b = Client::new(&app);
    for (client, email) in [(&mut a, "race1@example.com"), (&mut b, "race2@example.com")] {
        client
            .post("/api/auth/email-code", json!({"email": email, "invite_code": &invite}))
            .await
            .assert_status(StatusCode::OK);
    }
    let code_a = fetch_dev_code(&mut a, "race1@example.com").await;
    let code_b = fetch_dev_code(&mut b, "race2@example.com").await;

    let body = |email: &str, code: &str| {
        json!({"email": email, "email_code": code, "invite_code": &invite, "password": "abc12345"})
    };
    let (ra, rb) = tokio::join!(
        a.post("/api/auth/register", body("race1@example.com", &code_a)),
        b.post("/api/auth/register", body("race2@example.com", &code_b)),
    );

    let ok_count = [&ra, &rb].iter().filter(|r| r.status == StatusCode::OK).count();
    let fail_count = [&ra, &rb]
        .iter()
        .filter(|r| r.error_code() == Some("invite_exhausted"))
        .count();
    assert_eq!(ok_count, 1, "只有一个注册能成功：{ra:?} / {rb:?}");
    assert_eq!(fail_count, 1, "另一个必须明确报 invite_exhausted");

    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    assert_eq!(items.iter().find(|i| i.code == invite).unwrap().used_count, 1, "用量只能加一次");
}

#[tokio::test]
async fn unused_filter_excludes_used_invites() {
    let app = spawn().await;
    let used = app.new_invite(1, 7).await;
    let spare = app.new_invite(1, 7).await;

    let mut client = Client::new(&app);
    register(&app, &mut client, "filter@example.com", "abc12345", &used)
        .await
        .assert_status(StatusCode::OK);

    let (unused, total) = app.state.service.list_invites(InviteFilter::Unused, 1, 50).await.unwrap();
    let codes: Vec<&str> = unused.iter().map(|i| i.code.as_str()).collect();
    assert!(codes.contains(&spare.as_str()));
    assert!(!codes.contains(&used.as_str()));
    assert!(total >= 1);
}

// ---------- 邀请码的新定位：不再是注册门槛，而是「升级/兑换券」 ----------

#[tokio::test]
async fn registration_without_invite_creates_a_normal_user() {
    let app = spawn().await;
    let mut client = Client::new(&app);

    register_open(&app, &mut client, "open@example.com", "abc12345")
        .await
        .assert_status(StatusCode::OK);

    let me = client.me().await;
    me.assert_status(StatusCode::OK);
    assert_eq!(me.data("user")["role"], "user", "不填邀请码应是普通用户：{}", me.body);
}

#[tokio::test]
async fn registration_with_invite_grants_the_role_from_the_code() {
    let app = spawn().await;
    // ⚠️ P0-5 之前这里是「带码注册即管理员」——那是当时最大的权限漏洞：
    // 任何一张码（哪怕备注写着「给某同学」）都能造出管理员。
    // 现在角色**只由码上的 `grant_role` 决定**：
    //   · 普通码（默认）→ user，带着码也只是普通用户（下面这段）
    //   · 管理员码 → admin（用 new_invite_with_role 显式要）
    let user_invite = app.new_invite(1, 7).await;
    let mut client = Client::new(&app);

    register(&app, &mut client, "vip@example.com", "abc12345", &user_invite)
        .await
        .assert_status(StatusCode::OK);

    let me = client.me().await;
    me.assert_status(StatusCode::OK);
    assert_eq!(
        me.data("user")["role"], "user",
        "普通邀请码只能给普通用户（不再自动升管理员）：{}",
        me.body
    );

    // 而且邀请码的额度被正常记账
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    let row = items.iter().find(|i| i.code == user_invite).expect("邀请码在列表里");
    assert_eq!(row.used_count, 1);
    assert_eq!(row.status, "used");
    assert_eq!(row.grant_role, "user");
}

#[tokio::test]
async fn admin_invite_grants_admin_role_and_carries_the_prefix() {
    let app = spawn().await;
    let admin_invite = app.new_invite_with_role(1, 7, "admin").await;
    assert!(
        admin_invite.starts_with("ADMIN-"),
        "管理员码应当带给人看的前缀：{admin_invite}"
    );

    let mut client = Client::new(&app);
    register(&app, &mut client, "boss@example.com", "abc12345", &admin_invite)
        .await
        .assert_status(StatusCode::OK);

    let me = client.me().await;
    assert_eq!(me.data("user")["role"], "admin", "管理员码应当给管理员：{}", me.body);
}

#[tokio::test]
async fn prefix_alone_does_not_grant_admin() {
    // 前缀只是给人看的：判定永远查库读 grant_role。
    // 手工拼一个带 ADMIN- 前缀、但库里根本没这一行的码 —— 连验证码都发不出来。
    //
    // ⚠️ 走的是 `email-code` 这一步而不是注册：邀请码在**发验证码时就校验**，
    // 所以这种请求连一条 `email_codes` 都不该留下（P0-2 的验收口径）。
    let app = spawn().await;
    let plain = app.new_invite(1, 7).await; // 库里 grant_role = user
    let faked = format!("ADMIN-{plain}");

    let mut client = Client::new(&app);
    let res = client
        .post(
            "/api/auth/email-code",
            json!({ "email": "faker@example.com", "invite_code": &faked }),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");

    assert_eq!(app.email_code_rows("faker@example.com").await, 0, "伪造前缀不该发出验证码");
    assert_eq!(app.role_of("faker@example.com").await, "<不存在>");
    // 而且库里那张真码一次都没被用掉
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    let row = items.iter().find(|i| i.code == plain).expect("真码还在");
    assert_eq!(row.used_count, 0);
}

#[tokio::test]
async fn multiple_codes_are_rejected_with_a_clear_message() {
    let app = spawn().await;
    let first = app.new_invite(1, 7).await;
    let second = app.new_invite(1, 7).await;

    let mut client = Client::new(&app);
    // 一次填两张券：现在**明确拒绝** —— 两张码的等级可能不同（一张 user、一张 admin），
    // 「取最高的」这种规则在出问题时很难向用户解释。要两个身份就注册两个号。
    let codes = format!("{first}  {second}");
    let res = register_with_codes(&app, &mut client, "multi@example.com", "abc12345", &codes).await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_params");

    // 没有任何一张券被核销（拒绝发生在事务里，什么都没留下）
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    for code in [&first, &second] {
        let row = items.iter().find(|i| &i.code == code).expect("邀请码在列表里");
        assert_eq!(row.used_count, 0, "多张码被拒时不该核销任何一张：{}", row.code);
    }
}

#[tokio::test]
async fn the_same_code_written_twice_is_only_consumed_once() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;

    let mut client = Client::new(&app);
    // 手抖写两遍同一个码：去重之后仍然是一张券，注册成功且只扣一次
    let codes = format!("{invite} {}", invite.to_lowercase());
    register_with_codes(&app, &mut client, "dup@example.com", "abc12345", &codes)
        .await
        .assert_status(StatusCode::OK);

    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    assert_eq!(items.iter().find(|i| i.code == invite).unwrap().used_count, 1);
}

#[tokio::test]
async fn unknown_code_still_rejects_registration() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    // 填了但查不到的码：明确报错，不要静默降级成普通用户（否则用户以为升级成功了）
    let code = fetch_dev_code_after_send(&mut client, "ghost@example.com").await;
    let res = client
        .post(
            "/api/auth/register",
            json!({"email": "ghost@example.com", "email_code": code, "invite_code": "ZZZZZZZZZZZZZZZZ", "password": "abc12345"}),
        )
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_invite");
}

/// 先正常发码（不带邀请码），拿到验证码后再插一个无效邀请码去注册
async fn fetch_dev_code_after_send(client: &mut Client, email: &str) -> String {
    client
        .post("/api/auth/email-code", json!({ "email": email }))
        .await
        .assert_status(StatusCode::OK);
    fetch_dev_code(client, email).await
}
