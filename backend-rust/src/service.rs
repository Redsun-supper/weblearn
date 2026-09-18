//! 服务层：账号系统的全部业务规则（HTTP 与 CLI 共用同一套实现）
//!
//! 分层约定：
//!   - 慢操作（Argon2 哈希/校验、发信）**一律在数据库锁之外**完成；
//!   - 需要原子的动作（占邀请码 + 校验验证码 + 建用户 + 建会话）放进一个事务；
//!   - 每个状态变更都写审计日志（`audit_logs`），但**绝不写密码与验证码明文**。

use std::sync::Arc;

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::config::Config;
use crate::core::email_code::{self, CodeCheck};
use crate::core::invite::{self, InviteState};
use crate::core::password::PasswordCodec;
use crate::core::token::TokenIssuer;
use crate::core::validate;
use crate::error::{AuthError, Result};
use crate::mail::Mailer;
use crate::models::{
    InvitePublic, InviteRow, NewAudit, SessionPublic, SessionRow, UserPublic, UserRow, PURPOSE_REGISTER,
};
use crate::rate_limit::RateLimiter;
use crate::store::sql::{self, InviteFilter};
use crate::store::SqliteStore;

/// 事务内的两种结局
///
/// 为什么需要它：`store::write` 的约定是「闭包返回 `Err` 就回滚」。
/// 但有些「业务拒绝」**必须提交**——例如验证码试错次数、登录失败次数，
/// 回滚掉就等于没有记账：攻击者可以无限猜验证码。
/// 所以这些分支返回 `Reject`（事务照常提交），只有真正的数据库故障才回滚。
enum TxOutcome<T> {
    Commit(T),
    Reject(AuthError),
}

/// 查邀请码：先按**原样**查（`-` 是邀请码自身格式的一部分，后台将来靠它区分用途），
/// 查不到再按「去掉全部 `-`」查一次 —— 这样管理员手抄成
/// `XXXX-XXXX-XXXX-XXXX` 的老习惯仍然能用。
fn find_invite(conn: &rusqlite::Connection, code: &str) -> crate::store::StoreResult<Option<InviteRow>> {
    if let Some(inv) = sql::find_invite_by_code(conn, code)? {
        return Ok(Some(inv));
    }
    let stripped = code.replace('-', "");
    if stripped != code {
        return sql::find_invite_by_code(conn, &stripped);
    }
    Ok(None)
}

/// 一次成功的登录/注册/刷新所返回的东西
#[derive(Debug, Clone)]
pub struct AuthOutcome {
    pub user: UserPublic,
    pub access_token: String,
    pub refresh_token: String,
    pub device_label: String,
    pub access_expires_in: i64,
    pub refresh_expires_in: i64,
}

/// 验证码发送结果
#[derive(Debug, Clone)]
pub struct CodeSent {
    pub email: String,
    pub expires_in: i64,
}

/// 已鉴权的调用者（从 access cookie 解析出来）
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i64,
    pub session_id: i64,
    pub role: String,
    pub email: String,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RegisterInput {
    pub email: String,
    pub email_code: String,
    /// 邀请码（可选，多个用空格分隔）：空 = 普通用户；非空 = 兑换成功即升级为管理员
    pub invite_code: String,
    pub password: String,
    pub username: Option<String>,
    pub device_label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
    pub device_label: Option<String>,
}

/// 邀请码创建结果
#[derive(Debug, Clone)]
pub struct InviteCreated {
    pub codes: Vec<InvitePublic>,
}

pub struct AuthService {
    pub cfg: Arc<Config>,
    pub store: Arc<SqliteStore>,
    pub clock: Arc<dyn Clock>,
    pub mailer: Arc<dyn Mailer>,
    pub limiter: Arc<RateLimiter>,
    password: PasswordCodec,
    tokens: TokenIssuer,
}

impl AuthService {
    pub fn new(
        cfg: Arc<Config>,
        store: Arc<SqliteStore>,
        clock: Arc<dyn Clock>,
        mailer: Arc<dyn Mailer>,
        limiter: Arc<RateLimiter>,
    ) -> Result<Self> {
        let password = PasswordCodec::new(cfg.argon2)?;
        let tokens = TokenIssuer::new(cfg.jwt_secret.clone(), cfg.access_ttl);
        Ok(Self { cfg, store, clock, mailer, limiter, password, tokens })
    }

    // ------------------------------------------------------------ 内部小工具

    fn refresh_ttl_seconds(&self) -> i64 {
        self.cfg.refresh_ttl.as_secs() as i64
    }

    /// 只读查询（错误类型固定为 AuthError，业务代码不必写类型标注）
    async fn read<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.store.read::<T, AuthError, F>(f).await
    }

    /// 事务写
    async fn write<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.store.write::<T, AuthError, F>(f).await
    }

    fn finish_auth(
        &self,
        user: &UserRow,
        session_id: i64,
        refresh_token: String,
        device_label: String,
        now: OffsetDateTime,
    ) -> Result<AuthOutcome> {
        let access_token = self.tokens.issue_access(user.id, session_id, &user.role, now)?;
        Ok(AuthOutcome {
            user: user.public(),
            access_token,
            refresh_token,
            device_label,
            access_expires_in: self.tokens.access_ttl_seconds(),
            refresh_expires_in: self.refresh_ttl_seconds(),
        })
    }

    async fn audit_fail(&self, actor: Option<i64>, email: &str, ip: &str, ua: &str, action: &str, detail: &str) {
        let now = self.clock.now();
        let entry = NewAudit::new(action, now)
            .actor(actor)
            .target(email)
            .client(ip, ua)
            .detail(detail);
        let action_owned = entry.action.clone();
        let detail_owned = entry.detail.clone();
        let res = self
            .write(move |conn| {
                sql::insert_audit(conn, &entry)?;
                Ok(())
            })
            .await;
        if let Err(e) = res {
            // 审计写失败不应该影响主流程，但要留痕
            tracing::error!(action = %action_owned, detail = %detail_owned, error = ?e, "写审计日志失败");
        }
    }

    // ------------------------------------------------------------ 邮箱验证码

    /// 发送注册验证码。
    ///
    /// 邀请码是**可选**的：不填就是开放注册（邮箱验证码是唯一门槛），
    /// 填了就逐个校验，让用户在这一步就拿到「码不对」的反馈，而不是等到注册才失败。
    pub async fn request_email_code(&self, email_raw: &str, invite_code_raw: &str, ip: &str) -> Result<CodeSent> {
        let email = validate::normalize_email(email_raw)
            .ok_or_else(|| AuthError::InvalidParams("邮箱格式不正确".into()))?;
        let codes = invite::split_codes(invite_code_raw);
        for code in &codes {
            if !invite::is_plausible(code) {
                return Err(AuthError::InvalidInvite);
            }
        }

        // 限流：同邮箱 1 次/分钟、5 次/小时；同 IP 20 次/小时
        self.limiter.check(
            &format!("code:email:{email}"),
            &[self.cfg.rate.code_email_minute, self.cfg.rate.code_email_hour],
        )?;
        self.limiter.check(&format!("code:ip:{ip}"), &[self.cfg.rate.code_ip])?;

        let now = self.clock.now();
        let email_q = email.clone();
        let codes_q = codes.clone();
        let registered = self
            .read(move |conn| {
                for code in &codes_q {
                    let inv = find_invite(conn, code)?.ok_or(AuthError::InvalidInvite)?;
                    match invite::evaluate(inv.disabled, inv.used_count, inv.max_uses, inv.expires_at, now) {
                        InviteState::Usable => {}
                        InviteState::Disabled => return Err(AuthError::InvalidInvite),
                        InviteState::Expired => return Err(AuthError::InviteExpired),
                        InviteState::Exhausted => return Err(AuthError::InviteExhausted),
                    }
                }
                Ok(sql::find_user_by_email(conn, &email_q)?.is_some())
            })
            .await?;
        if registered {
            return Err(AuthError::EmailTaken);
        }

        let code = email_code::generate_code();
        let code_hash = email_code::hash_code(&self.cfg.code_pepper, &email, &code);
        let expires_at = now + time::Duration::seconds(self.cfg.code_ttl.as_secs() as i64);
        let email_w = email.clone();
        let hash_w = code_hash.clone();
        let ip_w = ip.to_string();
        let ip_w2 = ip_w.clone();
        let code_id = self
            .write(move |conn| {
                // 同一邮箱同一用途只保留一个有效码
                sql::consume_active_email_codes(conn, &email_w, PURPOSE_REGISTER, now)?;
                let id = sql::insert_email_code(conn, &email_w, PURPOSE_REGISTER, &hash_w, expires_at, &ip_w, now)?;
                audit(now, conn, "email_code_sent", None, &email_w, &ip_w, "", "注册验证码").map(|_| ())?;
                Ok(id)
            })
            .await?;

        let ttl_seconds = self.cfg.code_ttl.as_secs() as i64;
        if let Err(e) = self.mailer.send_code(&email, &code, PURPOSE_REGISTER, ttl_seconds).await {
            // 发信失败：把刚写下的码作废，避免用户拿不到码却要等一分钟重发
            let email_r = email.clone();
            let _ = self
                .write(move |conn| {
                    sql::consume_email_code(conn, code_id, now)?;
                    audit(now, conn, "email_code_send_failed", None, &email_r, &ip_w2, "", "邮件发送失败").map(|_| ())
                })
                .await;
            return Err(e);
        }

        Ok(CodeSent { email, expires_in: ttl_seconds })
    }

    /// 仅开发模式：取最近一次发给该邮箱的验证码明文
    pub fn dev_code(&self, email_raw: &str) -> Option<String> {
        let email = validate::normalize_email(email_raw)?;
        self.mailer.last_dev_code(&email)
    }

    // ------------------------------------------------------------ 注册

    pub async fn register(&self, input: RegisterInput, ip: &str, ua: &str) -> Result<AuthOutcome> {
        let email = validate::normalize_email(&input.email)
            .ok_or_else(|| AuthError::InvalidParams("邮箱格式不正确".into()))?;
        validate::validate_password(&input.password).map_err(AuthError::InvalidParams)?;
        if !email_code::is_well_formed(&input.email_code) {
            return Err(AuthError::InvalidParams("验证码应为 6 位数字".into()));
        }
        // 邀请码**可选**：不填 = 普通用户（开放注册）；填了就逐个校验，
        // 注册成功后把账号升级成管理员（将来还可以按 `-` 前缀区分成积分 / 礼物等用途）。
        let codes = invite::split_codes(&input.invite_code);
        for code in &codes {
            if !invite::is_plausible(code) {
                return Err(AuthError::InvalidInvite);
            }
        }
        self.limiter.check(&format!("register:ip:{ip}"), &[self.cfg.rate.register_ip])?;

        let now = self.clock.now();
        // 慢操作放在事务外（Argon2 哈希不能占着数据库锁）
        let password_hash = self.password.hash(&input.password)?;

        let username = validate::normalize_username(input.username.as_deref());
        let device = validate::device_label(input.device_label.as_deref(), ua);
        let pepper = self.cfg.code_pepper.clone();
        let max_attempts = self.cfg.code_max_attempts;
        let tokens = self.tokens.clone();
        let ttl = self.refresh_ttl_seconds();

        let email_w = email.clone();
        let codes_w = codes.clone();
        let code_w = input.email_code.clone();
        let device_w = device.clone();
        let ua_w = ua.to_string();
        let ip_w = ip.to_string();

        let (user_id, session_id, refresh_token) = match self
            .write(move |conn| -> Result<TxOutcome<(i64, i64, String)>> {
                // ① 邀请码（可选，可多张）：逐个校验，全部有效才继续。
                //    放在验证码之前是有意的——填错了码要优先报「邀请码无效」。
                let mut redeemed: Vec<InviteRow> = Vec::new();
                for code in &codes_w {
                    let Some(inv) = find_invite(conn, code)? else {
                        return Ok(TxOutcome::Reject(AuthError::InvalidInvite));
                    };
                    match invite::evaluate(inv.disabled, inv.used_count, inv.max_uses, inv.expires_at, now) {
                        InviteState::Usable => {}
                        InviteState::Disabled => return Ok(TxOutcome::Reject(AuthError::InvalidInvite)),
                        InviteState::Expired => return Ok(TxOutcome::Reject(AuthError::InviteExpired)),
                        InviteState::Exhausted => return Ok(TxOutcome::Reject(AuthError::InviteExhausted)),
                    }
                    redeemed.push(inv);
                }

                // ② 邮箱验证码（错误一次就累加尝试次数）
                let Some(row) = sql::find_active_email_code(conn, &email_w, PURPOSE_REGISTER)? else {
                    return Ok(TxOutcome::Reject(AuthError::InvalidCode));
                };
                let matched = email_code::verify_hash(&pepper, &email_w, &code_w, &row.code_hash);
                match email_code::evaluate(row.attempts, max_attempts, row.expires_at, now, matched) {
                    CodeCheck::Ok => sql::consume_email_code(conn, row.id, now)?,
                    CodeCheck::Expired => return Ok(TxOutcome::Reject(AuthError::CodeExpired)),
                    CodeCheck::AttemptsExceeded => {
                        return Ok(TxOutcome::Reject(AuthError::CodeAttemptsExceeded))
                    }
                    CodeCheck::Mismatch => {
                        // ⚠️ 这一笔必须提交（见 TxOutcome 的说明）：
                        // 回滚掉的话「最多尝试 5 次」就形同虚设，可以无限猜验证码。
                        sql::bump_email_code_attempts(conn, row.id)?;
                        return Ok(TxOutcome::Reject(AuthError::InvalidCode));
                    }
                }

                // ③ 邮箱唯一（UNIQUE 索引兜底）
                if sql::find_user_by_email(conn, &email_w)?.is_some() {
                    return Ok(TxOutcome::Reject(AuthError::EmailTaken));
                }

                // ④ 逐个占邀请码额度（条件更新，返回值 0 说明刚好被别人用掉了）。
                //    ⚠️ 这一步排在验证码之后：这里的 Reject 是要**提交**的（验证码已经被正确用掉，
                //    烧掉它是应该的）。代价是极端并发下若第 N 张券刚好被抢光，前面几张的核销
                //    会留在账上——这是可接受的账目偏差，换取的是失败语义与原来完全一致。
                for inv in &redeemed {
                    if sql::consume_invite(conn, inv.id, now)? == 0 {
                        return Ok(TxOutcome::Reject(AuthError::InviteExhausted));
                    }
                }

                // ⑤ 建用户 + 登录标识（多方式登录的预留位）
                //    带邀请码 → 管理员（兑换成功的标志）；不带 → 普通用户
                let role = if redeemed.is_empty() { "user" } else { "admin" };
                let user_id =
                    sql::insert_user(conn, &email_w, username.as_deref(), &password_hash, role, Some(now), now)?;
                sql::insert_identity(conn, user_id, "email", &email_w, Some(now), now)?;
                for inv in &redeemed {
                    sql::insert_invite_use(conn, inv.id, Some(user_id), &email_w, &ip_w, now)?;
                    audit(now, conn, "invite_use", Some(user_id), &inv.code, &ip_w, &ua_w, "邀请码兑换")?;
                }
                let register_note = if redeemed.is_empty() { "邮箱注册" } else { "邮箱注册（邀请码升级为管理员）" };
                audit(now, conn, "register", Some(user_id), &email_w, &ip_w, &ua_w, register_note)?;

                // ⑥ 注册即登录：新建一个会话
                let family = uuid::Uuid::new_v4().to_string();
                let (refresh_plain, refresh_hash) = tokens.new_refresh_token();
                let session_id = sql::insert_session(
                    conn,
                    user_id,
                    &family,
                    &refresh_hash,
                    &device_w,
                    &ua_w,
                    &ip_w,
                    now,
                    now + time::Duration::seconds(ttl),
                )?;
                Ok(TxOutcome::Commit((user_id, session_id, refresh_plain)))
            })
            .await?
        {
            TxOutcome::Commit(value) => value,
            TxOutcome::Reject(error) => return Err(error),
        };

        let user = self.load_user(user_id).await?;
        self.finish_auth(&user, session_id, refresh_token, device, now)
    }

    // ------------------------------------------------------------ 登录

    pub async fn login(&self, input: LoginInput, ip: &str, ua: &str) -> Result<AuthOutcome> {
        let email = validate::normalize_email(&input.email)
            .ok_or_else(|| AuthError::InvalidParams("邮箱格式不正确".into()))?;
        self.limiter.check(&format!("login:ip:{ip}"), &[self.cfg.rate.login_ip])?;

        let now = self.clock.now();
        let email_q = email.clone();
        let found = self
            .read(move |conn| Ok(sql::find_user_by_identity(conn, "email", &email_q)?))
            .await?;

        let Some(user) = found else {
            // 账号不存在也走一次等时校验，避免用响应时间枚举邮箱
            self.password.dummy_verify(&input.password);
            self.audit_fail(None, &email, ip, ua, "login_fail", "账号不存在").await;
            return Err(AuthError::BadCredentials);
        };

        if !user.is_active() {
            self.audit_fail(Some(user.id), &email, ip, ua, "login_fail", "账号已停用").await;
            return Err(AuthError::Forbidden);
        }
        if user.is_locked(now) {
            self.audit_fail(Some(user.id), &email, ip, ua, "login_fail", "账号锁定中").await;
            return Err(AuthError::AccountLocked);
        }

        // Argon2 校验在数据库锁之外
        if !self.password.verify(&input.password, &user.password_hash) {
            let user_id = user.id;
            let threshold = self.cfg.lock_threshold;
            let minutes = self.cfg.lock_minutes;
            let (attempts, locked) = self
                .write(move |conn| Ok(sql::record_login_failure(conn, user_id, threshold, minutes, now)?))
                .await?;
            let detail = match locked {
                Some(_) => format!("密码不正确（第 {attempts} 次，已锁定）"),
                None => format!("密码不正确（第 {attempts} 次）"),
            };
            self.audit_fail(Some(user.id), &email, ip, ua, "login_fail", &detail).await;
            return Err(if locked.is_some() { AuthError::AccountLocked } else { AuthError::BadCredentials });
        }

        let device = validate::device_label(input.device_label.as_deref(), ua);
        let tokens = self.tokens.clone();
        let ttl = self.refresh_ttl_seconds();
        let user_id = user.id;
        let email_w = email.clone();
        let device_w = device.clone();
        let ua_w = ua.to_string();
        let ip_w = ip.to_string();

        let (session_id, refresh_token) = self
            .write(move |conn| -> Result<(i64, String)> {
                sql::update_login_success(conn, user_id, now)?;
                // 每次登录都新建一个会话：多端并存、互不影响
                let family = uuid::Uuid::new_v4().to_string();
                let (plain, hash) = tokens.new_refresh_token();
                let sid = sql::insert_session(
                    conn,
                    user_id,
                    &family,
                    &hash,
                    &device_w,
                    &ua_w,
                    &ip_w,
                    now,
                    now + time::Duration::seconds(ttl),
                )?;
                audit(now, conn, "login_ok", Some(user_id), &email_w, &ip_w, &ua_w, &device_w)?;
                Ok((sid, plain))
            })
            .await?;

        self.finish_auth(&user, session_id, refresh_token, device, now)
    }

    // ------------------------------------------------------------ 刷新 / 登出

    /// 轮换 refresh token：旧的作废、发新的；命中「已经轮换过」的令牌即判定重放，整族吊销
    pub async fn refresh(&self, refresh_plain: &str, ip: &str, ua: &str) -> Result<AuthOutcome> {
        let now = self.clock.now();
        let hash = self.tokens.hash_refresh(refresh_plain);
        let found = self
            .read(move |conn| Ok(sql::find_session_by_refresh_hash(conn, &hash)?))
            .await?;
        let session = found.ok_or(AuthError::Unauthenticated)?;

        if session.is_revoked() {
            // 重放检测：一枚已经用过的 refresh 又出现了
            let family = session.family_id.clone();
            let ua_w = ua.to_string();
            let ip_w = ip.to_string();
            let n = self
                .write(move |conn| {
                    let n = sql::revoke_family(conn, &family, "refresh_replay", now)?;
                    audit(now, conn, "refresh_replay", None, &family, &ip_w, &ua_w, &format!("吊销 {n} 个会话"))?;
                    Ok(n)
                })
                .await?;
            tracing::warn!(family = %session.family_id, revoked = n, "检测到 refresh 令牌重放，已吊销整条轮换链");
            return Err(AuthError::Unauthenticated);
        }

        if session.expires_at <= now {
            let sid = session.id;
            let _ = self
                .write(move |conn| Ok(sql::revoke_session(conn, sid, "expired", now)?))
                .await;
            return Err(AuthError::Unauthenticated);
        }

        let user = self.load_user(session.user_id).await?;
        if !user.is_active() {
            let sid = session.id;
            let _ = self
                .write(move |conn| Ok(sql::revoke_session(conn, sid, "user_disabled", now)?))
                .await;
            return Err(AuthError::Forbidden);
        }

        let device = if session.device_label.is_empty() {
            validate::device_label(None, ua)
        } else {
            session.device_label.clone()
        };
        let tokens = self.tokens.clone();
        let ttl = self.refresh_ttl_seconds();
        let user_id = user.id;
        let family = session.family_id.clone();
        let old_id = session.id;
        let device_w = device.clone();
        let ua_w = ua.to_string();
        let ip_w = ip.to_string();

        let (session_id, refresh_token) = self
            .write(move |conn| -> Result<(i64, String)> {
                let (plain, hash) = tokens.new_refresh_token();
                let new_id = sql::insert_session(
                    conn,
                    user_id,
                    &family,
                    &hash,
                    &device_w,
                    &ua_w,
                    &ip_w,
                    now,
                    now + time::Duration::seconds(ttl),
                )?;
                sql::mark_session_rotated(conn, old_id, new_id, now)?;
                audit(now, conn, "refresh", Some(user_id), &device_w, &ip_w, &ua_w, "")?;
                Ok((new_id, plain))
            })
            .await?;

        self.finish_auth(&user, session_id, refresh_token, device, now)
    }

    /// 登出：只吊销这一个端的会话（幂等：没有会话也当成功）
    pub async fn logout(&self, access_token: Option<&str>, refresh_token: Option<&str>) -> Result<()> {
        let now = self.clock.now();

        // 优先用 access 里的 sid；access 过期/缺失时退回 refresh 摘要
        let mut session_id = None;
        if let Some(token) = access_token {
            if let Ok(claims) = self.tokens.verify_access(token, now) {
                session_id = Some(claims.sid);
            }
        }
        if session_id.is_none() {
            if let Some(plain) = refresh_token {
                let hash = self.tokens.hash_refresh(plain);
                let found = self
                    .read(move |conn| Ok(sql::find_session_by_refresh_hash(conn, &hash)?))
                    .await?;
                session_id = found.map(|s| s.id);
            }
        }

        let Some(sid) = session_id else {
            return Ok(());
        };
        self.write(move |conn| {
            sql::revoke_session(conn, sid, "logout", now)?;
            audit(now, conn, "logout", None, &sid.to_string(), "", "", "")?;
            Ok(())
        })
        .await
    }

    /// 吊销当前用户的全部会话（`keep_current` 为 true 时保留当前这一个端）
    pub async fn logout_all(&self, user_id: i64, current_session_id: i64, keep_current: bool) -> Result<usize> {
        let now = self.clock.now();
        let keep = if keep_current { Some(current_session_id) } else { None };
        self.write(move |conn| {
            let n = sql::revoke_user_sessions(conn, user_id, "logout_all", now, keep)?;
            audit(now, conn, "logout_all", Some(user_id), &format!("吊销 {n} 个会话"), "", "", "")?;
            Ok(n)
        })
        .await
    }

    // ------------------------------------------------------------ 鉴权

    /// 每个受保护请求都会走这里：验签 → 查会话（未吊销/未过期）→ 查用户（未停用）
    pub async fn authenticate(&self, access_token: &str) -> Result<AuthUser> {
        let now = self.clock.now();
        let claims = self.tokens.verify_access(access_token, now)?;

        let session = {
            let sid = claims.sid;
            self.read(move |conn| Ok(sql::find_session_by_id(conn, sid)?)).await?
        }
        .ok_or(AuthError::Unauthenticated)?;

        if !session.is_usable(now) || session.user_id != claims.sub {
            return Err(AuthError::Unauthenticated);
        }

        let user = self.load_user(session.user_id).await?;
        if !user.is_active() {
            return Err(AuthError::Forbidden);
        }

        // 记一次活跃时间（每次请求一次轻量写）
        let sid = session.id;
        let _ = self.write(move |conn| Ok(sql::touch_session(conn, sid, now)?)).await;

        Ok(AuthUser {
            user_id: user.id,
            session_id: session.id,
            role: user.role.clone(),
            email: user.email.clone(),
            username: user.username.clone(),
        })
    }

    pub async fn user_public(&self, user_id: i64) -> Result<UserPublic> {
        Ok(self.load_user(user_id).await?.public())
    }

    async fn load_user(&self, user_id: i64) -> Result<UserRow> {
        self.read(move |conn| Ok(sql::find_user_by_id(conn, user_id)?))
            .await?
            .ok_or(AuthError::Unauthenticated)
    }

    // ------------------------------------------------------------ 会话列表

    pub async fn sessions(&self, user_id: i64, current_session_id: i64) -> Result<Vec<SessionPublic>> {
        let now = self.clock.now();
        let rows: Vec<SessionRow> = self
            .read(move |conn| Ok(sql::list_user_sessions(conn, user_id, now)?))
            .await?;
        Ok(rows.iter().map(|s| s.public(current_session_id)).collect())
    }

    // ------------------------------------------------------------ 邀请码（管理）

    /// 批量创建邀请码；明文只在返回值里出现这一次
    pub async fn create_invites(
        &self,
        actor_user_id: Option<i64>,
        count: i64,
        max_uses: i64,
        expires_in_days: i64,
        note: &str,
    ) -> Result<InviteCreated> {
        let count = count.clamp(1, 50);
        let max_uses = max_uses.clamp(1, 1000);
        let now = self.clock.now();
        let expires_at = if expires_in_days <= 0 {
            None
        } else {
            Some(now + time::Duration::days(expires_in_days.min(365)))
        };
        let note = validate::truncate(note.trim(), 100);
        let ip = "";

        let rows: Vec<InviteRow> = self
            .write(move |conn| -> Result<Vec<InviteRow>> {
                let mut out = Vec::new();
                for _ in 0..count {
                    // 极小概率撞码：撞了就换一个，最多试 5 次
                    let mut created = None;
                    for _ in 0..5 {
                        let code = invite::generate_code();
                        match sql::insert_invite(conn, &code, &note, max_uses, expires_at, actor_user_id, now) {
                            Ok(id) => {
                                created = Some(InviteRow {
                                    id,
                                    code,
                                    note: note.clone(),
                                    max_uses,
                                    used_count: 0,
                                    expires_at,
                                    disabled: false,
                                    created_by: actor_user_id,
                                    created_at: now,
                                });
                                break;
                            }
                            Err(crate::error::StoreError::Sql(rusqlite::Error::SqliteFailure(e, _)))
                                if e.code == rusqlite::ErrorCode::ConstraintViolation => continue,
                            Err(e) => return Err(e.into()),
                        }
                    }
                    let row = created.ok_or_else(|| AuthError::Internal("生成邀请码失败（连续撞码）".into()))?;
                    out.push(row);
                }
                let summary = out.iter().map(|r| r.code.clone()).collect::<Vec<_>>().join(",");
                audit(now, conn, "invite_create", actor_user_id, &summary, ip, "", &format!("共 {count} 个"))?;
                Ok(out)
            })
            .await?;

        Ok(InviteCreated { codes: rows.iter().map(|r| r.public(now)).collect() })
    }

    pub async fn list_invites(
        &self,
        filter: InviteFilter,
        page: i64,
        size: i64,
    ) -> Result<(Vec<InvitePublic>, i64)> {
        let page = page.max(1);
        let size = size.clamp(1, 100);
        let offset = (page - 1) * size;
        let now = self.clock.now();
        let (rows, total) = self
            .read(move |conn| {
                let total = sql::count_invites(conn, filter, now)?;
                let rows = sql::list_invites(conn, filter, size, offset, now)?;
                Ok((rows, total))
            })
            .await?;
        Ok((rows.iter().map(|r| r.public(now)).collect(), total))
    }

    pub async fn disable_invite(&self, id: i64) -> Result<()> {
        let now = self.clock.now();
        let n = self
            .write(move |conn| {
                let n = sql::disable_invite(conn, id)?;
                audit(now, conn, "invite_disable", None, &id.to_string(), "", "", "")?;
                Ok(n)
            })
            .await?;
        if n == 0 {
            return Err(AuthError::NotFound);
        }
        Ok(())
    }

    // ------------------------------------------------------------ 管理员

    /// 确保管理员账号存在（幂等：已存在就跳过，绝不覆盖已有密码）
    pub async fn seed_admin(&self, email_raw: &str, password: &str) -> Result<bool> {
        let email = validate::normalize_email(email_raw)
            .ok_or_else(|| AuthError::InvalidParams("管理员邮箱格式不正确".into()))?;
        validate::validate_password(password).map_err(AuthError::InvalidParams)?;

        let existing = {
            let email_q = email.clone();
            self.read(move |conn| Ok(sql::find_user_by_email(conn, &email_q)?)).await?
        };
        if existing.is_some() {
            return Ok(false);
        }

        let now = self.clock.now();
        let hash = self.password.hash(password)?;
        let email_w = email.clone();
        self.write(move |conn| -> Result<bool> {
            if sql::find_user_by_email(conn, &email_w)?.is_some() {
                return Ok(false);
            }
            let user_id =
                sql::insert_user(conn, &email_w, Some("管理员"), &hash, "admin", Some(now), now)?;
            sql::insert_identity(conn, user_id, "email", &email_w, Some(now), now)?;
            audit(now, conn, "admin_seed", Some(user_id), &email_w, "", "", "初始管理员")?;
            Ok(true)
        })
        .await
    }

    /// 改密码（CLI 用；会顺带吊销该用户的全部会话）
    pub async fn set_password(&self, email_raw: &str, password: &str) -> Result<()> {
        let email = validate::normalize_email(email_raw)
            .ok_or_else(|| AuthError::InvalidParams("邮箱格式不正确".into()))?;
        validate::validate_password(password).map_err(AuthError::InvalidParams)?;
        let now = self.clock.now();
        let hash = self.password.hash(password)?;
        let email_w = email.clone();
        self.write(move |conn| -> Result<()> {
            let user = sql::find_user_by_email(conn, &email_w)?.ok_or(AuthError::NotFound)?;
            sql::update_password_hash(conn, user.id, &hash, now)?;
            sql::revoke_user_sessions(conn, user.id, "password_changed", now, None)?;
            audit(now, conn, "password_change", Some(user.id), &email_w, "", "", "CLI 改密")?;
            Ok(())
        })
        .await
    }
}

/// 写一条审计日志（`now` 来自注入的时钟，保证测试可控）
fn audit(
    now: OffsetDateTime,
    conn: &Connection,
    action: &str,
    actor: Option<i64>,
    target: &str,
    ip: &str,
    ua: &str,
    detail: &str,
) -> Result<()> {
    let entry = NewAudit::new(action, now)
        .actor(actor)
        .target(target)
        .client(ip, ua)
        .detail(detail);
    sql::insert_audit(conn, &entry)?;
    Ok(())
}
