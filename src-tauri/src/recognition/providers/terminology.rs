//! 大模型术语保护：代码、URL、占位符和常见变量先替换，返回后严格校验并原样恢复。
pub(super) struct ProtectedText {
    pub text: String,
    tokens: Vec<(String, String)>,
}
impl ProtectedText {
    pub fn new(source: &str) -> Self {
        let prefix = format!("SDKEEP{}", &blake3::hash(source.as_bytes()).to_hex()[..16]);
        let mut value = Self {
            text: String::new(),
            tokens: Vec::new(),
        };
        let mut rest = source;
        while !rest.is_empty() {
            let special = if rest.starts_with('`') {
                let ticks = rest.chars().take_while(|c| *c == '`').count();
                rest[ticks..]
                    .find(&"`".repeat(ticks))
                    .map(|end| ticks + end + ticks)
            } else if rest.starts_with("https://") || rest.starts_with("http://") {
                Some(rest.find(char::is_whitespace).unwrap_or(rest.len()))
            } else if rest.starts_with('{') {
                rest.find('}').map(|end| end + 1)
            } else {
                None
            };
            if let Some(end) = special {
                let token = format!("{prefix}N{}X", value.tokens.len());
                value.text.push_str(&token);
                value.tokens.push((token, rest[..end].into()));
                rest = &rest[end..];
            } else {
                let length = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .map(char::len_utf8)
                    .sum::<usize>();
                let word = &rest[..length];
                if length > 1
                    && (word.contains('_')
                        || word.chars().skip(1).any(|c| c.is_ascii_uppercase())
                            && word.chars().any(|c| c.is_ascii_lowercase()))
                {
                    let token = format!("{prefix}N{}X", value.tokens.len());
                    value.text.push_str(&token);
                    value.tokens.push((token, word.into()));
                    rest = &rest[length..];
                } else {
                    let end = if length > 0 {
                        length
                    } else {
                        rest.chars().next().unwrap().len_utf8()
                    };
                    value.text.push_str(&rest[..end]);
                    rest = &rest[end..];
                }
            }
        }
        value
    }
    pub fn restore(&self, mut text: String) -> Result<String, String> {
        for (token, original) in &self.tokens {
            if text.matches(token).count() != 1 {
                return Err("服务未保留受保护术语，请缩小选区或更换模型。".into());
            }
            text = text.replace(token, original);
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terms_round_trip_and_missing_tokens_fail() {
        let source = "打开 https://example.com/a 并运行 `let foo = 1` {name} userId user_name";
        let protected = ProtectedText::new(source);
        assert!(!protected.text.contains("example.com"));
        assert_eq!(protected.restore(protected.text.clone()).unwrap(), source);
        assert!(protected.restore("lost".into()).is_err());
    }
}
