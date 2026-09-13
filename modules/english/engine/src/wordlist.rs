//! 词表文本解析（后台「批量导入」用）
//!
//! 把用户粘贴进来的文本解析成结构化词条。之所以放在 Rust 而不是 JS：粘贴格式极其杂乱，
//! 容错规则需要可测试。这里全部是纯字符串处理，宿主上可直接 `cargo test`。
//!
//! ## 支持的格式（按优先级自动判断）
//!
//! ```text
//! abandon                                          # 只有单词
//! abandon	/əˈbæn.dən/	v. 放弃；抛弃	He abandoned it.  # 制表符（Excel 粘贴）
//! abandon | /əˈbæn.dən/ | v. 放弃；抛弃 | He... | 他放弃了。  # 竖线（第 5 列是例句翻译）
//! abandon, /əˈbæn.dən/, v. 放弃；抛弃, He..., 他放弃了。     # 逗号（CSV）
//! abandon /əˈbæn.dən/ v. 放弃；抛弃                 # 空格：第二个词像音标就当音标
//! abandon 放弃；抛弃                                # 空格：第二个词不像音标就当释义
//! ```
//!
//! - 以 `#` 或 `//` 开头的行、空行会被跳过
//! - 字段按位置对应：单词 / 音标 / 释义 / 例句 / 例句翻译，多出的字段忽略
//! - 同一文件内重复的单词会被单独列出，不进入待导入列表
//! - 单词列整列都是非 ASCII（例如误把中文列放在第一列）会作为错误行提示

use serde::Serialize;
use std::collections::HashSet;
use wasm_bindgen::prelude::*;

/// 音标常见符号：用于判断「空格分隔时第二个词到底是音标还是释义」
const IPA_MARKERS: &[char] = &[
    'ə', 'æ', 'ː', 'ˈ', 'ˌ', 'ʊ', 'ɔ', 'ɒ', 'ʃ', 'ʒ', 'θ', 'ð', 'ŋ', 'ʌ', 'ɪ', 'ɛ', 'ɑ', 'ɜ', 'ɡ',
];

/// 一行解析出的词条
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedWord {
    pub word: String,
    pub phonetic: String,
    pub meaning: String,
    pub example: String,
    /// 例句的中文翻译（第 5 列，可空）
    pub example_translation: String,
    /// 来源行号（从 1 开始），便于前端定位
    pub line: usize,
}

/// 解析失败的行
#[derive(Debug, Clone, Serialize)]
pub struct ParseError {
    pub line: usize,
    pub text: String,
    pub reason: String,
}

/// 文件内重复的词
#[derive(Debug, Clone, Serialize)]
pub struct ParseDuplicate {
    pub line: usize,
    pub word: String,
}

/// 统计信息
#[derive(Debug, Clone, Serialize)]
pub struct ParseStats {
    /// 参与解析的行数（已排除空行与注释）
    pub total_lines: usize,
    pub ok: usize,
    pub errors: usize,
    pub duplicates: usize,
}

/// 解析结果
#[derive(Debug, Clone, Serialize)]
pub struct WordListParse {
    pub rows: Vec<ParsedWord>,
    pub errors: Vec<ParseError>,
    pub duplicates: Vec<ParseDuplicate>,
    pub stats: ParseStats,
}

/// 该 token 看起来像音标吗？
/// 接受 `/.../` 与 `[...]` 包裹，或含有常见 IPA 符号（应对没写斜杠的词表）
fn looks_like_phonetic(token: &str) -> bool {
    let t = token.trim();
    if t.chars().count() < 2 {
        return false;
    }
    let wrapped = (t.starts_with('/') && t.ends_with('/')) || (t.starts_with('[') && t.ends_with(']'));
    if wrapped && t.chars().count() > 2 {
        return true;
    }
    t.chars().any(|c| IPA_MARKERS.contains(&c))
}

/// 是否整串都是非 ASCII 字符（用来识别「单词列其实是中文」这种列错位）
fn is_all_non_ascii(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| !c.is_ascii())
}

/// 按字符安全截断，用于错误信息里回显原始行
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// 按分隔符切分并逐段 trim。**不过滤空段**，保证字段位置不错位
/// （例如 `apple||n. 苹果` 的音标为空、释义仍在第 3 段）
fn split_delimited(line: &str, sep: char) -> Vec<String> {
    line.split(sep).map(|s| s.trim().to_string()).collect()
}

/// 空格分隔的智能切分：第一个词是单词，第二个词若像音标则作音标，其余为释义
fn split_whitespace_smart(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let pos = match trimmed.find(char::is_whitespace) {
        None => return vec![trimmed.to_string()],
        Some(p) => p,
    };
    let word = trimmed[..pos].to_string();
    let rest = trimmed[pos..].trim();
    if rest.is_empty() {
        return vec![word];
    }

    // 判断第二个 token（rest 的第一段）是否像音标
    match rest.find(char::is_whitespace) {
        None => {
            if looks_like_phonetic(rest) {
                vec![word, rest.to_string(), String::new()]
            } else {
                vec![word, String::new(), rest.to_string()]
            }
        }
        Some(p2) => {
            let second = rest[..p2].trim();
            let remainder = rest[p2..].trim();
            if looks_like_phonetic(second) {
                vec![word, second.to_string(), remainder.to_string()]
            } else {
                // 第二段不像音标 → 整段 rest 都算释义（释义本身可能含空格）
                vec![word, String::new(), rest.to_string()]
            }
        }
    }
}

/// 按优先级选择切分方式：制表符 → 竖线 → 逗号 → 空格
fn split_fields(line: &str) -> Vec<String> {
    if line.contains('\t') {
        return split_delimited(line, '\t');
    }
    if line.contains('|') {
        return split_delimited(line, '|');
    }
    if line.contains(',') {
        return split_delimited(line, ',');
    }
    split_whitespace_smart(line)
}

/// 解析词表文本（纯计算，无 wasm 依赖）
pub fn parse_word_list_core(text: &str) -> WordListParse {
    let mut rows: Vec<ParsedWord> = Vec::new();
    let mut errors: Vec<ParseError> = Vec::new();
    let mut duplicates: Vec<ParseDuplicate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_lines = 0usize;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        // 跳过空行与注释
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        total_lines += 1;

        let fields = split_fields(line);
        let word = fields.first().map(|s| s.trim().to_string()).unwrap_or_default();

        if word.is_empty() {
            errors.push(ParseError {
                line: line_no,
                text: truncate(line, 80),
                reason: "缺少单词".to_string(),
            });
            continue;
        }
        if is_all_non_ascii(&word) {
            errors.push(ParseError {
                line: line_no,
                text: truncate(line, 80),
                reason: "第一列疑似中文，请检查列顺序（单词应为英文）".to_string(),
            });
            continue;
        }

        let key = word.to_lowercase();
        if !seen.insert(key) {
            duplicates.push(ParseDuplicate {
                line: line_no,
                word,
            });
            continue;
        }

        let field = |i: usize| -> String {
            fields.get(i).map(|s| s.trim().to_string()).unwrap_or_default()
        };
        rows.push(ParsedWord {
            word,
            phonetic: field(1),
            meaning: field(2),
            example: field(3),
            example_translation: field(4),
            line: line_no,
        });
    }

    WordListParse {
        stats: ParseStats {
            total_lines,
            ok: rows.len(),
            errors: errors.len(),
            duplicates: duplicates.len(),
        },
        rows,
        errors,
        duplicates,
    }
}

/// WASM 导出：解析词表文本，返回 JSON
/// `{"rows":[...],"errors":[...],"duplicates":[...],"stats":{...}}`
#[wasm_bindgen]
pub fn parse_word_list(text: &str) -> String {
    serde_json::to_string(&parse_word_list_core(text)).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_from_excel() {
        let out = parse_word_list_core("abandon\t/əˈbæn.dən/\tv. 放弃；抛弃\tHe abandoned it.");
        assert_eq!(out.rows.len(), 1);
        let r = &out.rows[0];
        assert_eq!(r.word, "abandon");
        assert_eq!(r.phonetic, "/əˈbæn.dən/");
        assert_eq!(r.meaning, "v. 放弃；抛弃");
        assert_eq!(r.example, "He abandoned it.");
    }

    #[test]
    fn parses_pipe_separated() {
        let out = parse_word_list_core("ability | /əˈbɪl.ə.ti/ | n. 能力；才能 | She has ability.");
        assert_eq!(out.rows[0].word, "ability");
        assert_eq!(out.rows[0].meaning, "n. 能力；才能");
    }

    #[test]
    fn parses_comma_separated() {
        let out = parse_word_list_core("achieve, /əˈtʃiːv/, v. 实现, She achieved it.");
        assert_eq!(out.rows[0].word, "achieve");
        assert_eq!(out.rows[0].phonetic, "/əˈtʃiːv/");
    }

    #[test]
    fn parses_word_only() {
        let out = parse_word_list_core("abandon");
        assert_eq!(out.rows[0].word, "abandon");
        assert!(out.rows[0].phonetic.is_empty());
        assert!(out.rows[0].meaning.is_empty());
    }

    #[test]
    fn space_separated_with_slashed_phonetic() {
        // 第二个词带斜杠 → 判为音标，其余为释义
        let out = parse_word_list_core("abandon /əˈbæn.dən/ v. 放弃；抛弃");
        let r = &out.rows[0];
        assert_eq!(r.word, "abandon");
        assert_eq!(r.phonetic, "/əˈbæn.dən/");
        assert_eq!(r.meaning, "v. 放弃；抛弃");
    }

    #[test]
    fn space_separated_without_slashes_still_detects_ipa() {
        // 没有斜杠但含 IPA 符号 → 仍判为音标（应对常见的中文词表）
        let out = parse_word_list_core("abandon əˈbæn.dən v. 放弃；抛弃");
        let r = &out.rows[0];
        assert_eq!(r.phonetic, "əˈbæn.dən");
        assert_eq!(r.meaning, "v. 放弃；抛弃");
    }

    #[test]
    fn space_separated_without_phonetic_keeps_meaning_whole() {
        // 第二个词不像音标 → 整段都当释义（释义里可能含空格）
        let out = parse_word_list_core("give up 放弃；认输");
        let r = &out.rows[0];
        assert_eq!(r.word, "give");
        assert_eq!(r.meaning, "up 放弃；认输");
        assert!(r.phonetic.is_empty());
    }

    #[test]
    fn skips_blank_and_comment_lines() {
        let text = "\n# 这是注释\n// 也是注释\n\nabandon\n";
        let out = parse_word_list_core(text);
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.stats.total_lines, 1, "注释与空行不计入统计");
        assert_eq!(out.rows[0].line, 5, "行号应保留原始位置");
    }

    #[test]
    fn reports_duplicates_within_file() {
        let out = parse_word_list_core("abandon\nability\nAbandon");
        assert_eq!(out.rows.len(), 2);
        assert_eq!(out.duplicates.len(), 1, "大小写不同也算重复");
        assert_eq!(out.duplicates[0].line, 3);
        assert_eq!(out.stats.duplicates, 1);
    }

    #[test]
    fn reports_chinese_first_column_as_error() {
        let out = parse_word_list_core("放弃；抛弃 | abandon");
        assert!(out.rows.is_empty());
        assert_eq!(out.errors.len(), 1);
        assert!(out.errors[0].reason.contains("中文"), "原因: {}", out.errors[0].reason);
    }

    #[test]
    fn keeps_latin_word_with_accent() {
        // 拉丁字母 + 重音符号不算「整串非 ASCII」
        let out = parse_word_list_core("café | n. 咖啡馆");
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].word, "café");
    }

    #[test]
    fn empty_phonetic_field_does_not_shift_columns() {
        // 中间字段为空时，释义不能顶到音标位置上
        let out = parse_word_list_core("apple||n. 苹果|I eat an apple.");
        let r = &out.rows[0];
        assert_eq!(r.phonetic, "", "音标应为空");
        assert_eq!(r.meaning, "n. 苹果", "释义不能被顶到音标位");
        assert_eq!(r.example, "I eat an apple.");
    }

    #[test]
    fn parses_example_translation_column() {
        // 第 5 列是例句的中文翻译
        let out = parse_word_list_core("abandon\t/əˈbæn.dən/\tv. 放弃；抛弃\tHe abandoned it.\t他放弃了它。");
        let r = &out.rows[0];
        assert_eq!(r.example, "He abandoned it.");
        assert_eq!(r.example_translation, "他放弃了它。");
    }

    #[test]
    fn missing_translation_column_is_empty() {
        let out = parse_word_list_core("apple|/ˈæp.əl/|n. 苹果|I eat an apple.");
        assert!(out.rows[0].example_translation.is_empty());
    }

    #[test]
    fn ignores_extra_fields_beyond_five() {
        let out = parse_word_list_core("apple|/ˈæp.əl/|n. 苹果|I eat an apple.|我吃苹果。|多余字段|再来一个");
        let r = &out.rows[0];
        assert_eq!(r.example, "I eat an apple.");
        assert_eq!(r.example_translation, "我吃苹果。");
        assert_eq!(out.rows.len(), 1);
    }

    #[test]
    fn stats_add_up() {
        let text = "abandon\n# 注释\n\nability\nabandon\n苹果\n";
        let out = parse_word_list_core(text);
        assert_eq!(out.stats.total_lines, 4, "空行/注释不计");
        assert_eq!(out.stats.ok, 2);
        assert_eq!(out.stats.duplicates, 1);
        assert_eq!(out.stats.errors, 1);
    }

    #[test]
    fn parses_multi_line_realistic_list() {
        let text = "# 必修一 Unit 1\n\
                    abandon\t/əˈbæn.dən/\tv. 放弃；抛弃\tHe abandoned his old car.\n\
                    ability\t/əˈbɪl.ə.ti/\tn. 能力；才能\tShe has the ability to lead.\n\
                    achieve\t/əˈtʃiːv/\tv. 实现；达到\tShe achieved her goal.\n";
        let out = parse_word_list_core(text);
        assert_eq!(out.rows.len(), 3);
        assert_eq!(out.stats.errors, 0);
        assert_eq!(out.rows[2].word, "achieve");
        assert_eq!(out.rows[2].line, 4);
    }

    #[test]
    fn phonetic_detector_edge_cases() {
        assert!(looks_like_phonetic("/əˈbæn.dən/"));
        assert!(looks_like_phonetic("[əˈbæn.dən]"));
        assert!(looks_like_phonetic("əˈbæn.dən"));
        assert!(!looks_like_phonetic("放弃"));
        assert!(!looks_like_phonetic("n."));
        assert!(!looks_like_phonetic(""));
        assert!(!looks_like_phonetic("/"));
    }
}
