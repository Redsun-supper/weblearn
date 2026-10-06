// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 请求提取器：从 Cookie 里解析出已鉴权的用户

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::error::AuthError;
use crate::http::middleware::cookie;
use crate::http::ACCESS_COOKIE;
use crate::service::AuthUser;
use crate::AppState;

/// 已登录用户（未登录直接 401）
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = cookie(&parts.headers, ACCESS_COOKIE).ok_or(AuthError::Unauthenticated)?;
        state.service.authenticate(&token).await
    }
}

/// 管理员（在 `AuthUser` 之上加一条「能进后台」的准入）。
///
/// 放行 `admin` 与 `super_admin`（两者都能进后台、管词条、看数据）。
/// 判定收口在 [`crate::models::can_enter_admin`]，避免同一套字符串散落在各处。
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !crate::models::can_enter_admin(&user.role) {
            tracing::warn!(user_id = user.user_id, role = %user.role, "非管理员访问管理接口");
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}

/// 超级管理员（治理动作：发码、改他人角色、封禁、查审计）。
///
/// ⚠️ 比 [`AdminUser`] **严格更窄**：普通管理员做不了这些事。
/// 权限矩阵见 `docs/launch-plan.md` 第 3 节；判定收口在 [`crate::models::is_super_role`]。
pub struct SuperAdminUser(pub AuthUser);

impl FromRequestParts<AppState> for SuperAdminUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !crate::models::is_super_role(&user.role) {
            tracing::warn!(user_id = user.user_id, role = %user.role, "非超管尝试治理动作");
            return Err(AuthError::Forbidden);
        }
        Ok(SuperAdminUser(user))
    }
}
