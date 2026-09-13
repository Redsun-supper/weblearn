//! 卡片展示文本处理（纯计算，可在宿主上单元测试）
//!
//! 两件事：
//! 1. **拆词性**：词库里 `meaning` 存的是 `"v. 放弃；抛弃"` 这种形态，界面上要显示成
//!    `v.` 小标签 + 释义正文；
//! 2. **例句切分**：把例句按目标词切成「命中 / 未命中」片段，供前端把目标词高亮。
//!    这件事放 Rust 而不是 JS，是因为大小写与词边界的判断很容易写错，需要测试兜住
//!    （例如 `purpose` 不应该命中 `purposed`）。

use serde::Serialize;

/// 常见词性标签。**必须按长度倒序**排列，否则 `n.` 会抢先匹配掉 `num.`
const POS_TAGS: &[&str] = &[
    "abbr.", "conj.", "prep.", "pron.", "adj.", "adv.", "art.", "aux.", "int.", "num.", "vt.", "vi.",
    "n.", "v.",
];

/// 词性标签之间的分隔符
const POS_SEPARATORS: &[char] = &['/', '、', ','];

/// 拆出的词性 + 释义
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordMeaning {
    /// 词性标签，如 `"v."`；多个时用 `/` 连接（如 `"n./v."`）；没有则为空串
    pub pos: String,
    /// 去掉词性前缀后的释义正文
    pub text: String,
}

/// 把 `meaning` 拆成词性 + 正文。
///
/// 支持连续多个词性（`n./v.`、`adj. adv.`），也支持完全没有词性（直接返回原文）。
pub fn split_pos(meaning: &str) -> WordMeaning {
    let mut rest = meaning.trim();
    let mut pos = String::new();

    loop {
        let trimmed = rest.trim_start();
        let mut matched_len = None;
        for tag in POS_TAGS {
            // 用 get() 而不是切片索引：避免在多字节字符中间切断而 panic
            if let Some(head) = trimmed.get(..tag.len()) {
                if head.eq_ignore_ascii_case(tag) {
                    matched_len = Some(tag.len());
                    break;
                }
            }
        }
        match matched_len {
            None => break,
            Some(len) => {
                if !pos.is_empty() {
                    pos.push('/');
                }
                pos.push_str(&trimmed[..len]);
                // 吃掉标签后面的分隔符（/ 、 , 与空白）
                let after = &trimmed[len..];
                let after = after.trim_start();
                let after = after.trim_start_matches(POS_SEPARATORS);
                rest = after;
            }
        }
    }

    WordMeaning {
        pos,
        text: rest.trim().to_string(),
    }
}

/// 例句片段
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExamplePart {
    pub text: String,
    /// 是否为命中的目标词
    pub hit: bool,
}

/// 构成「词」的字节：字母数字，以及词内允许的 `'` 与 `-`
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'\'' || b == b'-'
}

/// 在 `haystack` 中查找 `needle`（ASCII 大小写不敏感），返回字节区间。
///
/// 优先返回**词边界**匹配，避免 `purpose` 命中 `purposed` 的中间；
/// 若找不到边界匹配，退回第一个普通命中（例如例句里就是 `purposes`）。
fn find_ci(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    let mut fallback: Option<(usize, usize)> = None;

    for start in 0..=(h.len() - n.len()) {
        if !haystack.is_char_boundary(start) {
            continue;
        }
        let mut same = true;
        for k in 0..n.len() {
            if !h[start + k].eq_ignore_ascii_case(&n[k]) {
                same = false;
                break;
            }
        }
        if !same {
            continue;
        }
        let end = start + n.len();
        let boundary_ok = (start == 0 || !is_word_byte(h[start - 1]))
            && (end >= h.len() || !is_word_byte(h[end]));
        if boundary_ok {
            return Some((start, end));
        }
        if fallback.is_none() {
            // 退回匹配时把范围扩展成完整单词：例句里是 purposes 时整体高亮，
            // 而不是只亮出 purpose 再留个 s 在外面（视觉上很怪）。
            // 只在 ASCII 单词字节上扩展，因此不会切进多字节字符中间。
            let mut s = start;
            while s > 0 && is_word_byte(h[s - 1]) {
                s -= 1;
            }
            let mut e = end;
            while e < h.len() && is_word_byte(h[e]) {
                e += 1;
            }
            fallback = Some((s, e));
        }
    }
    fallback
}

/// 把例句按目标词切分；找不到时返回单个未命中片段（前端照常显示，只是没有高亮）
pub fn split_example(example: &str, word: &str) -> Vec<ExamplePart> {
    let ex = example.trim();
    if ex.is_empty() {
        return Vec::new();
    }
    match find_ci(ex, word.trim()) {
        None => vec![ExamplePart {
            text: ex.to_string(),
            hit: false,
        }],
        Some((start, end)) => {
            let mut parts = Vec::new();
            if start > 0 {
                parts.push(ExamplePart {
                    text: ex[..start].to_string(),
                    hit: false,
                });
            }
            parts.push(ExamplePart {
                text: ex[start..end].to_string(),
                hit: true,
            });
            if end < ex.len() {
                parts.push(ExamplePart {
                    text: ex[end..].to_string(),
                    hit: false,
                });
            }
            parts
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_single_pos() {
        let m = split_pos("v. 放弃；抛弃");
        assert_eq!(m.pos, "v.");
        assert_eq!(m.text, "放弃；抛弃");
    }

    #[test]
    fn splits_adj_and_adv() {
        assert_eq!(split_pos("adj. 独立的；自主的").pos, "adj.");
        assert_eq!(split_pos("adv. 逐渐地").pos, "adv.");
        assert_eq!(split_pos("prep. 在……之间").pos, "prep.");
    }

    #[test]
    fn splits_multiple_pos_joined_by_slash() {
        let m = split_pos("n./v. 影响");
        assert_eq!(m.pos, "n./v.");
        assert_eq!(m.text, "影响");
    }

    #[test]
    fn splits_multiple_pos_separated_by_space() {
        let m = split_pos("adj. adv. 好的");
        assert_eq!(m.pos, "adj./adv.");
        assert_eq!(m.text, "好的");
    }

    #[test]
    fn longest_tag_wins() {
        // "num." 不能被 "n." 抢先匹配
        let m = split_pos("num. 数字");
        assert_eq!(m.pos, "num.");
        assert_eq!(m.text, "数字");
        let m2 = split_pos("conj. 但是");
        assert_eq!(m2.pos, "conj.");
    }

    #[test]
    fn meaning_without_pos_is_kept_as_text() {
        let m = split_pos("目的；意图");
        assert_eq!(m.pos, "");
        assert_eq!(m.text, "目的；意图");
    }

    #[test]
    fn does_not_panic_on_multibyte_without_pos() {
        // 中文开头时不能因为按字节切片而 panic
        let m = split_pos("释义直接写中文");
        assert_eq!(m.pos, "");
        assert_eq!(m.text, "释义直接写中文");
    }

    #[test]
    fn empty_meaning() {
        let m = split_pos("   ");
        assert_eq!(m.pos, "");
        assert_eq!(m.text, "");
    }

    #[test]
    fn highlights_target_word() {
        let parts = split_example("The purpose of this meeting is to discuss.", "purpose");
        assert_eq!(parts.len(), 3);
        assert!(!parts[0].hit);
        assert_eq!(parts[1].text, "purpose");
        assert!(parts[1].hit);
        assert!(parts[2].text.starts_with(" of this meeting"));
    }

    #[test]
    fn highlight_is_case_insensitive_and_keeps_original_case() {
        let parts = split_example("Purpose matters here.", "purpose");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].text, "Purpose", "高亮片段要保留原文大小写");
        assert!(parts[0].hit);
    }

    #[test]
    fn prefers_word_boundary_over_substring() {
        // 句中先出现 purposed（不该命中），后出现 purpose（该命中）
        let parts = split_example("He purposed it, but the purpose was clear.", "purpose");
        let hit = parts.iter().find(|p| p.hit).unwrap();
        assert_eq!(hit.text, "purpose");
        // 前面那段应包含 purposed
        assert!(parts[0].text.contains("purposed"));
    }

    #[test]
    fn falls_back_to_substring_when_no_boundary_match() {
        // 例句里只有复数形式：应把整个 purposes 高亮出来（而不是只亮 purpose）
        let parts = split_example("Many purposes exist.", "purpose");
        let hit = parts.iter().find(|p| p.hit).expect("没有边界匹配时应退回普通命中");
        assert_eq!(hit.text, "purposes");
    }

    #[test]
    fn fallback_expands_to_full_word() {
        // 变形词：purposed / purposes 都应整体高亮
        let a = split_example("He purposed to go.", "purpose");
        assert_eq!(a.iter().find(|p| p.hit).unwrap().text, "purposed");
        let b = split_example("Its purposes vary.", "purpose");
        assert_eq!(b.iter().find(|p| p.hit).unwrap().text, "purposes");
    }

    #[test]
    fn no_match_returns_single_part() {
        let parts = split_example("Nothing to see here.", "purpose");
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].hit);
        assert_eq!(parts[0].text, "Nothing to see here.");
    }

    #[test]
    fn empty_example_returns_empty() {
        assert!(split_example("", "purpose").is_empty());
        assert!(split_example("   ", "purpose").is_empty());
    }

    #[test]
    fn empty_word_returns_single_part() {
        let parts = split_example("Some sentence.", "");
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].hit);
    }

    #[test]
    fn highlights_word_at_start_and_end() {
        let a = split_example("Purpose is key.", "purpose");
        assert_eq!(a.len(), 2);
        assert!(a[0].hit);
        let b = split_example("This is purpose", "purpose");
        assert_eq!(b.len(), 2);
        assert!(b[1].hit);
        assert_eq!(b[1].text, "purpose");
    }

    #[test]
    fn only_first_occurrence_is_highlighted() {
        let parts = split_example("purpose and purpose", "purpose");
        assert_eq!(parts.iter().filter(|p| p.hit).count(), 1);
    }

    #[test]
    fn punctuation_adjacent_still_matches() {
        let parts = split_example("(purpose), indeed!", "purpose");
        let hit = parts.iter().find(|p| p.hit).expect("应命中");
        assert_eq!(hit.text, "purpose");
    }
}
