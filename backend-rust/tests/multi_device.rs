//! 多端同时登录：会话互不影响、单端登出、全部登出、会话列表

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// 一个账号在两个「端」上登录：一个注册（自带会话），一个走登录
async fn two_devices(app: &TestApp, email: &str, password: &str) -> (Client, Client) {
    let mut phone = Client::new(app);
    phone.ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Safari/604.1".to_string();
    register_new(app, &mut phone, email, password).await;

    let mut laptop = Client::new(app);
    laptop.ua = "Mozilla/5.0 (Windows NT 10.0; Win64) Chrome/120.0 Safari/537.36".to_string();
    let login = laptop
        .post("/api/auth/login", json!({"email": email, "password": password, "device_label": "我的笔记本"}))
        .await;
    login.assert_status(StatusCode::OK);
    (phone, laptop)
}

#[tokio::test]
async fn same_account_can_be_logged_in_on_two_devices() {
    let app = spawn().await;
    let (mut phone, mut laptop) = two_devices(&app, "multi@example.com", "abc12345").await;

    phone.get("/api/auth/me").await.assert_status(StatusCode::OK);
    laptop.get("/api/auth/me").await.assert_status(StatusCode::OK);

    let list = phone.get("/api/auth/sessions").await;
    list.assert_status(StatusCode::OK);
    let items = list.data("items").as_array().expect("会话数组").clone();
    assert_eq!(items.len(), 2, "两端应各有一个会话：{list:?}");

    let current_count = items.iter().filter(|s| s["current"] == json!(true)).count();
    assert_eq!(current_count, 1, "只有当前这个端应标 current");

    let labels: Vec<String> = items
        .iter()
        .map(|s| s["device_label"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(labels.iter().any(|l| l == "我的笔记本"), "登录时指定的设备名应被记录：{labels:?}");
    assert!(labels.iter().any(|l| l.contains("iPhone")), "另一端的设备名应从 UA 推断：{labels:?}");
}

#[tokio::test]
async fn logging_out_one_device_keeps_the_other() {
    let app = spawn().await;
    let (mut phone, mut laptop) = two_devices(&app, "multi2@example.com", "abc12345").await;

    phone.post("/api/auth/logout", json!({})).await.assert_status(StatusCode::OK);
    phone.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);

    laptop.get("/api/auth/me").await.assert_status(StatusCode::OK);
    let refresh = laptop.post("/api/auth/refresh", json!({})).await;
    refresh.assert_status(StatusCode::OK);
}

#[tokio::test]
async fn logout_all_can_keep_current_device() {
    let app = spawn().await;
    let (mut phone, mut laptop) = two_devices(&app, "multi3@example.com", "abc12345").await;

    let res = phone.post("/api/auth/logout-all", json!({"keep_current": true})).await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("revoked").as_u64().unwrap(), 1, "应当只吊销另一个端");

    phone.get("/api/auth/me").await.assert_status(StatusCode::OK);
    laptop.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_all_revokes_every_device() {
    let app = spawn().await;
    let (mut phone, mut laptop) = two_devices(&app, "multi4@example.com", "abc12345").await;

    let res = phone.post("/api/auth/logout-all", json!({})).await;
    res.assert_status(StatusCode::OK);
    assert_eq!(res.data("revoked").as_u64().unwrap(), 2);

    phone.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
    laptop.get("/api/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
    // Cookie 也应被清掉
    assert!(res.raw_cookie("gx_access").unwrap().contains("Max-Age=0"));
}

#[tokio::test]
async fn each_device_refreshes_independently() {
    let app = spawn().await;
    let (mut phone, mut laptop) = two_devices(&app, "multi5@example.com", "abc12345").await;

    // 手机刷新自己的会话
    phone.post("/api/auth/refresh", json!({})).await.assert_status(StatusCode::OK);
    // 笔记本的令牌不受影响
    laptop.get("/api/auth/me").await.assert_status(StatusCode::OK);
    laptop.post("/api/auth/refresh", json!({})).await.assert_status(StatusCode::OK);

    let sessions = phone.get("/api/auth/sessions").await;
    assert_eq!(sessions.data("items").as_array().unwrap().len(), 2, "两个端都还在");
}
