//! 配置：环境变量 → `Config`
//!
//! 约定与 Go 侧保持一致（`APP_ENV` / 开发默认值），额外的前缀统一是 `AUTH_`。
//! ⚠️ 生产环境（`APP_ENV=production`）有两条硬性要求，不满足直接启动失败：
//!   1. 必须显式提供 `AUTH_JWT_SECRET`（否则每次重启密钥都变，且令牌可被伪造）；
//!   2. 必须显式提供 `AUTH_ADMIN_PASSWORD`（或显式关掉 `AUTH_SEED_ADMIN`），
//!      不允许把源码里的默认口令带上线。

use std::time::Duration;

use rand::RngCore;
use base64::Engine as _;

/// 初始管理员邮箱（需求指定）
pub const DEFAULT_ADMIN_EMAIL: &str = "2262997289@qq.com";
/// 初始管理员口令（**仅 development 允许作为默认值**）
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
#[derive(Debug, Clone, Copy)]
pub struct RateConfig {
    /// 登录：同 IP 15 分钟内的尝试次数
    pub login_ip: RateRule,
    /// 发验证码：同邮箱 60 秒 1 次
    pub code_email_minute: RateRule,
    /// 发验证码：同邮箱 1 小时 5 次
    pub code_email_hour: RateRule,
    /// 发验证码：同 IP 1 小时 20 次
    pub code_ip: RateRule,
    /// 注册：同 IP 1 小时 10 次
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

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    /// tls / starttls / none
    pub tls: String,
    pub timeout: Duration,
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
    pub admin_email: String,
    pub admin_password: Option<String>,
    pub seed_admin: bool,
    pub argon2: Argon2Config,
    pub rate: RateConfig,
    /// 连续失败多少次锁定账号
    pub lock_threshold: i64,
    /// 锁定时长（分钟）
    pub lock_minutes: i64,
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
            admin_email: DEFAULT_ADMIN_EMAIL.to_string(),
            admin_password: Some(DEFAULT_ADMIN_PASSWORD.to_string()),
            seed_admin: true,
            argon2: Argon2Config { m_cost: 19456, t_cost: 2, p_cost: 1 },
            rate: RateConfig {
                login_ip: RateRule::new(20, Duration::from_secs(15 * 60)),
                code_email_minute: RateRule::new(1, Duration::from_secs(60)),
                code_email_hour: RateRule::new(5, Duration::from_secs(3600)),
                code_ip: RateRule::new(20, Duration::from_secs(3600)),
                register_ip: RateRule::new(10, Duration::from_secs(3600)),
            },
            lock_threshold: 5,
            lock_minutes: 15,
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

        let mut cfg = Config::development();
        cfg.env = env_str("APP_ENV", &cfg.env);
        cfg.host = env_str("AUTH_HOST", &cfg.host);
        cfg.port = env_parse("AUTH_PORT", cfg.port)?;
        cfg.db_path = env_str("AUTH_DB_PATH", &cfg.db_path);
        cfg.access_ttl = Duration::from_secs(env_parse("AUTH_ACCESS_TTL_SECONDS", cfg.access_ttl.as_secs())?);
        cfg.refresh_ttl =
            Duration::from_secs(env_parse("AUTH_REFRESH_TTL_DAYS", cfg.refresh_ttl_days())? as u64 * 86400);
        cfg.code_ttl = Duration::from_secs(env_parse("AUTH_CODE_TTL_SECONDS", cfg.code_ttl.as_secs())?);
        cfg.argon2 = Argon2Config {
            m_cost: env_parse("AUTH_ARGON2_M_COST", cfg.argon2.m_cost)?,
            t_cost: env_parse("AUTH_ARGON2_T_COST", cfg.argon2.t_cost)?,
            p_cost: env_parse("AUTH_ARGON2_P_COST", cfg.argon2.p_cost)?,
        };
        cfg.lock_threshold = env_parse_i64("AUTH_LOCK_THRESHOLD", cfg.lock_threshold)?;
        cfg.lock_minutes = env_parse_i64("AUTH_LOCK_MINUTES", cfg.lock_minutes)?;

        // 密钥：生产必须显式提供；开发缺失则随机生成（重启后旧令牌失效，并打印告警）
        match env_opt("AUTH_JWT_SECRET") {
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
        cfg.code_pepper = match env_opt("AUTH_CODE_PEPPER") {
            Some(p) => p.into_bytes(),
            None => cfg.jwt_secret.clone(),
        };

        // Cookie
        let default_secure = cfg.is_production();
        cfg.cookie_secure = env_parse_bool("AUTH_COOKIE_SECURE", default_secure)?;
        cfg.cookie_domain = env_opt("AUTH_COOKIE_DOMAIN");
        if let Some(list) = env_opt("AUTH_ALLOWED_ORIGINS") {
            cfg.allowed_origins = list
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        // 邮件
        let mode = env_str("AUTH_MAIL_MODE", "log").to_ascii_lowercase();
        cfg.mail = match mode.as_str() {
            "log" => MailConfig { mode: MailMode::Log, smtp: None },
            "smtp" => {
                let host = env_opt("AUTH_SMTP_HOST")
                    .ok_or_else(|| "AUTH_MAIL_MODE=smtp 时必须提供 AUTH_SMTP_HOST".to_string())?;
                let username = env_opt("AUTH_SMTP_USERNAME")
                    .ok_or_else(|| "AUTH_MAIL_MODE=smtp 时必须提供 AUTH_SMTP_USERNAME".to_string())?;
                let password = env_opt("AUTH_SMTP_PASSWORD")
                    .ok_or_else(|| "AUTH_MAIL_MODE=smtp 时必须提供 AUTH_SMTP_PASSWORD（邮箱 SMTP 授权码）".to_string())?;
                let from = env_opt("AUTH_SMTP_FROM").unwrap_or_else(|| username.clone());
                MailConfig {
                    mode: MailMode::Smtp,
                    smtp: Some(SmtpConfig {
                        host,
                        port: env_parse("AUTH_SMTP_PORT", 465u16)?,
                        username,
                        password,
                        from,
                        tls: env_str("AUTH_SMTP_TLS", "tls").to_ascii_lowercase(),
                        timeout: Duration::from_secs(env_parse("AUTH_SMTP_TIMEOUT_SECONDS", 15u64)?),
                    }),
                }
            }
            other => return Err(format!("AUTH_MAIL_MODE 只能是 log 或 smtp，收到：{other}")),
        };

        cfg.dev_endpoints = env_parse_bool("AUTH_DEV_ENDPOINTS", cfg.is_dev())?;
        if cfg.is_production() && cfg.dev_endpoints {
            return Err("生产环境不允许开启 AUTH_DEV_ENDPOINTS（调试接口会泄露验证码）".to_string());
        }

        // 管理员
        cfg.admin_email = env_str("AUTH_ADMIN_EMAIL", &cfg.admin_email);
        cfg.admin_password = match env_opt("AUTH_ADMIN_PASSWORD") {
            Some(p) => {
                cfg.admin_password_is_default = p == DEFAULT_ADMIN_PASSWORD;
                if cfg.is_production() && cfg.admin_password_is_default {
                    return Err("生产环境不允许使用默认管理员口令，请设置 AUTH_ADMIN_PASSWORD".to_string());
                }
                Some(p)
            }
            None => {
                cfg.admin_password_is_default = false;
                if cfg.is_production() {
                    None
                } else {
                    // 开发环境用需求里指定的默认口令，开箱即可登录
                    cfg.admin_password_is_default = true;
                    Some(DEFAULT_ADMIN_PASSWORD.to_string())
                }
            }
        };
        cfg.seed_admin = env_parse_bool("AUTH_SEED_ADMIN", cfg.is_dev())?;
        if cfg.seed_admin && cfg.admin_password.is_none() {
            return Err("AUTH_SEED_ADMIN=true 时必须提供 AUTH_ADMIN_PASSWORD".to_string());
        }

        Ok(cfg)
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_str(key: &str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> Result<T, String> {
    match env_opt(key) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<T>()
            .map_err(|_| format!("环境变量 {key} 的值无法解析：{raw}")),
    }
}

fn env_parse_i64(key: &str, default: i64) -> Result<i64, String> {
    env_parse(key, default)
}

fn env_parse_bool(key: &str, default: bool) -> Result<bool, String> {
    match env_opt(key) {
        None => Ok(default),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => Err(format!("环境变量 {key} 只能是 true/false，收到：{other}")),
        },
    }
}
