//! 管理接口与初始管理员

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

const ADMIN_EMAIL: &str = "2262997289@qq.com";
const ADMIN_PASSWORD: &str = "7289HR_RedSun";

async fn login_as_admin(app: &TestApp) -> Client {
    app.seed_admin(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let mut client = Client::new(app);
    let res = client
        .post("/api/auth/login", json!({"email": ADMIN_EMAIL, "password": ADMIN_PASSWORD, "device_label": "管理台"}))
        .await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.field("/data/user/role").unwrap(), "admin");
    client
}

#[tokio::test]
async fn seeded_admin_can_login_with_the_given_password() {
    let app = spawn().await;
    let mut client = login_as_admin(&app).await;
    let me = client.me().await;
    assert_eq!(me.field("/data/user/email").unwrap(), ADMIN_EMAIL);
    assert_eq!(me.field("/data/user/role").unwrap(), "admin");
}

#[tokio::test]
async fn seeding_is_idempotent_and_never_overwrites_password() {
    let app = spawn().await;
    let created = app.state.service.seed_admin(ADMIN_EMAIL, ADMIN_PASSWORD).await.unwrap();
    assert!(created, "第一次应当创建");
    let again = app.state.service.seed_admin(ADMIN_EMAIL, "Different123").await.unwrap();
    assert!(!again, "第二次应当跳过");

    // 原密码仍然可用，说明没被覆盖
    let mut client = Client::new(&app);
    client
        .post("/api/auth/login", json!({"email": ADMIN_EMAIL, "password": ADMIN_PASSWORD}))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn normal_user_is_rejected_from_admin_endpoints() {
    let app = spawn().await;
    let mut user = Client::new(&app);
    // ⚠️ 必须走「不带邀请码」的开放注册：带邀请码注册现在会升级成管理员
    register_open(&app, &mut user, "plain@example.com", "abc12345")
        .await
        .assert_status(StatusCode::OK);

    user.get("/api/auth/admin/invites").await.assert_status(StatusCode::FORBIDDEN);
    user.post("/api/auth/admin/invites", json!({"count": 1}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    user.post("/api/auth/admin/invites/1/disable", json!({}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_creates_lists_and_disables_invites() {
    let app = spawn().await;
    let mut admin = login_as_admin(&app).await;

    let created = admin
        .post(
            "/api/auth/admin/invites",
            json!({"count": 2, "max_uses": 3, "expires_in_days": 5, "note": "第一批"}),
        )
        .await;
    created.assert_status(StatusCode::OK);
    let codes = created.data("codes").as_array().expect("返回明文码").clone();
    assert_eq!(codes.len(), 2);
    for code in &codes {
        let code = code.as_str().unwrap();
        assert_eq!(code.len(), 16, "邀请码长度固定");
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()), "{code}");
    }

    let listed = admin.get("/api/auth/admin/invites?status=unused").await;
    listed.assert_status(StatusCode::OK);
    let items = listed.data("items").as_array().unwrap().clone();
    assert!(items.iter().any(|i| Some(i["code"].as_str().unwrap()) == codes[0].as_str()));
    let target = items.iter().find(|i| Some(i["code"].as_str().unwrap()) == codes[0].as_str()).unwrap();
    assert_eq!(target["max_uses"], json!(3));
    assert_eq!(target["used_count"], json!(0));
    let id = target["id"].as_i64().unwrap();

    admin
        .post(&format!("/api/auth/admin/invites/{id}/disable"), json!({}))
        .await
        .assert_status(StatusCode::OK);

    let after = admin.get("/api/auth/admin/invites?status=disabled").await;
    let disabled = after.data("items").as_array().unwrap().clone();
    assert!(disabled.iter().any(|i| i["id"] == json!(id)));

    admin
        .post("/api/auth/admin/invites/999999/disable", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .assert_error("not_found");
}

#[tokio::test]
async fn admin_created_invite_can_be_used_to_register() {
    let app = spawn().await;
    let mut admin = login_as_admin(&app).await;

    let created = admin
        .post("/api/auth/admin/invites", json!({"count": 1, "max_uses": 1, "expires_in_days": 1}))
        .await;
    let code = created.data("codes")[0].as_str().unwrap().to_string();

    let mut newbie = Client::new(&app);
    register(&app, &mut newbie, "invited@example.com", "abc12345", &code)
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn login_records_admin_device_label() {
    let app = spawn().await;
    let mut client = login_as_admin(&app).await;
    let sessions = client.sessions().await;
    let items = sessions.data("items").as_array().unwrap();
    assert!(items.iter().any(|s| s["device_label"] == json!("管理台")));
}
