// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 密码哈希：Argon2id（PHC 字符串）
//!
//! 要点：
//!   - 参数（内存/迭代/并行度）**写进哈希本身**，所以日后调整参数不会让老哈希失效；
//!   - 每个密码用独立随机盐（16 字节）；
//!   - 校验走 `password_hash` 的比对实现（恒定时间）；
//!   - 提供 `dummy_verify`：账号不存在时也做一次同参数哈希，让响应时间与
//!     「账号存在但密码错」接近，避免通过响应耗时枚举邮箱。

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;

use crate::config::Argon2Config;
use crate::error::{AuthError, Result};

/// 盐长度（字节）
const SALT_LEN: usize = 16;
/// 生成假哈希用的密码（只为耗时，不代表任何真实账号）
const DUMMY_PASSWORD: &str = "dummy-password-for-timing-only-0";

pub struct PasswordCodec {
    params: Params,
    /// 启动时算一次的假哈希，用于不存在账号的等时校验
    dummy_hash: String,
}

impl PasswordCodec {
    pub fn new(cfg: Argon2Config) -> Result<Self> {
        let params = Params::new(cfg.m_cost, cfg.t_cost, cfg.p_cost, None)
            .map_err(|e| AuthError::Internal(format!("Argon2 参数不合法: {e}")))?;
        let codec = Self { params, dummy_hash: String::new() };
        let dummy_hash = codec.hash(DUMMY_PASSWORD)?;
        Ok(Self { params: codec.params, dummy_hash })
    }

    fn hasher(&self) -> Argon2<'_> {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params.clone())
    }

    /// 生成 PHC 字符串（形如 `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`）
    pub fn hash(&self, password: &str) -> Result<String> {
        let mut salt_bytes = [0u8; SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt_bytes);
        let salt = SaltString::encode_b64(&salt_bytes)
            .map_err(|e| AuthError::Internal(format!("生成盐失败: {e}")))?;
        self.hasher()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AuthError::Internal(format!("计算密码哈希失败: {e}")))
    }

    /// 校验密码；解析失败（库里的哈希被写坏）一律按「不匹配」处理
    pub fn verify(&self, password: &str, phc: &str) -> bool {
        let Ok(parsed) = PasswordHash::new(phc) else {
            tracing::error!("库中的密码哈希无法解析，按不匹配处理");
            return false;
        };
        self.hasher().verify_password(password.as_bytes(), &parsed).is_ok()
    }

    /// 等时校验：账号不存在时调用，消耗与真实校验相当的时间
    pub fn dummy_verify(&self, password: &str) {
        let _ = self.verify(password, &self.dummy_hash);
    }
}

impl std::fmt::Debug for PasswordCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 刻意不输出哈希内容
        f.debug_struct("PasswordCodec").field("params", &self.params).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> PasswordCodec {
        // 测试用低参数：只要行为一致，哈希强度不是单元测试的关心点
        PasswordCodec::new(Argon2Config { m_cost: 8, t_cost: 1, p_cost: 1 }).unwrap()
    }

    #[test]
    fn hash_is_argon2id_phc_and_verifies() {
        let codec = fast();
        let phc = codec.hash("7289HR_RedSun").unwrap();
        assert!(phc.starts_with("$argon2id$v=19$"), "PHC 前缀不对：{phc}");
        assert!(codec.verify("7289HR_RedSun", &phc));
        assert!(!codec.verify("wrong-password-1", &phc));
    }

    #[test]
    fn same_password_gets_different_salt_each_time() {
        let codec = fast();
        let a = codec.hash("abc12345").unwrap();
        let b = codec.hash("abc12345").unwrap();
        assert_ne!(a, b, "每次都要用新的随机盐");
        assert!(codec.verify("abc12345", &a) && codec.verify("abc12345", &b));
    }

    #[test]
    fn verify_accepts_unicode_and_long_passwords() {
        let codec = fast();
        let unicode = "密码Abc123汉字";
        let long = format!("{}{}", "a1".repeat(60), "Z9");
        assert!(codec.verify(unicode, &codec.hash(unicode).unwrap()));
        assert!(codec.verify(&long, &codec.hash(&long).unwrap()));
    }

    #[test]
    fn broken_hash_never_verifies() {
        let codec = fast();
        assert!(!codec.verify("whatever1", "not-a-phc-string"));
        assert!(!codec.verify("whatever1", ""));
    }

    #[test]
    fn params_are_embedded_in_hash() {
        let codec = PasswordCodec::new(Argon2Config { m_cost: 16, t_cost: 3, p_cost: 1 }).unwrap();
        let phc = codec.hash("abc12345").unwrap();
        assert!(phc.contains("m=16,t=3,p=1"), "参数应当写进 PHC：{phc}");
    }
}
