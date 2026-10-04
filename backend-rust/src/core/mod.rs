// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 核心纯逻辑层：密码哈希、令牌、邀请码、邮箱验证码、入参校验
//!
//! 这一层**不碰数据库、不碰网络、不读系统时钟**（时间与随机都由调用方注入或
//! 明确使用系统随机源），因此可以用单元测试把边界条件钉死——与 `modules/english/engine`
//! 里「纯计算下沉 Rust 并配单元测试」的做法一致。

pub mod email_code;
pub mod invite;
pub mod password;
pub mod token;
pub mod validate;
