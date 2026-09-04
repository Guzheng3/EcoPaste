//! 文本「编辑」：拆词、链接/域名/邮箱/手机号提取。
//!
//! 分词策略：
//! - 中文走 jieba 词典切分；
//! - 英文 / 数字按成词片段切分，标点剥离为独立词块；
//! - 纯空白块丢弃。
//!
//! 提取规则：
//! - 链接：http(s)://、www. 前缀、裸域名（如 qinghan.vip）；
//! - 邮箱：xx@xx.xx；
//! - 手机号：1 开头的 11 位中国手机号（1[3-9]\d{9}）。

use jieba_rs::Jieba;
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

use super::entities::BARE_DOMAIN_TLDS;

// ---------------------------------------------------------------------------
// 正则：惰性编译，首次使用后复用
// ---------------------------------------------------------------------------

/// 提取链接用的正则：两条分支——带协议/www 的完整 URL，以及 TLD 白名单内的裸域名。
/// 裸域名分支左端刻意不用词边界（汉字也是单词字符，`\b` 在中英文交界处不成立，
/// 否则 `打开qinghan.vip` 提不出）；右端 TLD 后要求 `\b`，防止 `com` 误配到
/// `combination` 这类单词内部前缀。路径可选，末尾标点由字符类排除。
fn link_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let pattern = format!(
            r#"(?i)\b(?:(?:https?|ftp)://|www\.)[^\s<>\"\x27)]*[^\s<>\"\x27).,;:!?]|(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+(?:{BARE_DOMAIN_TLDS})\b(?:/[^\s<>\"\x27)]*[^\s<>\"\x27).,;:!?])?"#
        );
        Regex::new(&pattern).expect("link regex")
    })
}

fn email_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").expect("email regex")
    })
}

fn phone_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"1[3-9]\d{9}").expect("phone regex"))
}

// ---------------------------------------------------------------------------
// 进程级 jieba 单例
// ---------------------------------------------------------------------------

fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

// ---------------------------------------------------------------------------
// 提取函数
// ---------------------------------------------------------------------------

/// 提取所有链接（含协议链接、www 前缀、裸域名）。
/// 邮箱的域名部分（`a@b.com` 中的 `b.com`）前一个字符是 `@`，留给邮箱规则处理，这里跳过。
pub fn extract_links(text: &str) -> Vec<String> {
    let mut links: Vec<String> = link_regex()
        .find_iter(text)
        .filter(|m| !text[..m.start()].ends_with('@'))
        .map(|m| m.as_str().to_owned())
        .collect();
    links.sort();
    links.dedup();
    links
}

/// 提取所有邮箱地址。
pub fn extract_emails(text: &str) -> Vec<String> {
    let mut emails: Vec<String> = email_regex()
        .find_iter(text)
        .map(|m| m.as_str().to_owned())
        .collect();
    emails.sort();
    emails.dedup();
    emails
}

/// 提取所有中国手机号。
pub fn extract_phones(text: &str) -> Vec<String> {
    let mut phones: Vec<String> = phone_regex()
        .find_iter(text)
        .map(|m| m.as_str().to_owned())
        .collect();
    phones.sort();
    phones.dedup();
    phones
}

// ---------------------------------------------------------------------------
// 拆词
// ---------------------------------------------------------------------------

/// 是否为「应丢弃」的块：空、纯空白。
fn is_discardable(block: &str) -> bool {
    block.trim().is_empty()
}

/// 是否属于「构成词」的字符：汉字、字母、数字、下划线。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || cmp_cjk(c) || c == '_'
}

/// 判断一个词块内是否还混有需要二次切分的非中文片段（英文/数字连续夹着标点）。
fn needs_ascii_split(block: &str) -> bool {
    let mut mixed_word = false;
    let mut mixed_ascii = false;
    for c in block.chars() {
        let word = is_word_char(c);
        if word && c.is_ascii_alphanumeric() {
            mixed_ascii = true;
        } else if word && !c.is_ascii_alphanumeric() {
            mixed_word = true;
        }
        if mixed_word && mixed_ascii {
            return true;
        }
    }
    false
}

/// 把一段英文/数字连写按标点/空白二次切成成词片段，标点作为独立词块保留。
fn split_ascii_blocks(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for c in block.chars() {
        if c.is_whitespace() {
            if !current.is_empty() {
                out.push(current.clone());
                current.clear();
            }
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            current.push(c);
        } else {
            // 标点：先 flush 当前词块，再单独加入标点
            if !current.is_empty() {
                out.push(current.clone());
                current.clear();
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// 将文本拆成有序词块，标点/符号作为独立词块保留。
pub fn segment_text(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let words = jieba().cut_for_search(text, false);
    let mut out = Vec::with_capacity(words.len());

    for word in words {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }

        // 中文词块（含汉字）直接保留。
        if !needs_ascii_split(word) && word.chars().any(cmp_cjk) {
            if !is_discardable(word) {
                out.push(word.to_owned());
            }
            continue;
        }

        for piece in split_ascii_blocks(word) {
            if !is_discardable(&piece) {
                out.push(piece);
            }
        }
    }

    out
}

fn cmp_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

// ---------------------------------------------------------------------------
// 编辑结果
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentEditResult {
    pub text: String,
    pub blocks: Vec<String>,
    pub links: Vec<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

/// 对文本做完整编辑分析：拆词 + 链接/域名/邮箱/手机号提取。
pub fn segment_edit(text: &str) -> SegmentEditResult {
    SegmentEditResult {
        text: text.to_owned(),
        blocks: segment_text(text),
        links: extract_links(text),
        emails: extract_emails(text),
        phones: extract_phones(text),
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_chinese_with_punctuation() {
        let blocks = segment_text("你好，世界！");
        assert!(blocks.contains(&"，".to_owned()) || blocks.contains(&"！".to_owned()));
        assert!(blocks.iter().any(|w| w == "你好"));
        assert!(blocks.iter().any(|w| w == "世界"));
    }

    #[test]
    fn segments_chinese_sentence_into_words() {
        let blocks = segment_text("修复后新装即默认弹气泡");
        assert!(!blocks.is_empty());
        assert!(blocks.len() >= 4, "got {blocks:?}");
    }

    #[test]
    fn splits_english_by_punctuation() {
        let blocks = segment_text("hello,world  foo bar");
        assert!(blocks.contains(&"hello".to_owned()));
        assert!(blocks.contains(&",".to_owned()));
        assert!(blocks.contains(&"world".to_owned()));
        assert!(blocks.contains(&"foo".to_owned()));
        assert!(blocks.contains(&"bar".to_owned()));
    }

    #[test]
    fn drops_pure_whitespace() {
        let blocks = segment_text("， 。  ,.!@$   ");
        for b in &blocks {
            assert!(!b.trim().is_empty(), "unexpected block: {b:?}");
        }
    }

    #[test]
    fn handles_mixed_chinese_english() {
        let blocks = segment_text("打开GitHub程序并复制code");
        assert!(blocks.iter().any(|w| w == "GitHub"));
        assert!(blocks.iter().any(|w| w == "打开"));
    }

    #[test]
    fn empty_input_yields_empty_blocks() {
        assert!(segment_text("   ").is_empty());
        assert!(segment_text("").is_empty());
    }

    #[test]
    fn extracts_links_with_protocol() {
        let links = extract_links("访问 https://www.example.com/path 或 http://test.cn");
        assert!(links.iter().any(|l| l.contains("example.com")));
        assert!(links.iter().any(|l| l.contains("test.cn")));
    }

    #[test]
    fn extracts_www_links() {
        let links = extract_links("去 www.baidu.com 看看");
        assert!(links.iter().any(|l| l == "www.baidu.com"));
    }

    #[test]
    fn extracts_bare_domains() {
        let links = extract_links("访问 qinghan.vip 和 video.qinghan.vip");
        assert!(links.contains(&"qinghan.vip".to_owned()));
        assert!(links.contains(&"video.qinghan.vip".to_owned()));
    }

    #[test]
    fn extracts_emails() {
        let emails = extract_emails("联系 test@example.com 或 admin@site.cn");
        assert!(emails.contains(&"test@example.com".to_owned()));
        assert!(emails.contains(&"admin@site.cn".to_owned()));
    }

    #[test]
    fn extracts_phones() {
        let phones = extract_phones("电话 13812345678 或 15900001111");
        assert!(phones.contains(&"13812345678".to_owned()));
        assert!(phones.contains(&"15900001111".to_owned()));
    }

    #[test]
    fn ignores_invalid_phone() {
        let phones = extract_phones("12345678901 1999999999");
        assert!(phones.is_empty());
    }

    #[test]
    fn segment_edit_returns_all() {
        let result = segment_edit("你好 world，访问 qinghan.vip 联系 a@b.com 电话 13800001111");
        assert!(!result.blocks.is_empty());
        assert!(result.links.contains(&"qinghan.vip".to_owned()));
        assert!(result.emails.contains(&"a@b.com".to_owned()));
        assert!(result.phones.contains(&"13800001111".to_owned()));
    }
}
