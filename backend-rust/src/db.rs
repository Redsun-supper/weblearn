//! SQLite 连接、PRAGMA 与迁移
//!
//! 时间列的口径（重要）：**UTC、秒精度、固定宽度的 RFC3339 文本**
//! （`2026-09-15T15:53:22Z`）。SQLite 里比较时间靠字典序，一旦带上可变长度的
//! 小数秒（`12:00:00Z` vs `12:00:00.5Z`），顺序就会错。

use std::time::Duration;

use rusqlite::Connection;
use time::format_description::FormatItem;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// 写入用的固定格式（秒精度 + 字面量 `Z` 表示 UTC）
pub const TS_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

/// 解析用的格式（不含 `Z`）。
///
/// ⚠️ 为什么要拆成两个：`OffsetDateTime::parse` 要求格式里带**偏移量成分**
/// （`[offset_hour]` 之类），而我们的存储格式用字面量 `Z` 表示 UTC，
/// 所以先按 `PrimitiveDateTime` 解析，再 `assume_utc()`。
/// 直接拿 `TS_FORMAT` 去 `OffsetDateTime::parse` 会永远解析失败。
const TS_PARSE_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");

/// 迁移表：`(版本号, SQL)`，按版本号升序应用，用 `PRAGMA user_version` 记录进度
pub const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];

/// 时间 → 存储文本（UTC，秒精度）
pub fn ts(t: OffsetDateTime) -> String {
    t.to_offset(UtcOffset::UTC)
        .format(TS_FORMAT)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// 存储文本 → 时间（解析失败返回 None，调用方按「立刻过期」处理）
pub fn parse_ts(raw: &str) -> Option<OffsetDateTime> {
    let body = raw.trim().strip_suffix('Z')?;
    PrimitiveDateTime::parse(body, TS_PARSE_FORMAT).ok().map(|p| p.assume_utc())
}

/// 当前时刻的存储文本
pub fn now_ts() -> String {
    ts(OffsetDateTime::now_utc())
}

/// 打开连接并设置 PRAGMA
pub fn open_connection(path: &str) -> rusqlite::Result<Connection> {
    let conn = if path == ":memory:" {
        Connection::open_in_memory()?
    } else {
        Connection::open(path)?
    };
    // WAL 会返回一行结果，所以用 query_row 而不是 execute
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    Ok(conn)
}

/// 应用尚未执行的迁移
pub fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        tx.commit()?;
        tracing::info!(version, "已应用数据库迁移");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_text_is_fixed_width_and_round_trips() {
        let t = OffsetDateTime::from_unix_timestamp(1_760_000_000).unwrap();
        let text = ts(t);
        assert_eq!(text.len(), 20, "固定宽度才能安全地做字典序比较：{text}");
        assert!(text.ends_with('Z'));
        assert_eq!(parse_ts(&text).unwrap(), t);
    }

    #[test]
    fn lexicographic_order_matches_time_order() {
        let a = ts(OffsetDateTime::from_unix_timestamp(1_760_000_000).unwrap());
        let b = ts(OffsetDateTime::from_unix_timestamp(1_760_000_000 + 1).unwrap());
        assert!(a < b);
    }

    #[test]
    fn migration_is_idempotent() {
        let mut conn = open_connection(":memory:").unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0))
            .unwrap();
        assert!(n >= 7, "应当建出 users/sessions/invite_codes 等表，实际 {n}");
    }
}
