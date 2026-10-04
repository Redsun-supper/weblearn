//! `mail-test` —— 一条命令验收 P0-3「真发邮件」
//!
//! 用法（在 `backend-rust/` 目录下）：
//!
//! ```text
//! cargo run --bin mail-test -- 你的邮箱@qq.com
//! ```
//!
//! 它读的是**和账号服务同一份配置**（`.env` + 环境变量，见 `Config::from_env`），
//! 所以「这条命令能发出信」等于「注册时的验证码也能发出去」。
//!
//! 为什么单独做一个命令，而不是让用户去走注册流程：
//!   · 注册流程发的是**真验证码**，会占一次限流额度（同一邮箱每分钟 1 封 / 每小时 5 封）；
//!   · SMTP 配错时的报错往往只有一句 `MailFailed`，而这里会把**具体 SMTP 错误**
//!     打到终端上（认证失败 / 连不上 / 被拒），排查方向完全不同。
//!
//! ⚠️ 注意 `AUTH_MAIL_MODE=smtp` 之后，验证码**只走邮件、不再写日志**，
//!    也不再有 `/api/auth/dev/codes` 可查 —— 生产环境的调试接口是强制关闭的。

use std::process::ExitCode;

use guangxue_auth::config::{Config, MailMode};
use guangxue_auth::mail;

#[tokio::main]
async fn main() -> ExitCode {
    // 先开日志：发送失败时那条 tracing 的 error 行里有**具体的 SMTP 错误**
    //（认证失败 535 / 连不上 / 被拒），这才是排查的关键线索
    guangxue_auth::init_tracing("info");

    let to = match std::env::args().nth(1) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            eprintln!("用法：cargo run --bin mail-test -- 收件邮箱@example.com");
            eprintln!("说明：读的是与账号服务同一份配置（.env / 环境变量），用真实 SMTP 发一封测试邮件。");
            return ExitCode::from(2);
        }
    };

    let cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("✗ 配置有问题，服务也起不来：{e}");
            return ExitCode::FAILURE;
        }
    };

    // 先打印配置概览 —— 授权码不打印，只看长度，确认「不是空的、不是带空格的」
    match &cfg.mail.mode {
        MailMode::Log => {
            println!("✗ 当前 AUTH_MAIL_MODE=log：验证码只会打进日志，不会真的发信。");
            println!("  先在 backend-rust/.env 里配好 AUTH_MAIL_MODE=smtp 与 AUTH_SMTP_* 再跑这条命令。");
            return ExitCode::FAILURE;
        }
        MailMode::Smtp => {}
    }
    let smtp = cfg.mail.smtp.as_ref().expect("smtp 模式下必然有配置");
    println!("即将通过真实 SMTP 发一封测试邮件：");
    println!("  目标邮箱 : {to}");
    println!("  服务器   : {}:{}", smtp.host, smtp.port);
    println!("  加密模式 : {}", smtp.tls.as_str());
    println!("  账号     : {}", smtp.username);
    println!("  授权码   : {} 个字符（不打印内容）", smtp.password.chars().count());
    println!("  发件人   : {}", smtp.from);
    println!("  超时     : {} 秒", smtp.timeout.as_secs());

    let mailer = match mail::build(&cfg) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ 构造发送器失败：{e}");
            return ExitCode::FAILURE;
        }
    };

    // purpose 用 "test"：会落到「广学 · 邮箱验证码」这个标题上，
    // 邮件正文里明确写着是一条测试，免得收件人以为自己的账号出了问题
    match mailer.send_code(&to, "123456", "test", 600).await {
        Ok(()) => {
            println!("\n✓ SMTP 服务器已接收（250）。去收件箱看一眼：");
            println!("   · 主题是「广学 · 邮箱验证码」，正文里有测试验证码 123456；");
            println!("   · 收不到就先看**垃圾箱** —— SPF / DKIM / DMARC 没做，大概率进垃圾箱（见上线计划 P0-3 第 4 步）；");
            println!("   · 顺便确认发件人显示的是「{}」而不是一串乱码。", smtp.from);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("\n✗ 发送失败：{e}");
            eprintln!("  具体 SMTP 错误见上方日志（tracing 的 error 行）。常见原因：");
            eprintln!("   · 授权码不对（QQ 邮箱要用「设置 → 账号 → POP3/SMTP 服务」生成的授权码，不是登录密码）；");
            eprintln!("   · 端口与加密方式不匹配（465 配 implicit / 587 配 starttls）；");
            eprintln!("   · 发件人与登录账号不是同一个（多数邮箱只允许用它自己的地址发信）。");
            ExitCode::FAILURE
        }
    }
}
