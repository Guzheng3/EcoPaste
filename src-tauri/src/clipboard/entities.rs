//! 剪贴板文本内的可交互实体提取：链接 / 邮箱 / 手机号 / QQ 号。
//!
//! 与 `secrets` 的「判断是否存在」不同，这里需要给出实体的精确切片并按出现顺序返回，
//! 用于前端把一条长文本里散落的链接、号码提出来做一键打开 / 键入。
//!
//! 长文本里各实体可能互相嵌套（URL 里的端口数字、路径片段含数字、邮箱域名等），
//! 因此这里按「URL → 邮箱 → 手机号 → QQ」的优先级顺序提取，并维护已占用的字节区间，
//! 跳过落在已提取实体内部的子串，避免重复命中同一段文本。

use regex::{Match, Regex};
use std::sync::OnceLock;

/// 实体类型。序列化为小写字符串（url / email / phone / qq）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Url,
    Email,
    Phone,
    Qq,
}

/// 一个提取出的实体：`start` / `end` 为原文中的字节区间（仅用于前端排序与去重，不必展示）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedEntity {
    pub kind: EntityKind,
    pub value: String,
    pub start: usize,
    pub end: usize,
}

/// 从 `text` 中按出现顺序提取链接 / 邮箱 / 手机号 / QQ 号。
pub fn extract_entities(text: &str) -> Vec<ExtractedEntity> {
    let value = text.trim();
    if value.is_empty() {
        return Vec::new();
    }

    let mut occupied: Vec<(usize, usize)> = Vec::new();
    let mut out: Vec<ExtractedEntity> = Vec::new();

    // 链接优先级最高：其域名、端口、路径数字都要占住，避免被后续号码规则拆碎成子串。
    push_matches(value, &mut out, &mut occupied);
    push_emails(value, &mut out, &mut occupied);
    push_phones(value, &mut out, &mut occupied);
    push_qqs(value, &mut out, &mut occupied);

    out.sort_by_key(|entity| entity.start);
    out
}

/// 提取 http/https/www 链接，去掉常见的尾部停顿标点。
fn push_matches(text: &str, out: &mut Vec<ExtractedEntity>, occupied: &mut Vec<(usize, usize)>) {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    let re = URL_RE
        .get_or_init(|| Regex::new(r#"\b(?:https?://|www\.)[^\s<>"']+"#).expect("valid regex"));

    for m in re.find_iter(text) {
        if overlaps(m.start(), m.end(), occupied) {
            continue;
        }
        let raw = &text[m.start()..m.end()];
        let end = trim_trailing_punctuation(raw);
        push_slice(
            text,
            out,
            occupied,
            EntityKind::Url,
            &raw[..end],
            m.start(),
            m.start() + end,
        );
    }
}

/// 提取满足基本结构的邮箱。
fn push_emails(text: &str, out: &mut Vec<ExtractedEntity>, occupied: &mut Vec<(usize, usize)>) {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    let re = EMAIL_RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").expect("valid regex")
    });

    for m in re.find_iter(text) {
        if overlaps(m.start(), m.end(), occupied) {
            continue;
        }
        push_slice(
            text,
            out,
            occupied,
            EntityKind::Email,
            &text[m.start()..m.end()],
            m.start(),
            m.end(),
        );
    }
}

/// 提取中国大陆手机号：1[3-9] 开头共 11 位，允许 `+86` 前缀与常见分隔符。
/// 要求前后边界不是数字，避免把银行卡 / 长数字串中的子串误判为手机号。
fn push_phones(text: &str, out: &mut Vec<ExtractedEntity>, occupied: &mut Vec<(usize, usize)>) {
    static PHONE_RE: OnceLock<Regex> = OnceLock::new();
    let re = PHONE_RE
        .get_or_init(|| Regex::new(r"(?:\+?86[-\s]?)?1[3-9](?:[-\s]?\d){9}").expect("valid regex"));

    for m in re.find_iter(text) {
        if !is_digit_isolated(text, &m) || overlaps(m.start(), m.end(), occupied) {
            continue;
        }
        push_slice(
            text,
            out,
            occupied,
            EntityKind::Phone,
            &text[m.start()..m.end()],
            m.start(),
            m.end(),
        );
    }
}

/// 提取纯数字 QQ 号（5–10 位）。要求独立的数字块，且不落在已提取（URL / 邮箱 / 手机号）区间内，
/// 避免把链接数字、邮箱里的数字、手机号子串误判为 QQ。
fn push_qqs(text: &str, out: &mut Vec<ExtractedEntity>, occupied: &mut Vec<(usize, usize)>) {
    static QQ_RE: OnceLock<Regex> = OnceLock::new();
    let re = QQ_RE.get_or_init(|| Regex::new(r"[1-9]\d{4,9}").expect("valid regex"));

    for m in re.find_iter(text) {
        if !is_digit_isolated(text, &m) || overlaps(m.start(), m.end(), occupied) {
            continue;
        }
        push_slice(
            text,
            out,
            occupied,
            EntityKind::Qq,
            &text[m.start()..m.end()],
            m.start(),
            m.end(),
        );
    }
}

/// 归一化后追加一个实体，并把其字节区间登记为已占用。
#[allow(clippy::too_many_arguments)]
fn push_slice(
    text: &str,
    out: &mut Vec<ExtractedEntity>,
    occupied: &mut Vec<(usize, usize)>,
    kind: EntityKind,
    value: &str,
    start: usize,
    end: usize,
) {
    // 说明提取结果不可见字符/已退变成空，丢弃。
    debug_assert!(start <= end && end <= text.len(), "out of bounds slice");
    if value.is_empty() {
        return;
    }
    occupied.push((start, end));
    out.push(ExtractedEntity {
        kind,
        value: value.to_owned(),
        start,
        end,
    });
}

/// 判断 `m` 是否落在任一已占用区间内（含相邻端点跨接视为重叠）。
fn overlaps(start: usize, end: usize, occupied: &[(usize, usize)]) -> bool {
    occupied.iter().any(|&(s, e)| start < e && s < end)
}

/// 匹配前后都不是数字，视为独立数字块。
fn is_digit_isolated(text: &str, m: &Match<'_>) -> bool {
    let left_clear = text[..m.start()]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_ascii_digit());
    let right_clear = text[m.end()..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_digit());
    left_clear && right_clear
}

/// 去掉链接末尾常见的停顿标点（`.` `,` `;` `:` `!` `?`）。
/// 刻意不剥 `)` `]` `}`：这类闭合括号可能是链接路径的合法组成部分（如 `path/a(b)`）。
fn trim_trailing_punctuation(value: &str) -> usize {
    let mut end = value.len();
    while end > 0 {
        let c = value[..end].chars().next_back().expect("non-empty prefix");
        if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') {
            end -= c.len_utf8();
        } else {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::{extract_entities, EntityKind};

    fn kinds(text: &str) -> Vec<EntityKind> {
        extract_entities(text)
            .into_iter()
            .map(|entity| entity.kind)
            .collect()
    }

    fn values(text: &str) -> Vec<String> {
        extract_entities(text)
            .into_iter()
            .map(|entity| entity.value)
            .collect()
    }

    #[test]
    fn extracts_urls_and_trims_punctuation() {
        assert_eq!(
            values("看这里 https://example.com/a(b), 谢谢"),
            vec!["https://example.com/a(b)"]
        );
        assert_eq!(values("访问 www.example.com."), vec!["www.example.com"]);
    }

    #[test]
    fn extracts_emails_and_phones() {
        assert_eq!(
            values("联系 tom@example.com 或 138-1234-5678"),
            vec!["tom@example.com", "138-1234-5678"]
        );
        assert_eq!(
            kinds("a@b.cn +8613812345678"),
            vec![EntityKind::Email, EntityKind::Phone]
        );
    }

    #[test]
    fn extracts_qq_only_as_isolated_digit_block() {
        assert_eq!(values("我的 QQ 是 1234567890"), vec!["1234567890"]);
        assert_eq!(kinds("号码 1912345678"), vec![EntityKind::Qq]);
        // 11 位 1[3-9] 开头是手机号，QQ 位宽最多 10 位不应误拾到其子串。
        assert_eq!(kinds("13812345678"), vec![EntityKind::Phone]);
        // 长数字串内部的子块不是独立块，不应给出 QQ。
        assert!(extract_entities("6222021234567890123").is_empty());
    }

    #[test]
    fn url_digits_not_split_into_qq() {
        assert_eq!(
            values("详情见 https://example.com/path?page=12345"),
            vec!["https://example.com/path?page=12345"]
        );
    }

    #[test]
    fn empty_text_yields_nothing() {
        assert!(extract_entities("   ").is_empty());
        assert!(extract_entities("").is_empty());
    }
}
