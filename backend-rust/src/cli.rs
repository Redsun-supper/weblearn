// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
//! 命令行参数的小工具
//!
//! 不引 clap：这两个命令（`seed-admin` / `invite`）参数少且固定，
//! 手写解析更容易读，也少一个依赖。

use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct CliArgs {
    values: HashMap<String, String>,
    switches: HashSet<String>,
    pub positional: Vec<String>,
    pub help: bool,
}

impl CliArgs {
    /// 解析规则：
    ///   - `--key value` / `--key=value` → 带值参数
    ///   - `--flag`（后面没有值或下一个也是 `--`）→ 开关
    ///   - 其余 token → 位置参数
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Self {
        let mut out = CliArgs::default();
        let mut iter = args.into_iter().peekable();
        while let Some(token) = iter.next() {
            if token == "--help" || token == "-h" {
                out.help = true;
                continue;
            }
            if let Some(rest) = token.strip_prefix("--") {
                if let Some((key, value)) = rest.split_once('=') {
                    out.values.insert(format!("--{key}"), value.to_string());
                    continue;
                }
                let key = format!("--{rest}");
                match iter.peek() {
                    Some(next) if !next.starts_with("--") => {
                        let value = iter.next().unwrap_or_default();
                        out.values.insert(key, value);
                    }
                    _ => {
                        out.switches.insert(key);
                    }
                }
            } else {
                out.positional.push(token);
            }
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.switches.contains(key) || self.values.contains_key(key)
    }

    pub fn get_i64(&self, key: &str, default: i64) -> Result<i64, String> {
        match self.get(key) {
            None => Ok(default),
            Some(raw) => raw.parse::<i64>().map_err(|_| format!("{key} 需要一个整数，收到：{raw}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> CliArgs {
        CliArgs::parse(line.split_whitespace().map(|s| s.to_string()))
    }

    #[test]
    fn parses_values_switches_and_positional() {
        let args = parse("create --count 3 --note 一期 --db auth.db");
        assert_eq!(args.positional, vec!["create"]);
        assert_eq!(args.get("--count"), Some("3"));
        assert_eq!(args.get("--note"), Some("一期"));
        assert_eq!(args.get("--db"), Some("auth.db"));
        assert!(!args.has("--reset-password"));
    }

    #[test]
    fn parses_equals_form_and_switches() {
        let args = parse("seed-admin --reset-password --db=other.db");
        assert!(args.has("--reset-password"));
        assert_eq!(args.get("--db"), Some("other.db"));
        assert_eq!(args.get_i64("--count", 1).unwrap(), 1);
    }

    #[test]
    fn invalid_integer_is_reported() {
        let args = parse("create --count abc");
        assert!(args.get_i64("--count", 1).is_err());
    }
}
