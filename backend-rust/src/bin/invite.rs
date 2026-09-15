//! 邀请码 CLI：创建 / 列表 / 停用
//!
//! 用法（在 `backend-rust/` 目录下执行）：
//!
//! ```text
//! cargo run --release --bin invite -- create --count 3 --max-uses 1 --expires-in-days 7 --note "第一批"
//! cargo run --release --bin invite -- list --status unused
//! cargo run --release --bin invite -- disable 12
//! ```
//!
//! 与 HTTP 管理接口共用同一套服务层实现（`AuthService`），因此行为完全一致。

use std::process::ExitCode;

use guangxue_auth::cli::CliArgs;
use guangxue_auth::config::Config;
use guangxue_auth::store::sql::InviteFilter;
use guangxue_auth::{init_tracing, AppState};

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing("warn");
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
    let command = args.positional.first().cloned().unwrap_or_default();
    if args.help || command.is_empty() {
        print_help();
        return Ok(());
    }

    let cfg = cli_config(&args);
    let state = AppState::init(cfg).await.map_err(|e| e.to_string())?;

    match command.as_str() {
        "create" => {
            let count = args.get_i64("--count", 1)?;
            let max_uses = args.get_i64("--max-uses", 1)?;
            let days = args.get_i64("--expires-in-days", 7)?;
            let note = args.get("--note").unwrap_or("");
            let created = state
                .service
                .create_invites(None, count, max_uses, days, note)
                .await
                .map_err(|e| e.to_string())?;
            println!("已生成 {} 个邀请码（明文只显示这一次，请立即记录）：", created.codes.len());
            for item in &created.codes {
                println!("  {}", item.code);
            }
            println!(
                "  可用次数 {}，有效期 {}",
                max_uses,
                if days <= 0 { "永不过期".to_string() } else { format!("{days} 天") }
            );
            Ok(())
        }
        "list" => {
            let filter = match args.get("--status") {
                None => InviteFilter::All,
                Some(raw) => InviteFilter::parse(raw)
                    .ok_or_else(|| "status 只能是 unused/used/expired/disabled/all".to_string())?,
            };
            let page = args.get_i64("--page", 1)?;
            let size = args.get_i64("--size", 20)?;
            let (items, total) = state
                .service
                .list_invites(filter, page, size)
                .await
                .map_err(|e| e.to_string())?;
            if items.is_empty() {
                println!("没有符合条件的邀请码（共 {total} 条）");
                return Ok(());
            }
            println!("{:<5} {:<20} {:<9} {:<9} {:<20} {}", "ID", "邀请码", "状态", "已用/可用", "到期时间", "备注");
            for it in &items {
                println!(
                    "{:<5} {:<20} {:<9} {:<9} {:<20} {}",
                    it.id,
                    it.code,
                    it.status,
                    format!("{}/{}", it.used_count, it.max_uses),
                    it.expires_at.clone().unwrap_or_else(|| "永不过期".to_string()),
                    it.note
                );
            }
            println!("共 {total} 条，本页 {} 条", items.len());
            Ok(())
        }
        "disable" => {
            let raw = args
                .positional
                .get(1)
                .ok_or_else(|| "用法：invite disable <id>".to_string())?;
            let id: i64 = raw.parse().map_err(|_| format!("id 需要是整数，收到：{raw}"))?;
            state.service.disable_invite(id).await.map_err(|e| e.to_string())?;
            println!("已停用邀请码 id={id}");
            Ok(())
        }
        other => Err(format!("未知子命令：{other}（可用：create / list / disable）")),
    }
}

fn cli_config(args: &CliArgs) -> Config {
    let mut cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("提示：环境变量配置未通过校验，改用开发默认值（{e}）");
            Config::development()
        }
    };
    if let Some(db) = args.get("--db") {
        cfg.db_path = db.to_string();
    }
    cfg
}

fn print_help() {
    println!(
        r#"广学 · 邀请码 CLI

用法：
  cargo run --release --bin invite -- create [选项]
  cargo run --release --bin invite -- list [选项]
  cargo run --release --bin invite -- disable <id>

create 选项：
  --count <N>              生成几个（1~50，默认 1）
  --max-uses <N>           每个可用几次（1~1000，默认 1）
  --expires-in-days <N>    有效期天数（0 表示不过期，默认 7）
  --note <文字>            备注（后台列表可见）

list 选项：
  --status <状态>          unused / used / expired / disabled / all（默认 all）
  --page <N> --size <N>    分页（默认 1 / 20）

通用：
  --db <文件>              SQLite 文件，默认取 AUTH_DB_PATH 或 auth.db
  -h, --help               显示本帮助
"#
    );
}
