//! 卡片展示文本处理（纯计算，可在宿主上单元测试）
//!
//! 三件事：
//! 1. **拆词性**：词库里 `meaning` 存的是 `"v. 放弃；抛弃"` 这种形态，界面上要显示成
//!    `v.` 小标签 + 释义正文；一个词有多个义项时（`"n. 好处；益处 v. 有益于"`）
//!    还要拆成多块，一块一个词性——这就是 `split_senses`；
//! 2. **例句切分**：把例句按目标词切成「命中 / 未命中」片段，供前端把目标词高亮。
//!    这件事放 Rust 而不是 JS，是因为大小写与词边界的判断很容易写错，需要测试兜住
//!    （例如 `purpose` 不应该命中 `purposed`）。

use serde::Serialize;

/// 常见词性标签。**必须按长度倒序**排列，否则 `n.` 会抢先匹配掉 `num.`
const POS_TAGS: &[&str] = &[
    "abbr.", "conj.", "prep.", "pron.", "adj.", "adv.", "art.", "aux.", "int.", "num.", "vt.", "vi.",
    "n.", "v.",
];

const POS_SEPARATORS: &[char] = &['/', '、', ','];

/// 词性标签内外都算分隔的字符：空白 / 斜杠 / 顿号 / 逗号 / 分号
///
/// 用途有二：判断标签是否「独立成词」，以及把释义正文尾部多余的分隔符剪掉
/// （`"n. 好处； v. 益处"` 里的 `好处；` 要剪成 `好处`）。
fn is_tag_separator(c: char) -> bool {
    c.is_whitespace() || POS_SEPARATORS.contains(&c) || c == '；' || c == ';'
}

/// 剪掉首尾空白与尾部的分隔符
fn trim_body(text: &str) -> &str {
    text.trim().trim_end_matches(is_tag_separator)
}

/// 拆出的词性 + 释义
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordMeaning {
    /// 词性标签，如 `"v."`；多个时用 `/` 连接（如 `"n./v."`）；没有则为空串
    pub pos: String,
    /// 去掉词性前缀后的释义正文
    pub text: String,
}

/// 判断 `text` 的 `i` 位置是不是一个词性标签，是则返回标签的字节长度。
///
/// 要求标签**独立成词**：前面是开头 / 空白 / 分隔符，后面是结尾 / 空白 / 分隔符。
/// 没有这条约束的话，释义正文里出现的英文（例如 `"vt. 相当于 not."`）会被误当成词性。
fn match_pos_tag(text: &str, i: usize) -> Option<usize> {
    if i > 0 {
        let prev = text[..i].chars().next_back()?;
        if !is_tag_separator(prev) {
            return None;
        }
    }
    for tag in POS_TAGS {
        let end = i + tag.len();
        // 用 get() 而不是切片索引：避免在多字节字符中间切断而 panic
        let head = match text.get(i..end) {
            Some(h) => h,
            None => continue,
        };
        if !head.eq_ignore_ascii_case(tag) {
            continue;
        }
        let next_ok = match text[end..].chars().next() {
            None => true,
            Some(c) => is_tag_separator(c),
        };
        if next_ok {
            return Some(tag.len());
        }
    }
    None
}

/// 把 `meaning` 拆成词性 + 正文。
///
/// 只处理**开头的连续词性标签**（`"n./v. 影响"` → `pos = "n./v."`）。
/// 一个词条里塞了多个义项时请用 [`split_senses`]。
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

/// 把一个词条的 `meaning` 拆成**多条释义**：每遇到一个词性标签就另起一块。
///
/// 词库里历史数据把多个义项写在同一行里（`"n. 好处；益处 v. 有益于"`），
/// 后台也允许这么填，所以展示层要能把它们拆开——界面上就是「名词一块、动词一块」。
///
/// 规则：
/// - 连续的多个标签归下一块释义（`"adj. adv. 好的"` → 一块，词性显示 `adj./adv.`）；
/// - 正文出现在任何标签之前时，自成一块且词性为空；
/// - 释义正文尾部的分隔符会被剪掉（`"好处；"` → `"好处"`）；
/// - 空串返回空列表（前端据此不渲染释义区）。
pub fn split_senses(meaning: &str) -> Vec<WordMeaning> {
    let text = meaning.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut senses: Vec<WordMeaning> = Vec::new();
    // 已收集、尚未配到正文的词性标签
    let mut pending_pos: Vec<String> = Vec::new();
    // 当前这块正文的起点
    let mut cursor = 0usize;
    let mut i = 0usize;

    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let len = match match_pos_tag(text, i) {
            None => {
                i += 1;
                continue;
            }
            Some(len) => len,
        };

        // 标签之前攒下的正文归上一块
        let body = trim_body(&text[cursor..i]);
        if !body.is_empty() {
            senses.push(WordMeaning {
                pos: pending_pos.join("/"),
                text: body.to_string(),
            });
            pending_pos.clear();
        }

        pending_pos.push(text[i..i + len].to_string());
        i += len;
        // 跳过标签后面的分隔符（含中文分号）
        while i < text.len() {
            let c = match text[i..].chars().next() {
                Some(c) => c,
                None => break,
            };
            if !is_tag_separator(c) {
                break;
            }
            i += c.len_utf8();
        }
        cursor = i;
    }

    // 收尾：最后一段正文（词性是前面攒下的那些标签）
    let tail = trim_body(&text[cursor..]);
    if !tail.is_empty() {
        senses.push(WordMeaning {
            pos: pending_pos.join("/"),
            text: tail.to_string(),
        });
    }

    senses
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

/// 生成目标词在例句里可能出现的变形。
///
/// 英文例句经常用变形而不是原形（`apply` → `applied`、`study` → `studies`），
/// 而 `applied` 里并不含 `apply` 这个子串，只按原形匹配会漏掉高亮。
/// 这里按常见构词规则生成候选，够用且完全可测；不追求覆盖全部不规则变化。
fn word_forms(word: &str) -> Vec<String> {
    let w = word.trim().to_lowercase();
    if w.is_empty() {
        return Vec::new();
    }
    let mut forms = vec![w.clone()];

    // 以 y 结尾：study → studies / studied
    if let Some(stem) = w.strip_suffix('y') {
        forms.push(format!("{stem}ies"));
        forms.push(format!("{stem}ied"));
    }
    // 以 e 结尾：use → used / using
    if let Some(stem) = w.strip_suffix('e') {
        forms.push(format!("{stem}ed"));
        forms.push(format!("{stem}ing"));
    }

    forms.push(format!("{w}s"));
    forms.push(format!("{w}es"));
    forms.push(format!("{w}ed"));
    forms.push(format!("{w}d"));
    forms.push(format!("{w}ing"));

    // 双写尾辅音：stop → stopped / stopping
    if let Some(last) = w.chars().last() {
        if "bdgklmnprt".contains(last) {
            forms.push(format!("{w}{last}ed"));
            forms.push(format!("{w}{last}ing"));
        }
    }
    forms
}

/// 把例句按目标词切分；找不到时返回单个未命中片段（前端照常显示，只是没有高亮）
///
/// 匹配优先级：
/// 1. **原形**（含词边界优先、退回时扩展成整词）——它是词条本身，句中出现时应优先高亮
/// 2. 原形缺席时，再试常见变形（`apply` → `applied`）；多个变形命中时取最靠前的
pub fn split_example(example: &str, word: &str) -> Vec<ExamplePart> {
    let ex = example.trim();
    if ex.is_empty() {
        return Vec::new();
    }

    let mut best = find_ci(ex, word.trim());
    if best.is_none() {
        for form in word_forms(word).into_iter().skip(1) {
            if let Some((start, end)) = find_ci(ex, &form) {
                let better = match best {
                    None => true,
                    Some((bs, be)) => start < bs || (start == bs && (end - start) > (be - bs)),
                };
                if better {
                    best = Some((start, end));
                }
            }
        }
    }

    match best {
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

    // ---------- 多条释义拆分 ----------

    #[test]
    fn splits_two_senses_in_one_line() {
        // 词库里的真实数据：名词一块、动词一块
        let s = split_senses("n. 好处；益处 v. 有益于");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].pos, "n.");
        assert_eq!(s[0].text, "好处；益处");
        assert_eq!(s[1].pos, "v.");
        assert_eq!(s[1].text, "有益于");
    }

    #[test]
    fn splits_two_senses_with_ellipsis_text() {
        let s = split_senses("n. 挑战 v. 向……挑战");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].text, "挑战");
        assert_eq!(s[1].pos, "v.");
        assert_eq!(s[1].text, "向……挑战");
    }

    #[test]
    fn single_sense_stays_single() {
        let s = split_senses("v. 放弃；抛弃");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos, "v.");
        assert_eq!(s[0].text, "放弃；抛弃");
    }

    #[test]
    fn sense_without_pos_is_kept() {
        let s = split_senses("目的；意图");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos, "");
        assert_eq!(s[0].text, "目的；意图");
    }

    #[test]
    fn consecutive_tags_share_one_sense() {
        let s = split_senses("adj. adv. 好的");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos, "adj./adv.");
        assert_eq!(s[0].text, "好的");
    }

    #[test]
    fn slash_separated_tags_share_one_sense() {
        let s = split_senses("n./v. 影响");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos, "n./v.");
        assert_eq!(s[0].text, "影响");
    }

    #[test]
    fn longest_tag_wins_in_multi_sense() {
        // num. 不能被 n. 抢先匹配，否则会拆成 "n." + "um. 数字"
        let s = split_senses("num. 数字");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos, "num.");
        assert_eq!(s[0].text, "数字");
    }

    #[test]
    fn tag_inside_parentheses_is_not_split() {
        // 括号里的 abbr. 前后不是分隔符，不该被当成新义项
        let s = split_senses("adv. 副词（abbr. 缩写）");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos, "adv.");
        assert_eq!(s[0].text, "副词（abbr. 缩写）");
    }

    #[test]
    fn trailing_separators_are_trimmed() {
        let s = split_senses("n. 好处； v. 益处");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].text, "好处", "尾部的中文分号应剪掉");
        assert_eq!(s[1].text, "益处");
    }

    #[test]
    fn meaning_of_only_tags_yields_no_sense() {
        assert!(split_senses("n. v.").is_empty());
    }

    #[test]
    fn empty_meaning_has_no_senses() {
        assert!(split_senses("").is_empty());
        assert!(split_senses("   ").is_empty());
    }

    #[test]
    fn does_not_panic_on_multibyte() {
        let s = split_senses("中文释义直接写 v. 动词义");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].pos, "");
        assert_eq!(s[0].text, "中文释义直接写");
        assert_eq!(s[1].pos, "v.");
        assert_eq!(s[1].text, "动词义");
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

    // ---------- 词形变化匹配 ----------

    #[test]
    fn highlights_inflected_form_y_to_ied() {
        // 真实案例：词条是 apply，例句里却是 applied（并不含 apply 这个子串）
        let parts = split_example("She applied for the job yesterday.", "apply");
        let hit = parts.iter().find(|p| p.hit).expect("变形也应高亮");
        assert_eq!(hit.text, "applied");
    }

    #[test]
    fn highlights_common_inflections() {
        let cases = [
            ("She applied for the job.", "apply", "applied"),
            ("He studies every day.", "study", "studies"),
            ("They studied hard.", "study", "studied"),
            ("I am using it.", "use", "using"),
            ("He used it.", "use", "used"),
            ("She stopped there.", "stop", "stopped"),
            ("They are stopping now.", "stop", "stopping"),
            ("Two apples here.", "apple", "apples"),
            ("He watches TV.", "watch", "watches"),
        ];
        for (sentence, word, expect) in cases {
            let parts = split_example(sentence, word);
            let hit = parts
                .iter()
                .find(|p| p.hit)
                .unwrap_or_else(|| panic!("{word} 在「{sentence}」中应命中"));
            assert_eq!(hit.text, expect, "句子「{sentence}」");
        }
    }

    #[test]
    fn prefers_earliest_match_across_forms() {
        // 原形出现在变形之前 → 取原形
        let parts = split_example("Purpose matters, he purposed it.", "purpose");
        let hit = parts.iter().find(|p| p.hit).unwrap();
        assert_eq!(hit.text, "Purpose");
    }

    #[test]
    fn word_forms_covers_common_suffixes() {
        let forms = word_forms("apply");
        for want in ["apply", "applies", "applied", "applying"] {
            assert!(forms.contains(&want.to_string()), "缺少 {want}");
        }
        assert!(word_forms("").is_empty());
        assert!(word_forms("stop").contains(&"stopped".to_string()));
        assert!(word_forms("stop").contains(&"stopping".to_string()));
    }

    #[test]
    fn does_not_highlight_unrelated_word() {
        let parts = split_example("Nothing to see here.", "apply");
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].hit);
    }
}
