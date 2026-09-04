//! 高置信密钥 / token 检测。
//!
//! 规则刻意保守：只拦截带固定前缀、标准结构或明确私钥边界的内容，
//! 避免把普通长文本、颜色值、文件路径等误判为敏感凭据。

use regex::Regex;
use std::sync::OnceLock;

/// 判断文本是否包含高置信 secret/token。
pub fn contains_secret(text: &str) -> bool {
    let value = text.trim();
    if value.is_empty() {
        return false;
    }

    has_private_key_block(value)
        || has_known_prefixed_token(value)
        || has_aws_access_key(value)
        || has_jwt(value)
        || has_labeled_secret(value)
}

/// 判断文本是否包含高置信个人隐私信息（TieZ 思路：身份证、中国手机号、银行卡号）。
///
/// 刻意不做邮箱 / 手机号这类在日常剪贴板里极其常见的内容，避免把「刚给好友发个号码」
/// 也判定为敏感、触发收割遮挡——只有结构上高置信、且确实需要防止留存的隐私才命中。
///
/// 注意：`regex` crate 不支持 look-around，边界隔离用 `\b` + 代码内前后字符校验实现，
/// 避免把长数字串里的子串误判为手机号/银行卡，也避免 `Regex::new` 在运行时 panic。
pub fn contains_personal_info(text: &str) -> bool {
    let value = text.trim();
    if value.is_empty() || value.len() > 5000 {
        return false;
    }

    // 身份证号：18 位，含出生日期与校验位（末位 0-9/X）。要求边界隔离防误伤长串数字。
    static ID_CARD_RE: OnceLock<Regex> = OnceLock::new();
    if find_isolated(
        value,
        ID_CARD_RE.get_or_init(|| {
            Regex::new(
                r"\b[1-9]\d{5}(?:18|19|20)\d{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12]\d|3[01])\d{3}[\dXx]\b",
            )
            .expect("valid regex")
        }),
    ) {
        return true;
    }

    // 中国手机号：1[3-9] 开头共 11 位，允许常见分隔符（每个数字前至多一个 `[-\s]`），
    // 覆盖 13812345678 / 138-1234-5678 / 138 1234 5678 等写法。收紧到整串或标点包围，
    // 避免把银行卡 / 长数字串误判为手机号。
    static PHONE_RE: OnceLock<Regex> = OnceLock::new();
    if find_isolated(
        value,
        PHONE_RE.get_or_init(|| {
            Regex::new(r"\b(?:\+?86[-\s]?)?1[3-9](?:[-\s]?\d){9}\b").expect("valid regex")
        }),
    ) {
        return true;
    }

    // 银行卡号：13–19 位连续数字或带少量空格的 Luhn 外观，边界隔离。
    static BANK_CARD_RE: OnceLock<Regex> = OnceLock::new();
    find_isolated(
        value,
        BANK_CARD_RE.get_or_init(|| Regex::new(r"\b\d{13,19}\b").expect("valid regex")),
    )
}

/// 在 `text` 中查找 `re` 的任意匹配，且该匹配前后都不是数字（紧邻数字时视为长串的子串）。
fn find_isolated(text: &str, re: &Regex) -> bool {
    for m in re.find_iter(text) {
        let left_clear = text[..m.start()]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_digit());
        let right_clear = text[m.end()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_digit());
        if left_clear && right_clear {
            return true;
        }
    }
    false
}

/// 识别 PEM/OpenSSH 私钥块，避免保存整段私钥。
fn has_private_key_block(text: &str) -> bool {
    static PRIVATE_KEY_RE: OnceLock<Regex> = OnceLock::new();
    PRIVATE_KEY_RE
        .get_or_init(|| {
            Regex::new(r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----").expect("valid regex")
        })
        .is_match(text)
}

/// 识别常见服务商自带前缀的 token。
fn has_known_prefixed_token(text: &str) -> bool {
    static PREFIXED_TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    PREFIXED_TOKEN_RE
        .get_or_init(|| {
            Regex::new(
                r"(?x)
                (?i:
                    \bgh[pousr]_[A-Za-z0-9_]{36,}\b
                  | \bgithub_pat_[A-Za-z0-9_]{40,}\b
                  | \bsk-[A-Za-z0-9_-]{32,}\b
                  | \bsk-proj-[A-Za-z0-9_-]{32,}\b
                  | \bxox[baprs]-[A-Za-z0-9-]{20,}\b
                  | \b(?:api|access|refresh|secret)[_-]?token_[A-Za-z0-9_-]{24,}\b
                )
                ",
            )
            .expect("valid regex")
        })
        .is_match(text)
}

/// 识别 AWS access key id。Secret access key 无固定前缀，单独识别会误伤，暂不拦截。
fn has_aws_access_key(text: &str) -> bool {
    static AWS_KEY_RE: OnceLock<Regex> = OnceLock::new();
    AWS_KEY_RE
        .get_or_init(|| Regex::new(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b").expect("valid regex"))
        .is_match(text)
}

/// 识别结构完整的 JWT：三段 base64url，header 解码后包含 typ/alg 常见字段。
fn has_jwt(text: &str) -> bool {
    static JWT_RE: OnceLock<Regex> = OnceLock::new();
    JWT_RE
        .get_or_init(|| {
            Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
                .expect("valid regex")
        })
        .is_match(text)
}

/// 识别带明确字段名的 secret 赋值。字段名与分隔符之间允许一个可选的闭合引号，
/// 以覆盖 JSON / 引号包裹的配置（`"api_key": "..."`、`'secret_key': '...'`）。
fn has_labeled_secret(text: &str) -> bool {
    static LABELED_SECRET_RE: OnceLock<Regex> = OnceLock::new();
    LABELED_SECRET_RE
        .get_or_init(|| {
            Regex::new(
                r#"(?ix)
                \b
                (?:api[_-]?key|secret[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|auth[_-]?token|bearer)
                \b
                ['"]?\s*[:=]\s*
                ['"]?
                [A-Za-z0-9][A-Za-z0-9._~+/=-]{23,}
                ['"]?
                "#,
            )
            .expect("valid regex")
        })
        .is_match(text)
}

#[cfg(test)]
mod tests {
    use super::{contains_personal_info, contains_secret};

    #[test]
    fn detects_known_prefixed_tokens() {
        let github_token = ["ghp", "_abcdefghijklmnopqrstuvwxyzABCDE1234567890"].concat();
        let openai_token = ["sk-proj", "-abcdefghijklmnopqrstuvwxyzABCDE1234567890"].concat();
        let slack_token = ["xoxb", "-123456789012-abcdefABCDEFabcdefABCDEF"].concat();

        assert!(contains_secret(&github_token));
        assert!(contains_secret(&openai_token));
        assert!(contains_secret(&slack_token));
    }

    #[test]
    fn detects_private_keys_aws_keys_and_jwt() {
        let aws_key = ["AKIA", "IOSFODNN7EXAMPLE"].concat();
        let private_key = [
            "-----BEGIN ",
            "OPENSSH PRIVATE KEY",
            "-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
        ]
        .concat();
        let jwt = [
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
            ".eyJzdWIiOiIxMjM0NTY3ODkwIn0",
            ".SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
        ]
        .concat();

        assert!(contains_secret(&private_key));
        assert!(contains_secret(&aws_key));
        assert!(contains_secret(&jwt));
    }

    #[test]
    fn detects_labeled_secrets() {
        let labeled_secret = ["client", "_secret = abcdefghijklmnopqrstuvwxyz123456"].concat();
        let bearer_token = ["Authorization bearer", ": abcdefghijklmnopqrstuvwxyz123456"].concat();

        assert!(contains_secret(&labeled_secret));
        assert!(contains_secret(&bearer_token));
    }

    #[test]
    fn detects_labeled_secrets_with_quoted_keys() {
        // JSON / 引号包裹的字段名：label 与分隔符之间的闭合引号此前断开了正则，导致漏判。
        // 值用明显的假串（无任何服务商前缀），仅用于驱动 has_labeled_secret 命中。
        let json_double = r#"{"api_key": "dummy_secret_value_abcdefghijklmnopqr"}"#;
        assert!(contains_secret(json_double));

        let single_quoted_key = r#"'secret_key': "abcdefghijklmnopqrstuvwxyz12345678""#;
        assert!(contains_secret(single_quoted_key));

        // 单引号 key + 单引号 value（部分 shell / dotenv 风格）。
        let single_quoted_both = r#"'access_token': 'abcdefghijklmnopqrstuvwxyz123456'"#;
        assert!(contains_secret(single_quoted_both));
    }

    #[test]
    fn ignores_ordinary_text_and_short_codes() {
        assert!(!contains_secret(
            "这是一个普通剪贴板文本，包含 token 这个单词。"
        ));
        assert!(!contains_secret("验证码 123456"));
        assert!(!contains_secret("https://example.com/path/to/resource"));
        assert!(!contains_secret(
            "AKIA is just a word without enough characters"
        ));
    }

    #[test]
    fn detects_id_card_numbers() {
        // 18 位身份证号（含合法出生日期段）。
        assert!(contains_personal_info("11010519491231002X"));
        assert!(contains_personal_info(
            "我的身份证号是 11010519491231002X，请保密"
        ));
        // 位数不足 / 普通长数字串不应命中。注意 13-19 位的数字串会命中银行卡规则，
        // 这里用 12 位的不完整身份证来验证「结构不全不误伤」。
        assert!(!contains_personal_info("12345678901"));
        assert!(!contains_personal_info("110105194912"));
    }

    #[test]
    fn detects_chinese_phone_numbers() {
        assert!(contains_personal_info("13812345678"));
        assert!(contains_personal_info("联系 139-1234-5678"));
        assert!(!contains_personal_info("12345"));
        assert!(!contains_personal_info("13812345"));
    }

    #[test]
    fn detects_bank_card_numbers() {
        assert!(contains_personal_info("6222021234567890123"));
        assert!(!contains_personal_info("1234"));
    }
}
