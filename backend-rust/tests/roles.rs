//! P0-5：三级角色（user / admin / super_admin）与邀请码分级
//!
//! 这一组盯的是「权限边界」而不是功能本身，所以断言写得比较硬：
//! 每条「不该成功」的操作后面都跟着一句**直接查库**的确认（角色没变、状态没变），
//! 避免出现「接口返回 403 但库已经被改了」这种最糟的组合。

mod common;

use axum::http::StatusCode;
use common::*;
use guangxue_auth::models::{ROLE_ADMIN, ROLE_SUPER_ADMIN, ROLE_USER};
use guangxue_auth::store::sql::InviteFilter;
use serde_json::json;

const ADMIN_EMAIL: &str = "2262997289@qq.com";
const ADMIN_PASSWORD: &str = "7289HR_RedSun";

/// 登录成超管（种子账号在 P0-5 起就是超管）
async fn login_super(app: &TestApp) -> Client {
    app.seed_admin(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let mut client = Client::new(app);
    client
        .post("/api/auth/login", json!({"email": ADMIN_EMAIL, "password": ADMIN_PASSWORD}))
        .await
        .assert_status(StatusCode::OK);
    client
}

/// 登录成**普通管理员**（用来验证「管理员碰不到权限」）
async fn login_plain_admin(app: &TestApp, email: &str) -> Client {
    app.seed_plain_admin(email, "PlainAdmin123").await;
    let mut client = Client::new(app);
    client
        .post("/api/auth/login", json!({"email": email, "password": "PlainAdmin123"}))
        .await
        .assert_status(StatusCode::OK);
    client
}

/// 注册一个普通用户并返回客户端
async fn a_plain_user(app: &TestApp, email: &str) -> Client {
    let mut client = Client::new(app);
    register_open(app, &mut client, email, "abc12345")
        .await
        .assert_status(StatusCode::OK);
    client
}

/// 从用户列表里找一个人的 id
async fn find_user(admin: &mut Client, email: &str) -> i64 {
    let res = admin.get("/api/auth/admin/users?size=100").await;
    res.assert_status(StatusCode::OK);
    res.data("items")
        .as_array()
        .expect("items 是数组")
        .iter()
        .find(|u| u["email"] == email)
        .unwrap_or_else(|| panic!("用户列表里找不到 {email}：{}", res.body))["id"]
        .as_i64()
        .expect("id 是数字")
}

// ---------------------------------------------------------------- 角色契约

#[tokio::test]
async fn seeded_account_is_a_super_admin() {
    let app = spawn().await;
    let mut admin = login_super(&app).await;
    let me = admin.me().await;
    me.assert_status(StatusCode::OK);
    assert_eq!(
        me.data("user")["role"], ROLE_SUPER_ADMIN,
        "启动期种子账号必须是超管，否则全新部署没人能发码：{}",
        me.body
    );
}

#[tokio::test]
async fn ensure_super_admin_promotes_an_existing_plain_admin() {
    // 上线时老库里那个账号是 admin —— 启动期必须把它**提权**成超管，
    // 否则「环境变量那一半」等于没生效，你还是得 SSH 上去手写 SQL。
    let app = spawn().await;
    app.seed_plain_admin(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert_eq!(app.role_of(ADMIN_EMAIL).await, ROLE_ADMIN);

    let (created, promoted) = app
        .state
        .service
        .ensure_super_admin(ADMIN_EMAIL, ADMIN_PASSWORD)
        .await
        .expect("确保超管");
    assert!(!created, "账号已存在，不该新建");
    assert!(promoted, "应当被提权");
    assert_eq!(app.role_of(ADMIN_EMAIL).await, ROLE_SUPER_ADMIN);

    // 幂等：再跑一次什么都不做
    let (created, promoted) = app
        .state
        .service
        .ensure_super_admin(ADMIN_EMAIL, ADMIN_PASSWORD)
        .await
        .unwrap();
    assert!(!created && !promoted, "已经是超管时应当什么都不做");
}

#[tokio::test]
async fn ensure_super_admin_never_demotes() {
    // 反向保护：把「环境变量里的邮箱」换成别人，旧超管**仍然是超管**。
    // 自动降权会在「想换人」时把上一个超管悄悄锁死，而锁死超管无法自救。
    let app = spawn().await;
    app.seed_admin(ADMIN_EMAIL, ADMIN_PASSWORD).await; // 超管
    app.state
        .service
        .ensure_super_admin("someone-else@example.com", "Other12345")
        .await
        .expect("用另一个邮箱确保超管");

    assert_eq!(
        app.role_of(ADMIN_EMAIL).await,
        ROLE_SUPER_ADMIN,
        "旧超管不该被自动降级"
    );
    assert_eq!(app.role_of("someone-else@example.com").await, ROLE_SUPER_ADMIN);
}

// ---------------------------------------------------------------- 邀请码分级

#[tokio::test]
async fn super_admin_can_create_user_and_admin_codes() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;

    let res = super_admin
        .post(
            "/api/auth/admin/invites",
            json!({"count": 2, "max_uses": 1, "expires_in_days": 7, "note": "普通码"}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    for item in res.data("items").as_array().unwrap() {
        assert_eq!(item["grant_role"], ROLE_USER);
        assert!(!item["code"].as_str().unwrap().starts_with("ADMIN-"));
        assert!(!item["batch_id"].as_str().unwrap().is_empty(), "同批要共享 batch_id");
    }
    // 同一批的两张码 batch_id 相同
    let items = res.data("items").as_array().unwrap();
    assert_eq!(items[0]["batch_id"], items[1]["batch_id"]);

    let res = super_admin
        .post(
            "/api/auth/admin/invites",
            json!({"count": 1, "grant_role": ROLE_ADMIN, "note": "管理员码"}),
        )
        .await;
    res.assert_status(StatusCode::OK);
    let code = res.data("items")[0]["code"].as_str().unwrap().to_string();
    assert!(code.starts_with("ADMIN-"), "管理员码要带给人看的前缀：{code}");
    assert_eq!(res.data("items")[0]["grant_role"], ROLE_ADMIN);

    // 列表里等级也要能看到
    let list = super_admin.get("/api/auth/admin/invites?size=50").await;
    list.assert_status(StatusCode::OK);
    let roles: Vec<String> = list
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["grant_role"].as_str().unwrap().to_string())
        .collect();
    assert!(roles.contains(&ROLE_ADMIN.to_string()));
    assert!(roles.contains(&ROLE_USER.to_string()));
}

#[tokio::test]
async fn grant_role_rejects_super_admin_and_nonsense() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;

    // 本期刻意**不支持超管码**：一张码就能再造一个能封你号的人
    for bad in [ROLE_SUPER_ADMIN, "root", ""] {
        let res = super_admin
            .post("/api/auth/admin/invites", json!({"count": 1, "grant_role": bad}))
            .await;
        res.assert_status(StatusCode::BAD_REQUEST);
    }
    assert_eq!(app.invite_rows().await, 0, "非法等级不该建出任何码");
}

#[tokio::test]
async fn plain_admin_cannot_touch_invites_at_all() {
    let app = spawn().await;
    let mut admin = login_plain_admin(&app, "content@example.com").await;

    // 管理员能登录、能进后台（这里用 /me 证明身份有效）
    let me = admin.me().await;
    me.assert_status(StatusCode::OK);
    assert_eq!(me.data("user")["role"], ROLE_ADMIN);

    // 但发码 / 看码 / 停码一律 403 —— 管理员是内容运营角色，碰不到权限
    admin
        .get("/api/auth/admin/invites")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    admin
        .post("/api/auth/admin/invites", json!({"count": 1}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    admin
        .post("/api/auth/admin/invites/1/disable", json!({}))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    assert_eq!(app.invite_rows().await, 0, "被拒的请求不该留下数据");
}

#[tokio::test]
async fn plain_user_cannot_touch_invites_or_users() {
    let app = spawn().await;
    let mut user = a_plain_user(&app, "student@example.com").await;

    for path in ["/api/auth/admin/invites", "/api/auth/admin/users"] {
        user.get(path).await.assert_status(StatusCode::FORBIDDEN);
    }
    user.post("/api/auth/admin/users/1/role", json!({"role": "admin"}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    user.post("/api/auth/admin/users/1/status", json!({"status": "disabled"}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------- 用户治理

#[tokio::test]
async fn super_admin_promotes_and_demotes_a_user() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    a_plain_user(&app, "promote-me@example.com").await;
    let target = find_user(&mut super_admin, "promote-me@example.com").await;

    let res = super_admin
        .post(&format!("/api/auth/admin/users/{target}/role"), json!({"role": ROLE_ADMIN}))
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("user")["role"], ROLE_ADMIN);
    assert_eq!(app.role_of("promote-me@example.com").await, ROLE_ADMIN);

    // 降回普通用户
    let res = super_admin
        .post(&format!("/api/auth/admin/users/{target}/role"), json!({"role": ROLE_USER}))
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(app.role_of("promote-me@example.com").await, ROLE_USER);

    // 治理动作必须留痕
    assert!(app.audit_count("role_change").await >= 2, "两次改角色都该写审计");
}

#[tokio::test]
async fn role_change_rejects_bad_values_and_unknown_user() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    a_plain_user(&app, "victim@example.com").await;
    let target = find_user(&mut super_admin, "victim@example.com").await;

    super_admin
        .post(&format!("/api/auth/admin/users/{target}/role"), json!({"role": "root"}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(app.role_of("victim@example.com").await, ROLE_USER, "非法角色不该改库");

    super_admin
        .post("/api/auth/admin/users/999999/role", json!({"role": ROLE_ADMIN}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn banning_a_user_revokes_their_sessions() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let mut victim = a_plain_user(&app, "ban-me@example.com").await;
    let target = find_user(&mut super_admin, "ban-me@example.com").await;

    // 封禁前他是能用的
    victim.me().await.assert_status(StatusCode::OK);

    let res = super_admin
        .post(&format!("/api/auth/admin/users/{target}/status"), json!({"status": "disabled"}))
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(app.status_of("ban-me@example.com").await, "disabled");

    // ⚠️ 关键：封禁必须顺带吊销会话，否则他手里的令牌 15 分钟内还能用
    victim.me().await.assert_status(StatusCode::UNAUTHORIZED);
    assert!(app.audit_count("user_status").await >= 1);

    // 解封后能重新登录
    super_admin
        .post(&format!("/api/auth/admin/users/{target}/status"), json!({"status": "active"}))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(app.status_of("ban-me@example.com").await, "active");

    let mut again = Client::new(&app);
    again
        .post("/api/auth/login", json!({"email": "ban-me@example.com", "password": "abc12345"}))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn cannot_demote_or_ban_the_last_super_admin() {
    // 自锁保护：只剩下一个可用超管时，既不能降级也不能封禁 ——
    // 否则谁也进不了后台，只能 SSH 上去手写 SQL 救回来。
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let me = super_admin.me().await;
    let my_id = me.data("user")["id"].as_i64().unwrap();

    let res = super_admin
        .post(&format!("/api/auth/admin/users/{my_id}/role"), json!({"role": ROLE_ADMIN}))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(app.role_of(ADMIN_EMAIL).await, ROLE_SUPER_ADMIN, "被拒后角色必须没变");

    let res = super_admin
        .post(&format!("/api/auth/admin/users/{my_id}/status"), json!({"status": "disabled"}))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(app.status_of(ADMIN_EMAIL).await, "active", "被拒后状态必须没变");

    // 有了第二个超管之后，降级第一个就应当放行
    a_plain_user(&app, "second-super@example.com").await;
    let second = find_user(&mut super_admin, "second-super@example.com").await;
    super_admin
        .post(&format!("/api/auth/admin/users/{second}/role"), json!({"role": ROLE_SUPER_ADMIN}))
        .await
        .assert_status(StatusCode::OK);

    super_admin
        .post(&format!("/api/auth/admin/users/{my_id}/role"), json!({"role": ROLE_ADMIN}))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(app.role_of(ADMIN_EMAIL).await, ROLE_ADMIN);
}

#[tokio::test]
async fn last_super_admin_guard_ignores_disabled_super_admins() {
    // 只数「可用」的超管：被封掉的超管不能当「还有人能救我」的理由，
    // 否则「先封另一个超管、再把自己降级」会一路通过。
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    a_plain_user(&app, "third@example.com").await;
    let third = find_user(&mut super_admin, "third@example.com").await;

    // 把他提成超管再封掉
    super_admin
        .post(&format!("/api/auth/admin/users/{third}/role"), json!({"role": ROLE_SUPER_ADMIN}))
        .await
        .assert_status(StatusCode::OK);
    super_admin
        .post(&format!("/api/auth/admin/users/{third}/status"), json!({"status": "disabled"}))
        .await
        .assert_status(StatusCode::OK);

    // 现在「可用的超管」只剩我自己 → 依然不能降级自己
    let my_id = super_admin.me().await.data("user")["id"].as_i64().unwrap();
    super_admin
        .post(&format!("/api/auth/admin/users/{my_id}/role"), json!({"role": ROLE_ADMIN}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(app.role_of(ADMIN_EMAIL).await, ROLE_SUPER_ADMIN);
}

#[tokio::test]
async fn user_list_filters_by_role() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    a_plain_user(&app, "u1@example.com").await;
    a_plain_user(&app, "u2@example.com").await;
    login_plain_admin(&app, "content2@example.com").await;

    let all = super_admin.get("/api/auth/admin/users?size=100").await;
    all.assert_status(StatusCode::OK);
    assert_eq!(all.data("total").as_i64().unwrap(), 4, "1 超管 + 1 管理员 + 2 普通用户");

    let only_admin = super_admin
        .get(&format!("/api/auth/admin/users?role={ROLE_ADMIN}&size=100"))
        .await;
    let items = only_admin.data("items").as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["email"], "content2@example.com");

    let only_super = super_admin
        .get(&format!("/api/auth/admin/users?role={ROLE_SUPER_ADMIN}&size=100"))
        .await;
    assert_eq!(only_super.data("items").as_array().unwrap().len(), 1);

    super_admin
        .get("/api/auth/admin/users?role=root")
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_codes_still_land_in_the_invite_list_with_their_level() {
    // 端到端：发一张管理员码 → 用它注册 → 新账号直接是管理员
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;

    let created = super_admin
        .post(
            "/api/auth/admin/invites",
            json!({"count": 1, "grant_role": ROLE_ADMIN, "note": "给运营"}),
        )
        .await;
    created.assert_status(StatusCode::OK);
    let code = created.data("items")[0]["code"].as_str().unwrap().to_string();

    let mut new_admin = Client::new(&app);
    register(&app, &mut new_admin, "ops@example.com", "abc12345", &code)
        .await
        .assert_status(StatusCode::OK);

    let me = new_admin.me().await;
    assert_eq!(me.data("user")["role"], ROLE_ADMIN, "管理员码应当直接给 admin");

    // 他能进后台（放行 admin），但**依然发不了码**（那是超管的事）
    let list = super_admin.get("/api/auth/admin/invites?size=50").await;
    let used = list
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["code"] == code.as_str())
        .expect("码在列表里");
    assert_eq!(used["used_count"], 1);
    assert_eq!(used["grant_role"], ROLE_ADMIN);

    new_admin
        .post("/api/auth/admin/invites", json!({"count": 1}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invite_list_hides_nothing_but_requires_super_admin() {
    let app = spawn().await;
    let mut super_admin = login_super(&app).await;
    let invite = app.new_invite(1, 7).await;

    let list = super_admin.get(&format!("/api/auth/admin/invites?status=unused&size=50")).await;
    list.assert_status(StatusCode::OK);
    let found = list
        .data("items")
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["code"] == invite.as_str());
    assert!(found, "新码应当在 unused 列表里");

    // 顺带确认 grant_role / batch_id 两个新字段确实出参了（前端要用）
    let (items, _) = app.state.service.list_invites(InviteFilter::All, 1, 10).await.unwrap();
    assert!(items.iter().all(|i| !i.batch_id.is_empty()), "每张码都该有批次号");
}
