// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! P1 管理面板的服务端侧：审计查询 / 按批停用 / 踢下线 / 邮箱脱敏 / 整批发码 / 公开配置
//!
//! 这一批的接口都是「治理动作」，所以每个用例都同时验证两件事：
//! ① 动作本身对不对；② 权限边界对不对（普通用户 403、普通管理员该能读的能读、不该动的动不了）。

mod common;

use axum::http::StatusCode;
use common::*;
use guangxue_auth::models::{ROLE_ADMIN, ROLE_USER};
use serde_json::json;

const SUPER: &str = "2262997289@qq.com";
const SUPER_PWD: &str = "7289HR_RedSun";
const PLAIN_ADMIN: &str = "plain-admin@example.com";
const PLAIN_ADMIN_PWD: &str = "admin12345";

/// 登录一个超管（顺带把种子账号建出来）
async fn login_super(app: &TestApp) -> Client {
    app.seed_admin(SUPER, SUPER_PWD).await;
    let mut client = Client::new(app);
    let res = client
        .post("/api/auth/login", json!({"email": SUPER, "password": SUPER_PWD, "device_label": "超管"}))
        .await;
    res.assert_status(StatusCode::OK);
    client
}

/// 登录一个**普通管理员**（role = admin：只读用户列表，治理动作一律 403）
async fn login_plain_admin(app: &TestApp) -> Client {
    app.seed_plain_admin(PLAIN_ADMIN, PLAIN_ADMIN_PWD).await;
    let mut client = Client::new(app);
    let res = client
        .post(
            "/api/auth/login",
            json!({"email": PLAIN_ADMIN, "password": PLAIN_ADMIN_PWD, "device_label": "普通管理员"}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.field("/data/user/role").unwrap(), ROLE_ADMIN);
    client
}

async fn create_invites(client: &mut Client, count: i64) -> Vec<serde_json::Value> {
    let res = client
        .post("/api/auth/admin/invites", json!({"count": count, "max_uses": 1, "expires_in_days": 7}))
        .await;
    res.assert_status(StatusCode::OK);
    res.data("items").as_array().cloned().expect("返回 items")
}

// ---------------------------------------------------------------- 公开配置

#[tokio::test]
async fn config_endpoint_is_public_and_reports_the_invite_switch() {
    // 默认（开发）不强制邀请码
    let app = spawn().await;
    let mut anon = Client::new(&app);
    let res = anon.get("/api/auth/config").await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("require_invite"), &json!(false), "开发默认不强制邀请码");

    // 打开强制邀请制后，未登录也能读到这一位（注册表单要靠它把邀请码标成必填）
    let app = spawn_with(|cfg| cfg.require_invite = true).await;
    let mut anon = Client::new(&app);
    let res = anon.get("/api/auth/config").await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("require_invite"), &json!(true));
}

// ---------------------------------------------------------------- 审计日志

#[tokio::test]
async fn audit_list_returns_actions_and_filters() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let super_id = super_admin.me().await.field("/data/user/id").unwrap().as_i64().unwrap();

    let created = create_invites(&mut super_admin, 1).await;
    let invite_id = created[0]["id"].as_i64().unwrap();
    super_admin
        .post(&format!("/api/auth/admin/invites/{invite_id}/disable"), json!({}))
        .await
        .assert_status(StatusCode::OK);

    // 全量列表：动作清单（下拉用）与新写的日志都在
    let all = super_admin.get("/api/auth/admin/audit").await;
    all.assert_status(StatusCode::OK);
    let actions: Vec<String> = all
        .data("actions")
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(actions.contains(&"invite_create".to_string()), "动作清单：{actions:?}");
    assert!(actions.contains(&"invite_disable".to_string()), "动作清单：{actions:?}");
    // ⚠️ 动作名是 `login_ok` / `login_fail`（不是 `login`）—— 面板的下拉直接吃这个数组，
    // 所以这里断言的是「库里的真名」，写错了界面就查不到东西
    assert!(actions.contains(&"login_ok".to_string()), "动作清单：{actions:?}");
    assert!(all.data("total").as_i64().unwrap() >= 3, "至少：登录 + 建码 + 停用");

    // 按动作筛
    let only_create = super_admin.get("/api/auth/admin/audit?action=invite_create").await;
    only_create.assert_status(StatusCode::OK);
    let items = only_create.data("items").as_array().unwrap();
    assert!(!items.is_empty());
    for item in items {
        assert_eq!(item["action"], json!("invite_create"));
    }
    // 操作者邮箱是 join 出来的，不是存在日志里的
    assert_eq!(items[0]["actor_email"], json!(SUPER));

    // 按操作者筛（只留超管自己写的那些）
    let only_actor = super_admin
        .get(&format!("/api/auth/admin/audit?actor={super_id}"))
        .await;
    only_actor.assert_status(StatusCode::OK);
    assert!(only_actor.data("total").as_i64().unwrap() >= 3);
    for item in only_actor.data("items").as_array().unwrap() {
        assert_eq!(item["actor_user_id"], json!(super_id));
    }

    // 分页：size=1 只回一条，但 total 仍是全量
    let one = super_admin.get("/api/auth/admin/audit?size=1&page=1").await;
    one.assert_status(StatusCode::OK);
    assert_eq!(one.data("items").as_array().unwrap().len(), 1);
    assert_eq!(one.data("total"), all.data("total"));

    // 时间格式写错要报错，而不是静默当成「没筛」
    super_admin
        .get("/api/auth/admin/audit?from=昨天")
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .assert_error("invalid_params");
}

#[tokio::test]
async fn audit_time_range_filters_days() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    create_invites(&mut super_admin, 1).await;

    // 测试时钟固定在 2025-10-09T08:53:20Z（见 common::TEST_START_UNIX）
    let inside = super_admin.get("/api/auth/admin/audit?from=2025-10-01").await;
    inside.assert_status(StatusCode::OK);
    assert!(inside.data("total").as_i64().unwrap() > 0, "10-01 之后应当有记录");

    let before = super_admin.get("/api/auth/admin/audit?to=2025-10-01").await;
    before.assert_status(StatusCode::OK);
    assert_eq!(before.data("total"), &json!(0), "10-01 之前不该有记录");

    // 只给日期 = 当天 00:00:00 UTC（不做「自动补到 23:59:59」的猜测）
    let same_day = super_admin.get("/api/auth/admin/audit?from=2025-10-09").await;
    same_day.assert_status(StatusCode::OK);
    assert!(same_day.data("total").as_i64().unwrap() > 0);

    let after_day = super_admin.get("/api/auth/admin/audit?from=2025-10-10").await;
    after_day.assert_status(StatusCode::OK);
    assert_eq!(after_day.data("total"), &json!(0));
}

#[tokio::test]
async fn audit_is_super_admin_only() {
    let app = spawn().await;
    let mut plain_admin = login_plain_admin(&app).await;
    plain_admin
        .get("/api/auth/admin/audit")
        .await
        .assert_status(StatusCode::FORBIDDEN);

    let mut user = Client::new(&app);
    register_open(&app, &mut user, "nobody@example.com", "abc12345").await.assert_status(StatusCode::OK);
    user.get("/api/auth/admin/audit").await.assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn prune_audit_drops_only_rows_older_than_the_retention_window() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    create_invites(&mut super_admin, 1).await;
    assert!(app.audit_count("login_ok").await >= 1);

    // 保留期 0 = 永不清理
    assert_eq!(app.state.service.prune_audit(0).await.unwrap(), 0);
    assert!(app.audit_count("login_ok").await >= 1);

    // 过 181 天之后，180 天的保留期应当把**之前写的那批**日志一起带走
    // （login_ok 与 invite_create 是同一时刻写进去的，年龄一样）
    app.advance(181 * 24 * 3600);
    let removed = app.state.service.prune_audit(180).await.unwrap();
    assert!(removed >= 1, "应当清掉过期记录，实际清了 {removed} 条");
    assert_eq!(app.audit_count("login_ok").await, 0, "过期记录必须清干净");
    assert_eq!(app.audit_count("invite_create").await, 0, "同一时刻写的也过期了");

    // 但在新时刻再写一条：它必须活下来 —— 「清掉旧的」不等于「清空表」
    let mut fresh = login_super(&app).await;
    create_invites(&mut fresh, 1).await;
    assert_eq!(
        app.state.service.prune_audit(180).await.unwrap(),
        0,
        "刚写的不该被清，此时应当没有可清的了"
    );
    assert!(app.audit_count("login_ok").await >= 1, "刚写的登录日志还在");
    assert!(app.audit_count("invite_create").await >= 1, "刚写的发码日志还在");
}

#[tokio::test]
async fn development_config_keeps_audit_logs_for_180_days_by_default() {
    let app = spawn().await;
    assert_eq!(app.state.cfg.audit_retention_days, 180, "定案：默认保留 180 天");
}

// ---------------------------------------------------------------- 邀请码：按批停用

#[tokio::test]
async fn batch_disable_disables_the_whole_batch_and_is_idempotent() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;

    // 两个批次：只停第一批量
    let first = create_invites(&mut super_admin, 3).await;
    let second = create_invites(&mut super_admin, 2).await;
    let batch_a = first[0]["batch_id"].as_str().unwrap().to_string();
    let batch_b = second[0]["batch_id"].as_str().unwrap().to_string();
    assert_ne!(batch_a, batch_b, "两次生成是不同的批次");
    for item in &first {
        assert_eq!(item["batch_id"].as_str().unwrap(), batch_a, "同一批共享 batch_id");
    }

    let res = super_admin
        .post(&format!("/api/auth/admin/invite-batches/{batch_a}/disable"), json!({}))
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("disabled"), &json!(3), "这一批 3 张全部停用");

    // 幂等：再点一次回 0，但不是错误
    let again = super_admin
        .post(&format!("/api/auth/admin/invite-batches/{batch_a}/disable"), json!({}))
        .await;
    again.assert_status(StatusCode::OK);
    assert_eq!(again.data("disabled"), &json!(0));

    // 另一批不受影响
    let disabled = super_admin.get("/api/auth/admin/invites?status=disabled&size=100").await;
    let ids: Vec<i64> = disabled
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    for item in &first {
        assert!(ids.contains(&item["id"].as_i64().unwrap()), "第一批应当已停用");
    }
    for item in &second {
        assert!(!ids.contains(&item["id"].as_i64().unwrap()), "第二批不该被动到");
    }

    // 不存在的批次 → 404
    super_admin
        .post("/api/auth/admin/invite-batches/deadbeef/disable", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .assert_error("not_found");

    // 审计留痕
    assert_eq!(app.audit_count("invite_disable_batch").await, 2);
}

#[tokio::test]
async fn batch_disable_requires_super_admin() {
    let app = spawn().await;
    let mut plain_admin = login_plain_admin(&app).await;
    plain_admin
        .post("/api/auth/admin/invite-batches/whatever/disable", json!({}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------- 邀请码：兑换记录

#[tokio::test]
async fn invite_list_shows_who_used_the_code() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let created = create_invites(&mut super_admin, 1).await;
    let code = created[0]["code"].as_str().unwrap().to_string();

    let mut newbie = Client::new(&app);
    register(&app, &mut newbie, "used-by@example.com", "abc12345", &code)
        .await
        .assert_status(StatusCode::OK);

    let listed = super_admin.get("/api/auth/admin/invites?status=used").await;
    listed.assert_status(StatusCode::OK);
    let item = listed
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["code"].as_str() == Some(code.as_str()))
        .expect("用过的码应当在 used 列表里")
        .clone();
    let uses = item["uses"].as_array().expect("列表要带兑换记录");
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0]["email"], json!("used-by@example.com"), "显示「谁用了」");
    assert!(uses[0]["used_at"].as_str().unwrap().ends_with('Z'), "时间是 UTC 文本");

    // 没用过的码：uses 是空数组而不是缺字段（前端不用判 undefined）
    let fresh = create_invites(&mut super_admin, 1).await;
    let fresh_id = fresh[0]["id"].as_i64().unwrap();
    let unused = super_admin.get("/api/auth/admin/invites?status=unused&size=100").await;
    let target = unused
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"].as_i64() == Some(fresh_id))
        .unwrap();
    assert_eq!(target["uses"], json!([]));
}

// ---------------------------------------------------------------- 用户列表：脱敏与搜索

#[tokio::test]
async fn user_list_masks_email_for_plain_admin_only() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let mut plain_admin = login_plain_admin(&app).await;
    let mut user = Client::new(&app);
    register_open(&app, &mut user, "plain-user@example.com", "abc12345")
        .await
        .assert_status(StatusCode::OK);

    let as_super = super_admin.get("/api/auth/admin/users?keyword=plain-user").await;
    as_super.assert_status(StatusCode::OK);
    assert_eq!(as_super.data("email_masked"), &json!(false), "超管看真邮箱");
    let items = as_super.data("items").as_array().unwrap();
    assert_eq!(items.len(), 1, "关键词应当命中这一个：{items:?}");
    assert_eq!(items[0]["email"], json!("plain-user@example.com"));

    let as_plain = plain_admin.get("/api/auth/admin/users?keyword=plain-user").await;
    as_plain.assert_status(StatusCode::OK);
    assert_eq!(as_plain.data("email_masked"), &json!(true), "管理员看打码邮箱");
    let items = as_plain.data("items").as_array().unwrap();
    assert_eq!(items[0]["email"], json!("pl***@example.com"), "脱敏在服务端做");

    // 关键词也匹配昵称（register_open 建的用户昵称是「测试用户」）
    let by_username = super_admin.get("/api/auth/admin/users?keyword=测试用户").await;
    by_username.assert_status(StatusCode::OK);
    assert!(by_username.data("total").as_i64().unwrap() >= 1);

    // 搜不到就是 0 条，而不是把全表回给前端
    let nothing = super_admin.get("/api/auth/admin/users?keyword=查无此人").await;
    assert_eq!(nothing.data("total"), &json!(0));
}

#[tokio::test]
async fn plain_admin_can_read_users_but_cannot_govern() {
    let app = spawn().await;
    let mut plain_admin = login_plain_admin(&app).await;
    let mut user = Client::new(&app);
    register_open(&app, &mut user, "target@example.com", "abc12345")
        .await
        .assert_status(StatusCode::OK);

    // 读：可以（权限矩阵里管理员本来就能只读用户列表）
    let listed = plain_admin.get("/api/auth/admin/users").await;
    listed.assert_status(StatusCode::OK);
    let target_id = listed
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["email"].as_str().unwrap().starts_with("ta***"))
        .expect("目标用户在列表里（邮箱已打码）")["id"]
        .as_i64()
        .unwrap();

    // 治理动作：一律 403
    plain_admin
        .post(&format!("/api/auth/admin/users/{target_id}/role"), json!({"role": ROLE_ADMIN}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    plain_admin
        .post(&format!("/api/auth/admin/users/{target_id}/status"), json!({"status": "disabled"}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    plain_admin
        .post(&format!("/api/auth/admin/users/{target_id}/logout-all"), json!({}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    assert_eq!(app.role_of("target@example.com").await, ROLE_USER, "角色没被动过");
}

// ---------------------------------------------------------------- 踢下线

#[tokio::test]
async fn logout_all_kills_every_session_of_the_target() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;

    // 同一个账号在两个端登录（两个 Client = 两个端）
    let mut phone = Client::new(&app);
    register_open(&app, &mut phone, "two-devices@example.com", "abc12345")
        .await
        .assert_status(StatusCode::OK);
    let mut laptop = Client::new(&app);
    laptop
        .post(
            "/api/auth/login",
            json!({"email": "two-devices@example.com", "password": "abc12345", "device_label": "笔记本"}),
        )
        .await
        .assert_status(StatusCode::OK);

    let user_id = phone.me().await.field("/data/user/id").unwrap().as_i64().unwrap();
    let res = super_admin
        .post(&format!("/api/auth/admin/users/{user_id}/logout-all"), json!({}))
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("revoked"), &json!(2), "两个端的会话都要吊销");

    // 两个端都掉线（access cookie 还在，但会话已吊销）
    phone.me().await.assert_status(StatusCode::UNAUTHORIZED);
    laptop.me().await.assert_status(StatusCode::UNAUTHORIZED);
    assert_eq!(app.audit_count("user_logout_all").await, 1);

    // 不存在的用户 → 404
    super_admin
        .post("/api/auth/admin/users/999999/logout-all", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .assert_error("not_found");
}

// ---------------------------------------------------------------- 整批发邮件

#[tokio::test]
async fn invite_mail_sends_each_code_in_log_mode() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let first = create_invites(&mut super_admin, 1).await;
    let second = create_invites(&mut super_admin, 1).await;
    let (id_a, code_a) = (first[0]["id"].as_i64().unwrap(), first[0]["code"].as_str().unwrap().to_string());
    let (id_b, code_b) = (second[0]["id"].as_i64().unwrap(), second[0]["code"].as_str().unwrap().to_string());

    let res = super_admin
        .post(
            "/api/auth/admin/invite-mail",
            json!({"pairs": [
                {"invite_id": id_a, "email": "Alice@Example.com"},
                {"invite_id": id_b, "email": "bob@example.com"}
            ]}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("sent"), &json!(2));
    assert_eq!(res.data("failed"), &json!(0));
    assert_eq!(res.data("mail_mode"), &json!("log"), "本机是 log 模式：不发真信");
    let items = res.data("items").as_array().unwrap();
    assert!(items.iter().all(|i| i["ok"] == json!(true)));

    // 收件邮箱规范化成小写，且码确实交给了发送器（log 发送器会记住最近一封）
    assert_eq!(items[0]["email"], json!("alice@example.com"));
    assert_eq!(
        app.state.mailer.last_dev_invite("alice@example.com").as_deref(),
        Some(code_a.as_str())
    );
    assert_eq!(
        app.state.mailer.last_dev_invite("bob@example.com").as_deref(),
        Some(code_b.as_str())
    );

    // 审计留痕，且**不写明文码**
    assert_eq!(app.audit_count("invite_email").await, 1);
    let logged = super_admin.get("/api/auth/admin/audit?action=invite_email").await;
    let detail = logged.data("items")[0]["detail"].as_str().unwrap().to_string();
    assert!(detail.contains("成功 2"), "detail 里要有统计：{detail}");
    assert!(!detail.contains(&code_a), "审计里绝不能出现邀请码明文");
    assert!(!logged.data("items")[0]["target"].as_str().unwrap().contains(&code_a));
}

#[tokio::test]
async fn invite_mail_reports_bad_rows_one_by_one() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let usable = create_invites(&mut super_admin, 1).await;
    let dying = create_invites(&mut super_admin, 1).await;
    let usable_id = usable[0]["id"].as_i64().unwrap();
    let dying_id = dying[0]["id"].as_i64().unwrap();
    super_admin
        .post(&format!("/api/auth/admin/invites/{dying_id}/disable"), json!({}))
        .await
        .assert_status(StatusCode::OK);

    let res = super_admin
        .post(
            "/api/auth/admin/invite-mail",
            json!({"pairs": [
                {"invite_id": usable_id, "email": "ok@example.com"},
                {"invite_id": dying_id, "email": "no@example.com"},
                {"invite_id": 999999, "email": "missing@example.com"}
            ]}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("sent"), &json!(1));
    assert_eq!(res.data("failed"), &json!(2));
    let items = res.data("items").as_array().unwrap();
    assert_eq!(items[0]["ok"], json!(true));
    assert_eq!(items[1]["ok"], json!(false));
    assert!(items[1]["error"].as_str().unwrap().contains("停用"), "要说清为什么：{:?}", items[1]);
    assert!(items[2]["error"].as_str().unwrap().contains("不存在"));
    // 一个失败不影响另一个：好的那封确实发出去了
    assert!(app.state.mailer.last_dev_invite("ok@example.com").is_some());
    assert!(app.state.mailer.last_dev_invite("no@example.com").is_none());
}

#[tokio::test]
async fn invite_mail_rejects_bad_input_upfront() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let created = create_invites(&mut super_admin, 1).await;
    let id = created[0]["id"].as_i64().unwrap();

    // 邮箱不合法：整单拒掉（不然要一封一封试）
    super_admin
        .post("/api/auth/admin/invite-mail", json!({"pairs": [{"invite_id": id, "email": "不是邮箱"}]}))
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .assert_error("invalid_params");

    // 空清单
    super_admin
        .post("/api/auth/admin/invite-mail", json!({"pairs": []}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // 超过 50 封
    let many: Vec<serde_json::Value> = (0..51)
        .map(|i| json!({"invite_id": id, "email": format!("user{i}@example.com")}))
        .collect();
    super_admin
        .post("/api/auth/admin/invite-mail", json!({"pairs": many}))
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .assert_error("invalid_params");

    // 普通管理员不能发
    let mut plain_admin = login_plain_admin(&app).await;
    plain_admin
        .post("/api/auth/admin/invite-mail", json!({"pairs": [{"invite_id": id, "email": "x@example.com"}]}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}
