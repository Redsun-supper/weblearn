//! 安全相关：鉴权覆盖、CSRF、Cookie 属性、口令与令牌不泄露、账号锁定

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

#[tokio::test]
async fn protected_endpoints_reject_anonymous_requests() {
    let app = spawn().await;
    let mut client = Client::new(&app);

    for uri in ["/api/auth/me", "/api/auth/sessions", "/api/auth/admin/invites"] {
        client.get(uri).await.assert_status(StatusCode::UNAUTHORIZED);
    }
    client
        .post("/api/auth/logout-all", json!({}))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    client
        .post("/api/auth/admin/invites", json!({}))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn state_changing_request_from_foreign_origin_is_rejected() {
    let app = spawn().await;
    let mut client = Client::new(&app);

    let res = client
        .post_with_headers(
            "/api/auth/login",
            json!({"email": "a@example.com", "password": "abc12345"}),
            vec![("Origin", "http://evil.example")],
        )
        .await;
    res.assert_status(StatusCode::FORBIDDEN).assert_error("forbidden");
}

#[tokio::test]
async fn form_post_without_origin_is_rejected() {
    let app = spawn().await;
    let mut client = Client::new(&app);

    let res = client
        .post_without_origin(
            "/api/auth/login",
            json!({"email": "a@example.com", "password": "abc12345"}),
            "application/x-www-form-urlencoded",
        )
        .await;
    res.assert_status(StatusCode::FORBIDDEN).assert_error("forbidden");
}

#[tokio::test]
async fn json_post_without_origin_is_allowed() {
    let app = spawn().await;
    let mut client = Client::new(&app).with_ip("198.51.100.30");

    // 非浏览器客户端（curl / 测试脚本）没有 Origin，但仍然要能调
    let res = client
        .post_without_origin(
            "/api/auth/login",
            json!({"email": "nobody@example.com", "password": "abc12345"}),
            "application/json",
        )
        .await;
    res.assert_status(StatusCode::UNAUTHORIZED).assert_error("bad_credentials");
}

#[tokio::test]
async fn session_cookies_are_httponly_lax_and_scoped() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    let res = register_new(&app, &mut client, "cookie@example.com", "abc12345").await;

    let access = res.raw_cookie("gx_access").expect("access cookie");
    assert!(access.contains("HttpOnly"), "{access}");
    assert!(access.contains("SameSite=Lax"), "{access}");
    assert!(access.contains("Path=/"), "{access}");
    assert!(!access.contains("Secure"), "本地 http 下不应加 Secure");

    let refresh = res.raw_cookie("gx_refresh").expect("refresh cookie");
    assert!(refresh.contains("Path=/api/auth"), "refresh 只应发给认证接口：{refresh}");
    assert!(refresh.contains("HttpOnly"), "{refresh}");
}

#[tokio::test]
async fn tokens_and_password_never_appear_in_responses() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    let password = "SuperSecret123";
    let res = register_new(&app, &mut client, "leak@example.com", password).await;

    let raw = serde_json::to_string(&res.body).unwrap();
    assert!(!raw.contains(password), "响应体里不能有口令明文");
    assert!(!raw.contains("password_hash"));
    let access = client.access_cookie().cloned().unwrap();
    assert!(!raw.contains(&access), "access 令牌只应出现在 Set-Cookie 里");

    let me = client.get("/api/auth/me").await;
    let me_raw = serde_json::to_string(&me.body).unwrap();
    assert!(!me_raw.contains("password"));
    assert!(!me_raw.contains(&access));
}

#[tokio::test]
async fn password_is_stored_as_argon2id_only() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    let password = "StoredSecret123";
    register_new(&app, &mut client, "hash@example.com", password).await;

    let conn = rusqlite::Connection::open(&app.state.cfg.db_path).expect("打开测试库");
    let stored: String = conn
        .query_row("SELECT password_hash FROM users WHERE email = ?1", ["hash@example.com"], |r| r.get(0))
        .expect("读到口令哈希");
    assert!(stored.starts_with("$argon2id$v=19$"), "库里只能是 Argon2id PHC：{stored}");
    assert!(!stored.contains(password));

    // 会话表里也不能有 refresh 明文（只有 SHA-256 摘要）
    let (hash, len): (String, i64) = conn
        .query_row("SELECT refresh_hash, length(refresh_hash) FROM sessions LIMIT 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("读到会话");
    assert_eq!(len, 64, "SHA-256 十六进制是 64 字符：{hash}");
    assert!(!hash.contains('.') && !hash.contains('-'), "不应是 base64（那是明文形式）");
}

#[tokio::test]
async fn account_locks_after_repeated_failures() {
    let app = spawn_with(|cfg| {
        cfg.lock_threshold = 3;
        cfg.lock_minutes = 15;
    })
    .await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "lock@example.com", "abc12345").await;

    let mut attacker = Client::new(&app).with_ip("198.51.100.99");
    for i in 0..3 {
        let res = attacker
            .post("/api/auth/login", json!({"email": "lock@example.com", "password": "wrong12345"}))
            .await;
        let expected = if i == 2 { "account_locked" } else { "bad_credentials" };
        res.assert_error(expected);
    }

    // 锁定期间即使口令正确也进不去
    let blocked = attacker
        .post("/api/auth/login", json!({"email": "lock@example.com", "password": "abc12345"}))
        .await;
    blocked.assert_status(StatusCode::TOO_MANY_REQUESTS).assert_error("account_locked");

    // 锁定到期后恢复
    app.advance(15 * 60 + 1);
    attacker
        .post("/api/auth/login", json!({"email": "lock@example.com", "password": "abc12345"}))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn audit_log_records_failures_without_secrets() {
    let app = spawn().await;
    let mut client = Client::new(&app);
    register_new(&app, &mut client, "audit@example.com", "abc12345").await;

    let mut attacker = Client::new(&app).with_ip("203.0.113.77");
    // 一次成功登录 + 一次失败登录，审计里应各留一条
    client
        .post("/api/auth/login", json!({"email": "audit@example.com", "password": "abc12345"}))
        .await
        .assert_status(StatusCode::OK);
    attacker
        .post("/api/auth/login", json!({"email": "audit@example.com", "password": "wrong12345"}))
        .await
        .assert_error("bad_credentials");

    let conn = rusqlite::Connection::open(&app.state.cfg.db_path).expect("打开测试库");
    let actions: Vec<String> = {
        let mut stmt = conn.prepare("SELECT action FROM audit_logs ORDER BY id").unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert!(actions.iter().any(|a| a == "register"));
    assert!(actions.iter().any(|a| a == "login_ok"));
    assert!(actions.iter().any(|a| a == "login_fail"));

    let details: Vec<String> = {
        let mut stmt = conn.prepare("SELECT detail FROM audit_logs").unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert!(
        details.iter().all(|d| !d.contains("wrong12345") && !d.contains("abc12345")),
        "审计日志里不能出现口令：{details:?}"
    );
}
