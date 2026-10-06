// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! `guangxue-monitor` —— 探活与告警（P2，2026-10）
//!
//! 服务器上由 `guangxue-monitor.timer` 每 5 分钟拉起一次：
//!
//! ```text
//! cargo run --release --bin guangxue-monitor      # 手动跑一次（本机也能跑，只是两个服务得在）
//! ```
//!
//! 它做三件事：
//!   1. 请求两个服务的**深度**健康检查（`?deep=1`），**只看 HTTP 状态码**；
//!   2. 把本轮结果与上一轮（状态文件）比较；
//!   3. 状态**变化**时发一封邮件：转坏发「开始故障」、转好发「已恢复」；
//!      持续故障按 `MONITOR_REPEAT_HOURS`（默认 6 小时）重复提醒一次。
//!
//! 环境变量（都有默认值）：
//!
//! | 变量 | 默认 | 说明 |
//! |---|---|---|
//! | `MONITOR_GO_URL` | `http://127.0.0.1:8080/api/health?deep=1` | Go 主后端 |
//! | `MONITOR_AUTH_URL` | `http://127.0.0.1:8081/api/auth/health?deep=1` | 账号服务 |
//! | `MONITOR_ALERT_TO` | 空 | 告警收件人；**空 = 只写日志不发信** |
//! | `MONITOR_STATE_FILE` | `/var/lib/guangxue-monitor/state.json` | 上一轮状态 |
//! | `MONITOR_TIMEOUT_SECONDS` | 5 | 单次请求超时 |
//! | `MONITOR_REPEAT_HOURS` | 6 | 持续故障时多久重发一次；0 = 只在状态变化时发 |
//!
//! 发信走**账号服务那一套配置**（`AUTH_MAIL_MODE` / `AUTH_SMTP_*`），所以
//! 「这条命令能发出告警」与「注册能收到验证码」是同一个前提；`AUTH_MAIL_MODE=log` 时
//! 告警只会写进日志（本命令会明确提示这一点）。
//!
//! **退出码：0 = 全部健康，1 = 有不健康的项。** systemd 因此会把它记成 failed，
//! `systemctl --failed` 一眼能看到，外部 uptime 服务（可选，见 deploy-runbook）也能接。
//!
//! 为什么手写 HTTP 请求而不引入 HTTP 客户端库：探活只需要「发一个 GET、读第一行状态码」，
//! 为此拉进 reqwest / hyper 一整棵依赖树不划算（账号服务本身只依赖 axum / lettre 这类必需品）。
//! 代价是**只支持 `http://`**、不跟随重定向 —— 回环上的两个服务都不需要这些；
//! 想监控公网 HTTPS 入口，用外部 uptime 服务（见 runbook），不要让这个命令去做 TLS。
//!
//! ⚠️ 本轮**不做**的两件事（用户 2026-10 明确没选）：磁盘余量、备份新鲜度。
//! 它们同样属于「悄悄坏掉」，要加的话就是在这里再加两个检查函数 + 两个阈值环境变量。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::ExitCode;
use std::time::Duration;

use guangxue_auth::config::{Config, MailMode};
use guangxue_auth::mail;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const DEFAULT_GO_URL: &str = "http://127.0.0.1:8080/api/health?deep=1";
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:8081/api/auth/health?deep=1";
const DEFAULT_STATE_FILE: &str = "/var/lib/guangxue-monitor/state.json";
const DEFAULT_TIMEOUT_SECONDS: u64 = 5;
const DEFAULT_REPEAT_HOURS: i64 = 6;
/// 响应体里最多抄多少字符进告警邮件（够看清「哪一项坏了、为什么」就行）
const SNIPPET_MAX_CHARS: usize = 300;
/// 一次最多读多少字节响应（防对端是个不关连接的东西，把内存吃满）
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// 一次检查的结果（也是写进日志/邮件的那一行）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Check {
    name: String,
    url: String,
    ok: bool,
    status: Option<u16>,
    detail: String,
}

/// 上一轮的状态（存成 JSON）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct State {
    failing: bool,
    /// 本轮故障从什么时候开始（RFC3339 / UTC）；没故障时是 None
    since: Option<String>,
    /// 上次**发出**提醒的时间（RFC3339 / UTC）
    notified_at: Option<String>,
}

/// 这一轮该做什么
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    /// 状态没变，别发信（否则每 5 分钟一封，收件人一周后就会把告警规则删掉）
    Nothing,
    /// 从健康转成故障
    Started,
    /// 故障仍在持续，到了重复提醒的时间
    StillFailing,
    /// 从故障转回健康
    Recovered,
}

/// 探活目标：只认 `http://host[:port]/path`
#[derive(Debug, Clone, PartialEq)]
struct Target {
    host: String,
    port: u16,
    path: String,
}

fn parse_target(raw: &str) -> Result<Target, String> {
    let rest = raw
        .strip_prefix("http://")
        .ok_or_else(|| format!("只支持 http:// 开头（收到 {raw}）；公网 HTTPS 入口请交给外部 uptime 服务"))?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(format!("缺少主机名（收到 {raw}）"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| format!("端口不是数字：{p}"))?;
            (h.to_string(), port)
        }
        None => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return Err(format!("缺少主机名（收到 {raw}）"));
    }
    Ok(Target { host, port, path: path.to_string() })
}

/// 发一个 GET，读回 `(状态码, 响应体片段)`
fn fetch(target: &Target, timeout: Duration) -> Result<(u16, String), String> {
    let addr = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|e| format!("解析地址失败：{e}"))?
        .next()
        .ok_or_else(|| format!("解析不出地址：{}:{}", target.host, target.port))?;

    let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("连不上 {addr}：{e}"))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| format!("设置读超时失败：{e}"))?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| format!("设置写超时失败：{e}"))?;
    let mut stream = stream;

    // HTTP/1.0 + Connection: close：对端回完就关，读侧不必猜「body 什么时候结束」
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}:{}\r\nConnection: close\r\nUser-Agent: guangxue-monitor\r\n\r\n",
        target.path, target.host, target.port
    );
    stream.write_all(request.as_bytes()).map_err(|e| format!("发送请求失败：{e}"))?;

    let mut raw = Vec::new();
    stream
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut raw)
        .map_err(|e| format!("读响应失败：{e}"))?;
    let text = String::from_utf8_lossy(&raw).into_owned();

    let mut parts = text.splitn(2, "\r\n");
    let status_line = parts.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("响应不是 HTTP（首行：{}）", snippet(status_line)))?;
    let rest = parts.next().unwrap_or_default();
    // 分隔头与 body 的空行是 \r\n\r\n（第一个 \r\n 已被 splitn 吃掉，所以这里找 \r\n）
    let body = rest.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or(rest);
    Ok((status, body.trim().to_string()))
}

fn check(name: &str, url: &str, timeout: Duration) -> Check {
    let mk = |ok: bool, status: Option<u16>, detail: String| Check {
        name: name.to_string(),
        url: url.to_string(),
        ok,
        status,
        detail,
    };
    match parse_target(url) {
        Err(e) => mk(false, None, format!("地址不合法：{e}")),
        Ok(target) => match fetch(&target, timeout) {
            Ok((status, body)) => {
                // 2xx 算健康：深度健康检查用 503 表达「进程活着但库坏了」，
                // 别的非 2xx（502/404）同样算不健康 —— 探活不该替它们找理由
                let ok = (200..300).contains(&status);
                mk(ok, Some(status), format!("HTTP {status} · {}", snippet(&body)))
            }
            Err(e) => mk(false, None, e),
        },
    }
}

/// 决定这一轮要不要发信
fn decide(prev: &State, failing: bool, now: OffsetDateTime, repeat_hours: i64) -> Action {
    if failing {
        if !prev.failing {
            return Action::Started;
        }
        if repeat_hours <= 0 {
            // 0 = 只在状态变化时发：适合不想收重复邮件的人
            return Action::Nothing;
        }
        let due = prev
            .notified_at
            .as_deref()
            .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
            .map(|last| now - last >= time::Duration::hours(repeat_hours))
            // 记不住上次什么时候发的（状态文件坏了/第一次）：宁可发一封
            .unwrap_or(true);
        if due {
            Action::StillFailing
        } else {
            Action::Nothing
        }
    } else if prev.failing {
        Action::Recovered
    } else {
        Action::Nothing
    }
}

fn snippet(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= SNIPPET_MAX_CHARS {
        return one_line;
    }
    let cut: String = one_line.chars().take(SNIPPET_MAX_CHARS).collect();
    format!("{cut}…")
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "（未知主机）".to_string())
}

fn failing_names(checks: &[Check]) -> String {
    let names: Vec<&str> = checks.iter().filter(|c| !c.ok).map(|c| c.name.as_str()).collect();
    if names.is_empty() {
        "无".to_string()
    } else {
        names.join("、")
    }
}

fn render_report(checks: &[Check], action: Action, now: OffsetDateTime) -> String {
    let stamp = now.format(&Rfc3339).unwrap_or_default();
    let mut out = format!("[{stamp}] guangxue-monitor\n");
    for c in checks {
        out.push_str(&format!("  {} {} {}\n", if c.ok { "✓" } else { "✗" }, c.name, c.detail));
    }
    match action {
        Action::Started => out.push_str("  → 开始故障：已发告警邮件\n"),
        Action::StillFailing => out.push_str("  → 故障仍在持续：已重发提醒\n"),
        Action::Recovered => out.push_str("  → 已恢复：已发恢复邮件\n"),
        Action::Nothing => {}
    }
    out
}

fn compose_alert(
    checks: &[Check],
    action: Action,
    state: &State,
    repeat_hours: i64,
) -> (String, String) {
    let stamp = OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default();
    let (subject, head) = match action {
        Action::Started => (
            format!("[广学] 服务异常：{}", failing_names(checks)),
            "监控发现服务不健康。".to_string(),
        ),
        Action::StillFailing => (
            format!("[广学] 仍未恢复：{}", failing_names(checks)),
            format!("故障仍在持续（每 {repeat_hours} 小时提醒一次，直到恢复）。"),
        ),
        Action::Recovered => (
            "[广学] 已恢复：全部正常".to_string(),
            "所有检查项都恢复正常了。".to_string(),
        ),
        Action::Nothing => unreachable!("Nothing 不该走到发信"),
    };

    let mut body = format!("{head}\n\n检查时间：{stamp}\n服务器：{}\n\n各项结果：\n", hostname());
    for c in checks {
        body.push_str(&format!(
            "  {} {}（{}）\n      {}\n",
            if c.ok { "✓" } else { "✗" },
            c.name,
            c.url,
            c.detail
        ));
    }
    if let Some(since) = state.since.as_deref() {
        body.push_str(&format!("\n故障开始于：{since}\n"));
    }
    body.push_str(
        "\n排查顺序（服务器上）：\n  \
         systemctl status guangxue-api guangxue-auth\n  \
         journalctl -u guangxue-api -n 50 （账号服务把 -u 换成 guangxue-auth）\n  \
         df -h（盘满是最常见的「进程活着但不能用」）\n\n\
         （本邮件由 guangxue-monitor 发出；恢复时也会发一封。）",
    );
    (subject, body)
}

fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => default.to_string(),
    }
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    match std::env::var(key) {
        Ok(v) => v.trim().parse().unwrap_or_else(|_| {
            eprintln!("⚠ {key}={v} 不是合法数字，改用默认值");
            default
        }),
        Err(_) => default,
    }
}

fn load_state(path: &str) -> State {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("⚠ 状态文件内容坏了（{path}）：{e} —— 按「没有历史」处理，可能多发一封");
            State::default()
        }),
        // 第一次跑、或目录还没建：都当「没有历史」（不是错误）
        Err(_) => State::default(),
    }
}

fn save_state(path: &str, state: &State) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    if let Some(dir) = std::path::Path::new(path).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("建目录 {} 失败：{e}", dir.display()))?;
        }
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

async fn send_alert(to: &str, checks: &[Check], action: Action, state: &State, repeat_hours: i64) {
    if to.is_empty() {
        eprintln!("⚠ 没配 MONITOR_ALERT_TO：告警只写在本条日志里（建议配上管理员邮箱）");
        return;
    }
    let cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("⚠ 配置有问题（账号服务也起不来）：{e} —— 发不出告警");
            return;
        }
    };
    if matches!(cfg.mail.mode, MailMode::Log) {
        eprintln!("⚠ AUTH_MAIL_MODE=log：告警邮件不会真的发出去，只会写日志（上线前记得切 smtp）");
    }
    let mailer = match mail::build(&cfg) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("⚠ 构造发送器失败：{e} —— 发不出告警");
            return;
        }
    };
    let (subject, body) = compose_alert(checks, action, state, repeat_hours);
    match mailer.send_notice(to, &subject, &body).await {
        Ok(()) => println!("  → 已发出告警邮件：{subject}"),
        Err(e) => eprintln!("  ✗ 告警邮件发送失败：{e}（具体 SMTP 错误见上方日志）"),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // warn 级别够用：我们自己的报告走 println（journald 收 stdout），
    // 而 tracing 里要看的是 SMTP 失败的原因
    guangxue_auth::init_tracing("warn");

    let go_url = env_or("MONITOR_GO_URL", DEFAULT_GO_URL);
    let auth_url = env_or("MONITOR_AUTH_URL", DEFAULT_AUTH_URL);
    let alert_to = env_or("MONITOR_ALERT_TO", "");
    let state_file = env_or("MONITOR_STATE_FILE", DEFAULT_STATE_FILE);
    let timeout = Duration::from_secs(env_parse("MONITOR_TIMEOUT_SECONDS", DEFAULT_TIMEOUT_SECONDS));
    let repeat_hours = env_parse("MONITOR_REPEAT_HOURS", DEFAULT_REPEAT_HOURS);

    // 顺序固定：先 Go 再账号服务（报告里也按这个顺序，肉眼比对两次输出时不用重新找）
    let checks = vec![
        check("Go 主后端", &go_url, timeout),
        check("账号服务", &auth_url, timeout),
    ];
    let failing = checks.iter().any(|c| !c.ok);

    let now = OffsetDateTime::now_utc();
    let prev = load_state(&state_file);
    let action = decide(&prev, failing, now, repeat_hours);
    print!("{}", render_report(&checks, action, now));

    // 先落状态再发信：SMTP 自己挂了的时候，不该变成「每 5 分钟一封新告警邮件」
    let stamp = now.format(&Rfc3339).unwrap_or_default();
    let next = State {
        failing,
        since: if failing { prev.since.clone().or_else(|| Some(stamp.clone())) } else { None },
        notified_at: match action {
            Action::Nothing => prev.notified_at.clone(),
            _ => Some(stamp),
        },
    };
    if let Err(e) = save_state(&state_file, &next) {
        eprintln!("⚠ 状态文件写不进去（{state_file}）：{e}");
        eprintln!("  后果：告警会每轮重发一次。检查 MONITOR_STATE_FILE 的目录是否存在、是否可写。");
    }

    if action != Action::Nothing {
        send_alert(&alert_to, &checks, action, &next, repeat_hours).await;
    }

    if failing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(failing: bool, notified_at: Option<&str>) -> State {
        State {
            failing,
            since: if failing { Some("2026-10-05T00:00:00Z".to_string()) } else { None },
            notified_at: notified_at.map(|s| s.to_string()),
        }
    }

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &Rfc3339).expect("测试用的时间要合法")
    }

    fn ok_check(name: &str) -> Check {
        Check {
            name: name.to_string(),
            url: "http://127.0.0.1:1/api/health?deep=1".to_string(),
            ok: true,
            status: Some(200),
            detail: "HTTP 200 · {\"status\":\"ok\"}".to_string(),
        }
    }

    #[test]
    fn target_parsing_accepts_the_two_loopback_urls() {
        let t = parse_target("http://127.0.0.1:8081/api/auth/health?deep=1").expect("合法");
        assert_eq!(t.host, "127.0.0.1");
        assert_eq!(t.port, 8081);
        assert_eq!(t.path, "/api/auth/health?deep=1");
        // 不写端口 = 80，不写路径 = /
        let t = parse_target("http://example.com").expect("合法");
        assert_eq!(t.port, 80);
        assert_eq!(t.path, "/");
    }

    #[test]
    fn target_parsing_rejects_https_and_garbage() {
        for bad in ["https://example.com/health", "example.com/health", "http://:8080/x", "http://h:abc/x"] {
            // ⚠️ 这里要的是 **Err**，不是「解析出来的 Target」——写错成 unwrap_or_else
            // 会把 Ok 当成错误信息用，测试反而永远通过（第一次就踩了）
            let err = match parse_target(bad) {
                Ok(t) => panic!("{bad} 应当被拒绝，却解析成了 {t:?}"),
                Err(e) => e,
            };
            assert!(!err.is_empty(), "拒绝时要给出能读的理由：{bad}");
        }
    }

    #[test]
    fn decide_fires_on_transitions_and_repeats_on_schedule() {
        let now = at("2026-10-05T12:00:00Z");
        // 健康 → 故障：立刻发
        assert_eq!(decide(&state(false, None), true, now, 6), Action::Started);
        // 故障持续、上次提醒是 1 小时前：还没到 6 小时，闭嘴
        assert_eq!(
            decide(&state(true, Some("2026-10-05T11:00:00Z")), true, now, 6),
            Action::Nothing
        );
        // 故障持续、上次提醒 7 小时前：重发一次
        assert_eq!(
            decide(&state(true, Some("2026-10-05T05:00:00Z")), true, now, 6),
            Action::StillFailing
        );
        // 记不住上次提醒时间（状态文件坏了）：宁可发一封
        assert_eq!(decide(&state(true, None), true, now, 6), Action::StillFailing);
        // repeat_hours = 0：只在状态变化时发
        assert_eq!(
            decide(&state(true, Some("2026-10-05T05:00:00Z")), true, now, 0),
            Action::Nothing
        );
        // 故障 → 健康：发恢复
        assert_eq!(decide(&state(true, Some("2026-10-05T11:00:00Z")), false, now, 6), Action::Recovered);
        // 一直健康：什么都不发
        assert_eq!(decide(&state(false, None), false, now, 6), Action::Nothing);
    }

    #[test]
    fn alert_mail_names_the_failing_service_and_how_to_look() {
        let mut bad = ok_check("账号服务");
        bad.ok = false;
        bad.status = Some(503);
        bad.detail = "HTTP 503 · {\"message\":\"数据库不可用\"}".to_string();
        let checks = vec![ok_check("Go 主后端"), bad];
        let (subject, body) = compose_alert(&checks, Action::Started, &state(true, None), 6);

        assert!(subject.contains("服务异常"), "主题要一眼看出是故障：{subject}");
        assert!(subject.contains("账号服务"), "主题要点名坏的是谁：{subject}");
        assert!(!subject.contains("Go 主后端"), "好的服务不该出现在故障主题里：{subject}");
        assert!(body.contains("HTTP 503"), "正文要带上状态码：{body}");
        assert!(body.contains("数据库不可用"), "正文要带上响应片段：{body}");
        assert!(body.contains("故障开始于"), "持续故障要说明从什么时候开始：{body}");
        assert!(body.contains("journalctl"), "正文要给出排查入口：{body}");

        // 恢复邮件不能长得像故障邮件
        let (subject, body) = compose_alert(&checks, Action::Recovered, &state(false, None), 6);
        assert!(subject.contains("已恢复"), "{subject}");
        assert!(body.contains("恢复正常"), "{body}");
    }

    #[test]
    fn snippet_collapses_whitespace_and_truncates() {
        assert_eq!(snippet("  a\n  b\t c  "), "a b c");
        let long = "x".repeat(SNIPPET_MAX_CHARS + 50);
        let cut = snippet(&long);
        assert_eq!(cut.chars().count(), SNIPPET_MAX_CHARS + 1, "截断后只多一个省略号");
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn state_round_trips_through_json() {
        // 状态文件是「跨进程」的接口：一边写一边读，字段名写错会让告警静默失效
        let s = state(true, Some("2026-10-05T12:00:00Z"));
        let text = serde_json::to_string(&s).expect("序列化");
        let back: State = serde_json::from_str(&text).expect("反序列化");
        assert_eq!(s, back);
        // 老版本/空文件也要能读成「没有历史」，而不是让命令崩掉
        assert_eq!(State::default().failing, false);
    }
}
