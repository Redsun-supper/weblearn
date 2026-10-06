// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 配置：环境变量 → `Config`
//!
//! 约定与 Go 侧保持一致（`APP_ENV` / 开发默认值），额外的前缀统一是 `AUTH_`。
//! ⚠️ 生产环境（`APP_ENV=production`）有两条硬性要求，不满足直接启动失败：
//!   1. 必须显式提供 `AUTH_JWT_SECRET`（否则每次重启密钥都变，且令牌可被伪造）；
//!   2. 必须显式提供 `AUTH_ADMIN_PASSWORD`（或显式关掉 `AUTH_SEED_ADMIN`），
//!      不允许把源码里的默认密码带上线。

use std::time::Duration;

use rand::RngCore;
use base64::Engine as _;

/// 初始管理员邮箱（需求指定）
pub const DEFAULT_ADMIN_EMAIL: &str = "2262997289@qq.com";
/// 初始管理员密码（**仅 development 允许作为默认值**）
pub const DEFAULT_ADMIN_PASSWORD: &str = "7289HR_RedSun";
/// 开发环境兜底密钥（生产必须覆盖）
const DEV_INSECURE_SECRET: &str = "guangxue-dev-insecure-secret";

/// 一条限流规则：`window` 时间内最多 `limit` 次；`limit == 0` 表示关闭
#[derive(Debug, Clone, Copy)]
pub struct RateRule {
    pub limit: usize,
    pub window: Duration,
}

impl RateRule {
    pub const fn new(limit: usize, window: Duration) -> Self {
        Self { limit, window }
    }
    pub const fn off() -> Self {
        Self { limit: 0, window: Duration::from_secs(0) }
    }
}

/// 限流配置（单进程内存实现，见 `rate_limit.rs`）
///
/// ⚠️ 每一条都能用 `AUTH_RL_*` 覆盖（P2，2026-10 才接上；在那之前这几个环境变量
/// 只出现在 `--help` 里、实际一行都没解析）。格式：`次数/窗口秒`，`0/0` 关闭。
#[derive(Debug, Clone, Copy)]
pub struct RateConfig {
    /// 登录：同 IP 15 分钟内的尝试次数（`AUTH_RL_LOGIN_IP`，默认 200）
    pub login_ip: RateRule,
    /// 发验证码：同邮箱 60 秒 1 次（`AUTH_RL_CODE_EMAIL_MINUTE`）
    pub code_email_minute: RateRule,
    /// 发验证码：同邮箱 1 小时 5 次（`AUTH_RL_CODE_EMAIL_HOUR`）
    pub code_email_hour: RateRule,
    /// 发验证码：同 IP 1 小时 200 次（`AUTH_RL_CODE_IP`）
    pub code_ip: RateRule,
    /// 注册：同 IP 1 小时 100 次（`AUTH_RL_REGISTER_IP`）
    pub register_ip: RateRule,
}

/// Argon2id 参数（默认即 OWASP 推荐值）
#[derive(Debug, Clone, Copy)]
pub struct Argon2Config {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailMode {
    /// 只打日志（并把验证码记在内存里，供 development 的调试接口读取）
    Log,
    /// 真发信（SMTP）
    Smtp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmtpTls {
    /// 25 端口那种明文 SMTP（只在明确知道自己在做什么时用）
    None,
    /// 587 端口那种「先明文、再 STARTTLS 升级」
    StartTls,
    /// 465 端口那种「一连上就是 TLS」（QQ / 163 邮箱都是这个）
    #[default]
    Implicit,
}

impl SmtpTls {
    /// 从配置值解析。`tls` 与 `implicit` 是同一个意思 —— 两种写法都在用
    ///（`.env.example` 与上线计划里写的是 `implicit`，代码默认值写的是 `tls`），
    /// 所以两个都收，其余一律报错：**未知值绝不能静默按「隐式 TLS」处理**，
    /// 否则配错模式的表现会变成「连接超时」这种查不出原因的症状。
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "tls" | "implicit" | "implicit_tls" | "ssl" => Ok(SmtpTls::Implicit),
            "starttls" | "start_tls" | "explicit" => Ok(SmtpTls::StartTls),
            "none" | "plain" | "insecure" => Ok(SmtpTls::None),
            other => Err(format!(
                "AUTH_SMTP_TLS 只能是 tls（= implicit，465 用）/ starttls（587 用）/ none（25 用），收到：{other}"
            )),
        }
    }

    /// 给启动横幅与日志看的名字
    pub fn as_str(self) -> &'static str {
        match self {
            SmtpTls::None => "none",
            SmtpTls::StartTls => "starttls",
            SmtpTls::Implicit => "implicit",
        }
    }

    /// 是否加密（选默认端口时用得上）
    pub fn is_encrypted(self) -> bool {
        !matches!(self, SmtpTls::None)
    }
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub tls: SmtpTls,
    pub timeout: Duration,
}

impl SmtpConfig {
    /// 校验 `AUTH_SMTP_FROM`。
    ///
    /// 为什么要在**启动时**查：`lettre` 要到「发第一封邮件」时才解析发件人，配错了的表现
    /// 会是「服务正常启动、用户点了发送才报错」—— 上线最不想遇到的失败方式。
    ///
    /// 这里只做**拒绝**、不做任何「纠正」：把发件人悄悄改成别的地址，比直接报错难查得多。
    /// 合法的两种写法都收：纯地址 `no-reply@example.com`，带显示名 `广学 <no-reply@example.com>`
    ///（中文显示名由 `lettre` 按 RFC 2047 编码，实测可用）。
    pub fn from_mailbox(raw: &str) -> Result<String, String> {
        use lettre::message::Mailbox;
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("AUTH_SMTP_FROM 不能是空的".to_string());
        }
        if raw.parse::<Mailbox>().is_ok() {
            return Ok(raw.to_string());
        }
        Err(format!(
            "AUTH_SMTP_FROM 不是合法邮箱：{raw}。可以写纯地址 no-reply@example.com，\
             或标准写法 广学 <no-reply@example.com>（显示名两侧要有 <>）"
        ))
    }
}

#[derive(Debug, Clone)]
pub struct MailConfig {
    pub mode: MailMode,
    pub smtp: Option<SmtpConfig>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// development / production（沿用 Go 侧的同名变量）
    pub env: String,
    pub host: String,
    pub port: u16,
    /// SQLite 文件路径（账号库独立于 Go 的 guangxue.db）
    pub db_path: String,
    pub jwt_secret: Vec<u8>,
    pub code_pepper: Vec<u8>,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    /// 邮箱验证码有效期
    pub code_ttl: Duration,
    /// 邮箱验证码最多尝试次数
    pub code_max_attempts: i64,
    pub cookie_secure: bool,
    pub cookie_domain: Option<String>,
    /// CSRF：允许的来源白名单
    pub allowed_origins: Vec<String>,
    pub mail: MailConfig,
    /// 是否注册「仅开发」的调试接口（生产环境路由根本不注册）
    pub dev_endpoints: bool,
    /// 是否**强制邀请码注册**（P0-2）
    ///
    /// 打开时「邀请制」才真正成立：没有有效邀请码，`email-code` 直接拒绝、
    /// **一个验证码都不会写库、也不会发信**（这一步很关键 —— 先发码后校验等于没拦）。
    /// 默认 false（本地开发保持开放注册方便），生产用 `AUTH_REQUIRE_INVITE=true` 打开。
    ///
    /// 为什么不写死：用户已确认「未来删档重来后会改成不强制、填了才带权限」，
    /// 那时把这个开关关掉即可，注册流程代码不用再改一遍。
    pub require_invite: bool,
    pub admin_email: String,
    pub admin_password: Option<String>,
    pub seed_admin: bool,
    pub argon2: Argon2Config,
    pub rate: RateConfig,
    /// 连续失败多少次锁定账号
    pub lock_threshold: i64,
    /// 锁定时长（分钟）
    pub lock_minutes: i64,
    /// 审计日志保留天数（P1）：启动时清一次，之后每 24 小时清一次；`0` = 永不清理
    ///
    /// 为什么不设成 `Option`：默认值 180 天是「已经和用户定过」的口径，
    /// 想永久保留就显式写 0 —— 配置里看得见，比一个隐含的「没配就不清理」好排查。
    pub audit_retention_days: i64,
    pub jwt_secret_is_default: bool,
    pub admin_password_is_default: bool,
}

impl Config {
    /// 全默认的开发配置（`from_env` 也以它为基底）。
    /// 单元/集成测试直接改字段即可，不必碰环境变量。
    pub fn development() -> Self {
        Self {
            env: "development".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8081,
            db_path: "auth.db".to_string(),
            jwt_secret: DEV_INSECURE_SECRET.as_bytes().to_vec(),
            code_pepper: DEV_INSECURE_SECRET.as_bytes().to_vec(),
            access_ttl: Duration::from_secs(900),
            refresh_ttl: Duration::from_secs(30 * 24 * 3600),
            code_ttl: Duration::from_secs(600),
            code_max_attempts: 5,
            cookie_secure: false,
            cookie_domain: None,
            allowed_origins: vec![
                "http://127.0.0.1:8899".to_string(),
                "http://localhost:8899".to_string(),
            ],
            mail: MailConfig { mode: MailMode::Log, smtp: None },
            dev_endpoints: true,
            // 开发默认**不**强制邀请码：本地调试要能随手注册账号。
            // 生产由 `AUTH_REQUIRE_INVITE=true` 打开（见 docs/launch-plan.md 的 P0-2）。
            require_invite: false,
            admin_email: DEFAULT_ADMIN_EMAIL.to_string(),
            admin_password: Some(DEFAULT_ADMIN_PASSWORD.to_string()),
            seed_admin: true,
            argon2: Argon2Config { m_cost: 19456, t_cost: 2, p_cost: 1 },
            rate: RateConfig {
                // P2（2026-10）按「上百人」重算，口径是**严在账号、宽在 IP**：
                // IP 是共享资源 —— 校园 / 公司 / 运营商 CGNAT 后面可能站着几十上百个用户，
                // 按「一个人一台机器」算出来的 IP 额度，落到共享出口上会在早高峰
                // 被自己人打满（症状是整栋楼一起收到 429，却没人做错什么）。
                // 真正要卡死的是**账号维度**：验证码每邮箱 1 分钟 1 封 / 1 小时 5 封、
                // 密码连错 5 次锁 15 分钟 —— 那几条保持原值不动。
                // 这几条都能用 AUTH_RL_* 覆盖（格式 `次数/窗口秒`，`0/0` 关闭）。
                login_ip: RateRule::new(200, Duration::from_secs(15 * 60)),
                code_email_minute: RateRule::new(1, Duration::from_secs(60)),
                code_email_hour: RateRule::new(5, Duration::from_secs(3600)),
                code_ip: RateRule::new(200, Duration::from_secs(3600)),
                register_ip: RateRule::new(100, Duration::from_secs(3600)),
            },
            lock_threshold: 5,
            lock_minutes: 15,
            // 审计日志保留 180 天（P1 与用户定案；0 表示不清理）
            audit_retention_days: 180,
            jwt_secret_is_default: true,
            admin_password_is_default: true,
        }
    }

    pub fn is_production(&self) -> bool {
        self.env.eq_ignore_ascii_case("production")
    }

    pub fn is_dev(&self) -> bool {
        !self.is_production()
    }

    pub fn access_ttl_seconds(&self) -> i64 {
        self.access_ttl.as_secs() as i64
    }

    pub fn refresh_ttl_days(&self) -> i64 {
        (self.refresh_ttl.as_secs() / 86400) as i64
    }

    /// 从环境变量加载（会先读同目录 `.env`，方便本地开发）
    pub fn from_env() -> Result<Self, String> {
        let _ = dotenvy::dotenv();
        Self::from_lookup(&|key| std::env::var(key).ok())
    }

    /// 真正的解析逻辑：环境变量从 `lookup` 里取。
    /// 之所以把它与环境变量解耦，是为了**能测**——`std::env::set_var` 在 Rust 2024 起是
    /// unsafe 的（多线程下改环境有数据竞争），测试里塞一张表进来就没这个问题。
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut cfg = Config::development();
        cfg.env = env_str(lookup, "APP_ENV", &cfg.env);
        cfg.host = env_str(lookup, "AUTH_HOST", &cfg.host);
        cfg.port = env_parse(lookup, "AUTH_PORT", cfg.port)?;
        cfg.db_path = env_str(lookup, "AUTH_DB_PATH", &cfg.db_path);
        cfg.access_ttl =
            Duration::from_secs(env_parse(lookup, "AUTH_ACCESS_TTL_SECONDS", cfg.access_ttl.as_secs())?);
        cfg.refresh_ttl = Duration::from_secs(
            env_parse(lookup, "AUTH_REFRESH_TTL_DAYS", cfg.refresh_ttl_days())? as u64 * 86400,
        );
        cfg.code_ttl = Duration::from_secs(env_parse(lookup, "AUTH_CODE_TTL_SECONDS", cfg.code_ttl.as_secs())?);
        cfg.argon2 = Argon2Config {
            m_cost: env_parse(lookup, "AUTH_ARGON2_M_COST", cfg.argon2.m_cost)?,
            t_cost: env_parse(lookup, "AUTH_ARGON2_T_COST", cfg.argon2.t_cost)?,
            p_cost: env_parse(lookup, "AUTH_ARGON2_P_COST", cfg.argon2.p_cost)?,
        };
        cfg.lock_threshold = env_parse_i64(lookup, "AUTH_LOCK_THRESHOLD", cfg.lock_threshold)?;
        cfg.lock_minutes = env_parse_i64(lookup, "AUTH_LOCK_MINUTES", cfg.lock_minutes)?;
        // P2：限流规则接进配置。此前 `--help` 里列着 AUTH_RL_*，但**一行解析都没有** ——
        // 「按上百人重算限流」只能改代码发版；上线后想临时放宽也得重新编译。
        cfg.rate.login_ip = env_rate_rule(lookup, "AUTH_RL_LOGIN_IP", cfg.rate.login_ip)?;
        cfg.rate.code_email_minute =
            env_rate_rule(lookup, "AUTH_RL_CODE_EMAIL_MINUTE", cfg.rate.code_email_minute)?;
        cfg.rate.code_email_hour =
            env_rate_rule(lookup, "AUTH_RL_CODE_EMAIL_HOUR", cfg.rate.code_email_hour)?;
        cfg.rate.code_ip = env_rate_rule(lookup, "AUTH_RL_CODE_IP", cfg.rate.code_ip)?;
        cfg.rate.register_ip = env_rate_rule(lookup, "AUTH_RL_REGISTER_IP", cfg.rate.register_ip)?;

        // 密钥：生产必须显式提供；开发缺失则随机生成（重启后旧令牌失效，并打印告警）
        match env_opt(lookup, "AUTH_JWT_SECRET") {
            Some(secret) => {
                cfg.jwt_secret = secret.into_bytes();
                cfg.jwt_secret_is_default = false;
            }
            None => {
                if cfg.is_production() {
                    return Err("生产环境必须提供 AUTH_JWT_SECRET（一串足够长的随机值）".to_string());
                }
                let mut buf = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut buf);
                cfg.jwt_secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf).into_bytes();
                cfg.jwt_secret_is_default = true;
            }
        }
        cfg.code_pepper = match env_opt(lookup, "AUTH_CODE_PEPPER") {
            Some(p) => p.into_bytes(),
            None => cfg.jwt_secret.clone(),
        };

        // Cookie
        let default_secure = cfg.is_production();
        cfg.cookie_secure = env_parse_bool(lookup, "AUTH_COOKIE_SECURE", default_secure)?;
        cfg.cookie_domain = env_opt(lookup, "AUTH_COOKIE_DOMAIN");
        if let Some(list) = env_opt(lookup, "AUTH_ALLOWED_ORIGINS") {
            cfg.allowed_origins = list
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        // 邮件
        let mode = env_str(lookup, "AUTH_MAIL_MODE", "log").to_ascii_lowercase();
        cfg.mail = match mode.as_str() {
            "log" => MailConfig { mode: MailMode::Log, smtp: None },
            "smtp" => {
                // ⚠️ SMTP 这几项**只去两侧空白，不把「只有一个空格」当成没配**：
                // 授权码有可能真的带空格，而被静默 trim 掉的密码会变成一个
                // 完全误导人的「认证失败」。
                let host = env_opt_keep_inner(lookup, "AUTH_SMTP_HOST")
                    .ok_or_else(|| "AUTH_MAIL_MODE=smtp 时必须提供 AUTH_SMTP_HOST".to_string())?;
                let username = env_opt_keep_inner(lookup, "AUTH_SMTP_USERNAME")
                    .ok_or_else(|| "AUTH_MAIL_MODE=smtp 时必须提供 AUTH_SMTP_USERNAME".to_string())?;
                let password = env_opt_keep_inner(lookup, "AUTH_SMTP_PASSWORD").ok_or_else(|| {
                    "AUTH_MAIL_MODE=smtp 时必须提供 AUTH_SMTP_PASSWORD（邮箱 SMTP 授权码）".to_string()
                })?;
                let from = env_opt(lookup, "AUTH_SMTP_FROM").unwrap_or_else(|| username.clone());
                // 发件人地址在这里就验一遍：`lettre` 要到**发第一封邮件时**才会解析它，
                // 那意味着哪怕配错了也是「服务正常启动、用户点发送才失败」——
                // 这正是上线最不想遇到的失败方式（见 SmtpConfig::from_mailbox）。
                let from = SmtpConfig::from_mailbox(&from)?;
                let tls = SmtpTls::parse(&env_str(lookup, "AUTH_SMTP_TLS", "tls"))?;
                MailConfig {
                    mode: MailMode::Smtp,
                    smtp: Some(SmtpConfig {
                        host,
                        port: env_parse(lookup, "AUTH_SMTP_PORT", default_smtp_port(tls))?,
                        username,
                        password,
                        from,
                        tls,
                        timeout: Duration::from_secs(env_parse(lookup, "AUTH_SMTP_TIMEOUT_SECONDS", 15u64)?),
                    }),
                }
            }
            other => return Err(format!("AUTH_MAIL_MODE 只能是 log 或 smtp，收到：{other}")),
        };

        cfg.dev_endpoints = env_parse_bool(lookup, "AUTH_DEV_ENDPOINTS", cfg.is_dev())?;
        if cfg.is_production() && cfg.dev_endpoints {
            return Err("生产环境不允许开启 AUTH_DEV_ENDPOINTS（调试接口会泄露验证码）".to_string());
        }

        // P0-2：强制邀请码注册。默认 false（本地开发方便），生产用 AUTH_REQUIRE_INVITE=true 打开。
        // ⚠️ 刻意**不**做成「生产默认 true」：默认值必须是显式的，
        // 否则「忘了配」与「故意关掉」在配置里长得一模一样。
        cfg.require_invite = env_parse_bool(lookup, "AUTH_REQUIRE_INVITE", cfg.require_invite)?;

        // P1：审计日志保留天数（默认 180 天；0 表示永不清理）
        cfg.audit_retention_days =
            env_parse_i64(lookup, "AUTH_AUDIT_RETENTION_DAYS", cfg.audit_retention_days)?;
        if cfg.audit_retention_days < 0 {
            return Err(
                "AUTH_AUDIT_RETENTION_DAYS 不能是负数（0 表示不清理，默认 180）".to_string()
            );
        }

        // 管理员
        cfg.admin_email = env_str(lookup, "AUTH_ADMIN_EMAIL", &cfg.admin_email);
        cfg.admin_password = match env_opt(lookup, "AUTH_ADMIN_PASSWORD") {
            Some(p) => {
                cfg.admin_password_is_default = p == DEFAULT_ADMIN_PASSWORD;
                if cfg.is_production() && cfg.admin_password_is_default {
                    return Err("生产环境不允许使用默认管理员密码，请设置 AUTH_ADMIN_PASSWORD".to_string());
                }
                Some(p)
            }
            None => {
                cfg.admin_password_is_default = false;
                if cfg.is_production() {
                    None
                } else {
                    // 开发环境用需求里指定的默认密码，开箱即可登录
                    cfg.admin_password_is_default = true;
                    Some(DEFAULT_ADMIN_PASSWORD.to_string())
                }
            }
        };
        cfg.seed_admin = env_parse_bool(lookup, "AUTH_SEED_ADMIN", cfg.is_dev())?;
        if cfg.seed_admin && cfg.admin_password.is_none() {
            return Err("AUTH_SEED_ADMIN=true 时必须提供 AUTH_ADMIN_PASSWORD".to_string());
        }

        Ok(cfg)
    }
}

/// 没配 `AUTH_SMTP_PORT` 时按加密方式挑默认端口：隐式 TLS 用 465、STARTTLS 用 587、
/// 明文用 25。按端口判据挑也行，但那样「465 + starttls」这种组合会被默默改成别的端口，
/// 反而看不出配错。
fn default_smtp_port(tls: SmtpTls) -> u16 {
    match tls {
        SmtpTls::Implicit => 465,
        SmtpTls::StartTls => 587,
        SmtpTls::None => 25,
    }
}

fn env_opt(lookup: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 与 `env_opt` 的区别：**只去两侧空白，空串仍算「配了」**。
/// 给 SMTP 凭据用 —— 见 `from_lookup` 里的说明。
fn env_opt_keep_inner(lookup: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key).map(|v| v.trim().to_string())
}

fn env_str(lookup: &dyn Fn(&str) -> Option<String>, key: &str, default: &str) -> String {
    env_opt(lookup, key).unwrap_or_else(|| default.to_string())
}

fn env_parse<T: std::str::FromStr>(
    lookup: &dyn Fn(&str) -> Option<String>,
    key: &str,
    default: T,
) -> Result<T, String> {
    match env_opt(lookup, key) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<T>()
            .map_err(|_| format!("环境变量 {key} 的值无法解析：{raw}")),
    }
}

fn env_parse_i64(lookup: &dyn Fn(&str) -> Option<String>, key: &str, default: i64) -> Result<i64, String> {
    env_parse(lookup, key, default)
}

/// 解析一条限流规则：`次数/窗口秒`（例如 `200/900` = 15 分钟内最多 200 次）。
///
/// * 没配 → 用 `default`；
/// * `0/0`（或**任一侧**写 0）→ 关闭这条规则，与 `RateRule::off()` 一致。
///   任一侧为 0 都当「关闭」是有意的：写 `0/60` 的人想表达的显然是「别限了」，
///   而不是「窗口 60 秒、一次都不许」—— 后者会让对应接口彻底不可用；
/// * 格式写错 → **拒绝启动**并点名配置项。静默退回默认值的症状是
///   「明明放宽了却还被 429」，而配置文件看上去完全正常，这种问题最难查。
///
/// 为什么用 `次数/窗口秒` 这种自定义格式而不是 `AUTH_RL_X_LIMIT` + `AUTH_RL_X_WINDOW`
/// 两个变量：五条规则 × 两个变量 = 十个环境变量，读的人要在脑子里配对；
/// 一行 `200/900` 自带「多少次、多久内」，抄进 ticket 里也不会丢一半。
fn env_rate_rule(
    lookup: &dyn Fn(&str) -> Option<String>,
    key: &str,
    default: RateRule,
) -> Result<RateRule, String> {
    let Some(raw) = env_opt_keep_inner(lookup, key) else {
        return Ok(default);
    };
    let bad = || {
        format!(
            "{key} 格式应为「次数/窗口秒」，例如 200/900（= 15 分钟内最多 200 次）；\
             0/0 表示关闭这条限流。收到：{raw}"
        )
    };
    let (limit_part, window_part) = raw.split_once('/').ok_or_else(|| bad())?;
    let limit: usize = limit_part.trim().parse().map_err(|_| bad())?;
    let window_secs: u64 = window_part.trim().parse().map_err(|_| bad())?;
    if limit == 0 || window_secs == 0 {
        return Ok(RateRule::off());
    }
    Ok(RateRule::new(limit, Duration::from_secs(window_secs)))
}

fn env_parse_bool(
    lookup: &dyn Fn(&str) -> Option<String>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match env_opt(lookup, key) {
        None => Ok(default),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => Err(format!("环境变量 {key} 只能是 true/false，收到：{other}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 拿一张表当环境变量用：`from_lookup` 就是为这个解耦的
    ///（`std::env::set_var` 在 Rust 2024 起是 unsafe 的，测试里改环境有数据竞争）。
    /// ⚠️ 同名键**后写的赢**，这样用例可以先铺一份 smtp_base() 再逐项覆盖。
    fn from_pairs(pairs: &[(&str, &str)]) -> Result<Config, String> {
        let pairs: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_lookup(&move |key| {
            pairs.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        })
    }

    /// 一份绕过邮件校验的最小 SMTP 配置（各用例在此基础上加变量）
    fn smtp_base() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AUTH_MAIL_MODE", "smtp"),
            ("AUTH_SMTP_HOST", "smtp.qq.com"),
            ("AUTH_SMTP_USERNAME", "no-reply@qq.com"),
            ("AUTH_SMTP_PASSWORD", "授权码不是登录密码"),
        ]
    }

    fn smtp_of(cfg: &Config) -> &SmtpConfig {
        cfg.mail.smtp.as_ref().expect("smtp 模式下必然有配置")
    }

    // ---------------------------------------------------------------- P2：限流规则

    #[test]
    fn rate_rules_default_to_the_tuned_values() {
        let cfg = from_pairs(&[]).expect("默认配置要能加载");
        // 口径「严在账号、宽在 IP」：IP 是共享资源（校园 / 公司 / CGNAT 出口），
        // 账号（邮箱）才是身份
        assert_eq!(cfg.rate.login_ip.limit, 200);
        assert_eq!(cfg.rate.login_ip.window, Duration::from_secs(900));
        assert_eq!(cfg.rate.code_email_minute.limit, 1);
        assert_eq!(cfg.rate.code_email_hour.limit, 5);
        assert_eq!(cfg.rate.code_ip.limit, 200);
        assert_eq!(cfg.rate.register_ip.limit, 100);
    }

    #[test]
    fn rate_rules_can_be_overridden_by_env() {
        let cfg = from_pairs(&[
            ("AUTH_RL_LOGIN_IP", "50/300"),
            ("AUTH_RL_CODE_IP", " 10 / 600 "), // 两侧空格要能容忍：从 ticket 里抄来的值常带空格
        ])
        .expect("加载");
        assert_eq!(cfg.rate.login_ip.limit, 50);
        assert_eq!(cfg.rate.login_ip.window, Duration::from_secs(300));
        assert_eq!(cfg.rate.code_ip.limit, 10);
        assert_eq!(cfg.rate.code_ip.window, Duration::from_secs(600));
        // 没写的那几条保持默认，互不影响
        assert_eq!(cfg.rate.code_email_hour.limit, 5);
        assert_eq!(cfg.rate.register_ip.limit, 100);
    }

    #[test]
    fn rate_rule_zero_means_disabled() {
        let cfg = from_pairs(&[("AUTH_RL_CODE_EMAIL_MINUTE", "0/0")]).expect("0/0 是合法的「关闭」");
        assert_eq!(cfg.rate.code_email_minute.limit, 0);
        // 任一侧写 0 都当关闭：写 `0/60` 的人想说的显然是「别限了」，
        // 而不是「窗口 60 秒、一次都不许」—— 后者会让接口彻底不可用
        let cfg = from_pairs(&[("AUTH_RL_REGISTER_IP", "0/60")]).expect("0/60 也合法");
        assert_eq!(cfg.rate.register_ip.limit, 0);
    }

    #[test]
    fn rate_rule_rejects_bad_format_and_names_the_variable() {
        for bad in ["200", "200/", "/900", "很多/900", "200/很久", "200-900", "200/-1"] {
            let err = match from_pairs(&[("AUTH_RL_LOGIN_IP", bad)]) {
                Ok(_) => panic!("「{bad}」应当被拒绝，却加载成功了"),
                Err(e) => e,
            };
            assert!(err.contains("AUTH_RL_LOGIN_IP"), "报错要点名配置项：{err}");
            assert!(err.contains("次数/窗口秒"), "报错要说清正确格式：{err}");
        }
    }

    #[test]
    fn mail_mode_defaults_to_log() {
        let cfg = from_pairs(&[]).expect("默认配置要能加载");
        assert_eq!(cfg.mail.mode, MailMode::Log);
        assert!(cfg.mail.smtp.is_none(), "log 模式不该带 SMTP 配置");
    }

    #[test]
    fn smtp_mode_requires_host_username_password() {
        // 三个必需项各缺一次，报错要点名是哪一项 —— 否则部署时只能靠猜
        for (missing, expected) in [
            ("AUTH_SMTP_HOST", "AUTH_SMTP_HOST"),
            ("AUTH_SMTP_USERNAME", "AUTH_SMTP_USERNAME"),
            ("AUTH_SMTP_PASSWORD", "AUTH_SMTP_PASSWORD"),
        ] {
            let pairs: Vec<(&str, &str)> =
                smtp_base().into_iter().filter(|(k, _)| *k != missing).collect();
            let err = from_pairs(&pairs).expect_err("缺 {missing} 时必须报错");
            assert!(err.contains(expected), "报错要点名缺的是 {missing}，实际：{err}");
        }
    }

    #[test]
    fn smtp_from_defaults_to_username_and_accepts_display_name() {
        // 不配 AUTH_SMTP_FROM 时用登录账号当发件人（多数邮箱只允许这样）
        let cfg = from_pairs(&smtp_base()).expect("加载");
        assert_eq!(smtp_of(&cfg).from, "no-reply@qq.com");

        // 标准写法（纯地址）与带显示名的写法都要能用
        for from in ["广学 <no-reply@qq.com>", "Guangxue <no-reply@qq.com>", "no-reply@qq.com"] {
            let mut pairs = smtp_base();
            pairs.push(("AUTH_SMTP_FROM", from));
            let cfg = from_pairs(&pairs).unwrap_or_else(|e| panic!("{from} 应当合法，却报：{e}"));
            assert_eq!(smtp_of(&cfg).from, from);
        }
    }

    #[test]
    fn smtp_from_rejects_bad_mailbox_at_startup() {
        // ⚠️ 这条是重点：lettre 要到发第一封信时才解析发件人，配错了会变成
        //「服务正常启动、用户点发送才失败」—— 所以必须在加载配置时就拦住
        for bad in ["这不是邮箱", "no-reply@", "<no-reply@qq.com", "no-reply@qq.com>", "@qq.com"] {
            let mut pairs = smtp_base();
            pairs.push(("AUTH_SMTP_FROM", bad));
            let err = from_pairs(&pairs)
                .err()
                .unwrap_or_else(|| panic!("{bad} 应当被判为非法发件人"));
            assert!(err.contains("AUTH_SMTP_FROM"), "报错要点名配置项：{err}");
        }
    }

    #[test]
    fn smtp_tls_accepts_documented_spellings() {
        for (raw, want) in [
            ("tls", SmtpTls::Implicit),
            ("implicit", SmtpTls::Implicit),      // 上线计划与 .env.example 里写的是这个
            ("IMPLICIT", SmtpTls::Implicit),      // 大小写不敏感
            (" starttls ", SmtpTls::StartTls),    // 两侧空格要容忍（.env 里很常见）
            ("none", SmtpTls::None),
        ] {
            let mut pairs = smtp_base();
            pairs.push(("AUTH_SMTP_TLS", raw));
            let cfg = from_pairs(&pairs).unwrap_or_else(|e| panic!("{raw} 应当合法，却报：{e}"));
            assert_eq!(smtp_of(&cfg).tls, want, "AUTH_SMTP_TLS={raw} 解析错了");
        }
    }

    #[test]
    fn smtp_tls_rejects_unknown_value_instead_of_guessing() {
        // 未知值**不能**静默按隐式 TLS 处理：如果本意是 587 + starttls，
        // 静默走隐式 TLS 的表现是「连接超时」，根本查不出配错了
        let err = SmtpTls::parse("ssl-on").expect_err("未知值必须报错");
        assert!(err.contains("starttls"), "报错要把可选项列出来：{err}");
        assert!(err.contains("implicit"), "报错要把可选项列出来：{err}");

        let mut pairs = smtp_base();
        pairs.push(("AUTH_SMTP_TLS", "ssl-on"));
        assert!(from_pairs(&pairs).is_err(), "非法 TLS 模式必须启动即失败");
    }

    #[test]
    fn smtp_port_defaults_follow_the_tls_mode() {
        // 465 隐式 TLS 是默认（QQ / 163 都是它）
        let cfg = from_pairs(&smtp_base()).expect("加载");
        assert_eq!(smtp_of(&cfg).port, 465);

        // 换成 starttls / none 时默认端口跟着变 —— 但**显式写了端口就以显式为准**
        for (raw, want) in [("starttls", 587u16), ("none", 25)] {
            let mut pairs = smtp_base();
            pairs.push(("AUTH_SMTP_TLS", raw));
            let cfg = from_pairs(&pairs).expect("加载");
            assert_eq!(smtp_of(&cfg).port, want, "AUTH_SMTP_TLS={raw} 的默认端口不对");
        }

        let mut pairs = smtp_base();
        pairs.push(("AUTH_SMTP_TLS", "starttls"));
        pairs.push(("AUTH_SMTP_PORT", "2525")); // 自建中继常用非标准端口
        let cfg = from_pairs(&pairs).expect("加载");
        assert_eq!(smtp_of(&cfg).port, 2525, "显式端口不该被默认值覆盖");
    }

    #[test]
    fn smtp_port_and_timeout_must_be_numbers() {
        let mut pairs = smtp_base();
        pairs.push(("AUTH_SMTP_PORT", "465端口"));
        let err = from_pairs(&pairs).expect_err("端口不是数字要报错");
        assert!(err.contains("AUTH_SMTP_PORT"), "报错要点名配置项：{err}");

        let mut pairs = smtp_base();
        pairs.push(("AUTH_SMTP_TIMEOUT_SECONDS", "很快"));
        let err = from_pairs(&pairs).expect_err("超时不是数字要报错");
        assert!(err.contains("AUTH_SMTP_TIMEOUT_SECONDS"), "报错要点名配置项：{err}");
    }

    #[test]
    fn mail_mode_typo_is_reported_with_the_received_value() {
        let err = from_pairs(&[("AUTH_MAIL_MODE", "smtps")]).expect_err("拼错要报错");
        assert!(err.contains("smtps"), "报错要带上收到的值，方便看出拼错在哪：{err}");
    }

    #[test]
    fn smtp_credentials_are_trimmed_but_never_dropped() {
        // 授权码是「配了就用」，两边空格去掉 —— 从 .env 复制粘贴时最容易带上空格，
        // 而带空格的密码只会报「认证失败」，查起来毫无线索
        let mut pairs = smtp_base();
        pairs.push(("AUTH_SMTP_PASSWORD", "  授权码  "));
        let cfg = from_pairs(&pairs).expect("加载");
        assert_eq!(smtp_of(&cfg).password, "授权码");

        // ⚠️ 但「只有一个空格」不会被当成没配 —— 那属于显式配置，静默替换成默认值
        //    比让它去登录失败更难查
        let mut pairs = smtp_base();
        pairs.push(("AUTH_SMTP_PASSWORD", " "));
        let cfg = from_pairs(&pairs).expect("显式给了值就不该报「没配」");
        assert_eq!(smtp_of(&cfg).password, "");
    }

    /// **端到端地过一遍 `.env` 文件**（不是查表）：这条是给现场排查用的。
    ///
    /// `dotenvy` 对「值里带空格 / 引号 / `<` `>` / 非 ASCII」是自己一套解析规则，
    /// 上面那些 `from_pairs` 用例全都绕过了它。真机上踩到的坑正在这里：
    /// `AUTH_SMTP_FROM=广学 <no-reply@qq.com>` **不带引号**时，dotenvy 会把这一行整个丢掉
    ///（**不报错**），于是发件人静默退化成登录账号 —— 「配置看起来生效了、其实没有」。
    ///
    /// ⚠️ 刻意用 `dotenvy::from_path_iter` 而**不是** `Config::from_env()`：
    /// `dotenvy::dotenv()` 只把键写进进程环境、并且**不覆盖已存在的键**，
    /// 于是「哪个测试先跑」会决定结果（`dotenvy` 是进程级全局状态，与其他用例并行时会互相干扰）。
    /// `from_path_iter` 只解析文件、不碰环境变量，测的就是「dotenvy 怎么读这个文件」。
    fn load_dotenv_file(text: &str) -> Config {
        let dir = tempfile::tempdir().expect("建临时目录");
        let path = dir.path().join(".env");
        std::fs::write(&path, text).expect("写 .env");
        let pairs: Vec<(String, String)> = dotenvy::from_path_iter(&path)
            .expect("解析 .env")
            .filter_map(|item| item.ok())
            .collect();
        Config::from_lookup(&|key| {
            pairs.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        })
        .expect("从 .env 加载")
    }

    #[test]
    fn from_env_end_to_end_quoted_and_unquoted() {
        let base = "AUTH_MAIL_MODE=smtp\n\
                    AUTH_SMTP_HOST=smtp.qq.com\n\
                    AUTH_SMTP_USERNAME=no-reply@qq.com\n";

        // ① 按 `.env.example` 推荐的写法（带引号）：值要原样保留
        let quoted = load_dotenv_file(&format!(
            "{base}AUTH_SMTP_PASSWORD=\"授权码 带空格\"\n\
             AUTH_SMTP_FROM=\"广学 <no-reply@qq.com>\"\n\
             AUTH_SMTP_TIMEOUT_SECONDS=8\n"
        ));
        let smtp = smtp_of(&quoted);
        assert_eq!(smtp.password, "授权码 带空格", "带引号的值被 dotenvy 解错了");
        assert_eq!(smtp.from, "广学 <no-reply@qq.com>", "引号里的中文显示名不该丢");
        assert_eq!(smtp.port, 465);
        assert_eq!(smtp.timeout, Duration::from_secs(8), "数字项要能读到");

        // ② 不带引号的显示名：断言的是「**丢了**」—— 这就是必须加引号的原因。
        //    哪天 dotenvy 改成支持（或改成报错），这条会红：那时该更新 `.env.example`，而不是删用例。
        let unquoted = load_dotenv_file(&format!(
            "{base}AUTH_SMTP_PASSWORD=授权码\n\
             AUTH_SMTP_FROM=广学 <no-reply@qq.com>\n"
        ));
        assert_eq!(
            smtp_of(&unquoted).from,
            "no-reply@qq.com",
            "dotenvy 的行为变了：要么开始支持不带引号的显示名，要么改成报错"
        );
    }

    // ---------------------------------------------------------------- 生产环境开关（P0-4）

    /// 生产配置的基底：显式密钥 + 强口令，再逐条加变量
    fn prod_base() -> Vec<(&'static str, &'static str)> {
        vec![
            ("APP_ENV", "production"),
            ("AUTH_JWT_SECRET", "生产用的足够长的随机密钥"),
            ("AUTH_ADMIN_PASSWORD", "不是默认口令的强口令123"),
        ]
    }

    #[test]
    fn production_forces_cookie_secure() {
        // 生产必须默认给 Cookie 加 Secure —— 忘了配的表现是「HTTPS 站点上登录态
        // 在刷新后消失」，而且浏览器不会报任何错，只在控制台留一行警告
        let cfg = from_pairs(&prod_base()).expect("生产配置要能加载");
        assert!(cfg.is_production());
        assert!(cfg.cookie_secure, "生产必须默认 AUTH_COOKIE_SECURE=true");
    }

    #[test]
    fn production_without_explicit_config_still_enables_secure_cookie() {
        // 不显式写 AUTH_COOKIE_SECURE 也一样（默认值跟着 APP_ENV 走）
        let mut pairs = prod_base();
        pairs.push(("AUTH_COOKIE_SECURE", "true")); // 显式写一遍也不该出错
        let cfg = from_pairs(&pairs).expect("加载");
        assert!(cfg.cookie_secure);
    }

    #[test]
    fn production_rejects_dev_endpoints() {
        // 调试接口会把验证码明文吐出来，生产开着等于把注册流程交出去
        let mut pairs = prod_base();
        pairs.push(("AUTH_DEV_ENDPOINTS", "true"));
        let err = from_pairs(&pairs).expect_err("生产开 dev 接口必须启动即失败");
        assert!(err.contains("AUTH_DEV_ENDPOINTS"), "报错要点名配置项：{err}");
    }

    #[test]
    fn production_rejects_the_default_admin_password() {
        // 默认口令写在仓库里、人人可见
        let mut pairs = prod_base();
        pairs.push(("AUTH_ADMIN_PASSWORD", "7289HR_RedSun")); // DEFAULT_ADMIN_PASSWORD
        let err = from_pairs(&pairs).expect_err("生产用默认管理员口令必须启动即失败");
        assert!(err.contains("AUTH_ADMIN_PASSWORD"), "报错要点名配置项：{err}");

        // 同一份配置在开发环境是允许的（开箱即登录），否则本地开发会很烦
        let dev = from_pairs(&[("AUTH_ADMIN_PASSWORD", "7289HR_RedSun")]).expect("开发环境允许默认口令");
        assert!(dev.admin_password_is_default);
    }

    #[test]
    fn production_requires_an_explicit_jwt_secret() {
        // 随机生成的密钥重启就变，所有已发出的令牌当场失效（表现为「刚登录就被登出」）
        let pairs: Vec<(&str, &str)> = prod_base()
            .into_iter()
            .filter(|(k, _)| *k != "AUTH_JWT_SECRET")
            .collect();
        let err = from_pairs(&pairs).expect_err("生产不给密钥必须启动即失败");
        assert!(err.contains("AUTH_JWT_SECRET"), "报错要点名配置项：{err}");

        // 开发环境不给也能起（随机生成 + 打告警）
        let dev = from_pairs(&[]).expect("开发环境允许缺密钥");
        assert!(dev.jwt_secret_is_default);
    }

    #[test]
    fn production_keeps_seeding_off_unless_asked() {
        // 种子账号只在明确要的时候建：生产默认 false（cfg.is_dev() 为假）
        let cfg = from_pairs(&prod_base()).expect("加载");
        assert!(!cfg.seed_admin, "生产不该默认建种子管理员");

        // 但显式打开时**必须**同时给口令，否则会在生产里建出一个没有口令的管理员
        let mut pairs = prod_base();
        pairs.push(("AUTH_SEED_ADMIN", "true"));
        pairs.push(("AUTH_ADMIN_PASSWORD", ""));
        assert!(from_pairs(&pairs).is_err(), "开了种子账号却没给口令，必须报错");
    }
}
