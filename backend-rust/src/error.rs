//! 统一错误类型与 HTTP 映射
//!
//! 响应体沿用 Go 侧的信封格式 `{code, message, data}`（`admin/admin.js` 的
//! `apiFetch` 依赖 `code === 200` 判业务成功），失败时额外带一个机器可读的
//! `error` 字段，方便前端（下一期）区分具体原因。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// 参数不合法（邮箱格式、口令强度、缺失字段……）
    #[error("{0}")]
    InvalidParams(String),
    #[error("邀请码无效")]
    InvalidInvite,
    #[error("邀请码已用尽")]
    InviteExhausted,
    #[error("邀请码已过期")]
    InviteExpired,
    #[error("该邮箱已注册")]
    EmailTaken,
    #[error("验证码不正确")]
    InvalidCode,
    #[error("验证码已过期，请重新获取")]
    CodeExpired,
    #[error("验证码尝试次数过多，请重新获取")]
    CodeAttemptsExceeded,
    #[error("邮箱或密码不正确")]
    BadCredentials,
    #[error("账号已被临时锁定，请稍后再试")]
    AccountLocked,
    #[error("请先登录")]
    Unauthenticated,
    #[error("没有权限")]
    Forbidden,
    #[error("操作过于频繁，请稍后再试")]
    RateLimited,
    #[error("邮件发送失败，请稍后再试")]
    MailFailed,
    #[error("记录不存在")]
    NotFound,
    #[error("服务内部错误")]
    Internal(String),
}

impl AuthError {
    pub fn status(&self) -> StatusCode {
        match self {
            AuthError::InvalidParams(_)
            | AuthError::InvalidInvite
            | AuthError::InviteExhausted
            | AuthError::InviteExpired
            | AuthError::InvalidCode
            | AuthError::CodeExpired
            | AuthError::CodeAttemptsExceeded => StatusCode::BAD_REQUEST,
            AuthError::BadCredentials | AuthError::Unauthenticated => StatusCode::UNAUTHORIZED,
            AuthError::Forbidden => StatusCode::FORBIDDEN,
            AuthError::EmailTaken => StatusCode::CONFLICT,
            AuthError::NotFound => StatusCode::NOT_FOUND,
            AuthError::AccountLocked | AuthError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            AuthError::MailFailed => StatusCode::BAD_GATEWAY,
            AuthError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// 机器可读的错误标识（前端按它做分支，不要去匹配中文文案）
    pub fn code(&self) -> &'static str {
        match self {
            AuthError::InvalidParams(_) => "invalid_params",
            AuthError::InvalidInvite => "invalid_invite",
            AuthError::InviteExhausted => "invite_exhausted",
            AuthError::InviteExpired => "invite_expired",
            AuthError::EmailTaken => "email_taken",
            AuthError::InvalidCode => "invalid_code",
            AuthError::CodeExpired => "code_expired",
            AuthError::CodeAttemptsExceeded => "code_attempts_exceeded",
            AuthError::BadCredentials => "bad_credentials",
            AuthError::AccountLocked => "account_locked",
            AuthError::Unauthenticated => "unauthenticated",
            AuthError::Forbidden => "forbidden",
            AuthError::RateLimited => "rate_limited",
            AuthError::MailFailed => "mail_failed",
            AuthError::NotFound => "not_found",
            AuthError::Internal(_) => "internal",
        }
    }

    /// 对外文案：内部错误不回显细节（细节只进服务端日志）
    pub fn public_message(&self) -> String {
        match self {
            AuthError::Internal(_) => "服务内部错误".to_string(),
            other => other.to_string(),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        if let AuthError::Internal(detail) = &self {
            tracing::error!(error = %detail, "内部错误");
        }
        let status = self.status();
        let body = json!({
            "code": status.as_u16(),
            "message": self.public_message(),
            "error": self.code(),
        });
        (status, Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AuthError>;

/// 存储层错误 → 认证错误
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("数据库任务失败: {0}")]
    Task(String),
    #[error("{0}")]
    Internal(String),
}

impl From<StoreError> for AuthError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::Sql(e) => AuthError::Internal(format!("sqlite: {e}")),
            StoreError::Task(e) => AuthError::Internal(format!("task: {e}")),
            StoreError::Internal(e) => AuthError::Internal(e),
        }
    }
}

/// 让事务闭包里可以直接对 rusqlite 的错误用 `?`
///（否则 `rusqlite::Error → StoreError → AuthError` 两步转换没法自动完成）
impl From<rusqlite::Error> for AuthError {
    fn from(value: rusqlite::Error) -> Self {
        AuthError::Internal(format!("sqlite: {value}"))
    }
}
