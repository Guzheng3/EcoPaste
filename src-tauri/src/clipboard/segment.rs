//! 文本「拆词」：把剪贴板内容切成一串有序词块，供前端「拆词填入」多选/滑选。
//!
//! 分词策略（贴合产品语义，目标是选出可读的「词」而不是标点/空白）：
//! - 中文走 jieba 词典切分（如「修复后新装即默认弹气泡」→ 修复 / 后 / 新装 / 即 / 默认 / 弹气泡）；
//! - 英文 / 数字 / 其余拉丁文本按成词片段切分，并把标点从词块中剥离（如 `hello,world` → hello / world）；
//! - 混合文本中英文混排时逐段处理，互不干扰；
//! - 纯标点块、纯空白块会被丢弃，保证返回的都是可展示的「词块」。
//!
//! jieba 实例内部持有词频词典，首次调用会惰性加载一次，之后复用（进程级单例），
//! 不阻塞路径上的首次建初始化之外的业务。

use jieba_rs::Jieba;
use std::sync::OnceLock;

/// 进程级 jieba 单例。目光只做惰性加载；占用内存主要集中在首次 `cut` 时。
fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

/// 是否为「应丢弃」的块：空、纯空白，或只含标点/符号（无任何字母数字汉字）。
fn is_discardable(block: &str) -> bool {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return true;
    }
    !trimmed.chars().any(is_word_char)
}

/// 是否属于「构成词」的字符：汉字、字母、数字、下划线，以及常见组合字符（· 等）。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || ('汉'..='鿿').contains(&c) || c == '_'
}

/// 判断一个词块内是否还混有需要二次切分的非中文片段（英文/数字连续夹着标点）。
fn needs_ascii_split(block: &str) -> bool {
    let mut mixed_word = false;
    let mut mixed_ascii = false;
    for c in block.chars() {
        let word = is_word_char(c);
        if word && is_ascii_char(c) {
            mixed_ascii = true;
        } else if word && !is_ascii_char(c) {
            mixed_word = true;
        }
        if mixed_word && mixed_ascii {
            return true;
        }
    }
    false
}

fn is_ascii_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 把一段英文/数字连写按标点/空白二次切成成词片段（如 `hello,world` → hello / world）。
fn split_ascii_blocks(block: &str) -> Vec<String> {
    block
        .split(|c: char| !is_ascii_char(c) && !c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 将文本拆成有序词块（已剥掉标点/空白、丢弃纯标点块）。
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

        // 中文词块（含汉字）直接保留；英文/数字块若夹标点再二次切分。
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

#[cfg(test)]
mod tests {
    use super::segment_text;

    #[test]
    fn segments_chinese_sentence_into_words() {
        // 「修复后新装即默认弹气泡」jieba 期望切出可读词块。
        let blocks = segment_text("修复后新装即默认弹气泡");
        assert!(!blocks.is_empty());
        assert!(blocks.iter().all(|w| !w.trim().is_empty()));
        // 拼回去中间应含可读中文词（含名词性质词块）。
        assert!(blocks.iter().any(|w| w == "气泡" || w == "弹气泡"));
        assert!(blocks.len() >= 4, "got {blocks:?}");
    }

    #[test]
    fn splits_english_by_punctuation_and_whitespace() {
        assert_eq!(
            segment_text("hello,world  foo bar"),
            vec!["hello", "world", "foo", "bar"]
        );
    }

    #[test]
    fn drops_pure_punctuation_and_whitespace() {
        let blocks = segment_text("， 。  ,.!@$   ");
        assert!(blocks.is_empty());
    }

    #[test]
    fn handles_mixed_chinese_english() {
        let blocks = segment_text("打开GitHub程序并复制code");
        assert!(!blocks.is_empty());
        // 中文词与英文词都应出现。
        assert!(blocks.iter().any(|w| w == "GitHub"));
        assert!(blocks.iter().any(|w| w == "打开"));
    }

    #[test]
    fn empty_input_yields_empty_blocks() {
        assert!(segment_text("   ").is_empty());
        assert!(segment_text("").is_empty());
    }
}
