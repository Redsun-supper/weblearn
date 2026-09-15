//! 集成测试脚手架
//!
//! 做法：用临时目录里的真 SQLite 库 + 真实 axum Router + `tower::oneshot` 打请求，
//! 时钟换成 `FakeClock`（能精确构造「验证码过期」「令牌过期」），
//! 限流默认关闭（只有专门测限流的用例才打开）。
//!
//! `Client` 模拟一个「端」：自带 cookie jar，所以两个 Client 就是两端同时登录。

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

use guangxue_auth::clock::FakeClock;
use guangxue_auth::config::{Argon2Config, Config, RateConfig, RateRule};
use guangxue_auth::mail::log_mailer::LogMailer;
use guangxue_auth::mail::Mailer;
use guangxue_auth::rate_limit::RateLimiter;
use guangxue_auth::service::AuthService;
use guangxue_auth::store::SqliteStore;
use guangxue_auth::{build_app, AppState};

/// 测试基准时间（2025-10-09T08:53:20Z 附近），固定下来才能断言「几秒后过期」
pub const TEST_START_UNIX: i64 = 1_760_000_000;

pub struct TestApp {
    pub state: AppState,
    pub router: Router,
    pub clock: Arc<FakeClock>,
    #[allow(dead_code)]
    tmp: TempDir,
}

/// 起一个默认配置的测试实例（开发默认值 + 低 Argon2 参数 + 关闭限流）
pub async fn spawn() -> TestApp {
    spawn_with(|_| {}).await
}

pub async fn spawn_with(configure: impl FnOnce(&mut Config)) -> TestApp {
    let tmp = tempfile::tempdir().expect("建临时目录");
    let mut cfg = Config::development();
    cfg.db_path = tmp.path().join("auth.db").to_string_lossy().to_string();
    // 测试不关心哈希强度，只关心行为
    cfg.argon2 = Argon2Config { m_cost: 8, t_cost: 1, p_cost: 1 };
    cfg.rate = RateConfig {
        login_ip: RateRule::off(),
        code_email_minute: RateRule::off(),
        code_email_hour: RateRule::off(),
        code_ip: RateRule::off(),
        register_ip: RateRule::off(),
    };
    configure(&mut cfg);

    let store = Arc::new(SqliteStore::open(&cfg.db_path).expect("打开数据库"));
    let clock = Arc::new(FakeClock::from_unix(TEST_START_UNIX));
    let mailer: Arc<dyn Mailer> = Arc::new(LogMailer::new());
    // 限流是否生效完全由 `cfg.rate` 决定：默认全关，需要测限流的用例自己打开
    let limiter = Arc::new(RateLimiter::new());

    let cfg = Arc::new(cfg);
    let service = Arc::new(
        AuthService::new(cfg.clone(), store, clock.clone(), mailer.clone(), limiter.clone())
            .expect("构建服务"),
    );
    let state = AppState { cfg, service, mailer, limiter, clock: clock.clone() };
    let router = build_app(state.clone());
    TestApp { state, router, clock, tmp }
}

impl TestApp {
    /// 直接建一个邀请码（测试里的准备动作，不走 HTTP）
    pub async fn new_invite(&self, max_uses: i64, expires_in_days: i64) -> String {
        let created = self
            .state
            .service
            .create_invites(None, 1, max_uses, expires_in_days, "测试")
            .await
            .expect("建邀请码");
        created.codes[0].code.clone()
    }

    /// 确保管理员存在（密码默认用开发默认值）
    pub async fn seed_admin(&self, email: &str, password: &str) {
        self.state.service.seed_admin(email, password).await.expect("建管理员");
    }

    pub fn advance(&self, secs: i64) {
        self.clock.advance(Duration::from_secs(secs as u64));
    }
}

/// 一个「端」：自带 cookie jar
pub struct Client {
    app: Router,
    jar: HashMap<String, String>,
    pub ip: String,
    pub ua: String,
}

impl Client {
    pub fn new(app: &TestApp) -> Self {
        Self {
            app: app.router.clone(),
            jar: HashMap::new(),
            ip: "203.0.113.10".to_string(),
            ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0 Safari/537.36".to_string(),
        }
    }

    pub fn with_ip(mut self, ip: &str) -> Self {
        self.ip = ip.to_string();
        self
    }

    pub fn with_ua(mut self, ua: &str) -> Self {
        self.ua = ua.to_string();
        self
    }

    pub fn access_cookie(&self) -> Option<&String> {
        self.jar.get("gx_access")
    }

    pub fn refresh_cookie(&self) -> Option<&String> {
        self.jar.get("gx_refresh")
    }

    pub fn set_cookie(&mut self, name: &str, value: &str) {
        self.jar.insert(name.to_string(), value.to_string());
    }

    pub async fn get(&mut self, uri: &str) -> Resp {
        self.send(Method::GET, uri, None, true).await
    }

    /// 当前登录用户
    pub async fn me(&mut self) -> Resp {
        self.get("/api/auth/me").await
    }

    /// 我的活跃会话列表
    pub async fn sessions(&mut self) -> Resp {
        self.get("/api/auth/sessions").await
    }

    pub async fn post(&mut self, uri: &str, body: Value) -> Resp {
        self.send(Method::POST, uri, Some(body), true).await
    }

    /// 带自定义头的 POST（用于构造跨站 Origin、表单 Content-Type 等场景）
    pub async fn post_with_headers(&mut self, uri: &str, body: Value, extra: Vec<(&str, &str)>) -> Resp {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::USER_AGENT, self.ua.clone())
            .header("x-forwarded-for", self.ip.clone());
        for (name, value) in extra {
            req = req.header(name, value);
        }
        if !self.jar.is_empty() {
            req = req.header(header::COOKIE, self.cookie_header());
        }
        let req = req.body(Body::from(body.to_string())).unwrap();
        self.execute(req).await
    }

    /// 不带 Origin 的裸请求（用于 CSRF 用例）
    pub async fn post_without_origin(&mut self, uri: &str, body: Value, content_type: &str) -> Resp {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::USER_AGENT, self.ua.clone())
            .header("x-forwarded-for", self.ip.clone());
        if !self.jar.is_empty() {
            req = req.header(header::COOKIE, self.cookie_header());
        }
        let body = if body.is_null() { String::new() } else { body.to_string() };
        let req = req.body(Body::from(body)).unwrap();
        self.execute(req).await
    }

    async fn send(&mut self, method: Method, uri: &str, body: Option<Value>, with_origin: bool) -> Resp {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::USER_AGENT, self.ua.clone())
            .header("x-forwarded-for", self.ip.clone());
        if with_origin {
            req = req.header(header::ORIGIN, "http://127.0.0.1:8899");
        }
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        if !self.jar.is_empty() {
            req = req.header(header::COOKIE, self.cookie_header());
        }
        let body = match body {
            Some(value) => Body::from(value.to_string()),
            None => Body::empty(),
        };
        let req = req.body(body).unwrap();
        self.execute(req).await
    }

    fn cookie_header(&self) -> String {
        let mut items: Vec<String> = self
            .jar
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        items.sort();
        items.join("; ")
    }

    async fn execute(&mut self, req: Request<Body>) -> Resp {
        let res = self.app.clone().oneshot(req).await.expect("请求失败");
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.expect("读响应体").to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        let cookies = parse_set_cookies(&headers);
        let set_cookie_raw = raw_set_cookies(&headers);
        for (name, value, cleared) in &cookies {
            if *cleared {
                self.jar.remove(name);
            } else {
                self.jar.insert(name.clone(), value.clone());
            }
        }
        Resp { status, body, cookies, set_cookie_raw }
    }
}

#[derive(Debug)]
pub struct Resp {
    pub status: StatusCode,
    pub body: Value,
    /// (名称, 值, 是否被清除)
    pub cookies: Vec<(String, String, bool)>,
    /// 原始 `Set-Cookie` 串（断言 HttpOnly / SameSite / Path 用）
    pub set_cookie_raw: Vec<String>,
}

impl Resp {
    pub fn data(&self, key: &str) -> &Value {
        &self.body["data"][key]
    }

    pub fn field(&self, pointer: &str) -> Option<&Value> {
        self.body.pointer(pointer)
    }

    pub fn error_code(&self) -> Option<&str> {
        self.body.get("error").and_then(|v| v.as_str())
    }

    pub fn message(&self) -> &str {
        self.body.get("message").and_then(|v| v.as_str()).unwrap_or("")
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, v, _)| v.as_str())
    }

    /// 某个 Cookie 的完整 `Set-Cookie` 串
    pub fn raw_cookie(&self, name: &str) -> Option<&str> {
        self.set_cookie_raw
            .iter()
            .find(|raw| raw.starts_with(&format!("{name}=")))
            .map(|s| s.as_str())
    }

    pub fn raw_set_cookie(&self, name: &str) -> Option<&(String, String, bool)> {
        self.cookies.iter().find(|(n, _, _)| n == name)
    }

    pub fn assert_status(&self, expected: StatusCode) -> &Self {
        assert_eq!(self.status, expected, "响应体：{}", self.body);
        self
    }

    pub fn assert_error(&self, expected: &str) -> &Self {
        assert_eq!(self.error_code(), Some(expected), "响应体：{}", self.body);
        self
    }
}

impl Clone for Resp {
    fn clone(&self) -> Self {
        Resp {
            status: self.status,
            body: self.body.clone(),
            cookies: self.cookies.clone(),
            set_cookie_raw: self.set_cookie_raw.clone(),
        }
    }
}

fn raw_set_cookies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect()
}

fn parse_set_cookies(headers: &HeaderMap) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for value in headers.get_all(header::SET_COOKIE).iter() {
        let Ok(raw) = value.to_str() else { continue };
        let mut parts = raw.split(';');
        let Some(first) = parts.next() else { continue };
        let Some((name, value)) = first.split_once('=') else { continue };
        let cleared = raw.to_ascii_lowercase().contains("max-age=0");
        out.push((name.trim().to_string(), value.trim().to_string(), cleared));
    }
    out
}

/// 完整走一遍「发码 → 取码 → 注册」，返回注册后的客户端（已带上会话 Cookie）
pub async fn register(
    _app: &TestApp,
    client: &mut Client,
    email: &str,
    password: &str,
    invite_code: &str,
) -> Resp {
    let sent = client
        .post("/api/auth/email-code", serde_json::json!({ "email": email, "invite_code": invite_code }))
        .await;
    assert_eq!(sent.status, StatusCode::OK, "发码失败：{}", sent.body);
    let code = fetch_dev_code(client, email).await;
    client
        .post(
            "/api/auth/register",
            serde_json::json!({
                "email": email,
                "email_code": code,
                "invite_code": invite_code,
                "password": password,
                "username": "测试用户",
            }),
        )
        .await
}

/// 从开发调试接口读取验证码
pub async fn fetch_dev_code(client: &mut Client, email: &str) -> String {
    let res = client.get(&format!("/api/auth/dev/codes?email={email}")).await;
    assert_eq!(res.status, StatusCode::OK, "取开发验证码失败：{}", res.body);
    res.data("code").as_str().expect("验证码是字符串").to_string()
}

/// 一步到位：建邀请码 + 注册一个新账号。
///
/// ⚠️ 带邀请码注册**会升级成管理员**（邀请码现在是兑换券，不是注册门槛）——
/// 需要普通用户请改用 [`register_open`]。
pub async fn register_new(app: &TestApp, client: &mut Client, email: &str, password: &str) -> Resp {
    let invite = app.new_invite(1, 7).await;
    register(app, client, email, password, &invite).await
}

/// **不填邀请码**的注册（开放注册路径）：注册出来应当是普通用户。
pub async fn register_open(_app: &TestApp, client: &mut Client, email: &str, password: &str) -> Resp {
    let sent = client
        .post("/api/auth/email-code", serde_json::json!({ "email": email }))
        .await;
    assert_eq!(sent.status, StatusCode::OK, "不带邀请码发码失败：{}", sent.body);
    let code = fetch_dev_code(client, email).await;
    client
        .post(
            "/api/auth/register",
            serde_json::json!({
                "email": email,
                "email_code": code,
                "password": password,
                "username": "测试用户",
            }),
        )
        .await
}

/// 用一串邀请码注册（多个用空格分隔）
pub async fn register_with_codes(
    _app: &TestApp,
    client: &mut Client,
    email: &str,
    password: &str,
    codes: &str,
) -> Resp {
    let sent = client
        .post("/api/auth/email-code", serde_json::json!({ "email": email, "invite_code": codes }))
        .await;
    assert_eq!(sent.status, StatusCode::OK, "带邀请码发码失败：{}", sent.body);
    let code = fetch_dev_code(client, email).await;
    client
        .post(
            "/api/auth/register",
            serde_json::json!({
                "email": email,
                "email_code": code,
                "invite_code": codes,
                "password": password,
                "username": "测试用户",
            }),
        )
        .await
}
