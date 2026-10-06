// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! SQL 语句集合
//!
//! 所有函数都是「同步 + 收 `&Connection`」，读路径与 `SqliteStore::write`
//! 的事务里都能直接调用。列名用显式清单 + 按名取值：写错列名会当场报错，
//! 而不是静默串位。

use rusqlite::{params, Connection, OptionalExtension};
use time::OffsetDateTime;

use crate::db::{parse_ts, ts};
use crate::error::StoreError;
use crate::models::{AuditRow, EmailCodeRow, InviteRow, InviteUseRow, NewAudit, SessionRow, UserRow};

type R<T> = Result<T, StoreError>;

const USER_COLUMNS: &str = "id, email, username, password_hash, role, status, email_verified_at, \
                            failed_attempts, locked_until, last_login_at, created_at, updated_at";
const SESSION_COLUMNS: &str = "id, user_id, family_id, refresh_hash, device_label, user_agent, ip, \
                               created_at, last_used_at, expires_at, revoked_at, revoked_reason, replaced_by";
const INVITE_COLUMNS: &str = "id, code, note, max_uses, used_count, expires_at, disabled, created_by, \
                               created_at, grant_role, batch_id";
const EMAIL_CODE_COLUMNS: &str =
    "id, email, purpose, code_hash, attempts, expires_at, consumed_at, created_at, ip";

fn opt_ts(raw: Option<String>) -> Option<OffsetDateTime> {
    raw.as_deref().and_then(parse_ts)
}

/// 必填时间列：解析失败时退回 Unix 纪元。
/// 对 `expires_at` 这类列来说这是**安全**的默认值（等于「立刻过期」）。
fn req_ts(raw: String) -> OffsetDateTime {
    parse_ts(&raw).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: row.get("id")?,
        email: row.get("email")?,
        username: row.get("username")?,
        password_hash: row.get("password_hash")?,
        role: row.get("role")?,
        status: row.get("status")?,
        email_verified_at: opt_ts(row.get("email_verified_at")?),
        failed_attempts: row.get("failed_attempts")?,
        locked_until: opt_ts(row.get("locked_until")?),
        last_login_at: opt_ts(row.get("last_login_at")?),
        created_at: req_ts(row.get("created_at")?),
        updated_at: req_ts(row.get("updated_at")?),
    })
}

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get("id")?,
        user_id: row.get("user_id")?,
        family_id: row.get("family_id")?,
        refresh_hash: row.get("refresh_hash")?,
        device_label: row.get("device_label")?,
        user_agent: row.get("user_agent")?,
        ip: row.get("ip")?,
        created_at: req_ts(row.get("created_at")?),
        last_used_at: opt_ts(row.get("last_used_at")?),
        expires_at: req_ts(row.get("expires_at")?),
        revoked_at: opt_ts(row.get("revoked_at")?),
        revoked_reason: row.get("revoked_reason")?,
        replaced_by: row.get("replaced_by")?,
    })
}

fn invite_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InviteRow> {
    Ok(InviteRow {
        id: row.get("id")?,
        code: row.get("code")?,
        note: row.get("note")?,
        max_uses: row.get("max_uses")?,
        used_count: row.get("used_count")?,
        expires_at: opt_ts(row.get("expires_at")?),
        disabled: row.get::<_, i64>("disabled")? != 0,
        created_by: row.get("created_by")?,
        created_at: req_ts(row.get("created_at")?),
        grant_role: row.get("grant_role")?,
        batch_id: row.get("batch_id")?,
    })
}

fn email_code_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmailCodeRow> {
    Ok(EmailCodeRow {
        id: row.get("id")?,
        email: row.get("email")?,
        purpose: row.get("purpose")?,
        code_hash: row.get("code_hash")?,
        attempts: row.get("attempts")?,
        expires_at: req_ts(row.get("expires_at")?),
        consumed_at: opt_ts(row.get("consumed_at")?),
        created_at: req_ts(row.get("created_at")?),
        ip: row.get("ip")?,
    })
}

// ---------------------------------------------------------------- 用户

/// 新建用户，返回自增 id
pub fn insert_user(
    conn: &Connection,
    email: &str,
    username: Option<&str>,
    password_hash: &str,
    role: &str,
    email_verified_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> R<i64> {
    conn.execute(
        "INSERT INTO users (email, username, password_hash, role, status, email_verified_at, \
         failed_attempts, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'active', ?5, 0, ?6, ?6)",
        params![email, username, password_hash, role, email_verified_at.map(ts), ts(now)],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn find_user_by_email(conn: &Connection, email: &str) -> R<Option<UserRow>> {
    let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE email = ?1");
    Ok(conn.query_row(&sql, params![email], user_from_row).optional()?)
}

/// 按登录标识查用户（多方式登录的入口；本期 provider 只有 `email`）
pub fn find_user_by_identity(conn: &Connection, provider: &str, identifier: &str) -> R<Option<UserRow>> {
    let sql = format!(
        "SELECT {} FROM users u JOIN user_identities i ON i.user_id = u.id \
         WHERE i.provider = ?1 AND i.identifier = ?2",
        // 列名前缀避免与 user_identities.id / created_at 撞名
        USER_COLUMNS.split(", ").map(|c| format!("u.{c}")).collect::<Vec<_>>().join(", ")
    );
    Ok(conn.query_row(&sql, params![provider, identifier], user_from_row).optional()?)
}

pub fn find_user_by_id(conn: &Connection, id: i64) -> R<Option<UserRow>> {
    let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1");
    Ok(conn.query_row(&sql, params![id], user_from_row).optional()?)
}

pub fn insert_identity(
    conn: &Connection,
    user_id: i64,
    provider: &str,
    identifier: &str,
    verified_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> R<()> {
    conn.execute(
        "INSERT INTO user_identities (user_id, provider, identifier, verified_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user_id, provider, identifier, verified_at.map(ts), ts(now)],
    )?;
    Ok(())
}

/// 登录成功：清零失败计数与锁定，记录最后登录时间
pub fn update_login_success(conn: &Connection, user_id: i64, now: OffsetDateTime) -> R<()> {
    conn.execute(
        "UPDATE users SET failed_attempts = 0, locked_until = NULL, last_login_at = ?2, updated_at = ?2 \
         WHERE id = ?1",
        params![user_id, ts(now)],
    )?;
    Ok(())
}

/// 数「还有几个这种角色的**可用**账号」——自锁保护的依据。
///
/// 只数 `status = 'active'`：被禁用/封禁的超管等于不存在，不能拿来当「还有人能救我」的理由。
/// 否则「先封掉另一个超管、再把自己降级」会一路通过，最后没人能进后台。
pub fn count_active_users_with_role(conn: &Connection, role: &str) -> R<i64> {
    let n = conn.query_row(
        "SELECT count(*) FROM users WHERE role = ?1 AND status = 'active'",
        params![role],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(n)
}

/// 改某个用户的角色；返回受影响行数
pub fn update_user_role(conn: &Connection, user_id: i64, role: &str, now: OffsetDateTime) -> R<usize> {
    let n = conn.execute(
        "UPDATE users SET role = ?2, updated_at = ?3 WHERE id = ?1",
        params![user_id, role, ts(now)],
    )?;
    Ok(n)
}

/// 改某个账号的状态（`active` / `disabled`）；返回受影响行数
pub fn update_user_status(conn: &Connection, user_id: i64, status: &str, now: OffsetDateTime) -> R<usize> {
    let n = conn.execute(
        "UPDATE users SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![user_id, status, ts(now)],
    )?;
    Ok(n)
}

/// 列出用户（分页，可按角色 / 状态 / 关键词过滤，按 id 倒序）
///
/// `keyword` 同时匹配邮箱与用户名（子串、大小写不敏感）：管理面板的搜索框用它。
/// ⚠️ 先把用户输入里的 `%` 与 `_` 剔掉再拼进 LIKE —— 那两个是 LIKE 的通配符，
/// 不处理的话搜 `a%` 会变成「以 a 开头的所有账号」，搜 `_` 会命中全部账号。
pub fn list_users(
    conn: &Connection,
    role: Option<&str>,
    status: Option<&str>,
    keyword: Option<&str>,
    limit: i64,
    offset: i64,
) -> R<(Vec<UserRow>, i64)> {
    let mut where_clause = String::from(" WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(role) = role {
        where_clause.push_str(" AND role = ?");
        args.push(Box::new(role.to_string()));
    }
    if let Some(status) = status {
        where_clause.push_str(" AND status = ?");
        args.push(Box::new(status.to_string()));
    }
    if let Some(keyword) = keyword {
        where_clause.push_str(" AND (email LIKE ? OR username LIKE ?)");
        let pattern = format!("%{}%", keyword.replace('%', "").replace('_', ""));
        args.push(Box::new(pattern.clone()));
        args.push(Box::new(pattern));
    }
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM users{where_clause}"),
        rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
        |row| row.get(0),
    )?;
    args.push(Box::new(limit.clamp(1, 200)));
    args.push(Box::new(offset.max(0)));
    let sql = format!("SELECT {USER_COLUMNS} FROM users{where_clause} ORDER BY id DESC LIMIT ? OFFSET ?");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
        user_from_row,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok((out, total))
}

/// 登录失败：累加计数，达到阈值则锁定并清零计数；返回 `(累加后的次数, 锁定到期时间)`
pub fn record_login_failure(
    conn: &Connection,
    user_id: i64,
    threshold: i64,
    lock_minutes: i64,
    now: OffsetDateTime,
) -> R<(i64, Option<OffsetDateTime>)> {
    conn.execute(
        "UPDATE users SET failed_attempts = failed_attempts + 1, updated_at = ?2 WHERE id = ?1",
        params![user_id, ts(now)],
    )?;
    let attempts: i64 =
        conn.query_row("SELECT failed_attempts FROM users WHERE id = ?1", params![user_id], |r| r.get(0))?;
    if threshold > 0 && attempts >= threshold {
        let until = now + time::Duration::minutes(lock_minutes.max(1));
        conn.execute(
            "UPDATE users SET failed_attempts = 0, locked_until = ?2, updated_at = ?3 WHERE id = ?1",
            params![user_id, ts(until), ts(now)],
        )?;
        return Ok((attempts, Some(until)));
    }
    Ok((attempts, None))
}

pub fn update_password_hash(conn: &Connection, user_id: i64, hash: &str, now: OffsetDateTime) -> R<()> {
    conn.execute(
        "UPDATE users SET password_hash = ?2, updated_at = ?3 WHERE id = ?1",
        params![user_id, hash, ts(now)],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- 会话

#[allow(clippy::too_many_arguments)]
pub fn insert_session(
    conn: &Connection,
    user_id: i64,
    family_id: &str,
    refresh_hash: &str,
    device_label: &str,
    user_agent: &str,
    ip: &str,
    now: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> R<i64> {
    conn.execute(
        "INSERT INTO sessions (user_id, family_id, refresh_hash, device_label, user_agent, ip, \
         created_at, last_used_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        params![user_id, family_id, refresh_hash, device_label, user_agent, ip, ts(now), ts(expires_at)],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn find_session_by_refresh_hash(conn: &Connection, refresh_hash: &str) -> R<Option<SessionRow>> {
    let sql = format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE refresh_hash = ?1");
    Ok(conn.query_row(&sql, params![refresh_hash], session_from_row).optional()?)
}

pub fn find_session_by_id(conn: &Connection, id: i64) -> R<Option<SessionRow>> {
    let sql = format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1");
    Ok(conn.query_row(&sql, params![id], session_from_row).optional()?)
}

pub fn touch_session(conn: &Connection, id: i64, now: OffsetDateTime) -> R<()> {
    conn.execute("UPDATE sessions SET last_used_at = ?2 WHERE id = ?1", params![id, ts(now)])?;
    Ok(())
}

/// 吊销单个会话
pub fn revoke_session(conn: &Connection, id: i64, reason: &str, now: OffsetDateTime) -> R<()> {
    conn.execute(
        "UPDATE sessions SET revoked_at = ?2, revoked_reason = ?3 WHERE id = ?1 AND revoked_at IS NULL",
        params![id, ts(now), reason],
    )?;
    Ok(())
}

/// 轮换：吊销旧会话并记住「被谁替换」
pub fn mark_session_rotated(conn: &Connection, old_id: i64, new_id: i64, now: OffsetDateTime) -> R<()> {
    conn.execute(
        "UPDATE sessions SET revoked_at = ?2, revoked_reason = 'rotated', replaced_by = ?3 \
         WHERE id = ?1 AND revoked_at IS NULL",
        params![old_id, ts(now), new_id],
    )?;
    Ok(())
}

/// 吊销整条轮换链（refresh 重放时用）
pub fn revoke_family(conn: &Connection, family_id: &str, reason: &str, now: OffsetDateTime) -> R<usize> {
    let n = conn.execute(
        "UPDATE sessions SET revoked_at = ?2, revoked_reason = ?3 \
         WHERE family_id = ?1 AND revoked_at IS NULL",
        params![family_id, ts(now), reason],
    )?;
    Ok(n)
}

/// 吊销某人全部会话（可保留当前这一个）
pub fn revoke_user_sessions(
    conn: &Connection,
    user_id: i64,
    reason: &str,
    now: OffsetDateTime,
    keep_session_id: Option<i64>,
) -> R<usize> {
    let n = match keep_session_id {
        Some(keep) => conn.execute(
            "UPDATE sessions SET revoked_at = ?2, revoked_reason = ?3 \
             WHERE user_id = ?1 AND revoked_at IS NULL AND id <> ?4",
            params![user_id, ts(now), reason, keep],
        )?,
        None => conn.execute(
            "UPDATE sessions SET revoked_at = ?2, revoked_reason = ?3 \
             WHERE user_id = ?1 AND revoked_at IS NULL",
            params![user_id, ts(now), reason],
        )?,
    };
    Ok(n)
}

/// 某人的活跃会话（未吊销、未过期），新的在前
pub fn list_user_sessions(conn: &Connection, user_id: i64, now: OffsetDateTime) -> R<Vec<SessionRow>> {
    let sql = format!(
        "SELECT {SESSION_COLUMNS} FROM sessions \
         WHERE user_id = ?1 AND revoked_at IS NULL AND expires_at > ?2 ORDER BY id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![user_id, ts(now)], session_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---------------------------------------------------------------- 邀请码

pub fn insert_invite(
    conn: &Connection,
    code: &str,
    note: &str,
    max_uses: i64,
    expires_at: Option<OffsetDateTime>,
    created_by: Option<i64>,
    now: OffsetDateTime,
    grant_role: &str,
    batch_id: &str,
) -> R<i64> {
    conn.execute(
        "INSERT INTO invite_codes \
           (code, note, max_uses, used_count, expires_at, disabled, created_by, created_at, grant_role, batch_id) \
         VALUES (?1, ?2, ?3, 0, ?4, 0, ?5, ?6, ?7, ?8)",
        params![code, note, max_uses.max(1), expires_at.map(ts), created_by, ts(now), grant_role, batch_id],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn find_invite_by_code(conn: &Connection, code: &str) -> R<Option<InviteRow>> {
    let sql = format!("SELECT {INVITE_COLUMNS} FROM invite_codes WHERE code = ?1");
    Ok(conn.query_row(&sql, params![code], invite_from_row).optional()?)
}

/// 占用一次使用额度：条件更新，返回受影响行数（0 表示已用尽/已停用/已过期）
pub fn consume_invite(conn: &Connection, id: i64, now: OffsetDateTime) -> R<usize> {
    let n = conn.execute(
        "UPDATE invite_codes SET used_count = used_count + 1 \
         WHERE id = ?1 AND disabled = 0 AND used_count < max_uses \
         AND (expires_at IS NULL OR expires_at > ?2)",
        params![id, ts(now)],
    )?;
    Ok(n)
}

/// 邀请码筛选（后台列表用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteFilter {
    All,
    Unused,
    Used,
    Expired,
    Disabled,
}

impl InviteFilter {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "all" => Some(InviteFilter::All),
            "unused" => Some(InviteFilter::Unused),
            "used" => Some(InviteFilter::Used),
            "expired" => Some(InviteFilter::Expired),
            "disabled" => Some(InviteFilter::Disabled),
            _ => None,
        }
    }

    fn where_clause(&self) -> &'static str {
        // ⚠️ 每个分支都必须引用 `?1`：下面 count/list 是**无条件**绑定 `?1 = ts(now)` 的，
        // 某个分支不引用它时 rusqlite 会直接报「参数个数不符」。
        // `?1 IS NOT NULL` 恒为真（`?1` 永远是合法的时间文本），只是为了让参数位始终对得上。
        match self {
            InviteFilter::All => "(?1 IS NOT NULL)",
            InviteFilter::Unused => {
                "disabled = 0 AND used_count < max_uses AND (expires_at IS NULL OR expires_at > ?1)"
            }
            InviteFilter::Used => "used_count >= max_uses AND (?1 IS NOT NULL)",
            InviteFilter::Expired => "disabled = 0 AND expires_at IS NOT NULL AND expires_at <= ?1",
            InviteFilter::Disabled => "disabled = 1 AND (?1 IS NOT NULL)",
        }
    }
}

pub fn count_invites(conn: &Connection, filter: InviteFilter, now: OffsetDateTime) -> R<i64> {
    let sql = format!("SELECT count(*) FROM invite_codes WHERE {}", filter.where_clause());
    Ok(conn.query_row(&sql, params![ts(now)], |r| r.get(0))?)
}

pub fn list_invites(
    conn: &Connection,
    filter: InviteFilter,
    limit: i64,
    offset: i64,
    now: OffsetDateTime,
) -> R<Vec<InviteRow>> {
    let sql = format!(
        "SELECT {INVITE_COLUMNS} FROM invite_codes WHERE {} ORDER BY id DESC LIMIT ?2 OFFSET ?3",
        filter.where_clause()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![ts(now), limit.max(1), offset.max(0)], invite_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn disable_invite(conn: &Connection, id: i64) -> R<usize> {
    let n = conn.execute("UPDATE invite_codes SET disabled = 1 WHERE id = ?1", params![id])?;
    Ok(n)
}

/// 改一张**已经存在**的码的规则（自定义码「沿用」时用）。
///
/// ⚠️ 只动 `max_uses` 与 `expires_at`，**绝不碰** `used_count` / `disabled` / `grant_role`：
/// 沿用一张已经发出去的码，管理员改的是「这张码以后还能用几次、什么时候到期」，
/// 而不是「把它重置成新的」。已用次数、停用状态、等级都是这张码的身份，
/// 顺手改掉就等于悄悄放宽（或收紧）了已发出的凭据 —— 那正是最不该发生的事。
pub fn update_invite_terms(
    conn: &Connection,
    id: i64,
    max_uses: i64,
    expires_at: Option<OffsetDateTime>,
) -> R<usize> {
    let n = conn.execute(
        "UPDATE invite_codes SET max_uses = ?2, expires_at = ?3 WHERE id = ?1",
        params![id, max_uses.max(1), expires_at.map(ts)],
    )?;
    Ok(n)
}

/// 把一张码的**已用次数清零**（管理面板的「重新启用已用过的码」）。
/// 返回 `None` = 没有这张码；`Some(n)` = 清掉了 n 次（0 表示本来就没被用过）。
/// **不动 `invite_uses`**：兑换记录是审计留痕，重置之后列表里仍然看得到
/// 「谁在什么时候用过这张码」——「这张码被回收过」这件事本身也必须留痕，
/// 否则界面看起来就跟凭空多出一张新码一样。
pub fn reset_invite(conn: &Connection, id: i64) -> R<Option<usize>> {
    let prev: Option<i64> = conn
        .query_row("SELECT used_count FROM invite_codes WHERE id = ?1", params![id], |row| row.get(0))
        .optional()?;
    let Some(prev) = prev else { return Ok(None) };
    conn.execute("UPDATE invite_codes SET used_count = 0 WHERE id = ?1", params![id])?;
    Ok(Some(prev.max(0) as usize))
}

pub fn insert_invite_use(
    conn: &Connection,
    invite_id: i64,
    user_id: Option<i64>,
    email: &str,
    ip: &str,
    now: OffsetDateTime,
) -> R<()> {
    conn.execute(
        "INSERT INTO invite_uses (invite_code_id, user_id, email, ip, used_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![invite_id, user_id, email, ip, ts(now)],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- 邮箱验证码

pub fn insert_email_code(
    conn: &Connection,
    email: &str,
    purpose: &str,
    code_hash: &str,
    expires_at: OffsetDateTime,
    ip: &str,
    now: OffsetDateTime,
) -> R<i64> {
    conn.execute(
        "INSERT INTO email_codes (email, purpose, code_hash, attempts, expires_at, created_at, ip) \
         VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6)",
        params![email, purpose, code_hash, ts(expires_at), ts(now), ip],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 作废同一邮箱同一用途下尚未消费的旧验证码（同一时刻只应有一个有效码）
pub fn consume_active_email_codes(conn: &Connection, email: &str, purpose: &str, now: OffsetDateTime) -> R<usize> {
    let n = conn.execute(
        "UPDATE email_codes SET consumed_at = ?3 WHERE email = ?1 AND purpose = ?2 AND consumed_at IS NULL",
        params![email, purpose, ts(now)],
    )?;
    Ok(n)
}

/// 取最近一条未消费的验证码
pub fn find_active_email_code(conn: &Connection, email: &str, purpose: &str) -> R<Option<EmailCodeRow>> {
    let sql = format!(
        "SELECT {EMAIL_CODE_COLUMNS} FROM email_codes \
         WHERE email = ?1 AND purpose = ?2 AND consumed_at IS NULL ORDER BY id DESC LIMIT 1"
    );
    Ok(conn.query_row(&sql, params![email, purpose], email_code_from_row).optional()?)
}

/// 某个邮箱收过多少条验证码（**含已消费**）。
///
/// 给「强制邀请码」的验收用：没带有效邀请码时，这一步必须**一条都不写库**。
/// 只数「未消费」的还不够 —— 那种断言在「写了一条又立刻标记为已消费」时也会通过，
/// 而 P0-2 要的恰恰是「连写都没写」。
pub fn count_email_codes(conn: &Connection, email: &str, purpose: &str) -> R<i64> {
    let n = conn.query_row(
        "SELECT count(*) FROM email_codes WHERE email = ?1 AND purpose = ?2",
        params![email, purpose],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(n)
}

pub fn bump_email_code_attempts(conn: &Connection, id: i64) -> R<()> {
    conn.execute("UPDATE email_codes SET attempts = attempts + 1 WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn consume_email_code(conn: &Connection, id: i64, now: OffsetDateTime) -> R<()> {
    conn.execute("UPDATE email_codes SET consumed_at = ?2 WHERE id = ?1", params![id, ts(now)])?;
    Ok(())
}

// ---------------------------------------------------------------- 审计

pub fn insert_audit(conn: &Connection, entry: &NewAudit) -> R<()> {
    conn.execute(
        "INSERT INTO audit_logs (actor_user_id, action, target, ip, user_agent, detail, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            entry.actor_user_id,
            entry.action,
            entry.target,
            entry.ip,
            entry.user_agent,
            entry.detail,
            ts(entry.created_at)
        ],
    )?;
    Ok(())
}

/// 某个动作在审计表里有多少条（测试用：治理动作必须留痕）
pub fn count_audit_by_action(conn: &Connection, action: &str) -> R<i64> {
    let n = conn.query_row(
        "SELECT count(*) FROM audit_logs WHERE action = ?1",
        params![action],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(n)
}

/// 邀请码总行数（含停用/过期的）。测试用：迁移要能证明「存量码都被停用了」。
pub fn count_all_invites(conn: &Connection) -> R<i64> {
    let n = conn.query_row("SELECT count(*) FROM invite_codes", [], |row| row.get::<_, i64>(0))?;
    Ok(n)
}

// ---------------------------------------------------------------- 审计查询（P1 管理面板）

/// 审计日志的筛选条件（每个字段都可空，空 = 不加这一条限制）
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub action: Option<String>,
    pub actor_user_id: Option<i64>,
    /// 起始时间（含）
    pub from: Option<OffsetDateTime>,
    /// 结束时间（含）
    pub to: Option<OffsetDateTime>,
}

impl AuditFilter {
    /// ⚠️ 四个参数位**始终**绑定（`?1 IS NULL OR ...` 的写法）：`count_audit` 与
    /// `list_audit` 共用这一段，某个筛选项为空时也不会让参数位错位。
    /// 时间列在库里是定宽的 `YYYY-MM-DDTHH:MM:SSZ` 文本（见 `db::ts`），
    /// 所以字符串比较的结果与时间先后一致，不需要再解析成时间。
    const WHERE_CLAUSE: &'static str = "(?1 IS NULL OR a.action = ?1) \
         AND (?2 IS NULL OR a.actor_user_id = ?2) \
         AND (?3 IS NULL OR a.created_at >= ?3) \
         AND (?4 IS NULL OR a.created_at <= ?4)";
}

pub fn count_audit(conn: &Connection, filter: &AuditFilter) -> R<i64> {
    let sql = format!("SELECT count(*) FROM audit_logs a WHERE {}", AuditFilter::WHERE_CLAUSE);
    let n = conn.query_row(
        &sql,
        params![filter.action, filter.actor_user_id, filter.from.map(ts), filter.to.map(ts)],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(n)
}

/// 审计日志（按 id 倒序 = 最新的在前），并把操作者邮箱关联出来
pub fn list_audit(conn: &Connection, filter: &AuditFilter, limit: i64, offset: i64) -> R<Vec<AuditRow>> {
    let sql = format!(
        "SELECT a.id, a.actor_user_id, u.email AS actor_email, a.action, a.target, a.ip, \
                a.user_agent, a.detail, a.created_at \
         FROM audit_logs a LEFT JOIN users u ON u.id = a.actor_user_id \
         WHERE {} ORDER BY a.id DESC LIMIT ?5 OFFSET ?6",
        AuditFilter::WHERE_CLAUSE
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            filter.action,
            filter.actor_user_id,
            filter.from.map(ts),
            filter.to.map(ts),
            limit.clamp(1, 200),
            offset.max(0)
        ],
        audit_from_row,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 审计里出现过的动作清单（管理面板的筛选下拉用它）：
/// **不**让前端硬编码 action 字符串 —— 以后新增了动作，界面自动就有了。
pub fn distinct_audit_actions(conn: &Connection) -> R<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT action FROM audit_logs ORDER BY action")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 删掉 `cutoff` 之前的审计日志，返回删掉的条数（保留期见 `AUTH_AUDIT_RETENTION_DAYS`）
pub fn prune_audit_before(conn: &Connection, cutoff: OffsetDateTime) -> R<usize> {
    let n = conn.execute("DELETE FROM audit_logs WHERE created_at < ?1", params![ts(cutoff)])?;
    Ok(n)
}

fn audit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRow> {
    Ok(AuditRow {
        id: row.get("id")?,
        actor_user_id: row.get("actor_user_id")?,
        actor_email: row.get("actor_email")?,
        action: row.get("action")?,
        target: row.get("target")?,
        ip: row.get("ip")?,
        user_agent: row.get("user_agent")?,
        detail: row.get("detail")?,
        created_at: req_ts(row.get("created_at")?),
    })
}

// ---------------------------------------------------------------- 邀请码：按批停用 / 兑换记录

/// 按批停用：把这一批里**还没停用**的码全停掉，返回受影响的行数。
/// 已经是停用状态的不重复计数（幂等：再点一次返回 0）。
pub fn disable_invites_by_batch(conn: &Connection, batch_id: &str) -> R<usize> {
    let n = conn.execute(
        "UPDATE invite_codes SET disabled = 1 WHERE batch_id = ?1 AND disabled = 0",
        params![batch_id],
    )?;
    Ok(n)
}

/// 这一批总共有多少张码（含已停用 / 已用完）。
/// 用来区分「这批根本不存在」（接口回 404）与「这批已经全部停用了」（幂等回 0）。
pub fn count_invites_in_batch(conn: &Connection, batch_id: &str) -> R<i64> {
    let n = conn.query_row(
        "SELECT count(*) FROM invite_codes WHERE batch_id = ?1",
        params![batch_id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(n)
}

/// 按 id 取邀请码（「整批发邮件」只有 id、没有码，必须从库里读）
pub fn find_invite_by_id(conn: &Connection, id: i64) -> R<Option<InviteRow>> {
    let sql = format!("SELECT {INVITE_COLUMNS} FROM invite_codes WHERE id = ?1");
    Ok(conn.query_row(&sql, params![id], invite_from_row).optional()?)
}

/// 查这些邀请码的兑换记录（按兑换时间倒序，最新在前）。
/// 调用方自己按 `invite_code_id` 分组并截取要显示的条数 —— SQL 里做「每组取前 N 条」
/// 要么用窗口函数、要么写很别扭的相关子查询，不值得。
pub fn list_invite_uses(conn: &Connection, invite_ids: &[i64]) -> R<Vec<InviteUseRow>> {
    if invite_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=invite_ids.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT invite_code_id, user_id, email, ip, used_at FROM invite_uses \
         WHERE invite_code_id IN ({placeholders}) ORDER BY used_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(invite_ids.iter()), invite_use_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn invite_use_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InviteUseRow> {
    Ok(InviteUseRow {
        invite_code_id: row.get("invite_code_id")?,
        user_id: row.get("user_id")?,
        email: row.get("email")?,
        ip: row.get("ip")?,
        used_at: req_ts(row.get("used_at")?),
    })
}
