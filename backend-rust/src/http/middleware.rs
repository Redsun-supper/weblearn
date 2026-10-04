// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! Cookie 读写、CSRF 防护与响应头工具
//!
//! 为什么手写而不是引 `cookie`/`tower-cookies`：这里只有两个固定 Cookie，
//! 属性写法一目了然，少一层依赖也少一处版本风险；规范化与解析都有单元测试。

use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::AuthError;
use crate::http::{ACCESS_COOKIE, REFRESH_COOKIE};
use crate::AppState;

/// 从 `Cookie` 头里取某个 Cookie 的值
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::COOKIE).iter() {
        let Ok(raw) = value.to_str() else { continue };
        for part in raw.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k.trim() == name {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// 拼一个 `Set-Cookie` 的值
///
/// 属性说明：
///   - `HttpOnly`：JS 读不到令牌（XSS 也偷不走）；
///   - `SameSite=Lax`：跨站 POST 不带 Cookie，配合下面的 Origin 校验；
///   - `Secure`：仅线上（HTTPS）开启，本地 http 下必须关，否则 Cookie 不落地。
pub fn cookie_header(
    name: &str,
    value: &str,
    path: &str,
    max_age: i64,
    secure: bool,
    domain: Option<&str>,
) -> String {
    let mut s = format!("{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax");
    if secure {
        s.push_str("; Secure");
    }
    if let Some(d) = domain {
        if !d.is_empty() {
            s.push_str(&format!("; Domain={d}"));
        }
    }
    s
}

/// 清空 Cookie（登出、refresh 失败时用）
pub fn clear_cookie_header(name: &str, path: &str, secure: bool, domain: Option<&str>) -> String {
    cookie_header(name, "", path, 0, secure, domain)
}

/// 把两个会话 Cookie 挂到响应上
pub fn attach_auth_cookies(res: &mut Response, state: &AppState, access: &str, refresh: &str, access_max_age: i64, refresh_max_age: i64) {
    let secure = state.cfg.cookie_secure;
    let domain = state.cfg.cookie_domain.as_deref();
    for value in [
        cookie_header(ACCESS_COOKIE, access, "/", access_max_age, secure, domain),
        cookie_header(REFRESH_COOKIE, refresh, "/api/auth", refresh_max_age, secure, domain),
    ] {
        if let Ok(header_value) = HeaderValue::from_str(&value) {
            res.headers_mut().append(header::SET_COOKIE, header_value);
        }
    }
}

/// 清掉两个会话 Cookie
pub fn clear_auth_cookies(res: &mut Response, state: &AppState) {
    let secure = state.cfg.cookie_secure;
    let domain = state.cfg.cookie_domain.as_deref();
    for value in [
        clear_cookie_header(ACCESS_COOKIE, "/", secure, domain),
        clear_cookie_header(REFRESH_COOKIE, "/api/auth", secure, domain),
    ] {
        if let Ok(header_value) = HeaderValue::from_str(&value) {
            res.headers_mut().append(header::SET_COOKIE, header_value);
        }
    }
}

/// CSRF 防护（所有改变状态的请求）
///
/// 两道闸门，任一满足即可：
///   1. 带 `Origin` 时必须命中白名单（浏览器同源请求一定会带）；
///   2. 不带 `Origin` 时要求 `Content-Type: application/json`
///      —— 跨站表单只能发 `application/x-www-form-urlencoded` / `multipart/form-data`，
///      发不出 JSON，因此这一条足以挡住表单型 CSRF。
///
/// 说明：这不是防「同源内的 XSS」，那是 HttpOnly + 输入转义的职责。
pub async fn csrf_guard(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    if method != Method::GET && method != Method::HEAD && method != Method::OPTIONS {
        let headers = req.headers();
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        match origin {
            Some(origin) => {
                if !state.cfg.allowed_origins.iter().any(|allowed| allowed == origin) {
                    tracing::warn!(origin, "Origin 不在白名单，拒绝该请求");
                    return AuthError::Forbidden.into_response();
                }
            }
            None => {
                let content_type = headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !content_type.starts_with("application/json") {
                    tracing::warn!(content_type, "缺少 Origin 且不是 JSON 请求，拒绝");
                    return AuthError::Forbidden.into_response();
                }
            }
        }
    }
    next.run(req).await
}

/// 客户端 IP：优先取代理链里的第一个（dev-server / Nginx 都会带上）
pub fn client_ip(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    if let Some(real) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if !real.trim().is_empty() {
            return real.trim().to_string();
        }
    }
    "unknown".to_string()
}

/// User-Agent（截断后使用）
pub fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| crate::core::validate::truncate(s, 300))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_cookie(raw: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(raw).unwrap());
        headers
    }

    #[test]
    fn parses_cookie_by_name() {
        let headers = headers_with_cookie("gx_access=abc.def.ghi; gx_refresh=xyz; other=1");
        assert_eq!(cookie(&headers, ACCESS_COOKIE).as_deref(), Some("abc.def.ghi"));
        assert_eq!(cookie(&headers, REFRESH_COOKIE).as_deref(), Some("xyz"));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn cookie_header_has_security_attributes() {
        let header = cookie_header(ACCESS_COOKIE, "v", "/", 900, false, None);
        assert!(header.contains("Path=/"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Lax"));
        assert!(!header.contains("Secure"), "本地 http 下不能带 Secure");

        let secure_header = cookie_header(REFRESH_COOKIE, "v", "/api/auth", 100, true, Some("a.com"));
        assert!(secure_header.contains("Secure"));
        assert!(secure_header.contains("Domain=a.com"));
        assert!(secure_header.contains("Path=/api/auth"));
    }

    #[test]
    fn cleared_cookie_expires_immediately() {
        let header = clear_cookie_header(ACCESS_COOKIE, "/", false, None);
        assert!(header.contains("Max-Age=0"));
    }

    #[test]
    fn client_ip_prefers_forwarded_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9, 10.0.0.1"));
        assert_eq!(client_ip(&headers), "203.0.113.9");
        let mut headers2 = HeaderMap::new();
        headers2.insert("x-real-ip", HeaderValue::from_static("198.51.100.7"));
        assert_eq!(client_ip(&headers2), "198.51.100.7");
        assert_eq!(client_ip(&HeaderMap::new()), "unknown");
    }
}
