-- 广学账号系统 · 初始 schema（auth.db）
--
-- 约定：
--   1. 所有时间列都是 **UTC、秒精度、固定宽度的 RFC3339 文本**（2026-09-15T15:53:22Z）。
--      固定宽度是刻意的：SQLite 里 expires_at > ? 这种比较是**字典序**，
--      一旦带上可变长度的小数秒（12:00:00Z vs 12:00:00.5Z）顺序就会错乱。
--   2. role / status / user_identities 是**权限等级与多方式登录的预留位**，
--      本期不写权限判定逻辑（唯一例外：邀请码管理接口做最小 role='admin' 准入）。
--   3. 不给 email 单开索引：UNIQUE 约束已经自带索引。

-- 用户（密码只存 Argon2id PHC 串，永远不存明文）
CREATE TABLE IF NOT EXISTS users (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    email             TEXT    NOT NULL UNIQUE,
    username          TEXT,
    password_hash     TEXT    NOT NULL,
    role              TEXT    NOT NULL DEFAULT 'user',   -- 预留：user / admin / ...
    status            TEXT    NOT NULL DEFAULT 'active', -- 预留：active / disabled
    email_verified_at TEXT,
    failed_attempts   INTEGER NOT NULL DEFAULT 0,
    locked_until      TEXT,
    last_login_at     TEXT,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL
);

-- 登录标识（多方式登录预留位）：本期只写 provider='email'
CREATE TABLE IF NOT EXISTS user_identities (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT    NOT NULL,
    identifier  TEXT    NOT NULL,
    verified_at TEXT,
    created_at  TEXT    NOT NULL,
    UNIQUE (provider, identifier)
);
CREATE INDEX IF NOT EXISTS idx_identities_user ON user_identities(user_id);

-- 会话（一行 = 一个端的登录，多端并存）
-- refresh 令牌只存 SHA-256 摘要；family_id + replaced_by 记录轮换链，
-- 已轮换过的令牌再次出现即判定重放，整族吊销。
CREATE TABLE IF NOT EXISTS sessions (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    family_id      TEXT    NOT NULL,
    refresh_hash   TEXT    NOT NULL UNIQUE,
    device_label   TEXT    NOT NULL DEFAULT '',
    user_agent     TEXT    NOT NULL DEFAULT '',
    ip             TEXT    NOT NULL DEFAULT '',
    created_at     TEXT    NOT NULL,
    last_used_at   TEXT,
    expires_at     TEXT    NOT NULL,
    revoked_at     TEXT,
    revoked_reason TEXT,
    replaced_by    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_sessions_user   ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_family ON sessions(family_id);

-- 邀请码（管理员派发；准入凭据，一次性/限次 + 过期 + 可停用 + 留痕）
-- code 存明文：它是管理员需要核对与转发的短期凭据，不是长期机密。
CREATE TABLE IF NOT EXISTS invite_codes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    code       TEXT    NOT NULL UNIQUE,
    note       TEXT    NOT NULL DEFAULT '',
    max_uses   INTEGER NOT NULL DEFAULT 1,
    used_count INTEGER NOT NULL DEFAULT 0,
    expires_at TEXT,
    disabled   INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    created_at TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_invites_state ON invite_codes(disabled, used_count, expires_at);

-- 邀请码使用留痕
CREATE TABLE IF NOT EXISTS invite_uses (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    invite_code_id INTEGER NOT NULL REFERENCES invite_codes(id) ON DELETE CASCADE,
    user_id        INTEGER,
    email          TEXT    NOT NULL DEFAULT '',
    ip             TEXT    NOT NULL DEFAULT '',
    used_at        TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_invite_uses_code ON invite_uses(invite_code_id);

-- 邮箱验证码（只存 HMAC-SHA256 摘要；10 分钟有效、最多 5 次尝试）
CREATE TABLE IF NOT EXISTS email_codes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    email       TEXT    NOT NULL,
    purpose     TEXT    NOT NULL DEFAULT 'register',
    code_hash   TEXT    NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0,
    expires_at  TEXT    NOT NULL,
    consumed_at TEXT,
    created_at  TEXT    NOT NULL,
    ip          TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_email_codes_lookup ON email_codes(email, purpose, consumed_at);

-- 审计日志（密码、验证码、refresh 明文一律不写进来）
CREATE TABLE IF NOT EXISTS audit_logs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id INTEGER,
    action        TEXT    NOT NULL,
    target        TEXT    NOT NULL DEFAULT '',
    ip            TEXT    NOT NULL DEFAULT '',
    user_agent    TEXT    NOT NULL DEFAULT '',
    detail        TEXT    NOT NULL DEFAULT '',
    created_at    TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_audit_action  ON audit_logs(action);
