// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
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

// ---------------------------------------------------------------- 重新启用已用过的码
//
// 背景（2026-10-05）：管理面板给「已用完」的码加了一个「重新启用」——把 `used_count` 清零，
// 让这张已经流出去的码还能被兑换。这是一个**主动降安全**的动作，所以这一组测试盯三件事：
// ① 清零之后真的能再用；② 旧兑换记录**必须留着**（留痕，能看出「这张码被回收过」）；
// ③ 准入与其它管理接口一致（只有超管），并且每次都要写审计。

const ADMIN_EMAIL: &str = "2262997289@qq.com";
const ADMIN_PASSWORD: &str = "7289HR_RedSun";

/// 登录成超管（种子账号在 P0-5 起就是超管）
async fn super_admin(app: &TestApp) -> Client {
    app.seed_admin(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let mut client = Client::new(app);
    client
        .post("/api/auth/login", json!({"email": ADMIN_EMAIL, "password": ADMIN_PASSWORD}))
        .await
        .assert_status(StatusCode::OK);
    client
}

/// 管理接口按 id 操作，而测试手上只有明文码 —— 按码反查 id
async fn invite_id(app: &TestApp, code: &str) -> i64 {
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 100).await.unwrap();
    items.iter().find(|i| i.code == code).expect("邀请码在列表里").id
}

#[tokio::test]
async fn reset_clears_usage_and_keeps_the_redemption_record() {
    let app = spawn().await;
    let mut admin = super_admin(&app).await;
    let invite = app.new_invite(1, 7).await;
    let id = invite_id(&app, &invite).await;

    // 先把这张单人码用掉
    let mut first = Client::new(&app);
    register(&app, &mut first, "reset-1@example.com", "abc12345", &invite)
        .await
        .assert_status(StatusCode::OK);

    // 用尽之后第二个人进不来（这就是「已用过」的风险本身）
    let mut second = Client::new(&app);
    second
        .post(
            "/api/auth/register",
            json!({"email": "reset-2@example.com", "email_code": "123456", "invite_code": &invite, "password": "abc12345"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .assert_error("invite_exhausted");

    // 重新启用：清零 + 写审计
    let res = admin.post(&format!("/api/auth/admin/invites/{id}/reset"), json!({})).await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("cleared").as_i64(), Some(1), "应该清掉 1 次：{}", res.body);
    assert_eq!(app.audit_count("invite_reset").await, 1, "重置必须留一条审计");

    // 兑换记录**不删**：这是「这张码被回收过」的唯一证据
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 100).await.unwrap();
    let row = items.iter().find(|i| i.code == invite).expect("码还在列表里");
    assert_eq!(row.used_count, 0);
    assert_eq!(row.status, "unused");
    assert_eq!(row.uses.len(), 1, "旧的兑换记录必须留着");
    assert_eq!(row.uses[0].email, "reset-1@example.com");

    // 清零之后它又能被兑换了（第二个人这次注册得进去）
    let mut third = Client::new(&app);
    register(&app, &mut third, "reset-2@example.com", "abc12345", &invite)
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn reset_needs_super_admin_and_unknown_id_is_404() {
    let app = spawn().await;
    let invite = app.new_invite(1, 7).await;
    let id = invite_id(&app, &invite).await;

    // 普通管理员：与发码 / 停用同一条准入线（碰不到权限相关的东西）
    app.seed_plain_admin("plain-reset@example.com", "PlainAdmin123").await;
    let mut plain = Client::new(&app);
    plain
        .post("/api/auth/login", json!({"email": "plain-reset@example.com", "password": "PlainAdmin123"}))
        .await
        .assert_status(StatusCode::OK);
    let res = plain.post(&format!("/api/auth/admin/invites/{id}/reset"), json!({})).await;
    res.assert_status(StatusCode::FORBIDDEN).assert_error("forbidden");
    assert_eq!(app.audit_count("invite_reset").await, 0, "被拒的请求不该留下审计");

    // 超管：不存在的 id → 404（而不是静默成功）
    let mut admin = super_admin(&app).await;
    let res = admin.post("/api/auth/admin/invites/999999/reset", json!({})).await;
    res.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reset_on_an_unused_invite_clears_nothing() {
    let app = spawn().await;
    let mut admin = super_admin(&app).await;
    let invite = app.new_invite(3, 7).await;
    let id = invite_id(&app, &invite).await;

    // 本来就没被用过：幂等返回 0，不是错误
    let res = admin.post(&format!("/api/auth/admin/invites/{id}/reset"), json!({})).await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("cleared").as_i64(), Some(0), "{}", res.body);

    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 100).await.unwrap();
    let row = items.iter().find(|i| i.code == invite).expect("码还在");
    assert_eq!(row.status, "unused");
    assert!(!row.disabled, "重置不该顺手改停用状态（两件事分开做）");
}

// ---------------------------------------------------------------- 自定义邀请码
//
// 背景（2026-10-05）：超管要能**自己指定**一串码（16 位、A-Z 与 0-9），并按需要定次数与有效期，
// 典型用法就是「永不过期但只能用一次」。这一组测试盯四件事：
// ① 人定的码与系统码**同长同形**（16 位、统一大写、手写的 `-` 抹掉）；
// ② 不合法的一律 400 且**一张都不许落库**（半成功最难查）；
// ③ 撞到已有码时必须**报冲突并带上那张码的现状**，而不是默默新建/覆盖；
// ④ 管理员确认「无视风险继续」之后照用，且**不动**旧的兑换记录与已注册用户。

/// 用自定义码建一张（服务层直调；HTTP 那条路由另有用例覆盖）
async fn create_custom(
    app: &TestApp,
    raw: &str,
    max_uses: i64,
    days: i64,
    allow_existing: bool,
) -> guangxue_auth::error::Result<guangxue_auth::service::InviteCreated> {
    app.state
        .service
        .create_invites(
            None,
            guangxue_auth::service::InviteSpec {
                count: 1,
                max_uses,
                expires_in_days: days,
                note: "自定义码测试".to_string(),
                grant_role: "user".to_string(),
                custom_code: Some(raw.to_string()),
                allow_existing,
            },
        )
        .await
}

#[tokio::test]
async fn custom_code_is_used_verbatim_and_can_never_expire() {
    let app = spawn().await;
    let created = create_custom(&app, "MYCODE1234567890", 1, 0, false).await.expect("自定义码应当建成");

    assert_eq!(created.codes.len(), 1, "自定义码一次只出一张");
    let row = &created.codes[0];
    assert_eq!(row.code, "MYCODE1234567890");
    assert_eq!(row.max_uses, 1);
    assert_eq!(row.used_count, 0);
    assert_eq!(row.status, "unused");
    // 天数传 0 = **永不过期**（用户要的「不过期但只能使用一次」就是这一档）
    assert_eq!(row.expires_at, None, "0 天必须是「永不过期」，不是「立刻过期」");
    assert!(row.batch_id.len() > 10, "自定义码也要有批次号（按批回收靠它）");

    // 前端会写小写、也可能按 4 位一组手抄，两种都要能用
    let created = create_custom(&app, "abcd-efgh-jklm-npqt", 1, 0, false).await.expect("小写 + 分组");
    assert_eq!(created.codes[0].code, "ABCDEFGHJKLMNPQT", "统一转大写并抹掉手写的 -");

    // 兑换这条路也要真的通：永不过期的码注册成功后直接变成「已用完」
    let mut client = Client::new(&app);
    register(&app, &mut client, "custom-1@example.com", "abc12345", "ABCDEFGHJKLMNPQT")
        .await
        .assert_status(StatusCode::OK);

    let mut second = Client::new(&app);
    second
        .post(
            "/api/auth/register",
            json!({"email": "custom-2@example.com", "email_code": "123456", "invite_code": "ABCDEFGHJKLMNPQT", "password": "abc12345"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .assert_error("invite_exhausted");

    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 100).await.unwrap();
    let row = items.iter().find(|i| i.code == "ABCDEFGHJKLMNPQT").expect("码在列表里");
    assert_eq!(row.status, "used");
    assert_eq!(row.max_uses, 1, "只能用一次的码，用完就该是 used");
}

#[tokio::test]
async fn custom_code_shape_is_rejected_before_anything_is_written() {
    let app = spawn().await;
    for bad in [
        "SHORT",              // 太短
        "TOOLONG1234567890",  // 17 位：太长
        "MYCODE_12345678",    // 下划线不是允许的字符
        "我的码1234567890",     // 非 ASCII
    ] {
        let err = create_custom(&app, bad, 1, 7, false).await.expect_err("本应被拒");
        assert!(
            matches!(err, guangxue_auth::error::AuthError::InvalidParams(_)),
            "{bad} 应当是 invalid_params，实际：{err:?}"
        );
    }
    assert_eq!(app.invite_rows().await, 0, "被拒的请求一张码都不许落库");
    assert_eq!(app.audit_count("invite_create").await, 0, "没建成就不该有 invite_create 审计");
}

#[tokio::test]
async fn http_level_custom_code_rejects_shape_errors_too() {
    let app = spawn().await;
    let mut admin = super_admin(&app).await;

    // 服务层有校验不够：HTTP 这一层也得把话原样传出去（前端靠它把错误显示给人看）
    let res = admin
        .post("/api/auth/admin/invites", json!({"custom_code": "SHORT", "max_uses": 1, "expires_in_days": 0}))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_params");
    assert!(res.message().contains("16"), "文案要说清需要几位：{}", res.message());

    let res = admin
        .post("/api/auth/admin/invites", json!({"custom_code": "MYCODE_12345678"}))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST).assert_error("invalid_params");

    // 正常的一路：小写进去、大写回来，并且 items/codes 两个字段都要有（面板两个都用）
    let res = admin
        .post(
            "/api/auth/admin/invites",
            json!({"custom_code": "mycode0987654321", "max_uses": 1, "expires_in_days": 0, "note": "HTTP 自定义"}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("custom").as_bool(), Some(true), "要能分辨这是自定义码：{}", res.body);
    assert_eq!(res.data("codes")[0].as_str(), Some("MYCODE0987654321"));
    assert_eq!(res.data("items")[0]["expires_at"], serde_json::Value::Null);
    assert_eq!(res.data("items")[0]["max_uses"].as_i64(), Some(1));
    assert_eq!(app.audit_count("invite_create").await, 1);
}

#[tokio::test]
async fn existing_custom_code_conflicts_then_can_be_reused_with_confirmation() {
    let app = spawn().await;
    let first = create_custom(&app, "REUSE12345678901", 1, 0, false).await.expect("先建一张");
    let id = first.codes[0].id;

    // 被人用掉一次：这时候它不再是「可用的新码」
    let mut client = Client::new(&app);
    register(&app, &mut client, "reuse-1@example.com", "abc12345", "REUSE12345678901")
        .await
        .assert_status(StatusCode::OK);

    // 再想用同一个串建：必须报冲突，**并且把那张码的现状带回来**（面板要靠它弹确认框）
    let err = create_custom(&app, "REUSE12345678901", 1, 0, false).await.expect_err("撞码要报冲突");
    match &err {
        guangxue_auth::error::AuthError::InviteCodeTaken { code, existing } => {
            assert_eq!(code, "REUSE12345678901");
            let existing = existing.as_ref().expect("必须带上那张码的现状");
            assert_eq!(existing.id, id);
            assert_eq!(existing.status, "used", "面板要能说清「这张码已经被用过了」");
            assert_eq!(existing.used_count, 1);
            assert_eq!(existing.uses.len(), 1, "还要能说清是谁用的");
            assert_eq!(existing.uses[0].email, "reuse-1@example.com");
        }
        other => panic!("应当是 InviteCodeTaken，实际：{other:?}"),
    }
    assert_eq!(err.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(err.code(), "invite_code_taken");
    assert_eq!(app.invite_rows().await, 1, "冲突时不许再建出第二张");

    // 超管确认「无视风险继续」：照用不误 —— 但沿用**不是「原样不动」**：
    // 次数上限与有效期会按这次填的改（这就是管理员重填一遍的意义），
    // 而 used_count / disabled / 等级 / 兑换记录都是这张码的身份，一个字都不许动。
    let reused = create_custom(&app, "REUSE12345678901", 3, 0, true).await.expect("确认后应当放行");
    assert_eq!(reused.codes[0].id, id, "沿用的还是同一张码，不是新建");
    assert_eq!(reused.codes[0].used_count, 1, "旧的兑换记录/已用次数都还在");
    assert_eq!(reused.codes[0].max_uses, 3, "沿用时改的是这张码自己的额度");
    assert_eq!(reused.codes[0].expires_at, None, "有效期也按这次填的（0 = 永不过期）");
    assert_eq!(reused.codes[0].uses.len(), 1);
    assert!(reused.reused_existing, "回执要能分辨「新建」还是「沿用」");
    assert_eq!(app.invite_rows().await, 1);
    assert_eq!(app.audit_count("invite_create").await, 2, "沿用也要留一条审计（写明是沿用）");

    // 额度调到 3 之后，它能再被兑换两次
    let mut second = Client::new(&app);
    register(&app, &mut second, "reuse-2@example.com", "abc12345", "REUSE12345678901")
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn http_existing_code_reports_409_with_the_existing_row() {
    let app = spawn().await;
    let mut admin = super_admin(&app).await;

    let res = admin
        .post("/api/auth/admin/invites", json!({"custom_code": "HTTPREUSE1234567"}))
        .await;
    res.assert_status(StatusCode::OK);

    let res = admin
        .post("/api/auth/admin/invites", json!({"custom_code": "httpreuse1234567"}))
        .await;
    res.assert_status(StatusCode::CONFLICT).assert_error("invite_code_taken");
    assert_eq!(res.data("code").as_str(), Some("HTTPREUSE1234567"), "回执要带上码本身");
    assert_eq!(res.data("existing")["id"].as_i64(), Some(1));
    assert_eq!(res.data("existing")["status"].as_str(), Some("unused"));

    // 带上确认标记：这次放行
    let res = admin
        .post(
            "/api/auth/admin/invites",
            json!({"custom_code": "HTTPREUSE1234567", "allow_existing": true, "max_uses": 2, "expires_in_days": 0}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("custom").as_bool(), Some(true));
    assert_eq!(res.data("reused").as_bool(), Some(true), "要能分辨这是沿用而不是新建：{}", res.body);
    assert_eq!(res.data("items")[0]["id"].as_i64(), Some(1), "还是原来那张");
    assert_eq!(res.data("items")[0]["max_uses"].as_i64(), Some(2), "额度按这次填的改");
    assert_eq!(app.invite_rows().await, 1);
}

#[tokio::test]
async fn custom_code_needs_super_admin() {
    let app = spawn().await;
    app.seed_plain_admin("plain-custom@example.com", "PlainAdmin123").await;
    let mut plain = Client::new(&app);
    plain
        .post("/api/auth/login", json!({"email": "plain-custom@example.com", "password": "PlainAdmin123"}))
        .await
        .assert_status(StatusCode::OK);

    // 普通管理员连随机码都发不了（既有的准入线），自定义码自然也一样
    let res = plain
        .post("/api/auth/admin/invites", json!({"custom_code": "PLAINCODE1234567"}))
        .await;
    res.assert_status(StatusCode::FORBIDDEN).assert_error("forbidden");
    assert_eq!(app.invite_rows().await, 0);
}
