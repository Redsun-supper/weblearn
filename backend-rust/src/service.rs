// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 服务层：账号系统的全部业务规则（HTTP 与 CLI 共用同一套实现）
//!
//! 分层约定：
//!   - 慢操作（Argon2 哈希/校验、发信）**一律在数据库锁之外**完成；
//!   - 需要原子的动作（占邀请码 + 校验验证码 + 建用户 + 建会话）放进一个事务；
//!   - 每个状态变更都写审计日志（`audit_logs`），但**绝不写密码与验证码明文**。

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::Connection;
use serde::Serialize;
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
    self, AuditPublic, InvitePublic, InviteRow, InviteUsePublic, InviteUseRow, NewAudit,
    SessionPublic, SessionRow, UserPublic, UserRow, PURPOSE_REGISTER,
};
use crate::rate_limit::RateLimiter;
use crate::store::sql::{self, AuditFilter, InviteFilter};
use crate::store::SqliteStore;

/// 邀请码列表里每张码最多带几条兑换记录（够看清「谁用了」即可，
/// 全量明细是另一个接口的事）
const INVITE_USES_SHOWN: usize = 3;

/// 单次「整批发邮件」最多几封（和发码上限 50 对齐）
const INVITE_MAIL_MAX: usize = 50;

/// 把「邀请码行」与「兑换记录」拼成对外结构：
/// 每张码最多挂 [`INVITE_USES_SHOWN`] 条记录（SQL 已按时间倒序，这里按顺序截取）
fn build_invite_publics(
    rows: Vec<InviteRow>,
    uses: Vec<InviteUseRow>,
    now: OffsetDateTime,
) -> Vec<InvitePublic> {
    let mut grouped: HashMap<i64, Vec<InviteUsePublic>> = HashMap::new();
    for item in uses {
        let bucket = grouped.entry(item.invite_code_id).or_default();
        if bucket.len() < INVITE_USES_SHOWN {
            bucket.push(item.public());
        }
    }
    rows.into_iter()
        .map(|row| {
            let mut public = row.public(now);
            public.uses = grouped.remove(&row.id).unwrap_or_default();
            public
        })
        .collect()
}

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
    /// 这次用的是管理员指定的码（而不是随机生成）
    pub custom: bool,
    /// 其中至少一张**沿用了已经存在的码**（`allow_existing` 那条路径）。
    ///
    /// 单独报出来是因为返回的 `codes` 里是这份**沿用后的新参数**，
    /// 而那张码上一个时刻的参数（尤其是旧的次数上限）已经不在了 ——
    /// 面板要能说清「你改的是一张老码，它的兑换记录还在」。
    pub reused_existing: bool,
}

/// 「生成邀请码」的全部入参。
///
/// 收成一个结构体而不是继续加形参：这条路已经有 6 个参数（数量 / 次数 / 天数 / 备注 / 等级），
/// 再加自定义码就是 8 个，调用处会退化成「一串看不出哪个是哪个」的字面量。
#[derive(Debug, Clone, Default)]
pub struct InviteSpec {
    /// 生成几个（系统随机码用；自定义码时被忽略）
    pub count: i64,
    /// 每个可用几次
    pub max_uses: i64,
    /// 有效期天数；`<= 0` 表示**不过期**（"永不过期" 在界面上是一个勾选框）
    pub expires_in_days: i64,
    pub note: String,
    /// 兑换后授予的角色：`user` / `admin`
    pub grant_role: String,
    /// 超管**自己指定**的码；`None`/空 = 照旧用系统随机生成。
    /// 只能是 [`crate::core::invite::CODE_LEN`] 位 A-Z 与 0-9（见 `invite::validate_custom_code`）
    pub custom_code: Option<String>,
    /// 自定义码**已经存在**时是否照用不误（等价于「无视风险继续」）。
    ///
    /// ⚠️ 这是**管理员显式确认过**才会为 true 的：面板先拿到 409 `invite_code_taken`
    /// （带那张码的状态与谁用过），弹确认框，用户点了「继续」才会带上来。
    /// 不带上它就一律拒绝 —— 免得手滑把一张已经发出去的码又「建」一遍，还以为新发了一张。
    pub allow_existing: bool,
}

/// 「整批发邮件」里单个收件人的结果
///
/// 逐条返回而不是「一失败整单失败」：批量发码时最常见的失败就是某一个邮箱打错了，
/// 那种情况下其余的必须照发，界面也要能指出是哪一条出的问题。
#[derive(Debug, Clone, Serialize)]
pub struct InviteMailResult {
    pub invite_id: i64,
    pub email: String,
    pub ok: bool,
    /// 失败原因（成功时是空串）
    pub error: String,
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

    /// 深度健康检查（P2）：真的碰一次库，证明「表读得出来」而不只是「进程还活着」。
    ///
    /// 为什么不能只用 `SELECT 1`：那条连表都不碰 —— 库文件被换成空文件、
    /// 迁移没跑、表被误删，它照样返回成功。而「进程活着但库坏了」正是最需要被告警的一类。
    /// 这里只读、极轻（auth.db 只有几十上百 KB，监控每 5 分钟跑一次）：
    /// 数一次 users 行 + 读一次 `PRAGMA user_version`（就是迁移版本）。
    pub async fn health_details(&self) -> Result<serde_json::Value> {
        self.read(|conn| {
            let users: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            Ok(serde_json::json!({
                "database": "ok",
                "users": users,
                "migration_version": version,
            }))
        })
        .await
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
    /// 邀请码的两种模式（由 `cfg.require_invite` 决定，见 P0-2）：
    /// - **强制模式**（生产）：邀请码**必填**，必须存在且状态可用；
    /// - **可选模式**（本地开发默认）：不填就是开放注册，填了就逐个校验，
    ///   让用户在这一步就拿到「码不对」的反馈，而不是等到注册才失败。
    ///
    /// ⚠️ 无论哪种模式，邀请码校验都**排在写 `email_codes` 之前**：
    /// 「先发码后校验」等于没拦（码已经落库、按现有流程还能被用掉）。
    /// `email_code_step_does_not_touch_db_when_invite_is_missing` 直接查库锁住这一点。
    pub async fn request_email_code(&self, email_raw: &str, invite_code_raw: &str, ip: &str) -> Result<CodeSent> {
        let email = validate::normalize_email(email_raw)
            .ok_or_else(|| AuthError::InvalidParams("邮箱格式不正确".into()))?;
        let codes = invite::split_codes(invite_code_raw);
        if self.cfg.require_invite && codes.is_empty() {
            // 强制邀请制：没填码连验证码都不发。用 invalid_invite 而不是新错误码，
            // 让前端沿用同一套「邀请码无效」提示（文案由前端决定要不要更具体）。
            return Err(AuthError::InvalidInvite);
        }
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
        // 邀请码：强制模式下必填（否则整个邀请制是假的），可选模式下留空 = 普通用户。
        // 填了（或必须填时）就逐个校验，注册成功后按码上的等级赋角色
        // （等级列 `grant_role` 随 P0-5 一起加，当前带码注册仍是「升级为管理员」）。
        // 邀请码：**一张**（P0-5 起不允许一次填多张，见下）。
        let codes = invite::split_codes(&input.invite_code);
        if self.cfg.require_invite && codes.is_empty() {
            return Err(AuthError::InvalidInvite);
        }
        if codes.len() > 1 {
            // ⚠️ 一次只收一张：多张码的等级可能冲突（一张 user、一张 admin），
            // 「取最高的」这种规则在出问题时很难向用户解释清楚。要两个身份就注册两个号。
            return Err(AuthError::InvalidParams("一次只能使用一张邀请码".into()));
        }
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
                //
                //    ⚠️ 角色**只看码上的 `grant_role`**（P0-5 之前是「带码即管理员」，
                //    那正是当时最大的权限漏洞：任何一张码都能造管理员）。
                //    不带码 = 普通用户（`AUTH_REQUIRE_INVITE=true` 时这一步根本走不到，
                //    上面 `codes.is_empty()` 就拦掉了；开关关掉时它就是开放注册）。
                let grant_role = redeemed
                    .first()
                    .map(|inv| inv.grant_role.clone())
                    .unwrap_or_else(|| models::ROLE_USER.to_string());
                let user_id =
                    sql::insert_user(conn, &email_w, username.as_deref(), &password_hash, &grant_role, Some(now), now)?;
                sql::insert_identity(conn, user_id, "email", &email_w, Some(now), now)?;
                for inv in &redeemed {
                    sql::insert_invite_use(conn, inv.id, Some(user_id), &email_w, &ip_w, now)?;
                    audit(
                        now,
                        conn,
                        "invite_use",
                        Some(user_id),
                        &inv.code,
                        &ip_w,
                        &ua_w,
                        &format!("邀请码兑换，授予角色 {grant_role}"),
                    )?;
                }
                let register_note = if redeemed.is_empty() {
                    "邮箱注册".to_string()
                } else {
                    format!("邮箱注册（邀请码授予角色 {grant_role}）")
                };
                audit(now, conn, "register", Some(user_id), &email_w, &ip_w, &ua_w, &register_note)?;

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
    ///
    /// `grant_role` 决定兑换后拿到的角色（只允许 `user` / `admin`，见 [`models::is_valid_grant_role`]）。
    /// ⚠️ 管理员码的 `ADMIN-` 前缀**只是给人看的**：真正的判定永远是查库读 `grant_role`，
    /// 所以伪造/删掉前缀都不会改变兑换结果（有测试把这一点钉死）。
    ///
    /// 两条路径（[`InviteSpec::custom_code`]）：
    /// - **留空 = 系统生成**：随机 16 位（字符表 A-Z + 0-9，与手填自定义码同一套），一次出 `count` 张，极小概率撞码时重试；
    /// - **填了 = 用管理员指定的码**：出 1 张，格式见 `invite::validate_custom_code`。
    ///
    /// ⚠️ 自定义码**已经存在**时不覆盖、不新建，回 [`AuthError::InviteCodeTaken`]（409）并带上
    /// 那张码的现状，由面板决定「无视风险继续」（`allow_existing`）。这样才不会出现
    /// 「以为新发了一张码，其实是把一张旧码又列了一遍」——那是最容易发错人的一种错。
    pub async fn create_invites(
        &self,
        actor_user_id: Option<i64>,
        spec: InviteSpec,
    ) -> Result<InviteCreated> {
        if !models::is_valid_grant_role(&spec.grant_role) {
            return Err(AuthError::InvalidParams(format!(
                "grant_role 只能是 user 或 admin，收到：{}",
                spec.grant_role
            )));
        }
        let max_uses = spec.max_uses.clamp(1, 1000);
        let now = self.clock.now();
        let expires_at = if spec.expires_in_days <= 0 {
            None
        } else {
            Some(now + time::Duration::days(spec.expires_in_days.min(365)))
        };
        let note = validate::truncate(spec.note.trim(), 100);
        // 一批共享一个 batch_id：便于「按批回收」与统计（例如「上周发的那 20 张用了几个」）
        let batch_id = uuid::Uuid::new_v4().to_string();
        let grant_role = spec.grant_role.clone();
        let custom = match spec.custom_code.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(raw) => Some(invite::validate_custom_code(raw).map_err(AuthError::InvalidParams)?),
        };
        let allow_existing = spec.allow_existing;
        // 自定义码一次只出一张：它的价值在「我认得这个串」，一填 50 个不同的码不是这个入口的用法
        let count = if custom.is_some() { 1 } else { spec.count.clamp(1, 50) };
        // 审计与回执里要写清「有效期」，0 天人话就是「永不过期」
        let expiry_text = if spec.expires_in_days <= 0 {
            "永不过期".to_string()
        } else {
            format!("{} 天", spec.expires_in_days.min(365))
        };
        let ip = "";
        // 闭包会拿走 `custom`，而返回时要报「这次是不是自定义码」，所以先记下来
        let is_custom = custom.is_some();

        let (rows, reused): (Vec<InviteRow>, bool) = self
            .write(move |conn| -> Result<(Vec<InviteRow>, bool)> {
                let mut out = Vec::new();
                let mut reused = false;
                for index in 0..count {
                    // 自定义码只在第一张（也就是唯一那张）上生效
                    let want = if index == 0 { custom.clone() } else { None };
                    match want {
                        Some(code) => match sql::find_invite_by_code(conn, &code)? {
                            Some(existing) => {
                                // 已经存在：默认拒绝并回现状；管理员确认过（allow_existing）才继续
                                if !allow_existing {
                                    let uses = sql::list_invite_uses(conn, &[existing.id])?;
                                    let public = build_invite_publics(vec![existing], uses, now)
                                        .pop()
                                        .map(Box::new);
                                    return Err(AuthError::InviteCodeTaken { code, existing: public });
                                }
                                // 沿用：把「这张码以后还能用几次 / 什么时候到期」改成这次填的，
                                // **不动** used_count / disabled / grant_role（见 sql::update_invite_terms）
                                sql::update_invite_terms(conn, existing.id, max_uses, expires_at)?;
                                let summary = code.clone();
                                audit(
                                    now,
                                    conn,
                                    "invite_create",
                                    actor_user_id,
                                    &summary,
                                    ip,
                                    "",
                                    &format!(
                                        "自定义码：沿用已存在的码（旧的兑换记录与已注册用户都不受影响），规则改为 次数上限 {max_uses}、有效期 {expiry_text}，等级 {grant_role}，批次 {batch_id}"
                                    ),
                                )?;
                                let row = sql::find_invite_by_code(conn, &code)?
                                    .ok_or(AuthError::NotFound)?;
                                reused = true;
                                out.push(row);
                            }
                            None => {
                                let id = sql::insert_invite(
                                    conn,
                                    &code,
                                    &note,
                                    max_uses,
                                    expires_at,
                                    actor_user_id,
                                    now,
                                    &grant_role,
                                    &batch_id,
                                )?;
                                audit(
                                    now,
                                    conn,
                                    "invite_create",
                                    actor_user_id,
                                    &code,
                                    ip,
                                    "",
                                    &format!(
                                        "自定义码，等级 {grant_role}，次数上限 {max_uses}，批次 {batch_id}"
                                    ),
                                )?;
                                out.push(InviteRow {
                                    id,
                                    code,
                                    note: note.clone(),
                                    max_uses,
                                    used_count: 0,
                                    expires_at,
                                    disabled: false,
                                    created_by: actor_user_id,
                                    created_at: now,
                                    grant_role: grant_role.clone(),
                                    batch_id: batch_id.clone(),
                                });
                            }
                        },
                        None => {
                            // 系统随机码：撞码（极小概率）就换一个重试。
                            // ⚠️ 只对随机码重试 —— 自定义码撞了必须**如实报冲突**（上面那个分支），
                            // 重试只会把「你写的码已经被占用」变成一句莫名其妙的「连续撞码」。
                            let mut created = None;
                            for _ in 0..5 {
                                let code = invite::generate_code_for(&grant_role);
                                match sql::insert_invite(
                                    conn,
                                    &code,
                                    &note,
                                    max_uses,
                                    expires_at,
                                    actor_user_id,
                                    now,
                                    &grant_role,
                                    &batch_id,
                                ) {
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
                                            grant_role: grant_role.clone(),
                                            batch_id: batch_id.clone(),
                                        });
                                        break;
                                    }
                                    Err(crate::error::StoreError::Sql(e)) if is_unique_violation(&e) => continue,
                                    Err(e) => return Err(e.into()),
                                }
                            }
                            out.push(created.ok_or_else(|| {
                                AuthError::Internal("生成邀请码失败（连续撞码）".into())
                            })?);
                        }
                    }
                }
                if custom.is_none() {
                    let summary = out.iter().map(|r| r.code.clone()).collect::<Vec<_>>().join(",");
                    audit(
                        now,
                        conn,
                        "invite_create",
                        actor_user_id,
                        &summary,
                        ip,
                        "",
                        &format!("共 {count} 个，等级 {grant_role}，批次 {batch_id}"),
                    )?;
                }
                Ok((out, reused))
            })
            .await?;

        let uses = self
            .read({
                let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
                move |conn| Ok(sql::list_invite_uses(conn, &ids)?)
            })
            .await?;
        Ok(InviteCreated {
            codes: build_invite_publics(rows, uses, now),
            custom: is_custom,
            reused_existing: reused,
        })
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
        let (rows, total, uses) = self
            .read(move |conn| {
                let total = sql::count_invites(conn, filter, now)?;
                let rows = sql::list_invites(conn, filter, size, offset, now)?;
                // 兑换记录只查**这一页**的码：管理面板要显示「谁用了 / 邮箱」，
                // 但没必要为了这一页去扫全表
                let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
                let uses = sql::list_invite_uses(conn, &ids)?;
                Ok((rows, total, uses))
            })
            .await?;
        Ok((build_invite_publics(rows, uses, now), total))
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

    // ------------------------------------------------ 邀请码批量 / 审计（P1 管理面板）

    /// 「重新启用一张已用过的码」：把已用次数清零，让它还能被兑换。
    ///
    /// ⚠️ 这是一个**主动降安全**的动作：码已经流出去过（可能已经被别人看到），
    /// 清零之后它又是一张可用的凭据。所以三件事必须同时成立：
    /// ① 只有超管能做（路由上挂 `SuperAdminUser`）；② 每次都写一条 `invite_reset` 审计；
    /// ③ `invite_uses` 里的兑换记录**保留**，面板里仍然看得到谁用过它。
    ///
    /// 码不存在 → 404；本来就没被用过 → 返回 0（幂等，不是错误，面板照实说一句）。
    pub async fn reset_invite(
        &self,
        actor_user_id: Option<i64>,
        id: i64,
        ip: &str,
        ua: &str,
    ) -> Result<usize> {
        let now = self.clock.now();
        let ip_owned = ip.to_string();
        let ua_owned = ua.to_string();
        let cleared = self
            .write(move |conn| -> Result<usize> {
                match sql::reset_invite(conn, id)? {
                    None => Err(AuthError::NotFound),
                    Some(cleared) => {
                        audit(
                            now,
                            conn,
                            "invite_reset",
                            actor_user_id,
                            &id.to_string(),
                            &ip_owned,
                            &ua_owned,
                            &format!("重新启用：清掉 {cleared} 次使用记录"),
                        )?;
                        Ok(cleared)
                    }
                }
            })
            .await?;
        Ok(cleared)
    }

    /// 按批停用（面板上的「停用这一批」）：整批一次性回收，返回真正被停用的张数。
    ///
    /// 幂等：已经是停用状态的不计数（重复点只会拿到 0）。只有这一批**根本不存在**才 404 ——
    /// 「已经全停用了」不该让面板弹一个错误出来。
    pub async fn disable_invites_by_batch(
        &self,
        actor_user_id: Option<i64>,
        batch_id: &str,
        ip: &str,
        ua: &str,
    ) -> Result<usize> {
        let batch = batch_id.trim().to_string();
        if batch.is_empty() {
            return Err(AuthError::InvalidParams("batch_id 不能为空".into()));
        }
        let now = self.clock.now();
        let ip_owned = ip.to_string();
        let ua_owned = ua.to_string();
        let n = self
            .write(move |conn| -> Result<usize> {
                if sql::count_invites_in_batch(conn, &batch)? == 0 {
                    return Err(AuthError::NotFound);
                }
                let n = sql::disable_invites_by_batch(conn, &batch)?;
                audit(
                    now,
                    conn,
                    "invite_disable_batch",
                    actor_user_id,
                    &batch,
                    &ip_owned,
                    &ua_owned,
                    &format!("整批停用 {n} 张"),
                )?;
                Ok(n)
            })
            .await?;
        Ok(n)
    }

    /// 让某个用户的**全部**会话立刻下线（超管动作，写审计）。
    ///
    /// 与自己点的「退出其它设备」不同：这里连**当前**会话一起吊销 ——
    /// 管理动作的语义就是「这个账号现在必须重新登录」。actor 就是目标本人时同样生效，
    /// 等于把自己也踢出去（界面会先确认一次）。
    pub async fn revoke_user_sessions_admin(
        &self,
        actor_user_id: i64,
        target_user_id: i64,
        ip: &str,
        ua: &str,
    ) -> Result<usize> {
        let now = self.clock.now();
        let ip_owned = ip.to_string();
        let ua_owned = ua.to_string();
        let n = self
            .write(move |conn| -> Result<usize> {
                if sql::find_user_by_id(conn, target_user_id)?.is_none() {
                    return Err(AuthError::NotFound);
                }
                let n = sql::revoke_user_sessions(conn, target_user_id, "admin_revoke", now, None)?;
                audit(
                    now,
                    conn,
                    "user_logout_all",
                    Some(actor_user_id),
                    &target_user_id.to_string(),
                    &ip_owned,
                    &ua_owned,
                    &format!("吊销 {n} 个会话"),
                )?;
                Ok(n)
            })
            .await?;
        Ok(n)
    }

    /// 审计日志查询（超管）：返回 `(条目, 总数, 出现过的动作清单)`。
    ///
    /// 动作清单跟着列表一起回：前端筛选下拉就不用再发一个请求（「尽量少请求」的落点之一），
    /// 也不会出现「界面里写死的动作列表」与库里实际动作对不上的情况。
    pub async fn list_audit(
        &self,
        filter: AuditFilter,
        page: i64,
        size: i64,
    ) -> Result<(Vec<AuditPublic>, i64, Vec<String>)> {
        let page = page.max(1);
        let size = size.clamp(1, 100);
        let offset = (page - 1) * size;
        let (rows, total, actions) = self
            .read(move |conn| {
                let total = sql::count_audit(conn, &filter)?;
                let rows = sql::list_audit(conn, &filter, size, offset)?;
                let actions = sql::distinct_audit_actions(conn)?;
                Ok((rows, total, actions))
            })
            .await?;
        Ok((rows.iter().map(|r| r.public()).collect(), total, actions))
    }

    /// 清理过期的审计日志（保留期见 `AUTH_AUDIT_RETENTION_DAYS`）。
    /// 启动时跑一次、之后每 24 小时一次（见 `main.rs`）；`retention_days <= 0` 表示不清理。
    pub async fn prune_audit(&self, retention_days: i64) -> Result<usize> {
        if retention_days <= 0 {
            return Ok(0);
        }
        let cutoff = self.clock.now() - time::Duration::days(retention_days);
        let n = self
            .write(move |conn| -> Result<usize> { Ok(sql::prune_audit_before(conn, cutoff)?) })
            .await?;
        Ok(n)
    }

    /// 把指定邀请码发给指定邮箱（面板上的「整批发邮件」）。
    ///
    /// 设计取舍：
    ///   - **一对一**：`pairs[i]` 就是「第 i 张码发给第 i 个邮箱」，由调用方配对。
    ///     一对一最好解释，也不会出现「一张码发给两个人」这种说不清的语义；
    ///   - 邮箱先**全部**规范化：有一个不合法就整单拒掉，不然用户要一封一封试；
    ///   - 单次最多 [`INVITE_MAIL_MAX`] 封，且**逐封独立**：某封失败不影响后面的，
    ///     结果逐条返回（批量发码最常见的手滑就是某一个邮箱打错）；
    ///   - 审计只记「哪几个码的 id、成功几封失败几封」，**绝不写邀请码明文** ——
    ///     审计表是给人翻的，把还能用的码抄进去等于到处撒钥匙。
    pub async fn send_invites_by_email(
        &self,
        actor_user_id: i64,
        pairs: Vec<(i64, String)>,
        ip: &str,
        ua: &str,
    ) -> Result<Vec<InviteMailResult>> {
        if pairs.is_empty() {
            return Err(AuthError::InvalidParams("至少要指定一个收件邮箱".into()));
        }
        if pairs.len() > INVITE_MAIL_MAX {
            return Err(AuthError::InvalidParams(format!(
                "单次最多发 {INVITE_MAIL_MAX} 封，收到 {}",
                pairs.len()
            )));
        }
        let mut normalized: Vec<(i64, String)> = Vec::with_capacity(pairs.len());
        for (id, email) in pairs {
            let to = validate::normalize_email(&email)
                .ok_or_else(|| AuthError::InvalidParams(format!("邮箱不合法：{email}")))?;
            normalized.push((id, to));
        }

        // 先把要用到的码一次读完（只读、不占写锁），再在锁外逐封发信 ——
        // 发信是网络操作，绝不能攥着数据库连接做（见 `store::mod` 顶部那段警告）
        let ids: Vec<i64> = normalized.iter().map(|(id, _)| *id).collect();
        let rows = self
            .read(move |conn| {
                let mut out = Vec::new();
                for id in ids {
                    out.push(sql::find_invite_by_id(conn, id)?);
                }
                Ok(out)
            })
            .await?;

        let mut results = Vec::with_capacity(normalized.len());
        let mut sent = 0usize;
        let mut failed = 0usize;
        let push_failed = |results: &mut Vec<InviteMailResult>, invite_id: i64, email: String, reason: &str| {
            results.push(InviteMailResult {
                invite_id,
                email,
                ok: false,
                error: reason.to_string(),
            });
        };
        for ((invite_id, email), row) in normalized.into_iter().zip(rows.into_iter()) {
            let Some(row) = row else {
                failed += 1;
                push_failed(&mut results, invite_id, email, "邀请码不存在");
                continue;
            };
            // 只发「确实还能用」的码：停用 / 用完 / 过期当场说明白，别让对方收到一封废码
            let status = row.status(self.clock.now());
            if status != "unused" {
                failed += 1;
                let reason = match status {
                    "disabled" => "这张码已停用",
                    "used" => "这张码已经用完",
                    "expired" => "这张码已过期",
                    _ => "这张码不可用",
                };
                push_failed(&mut results, invite_id, email, reason);
                continue;
            }
            let expires_hint = match row.expires_at {
                Some(at) => format!("有效期至 {}", at.date()),
                None => String::new(),
            };
            match self.mailer.send_invite(&email, &row.code, &row.note, &expires_hint).await {
                Ok(()) => {
                    sent += 1;
                    results.push(InviteMailResult {
                        invite_id,
                        email,
                        ok: true,
                        error: String::new(),
                    });
                }
                Err(e) => {
                    failed += 1;
                    // 具体失败原因可能含 SMTP 服务器信息：只进日志，对外给一句通用提示
                    tracing::warn!(error = %e, invite_id, "发送邀请码邮件失败");
                    push_failed(&mut results, invite_id, email, "发送失败（详见服务端日志）");
                }
            }
        }

        // 审计：只记码的 id 与统计，不记明文
        let now = self.clock.now();
        let ip_owned = ip.to_string();
        let ua_owned = ua.to_string();
        let ids_text = results.iter().map(|r| r.invite_id.to_string()).collect::<Vec<_>>().join(",");
        let detail = format!("共 {} 封，成功 {sent}，失败 {failed}", results.len());
        self.write(move |conn| {
            audit(
                now,
                conn,
                "invite_email",
                Some(actor_user_id),
                &ids_text,
                &ip_owned,
                &ua_owned,
                &detail,
            )
        })
        .await?;

        Ok(results)
    }

    // ------------------------------------------------------------ 管理员

    /// 确保管理员账号存在（幂等：已存在就跳过，绝不覆盖已有密码）
    ///
    /// `role` 只能是 [`models::is_valid_grant_role`] 允许的两级再加 `super_admin`
    /// （超管由 `--role super_admin` 或 `ensure_super_admin` 显式指定，
    /// **不能**通过发邀请码获得）。
    pub async fn seed_admin(&self, email_raw: &str, password: &str, role: &str) -> Result<bool> {
        if role != models::ROLE_ADMIN && role != models::ROLE_SUPER_ADMIN {
            return Err(AuthError::InvalidParams(format!("管理员角色只能是 admin / super_admin，收到：{role}")));
        }
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
        let role_owned = role.to_string();
        self.write(move |conn| -> Result<bool> {
            if sql::find_user_by_email(conn, &email_w)?.is_some() {
                return Ok(false);
            }
            let user_id = sql::insert_user(
                conn,
                &email_w,
                Some("管理员"),
                &hash,
                &role_owned,
                Some(now),
                now,
            )?;
            sql::insert_identity(conn, user_id, "email", &email_w, Some(now), now)?;
            audit(
                now,
                conn,
                "admin_seed",
                Some(user_id),
                &email_w,
                "",
                "",
                &format!("初始管理员（role={role_owned}）"),
            )?;
            Ok(true)
        })
        .await
    }

    /// 启动期幂等：确保 `AUTH_ADMIN_EMAIL` 那个账号是**超级管理员**。
    ///
    /// 这是「超管怎么产生」那条决策的落地点（用户选了「环境变量 + CLI 双保险」）：
    /// - 账号不存在 → 由 `seed_admin` 建出来（role=super_admin）；
    /// - 账号存在但是 admin → **提升**为 super_admin（上线时把老库那个管理员变超管）；
    /// - 已经是 super_admin → 什么也不做。
    ///
    /// ⚠️ 刻意**不做反向降级**：如果配置文件里换了邮箱，旧超管仍然是超管 ——
    /// 自动降权会在「改了环境变量想换人」时把上一个超管悄悄锁死，而锁死超管是无法自救的。
    /// 要降权请显式用 CLI 或后台改。
    ///
    /// 返回 `(是否新建, 是否提权)`，供启动日志与测试断言。
    pub async fn ensure_super_admin(&self, email_raw: &str, password: &str) -> Result<(bool, bool)> {
        let email = validate::normalize_email(email_raw)
            .ok_or_else(|| AuthError::InvalidParams("管理员邮箱格式不正确".into()))?;
        let created = self.seed_admin(&email, password, models::ROLE_SUPER_ADMIN).await?;
        if created {
            return Ok((true, false));
        }

        // 已存在：看看要不要提权
        let existing = {
            let email_q = email.clone();
            self.read(move |conn| Ok(sql::find_user_by_email(conn, &email_q)?)).await?
        };
        let Some(user) = existing else {
            // seed_admin 说建了，这里却查不到 —— 只能是并发删号，交给下一次启动
            return Ok((false, false));
        };
        if user.role == models::ROLE_SUPER_ADMIN {
            return Ok((false, false));
        }

        let now = self.clock.now();
        let user_id = user.id;
        let old_role = user.role.clone();
        let email_w = email.clone();
        self.write(move |conn| -> Result<()> {
            sql::update_user_role(conn, user_id, models::ROLE_SUPER_ADMIN, now)?;
            audit(
                now,
                conn,
                "role_change",
                None,
                &email_w,
                "",
                "",
                &format!("启动期确保超管：{old_role} → {}", models::ROLE_SUPER_ADMIN),
            )?;
            Ok(())
        })
        .await?;
        Ok((false, true))
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

    // ------------------------------------------------------------ 用户治理（超管）

    /// 列出用户（可按角色 / 状态过滤，`keyword` 搜邮箱或用户名）
    pub async fn list_users(
        &self,
        role: Option<String>,
        status: Option<String>,
        keyword: Option<String>,
        page: i64,
        size: i64,
    ) -> Result<(Vec<UserPublic>, i64)> {
        let page = page.max(1);
        let size = size.clamp(1, 100);
        let offset = (page - 1) * size;
        // 空字符串等于没给（前端清空搜索框后会原样传 `keyword=`）
        let keyword = keyword.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        let (rows, total) = self
            .read(move |conn| {
                let (rows, total) = sql::list_users(
                    conn,
                    role.as_deref(),
                    status.as_deref(),
                    keyword.as_deref(),
                    size,
                    offset,
                )?;
                Ok((rows, total))
            })
            .await?;
        Ok((rows.iter().map(|r| r.public()).collect(), total))
    }

    /// 改某个用户的角色（超管动作，写审计）。
    ///
    /// ⚠️ **自锁保护**：不允许把最后一个可用的超管降级 —— 那样谁也进不了后台，
    /// 只能 SSH 上去手写 SQL 救回来。判据是「改完之后还剩几个 active 的超管」。
    pub async fn change_user_role(
        &self,
        actor_user_id: i64,
        target_user_id: i64,
        new_role: &str,
        ip: &str,
        ua: &str,
    ) -> Result<UserPublic> {
        if !models::is_valid_grant_role(new_role) && new_role != models::ROLE_SUPER_ADMIN {
            return Err(AuthError::InvalidParams(format!(
                "角色只能是 user / admin / super_admin，收到：{new_role}"
            )));
        }
        let now = self.clock.now();
        let new_role_owned = new_role.to_string();
        let ip_owned = ip.to_string();
        let ua_owned = ua.to_string();
        let updated = self
            .write(move |conn| -> Result<Option<UserRow>> {
                let Some(target) = sql::find_user_by_id(conn, target_user_id)? else {
                    return Ok(None);
                };
                if target.role == new_role_owned {
                    return Ok(Some(target)); // 幂等：本来就是那个角色
                }
                // 只有「把某个超管降下去」才可能触发自锁
                if target.role == models::ROLE_SUPER_ADMIN
                    && target.is_active()
                    && sql::count_active_users_with_role(conn, models::ROLE_SUPER_ADMIN)? <= 1
                {
                    return Err(AuthError::InvalidParams(
                        "这是最后一个超级管理员，不能降级（否则没人能进后台了）".into(),
                    ));
                }
                sql::update_user_role(conn, target_user_id, &new_role_owned, now)?;
                audit(
                    now,
                    conn,
                    "role_change",
                    Some(actor_user_id),
                    &target.email,
                    &ip_owned,
                    &ua_owned,
                    &format!("{} → {}", target.role, new_role_owned),
                )?;
                Ok(sql::find_user_by_id(conn, target_user_id)?)
            })
            .await?;
        updated.map(|u| u.public()).ok_or(AuthError::NotFound)
    }

    /// 封禁 / 解封账号（超管动作，写审计）。
    ///
    /// 封禁会顺带吊销该用户全部会话（否则他手里的令牌 15 分钟内还能用）。
    /// ⚠️ 同样是自锁保护：不能封掉最后一个可用超管。
    pub async fn set_user_status(
        &self,
        actor_user_id: i64,
        target_user_id: i64,
        new_status: &str,
        ip: &str,
        ua: &str,
    ) -> Result<UserPublic> {
        if new_status != "active" && new_status != "disabled" {
            return Err(AuthError::InvalidParams(format!(
                "status 只能是 active / disabled，收到：{new_status}"
            )));
        }
        let now = self.clock.now();
        let status_owned = new_status.to_string();
        let ip_owned = ip.to_string();
        let ua_owned = ua.to_string();
        let updated = self
            .write(move |conn| -> Result<Option<UserRow>> {
                let Some(target) = sql::find_user_by_id(conn, target_user_id)? else {
                    return Ok(None);
                };
                if target.status == status_owned {
                    return Ok(Some(target)); // 幂等
                }
                if status_owned == "disabled"
                    && target.role == models::ROLE_SUPER_ADMIN
                    && sql::count_active_users_with_role(conn, models::ROLE_SUPER_ADMIN)? <= 1
                {
                    return Err(AuthError::InvalidParams(
                        "这是最后一个超级管理员，不能封禁（否则没人能进后台了）".into(),
                    ));
                }
                sql::update_user_status(conn, target_user_id, &status_owned, now)?;
                if status_owned == "disabled" {
                    sql::revoke_user_sessions(conn, target_user_id, "disabled", now, None)?;
                }
                audit(
                    now,
                    conn,
                    "user_status",
                    Some(actor_user_id),
                    &target.email,
                    &ip_owned,
                    &ua_owned,
                    &format!("{} → {}", target.status, status_owned),
                )?;
                Ok(sql::find_user_by_id(conn, target_user_id)?)
            })
            .await?;
        updated.map(|u| u.public()).ok_or(AuthError::NotFound)
    }
}

/// 是不是「唯一约束被撞了」（邀请码表的 `code` 唯一索引）。
///
/// 单独抽出来是因为**两种含义完全不同**：随机码撞了是「换一个再来」（极小概率，重试即可），
/// 自定义码撞了是「这个码已经被占了」（必须如实告诉管理员，重试只会把它变成一句莫名其妙的
/// 「连续撞码」）。rusqlite 只在 `SqliteFailure` 里给错误码，所以两级都判一下。
fn is_unique_violation(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(e, _) => {
            e.code == rusqlite::ErrorCode::ConstraintViolation
                || e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
        }
        _ => false,
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
