//! SQL 语句集合
//!
//! 所有函数都是「同步 + 收 `&Connection`」，读路径与 `SqliteStore::write`
//! 的事务里都能直接调用。列名用显式清单 + 按名取值：写错列名会当场报错，
//! 而不是静默串位。

use rusqlite::{params, Connection, OptionalExtension};
use time::OffsetDateTime;

use crate::db::{parse_ts, ts};
use crate::error::StoreError;
use crate::models::{EmailCodeRow, InviteRow, NewAudit, SessionRow, UserRow};

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

/// 列出用户（分页，可按角色/状态过滤，按 id 倒序）
pub fn list_users(
    conn: &Connection,
    role: Option<&str>,
    status: Option<&str>,
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
