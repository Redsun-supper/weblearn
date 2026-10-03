//! 建初始管理员（幂等）
//!
//! 用法（在 `backend-rust/` 目录下执行）：
//!
//! ```text
//! cargo run --release --bin seed-admin
//! cargo run --release --bin seed-admin -- --email 2262997289@qq.com --password 7289HR_RedSun
//! cargo run --release --bin seed-admin -- --reset-password --password 新的密码
//! cargo run --release --bin seed-admin -- --role super_admin
//! cargo run --release --bin seed-admin -- --db other.db
//! ```
//!
//! 行为：
//!   - 账号已存在且没给 `--reset-password` → 跳过（**绝不覆盖已有密码**）；
//!   - 给了 `--reset-password` → 重置密码，并吊销该用户的全部会话；
//!   - 密码来源优先级：`--password` > `AUTH_ADMIN_PASSWORD` > development 默认密码；
//!   - `--role` 默认 `super_admin`（服务启动时也是它，见 `main.rs` 的 `ensure_super_admin`）。

use std::process::ExitCode;

use guangxue_auth::cli::CliArgs;
use guangxue_auth::config::{Config, DEFAULT_ADMIN_EMAIL, DEFAULT_ADMIN_PASSWORD};
use guangxue_auth::models::{ROLE_ADMIN, ROLE_SUPER_ADMIN};
use guangxue_auth::{init_tracing, AppState};

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing("info");
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("失败：{message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args = CliArgs::parse(std::env::args().skip(1));
    if args.help {
        print_help();
        return Ok(());
    }

    let mut cfg = cli_config("--db", &args);
    let email = args.get("--email").unwrap_or(DEFAULT_ADMIN_EMAIL).to_string();
    let reset = args.has("--reset-password");
    // ⚠️ 默认是 super_admin（与启动期 `ensure_super_admin` 一致）：
    //    全新部署后「服务器上那个人」必须能发码、能调权限，否则谁也开不了张。
    let role = args.get("--role").unwrap_or(ROLE_SUPER_ADMIN).to_string();
    if role != ROLE_ADMIN && role != ROLE_SUPER_ADMIN {
        eprintln!("失败：--role 只能是 admin / super_admin，收到：{role}");
        return Err("角色不合法".to_string());
    }
    let password = args
        .get("--password")
        .map(|s| s.to_string())
        .or_else(|| cfg.admin_password.clone())
        .unwrap_or_else(|| DEFAULT_ADMIN_PASSWORD.to_string());
    if args.get("--password").is_none() && cfg.admin_password.is_none() {
        eprintln!("提示：未提供密码，使用 development 默认密码");
    }
    cfg.seed_admin = false; // 这个命令自己负责建号，不需要启动期的自动 seed

    let state = AppState::init(cfg).await.map_err(|e| e.to_string())?;

    if reset {
        state.service.set_password(&email, &password).await.map_err(|e| e.to_string())?;
        println!("已重置密码：{email}（该账号的全部会话已失效）");
        return Ok(());
    }

    let created = state
        .service
        .seed_admin(&email, &password, &role)
        .await
        .map_err(|e| e.to_string())?;
    if created {
        println!("已创建管理员：{email}（role={role}，邮箱视为已验证）");
    } else {
        println!("管理员已存在，未做任何修改：{email}");
        println!("  · 需要改密码请加 --reset-password");
        println!("  · 需要改角色请用后台的用户管理，或 `--role` 配一个新邮箱");
    }
    Ok(())
}

/// CLI 用的配置：环境变量能用就用，不能用就退回开发默认值（CLI 不依赖 JWT 密钥等）
fn cli_config(db_flag: &str, args: &CliArgs) -> Config {
    let mut cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("提示：环境变量配置未通过校验，改用开发默认值（{e}）");
            Config::development()
        }
    };
    if let Some(db) = args.get(db_flag) {
        cfg.db_path = db.to_string();
    }
    cfg
}

fn print_help() {
    println!(
        r#"广学 · 建初始管理员（幂等）

用法：
  cargo run --release --bin seed-admin [选项]

选项：
  --email <邮箱>        默认 {DEFAULT_ADMIN_EMAIL}
  --password <密码>     默认取自 AUTH_ADMIN_PASSWORD，再退回 development 默认密码
  --role <角色>         admin / super_admin（默认 super_admin）
  --reset-password      已存在时重置密码（并吊销该账号全部会话）
  --db <文件>           SQLite 文件，默认取 AUTH_DB_PATH 或 auth.db
  -h, --help            显示本帮助

说明：密码规则与接口一致（8~128 字符，且同时含字母与数字）。
      `--role` 只在**新建账号**时生效；已存在的账号不会被改角色（要改请用后台用户管理），
      这样「跑错一次命令」不会把别人的权限悄悄改掉。
"#
    );
}
