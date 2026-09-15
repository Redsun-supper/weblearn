//! 令牌：access token（HS256 JWT）+ refresh token（不透明随机串）
//!
//! 设计取舍：
//!   - access token 无状态（JWT），但它只是「入场券的一半」：受保护接口还要查
//!     `sessions` 表确认这个会话没被吊销，所以**登出后 access 立刻失效**，
//!     不必等它自然过期（15 分钟）。
//!   - refresh token 是 32 字节随机串，库里只存 SHA-256 摘要：即使库被读走，
//!     也无法直接拿来登录。
//!   - ⚠️ 过期判定用**注入的时钟**（`verify_access(now)`）而不是 jsonwebtoken 的
//!     内部系统时间，否则「令牌过期」这条分支在测试里无法精确构造。

use std::time::Duration;

use base64::Engine as _;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::error::{AuthError, Result};

/// access token 的载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessClaims {
    /// 用户 id
    pub sub: i64,
    /// 会话 id（用它去查 sessions 表，实现即时吊销）
    pub sid: i64,
    /// 角色快照（权限系统本身尚未实现，这里只带上以便后续使用）
    pub role: String,
    pub iat: i64,
    pub exp: i64,
    /// 令牌唯一 id（便于排障与将来的黑名单方案）
    pub jti: String,
}

/// 允许的时钟偏移（秒）
const LEEWAY_SECONDS: i64 = 60;

#[derive(Debug, Clone)]
pub struct TokenIssuer {
    secret: Vec<u8>,
    access_ttl_seconds: i64,
}

impl TokenIssuer {
    pub fn new(secret: Vec<u8>, access_ttl: Duration) -> Self {
        Self { secret, access_ttl_seconds: access_ttl.as_secs() as i64 }
    }

    pub fn access_ttl_seconds(&self) -> i64 {
        self.access_ttl_seconds
    }

    /// 签发 access token
    pub fn issue_access(
        &self,
        user_id: i64,
        session_id: i64,
        role: &str,
        now: OffsetDateTime,
    ) -> Result<String> {
        let iat = now.unix_timestamp();
        let claims = AccessClaims {
            sub: user_id,
            sid: session_id,
            role: role.to_string(),
            iat,
            exp: iat + self.access_ttl_seconds,
            jti: uuid::Uuid::new_v4().to_string(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(&self.secret))
            .map_err(|e| AuthError::Internal(format!("签发 access token 失败: {e}")))
    }

    /// 校验签名与过期；任何问题都统一返回 `Unauthenticated`（不区分原因，避免给攻击者线索）
    pub fn verify_access(&self, token: &str, now: OffsetDateTime) -> Result<AccessClaims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false; // 过期由下面的注入时钟判定
        validation.required_spec_claims.clear();
        let data = decode::<AccessClaims>(token, &DecodingKey::from_secret(&self.secret), &validation)
            .map_err(|_| AuthError::Unauthenticated)?;
        let claims = data.claims;
        if claims.exp + LEEWAY_SECONDS < now.unix_timestamp() {
            return Err(AuthError::Unauthenticated);
        }
        Ok(claims)
    }

    /// 生成一枚 refresh token，返回 `(明文, 摘要)`；**明文只回给客户端 Cookie**
    pub fn new_refresh_token(&self) -> (String, String) {
        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let plain = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
        let hash = sha256_hex(plain.as_bytes());
        (plain, hash)
    }

    /// 计算 refresh token 的摘要（用于按摘要查会话）
    pub fn hash_refresh(&self, plain: &str) -> String {
        sha256_hex(plain.as_bytes())
    }
}

/// SHA-256 的十六进制小写表示
pub fn sha256_hex(input: &[u8]) -> String {
    hex::encode(Sha256::digest(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issuer() -> TokenIssuer {
        TokenIssuer::new(b"unit-test-secret".to_vec(), Duration::from_secs(900))
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_760_000_000).unwrap()
    }

    #[test]
    fn access_token_round_trips_claims() {
        let issuer = issuer();
        let token = issuer.issue_access(7, 42, "admin", now()).unwrap();
        let claims = issuer.verify_access(&token, now()).unwrap();
        assert_eq!(claims.sub, 7);
        assert_eq!(claims.sid, 42);
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.exp, claims.iat + 900);
    }

    #[test]
    fn expired_token_is_rejected_by_injected_clock() {
        let issuer = issuer();
        let token = issuer.issue_access(1, 1, "user", now()).unwrap();
        // 刚过期：还在 60 秒 leeway 内，仍然可用
        let just_expired = now() + time::Duration::seconds(900 + 30);
        assert!(issuer.verify_access(&token, just_expired).is_ok());
        // 超出 leeway：拒绝
        let long_expired = now() + time::Duration::seconds(900 + 61);
        assert!(matches!(issuer.verify_access(&token, long_expired), Err(AuthError::Unauthenticated)));
    }

    #[test]
    fn token_signed_with_other_secret_is_rejected() {
        let other = TokenIssuer::new(b"another-secret".to_vec(), Duration::from_secs(900));
        let token = other.issue_access(1, 1, "user", now()).unwrap();
        assert!(matches!(issuer().verify_access(&token, now()), Err(AuthError::Unauthenticated)));
    }

    #[test]
    fn tampered_token_is_rejected() {
        let issuer = issuer();
        let token = issuer.issue_access(1, 2, "user", now()).unwrap();
        let mut tampered = token.clone();
        // 改动载荷里的一个字符（签名随即不匹配）
        let last_dot = tampered.rfind('.').unwrap();
        tampered.replace_range(last_dot - 1..last_dot, "A");
        assert!(matches!(issuer.verify_access(&tampered, now()), Err(AuthError::Unauthenticated)));
        assert!(matches!(issuer.verify_access("garbage", now()), Err(AuthError::Unauthenticated)));
    }

    #[test]
    fn refresh_tokens_are_unique_and_hashable() {
        let issuer = issuer();
        let (a, hash_a) = issuer.new_refresh_token();
        let (b, hash_b) = issuer.new_refresh_token();
        assert_ne!(a, b);
        assert_ne!(hash_a, hash_b);
        assert_eq!(issuer.hash_refresh(&a), hash_a);
        assert_eq!(hash_a.len(), 64, "SHA-256 十六进制应为 64 字符");
    }
}
