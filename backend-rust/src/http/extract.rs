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

/// 管理员（在 `AuthUser` 之上加一条 `role == "admin"` 的最小准入）。
///
/// ⚠️ 这是**邀请码管理接口的准入**，不是权限系统本身：需求里「权限等级暂时仅预留」
/// 指的是不做 RBAC、权限点、用户管理后台；`users.role` / `users.status` 只存不判。
/// 若哪天要实现完整权限系统，替换点就是这里。
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role != "admin" {
            tracing::warn!(user_id = user.user_id, role = %user.role, "非管理员访问管理接口");
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}
