//! DeepL 官方文本协议，与 Chat Completions 分离；仅允许 Free/Pro 官方地址。
use super::{language, send, TranslationRequest};
use crate::recognition::translation::AiSettings;
use serde::Deserialize;

pub fn endpoint(value: &str) -> Result<reqwest::Url, String> {
    if ![
        "https://api-free.deepl.com/v2/translate",
        "https://api.deepl.com/v2/translate",
    ]
    .contains(&value)
    {
        return Err("DeepL 仅支持官方 Free 或 Pro 的 /v2/translate 地址。".into());
    }
    reqwest::Url::parse(value).map_err(|_| "服务地址无效。".into())
}

pub(super) async fn translate(
    client: &reqwest::Client,
    settings: &AiSettings,
    request: &TranslationRequest,
    secret: &str,
) -> Result<Vec<String>, String> {
    let mut body = serde_json::json!({
        "text": request.texts,
        "target_lang": if request.target_language == "zh-CN" { "ZH-HANS" } else { language(&request.target_language)? },
        "preserve_formatting": true,
    });
    if let Some(source) = &request.source_language {
        body["source_lang"] = language(source)?.into();
    }
    let bytes = send(
        client
            .post(endpoint(&settings.endpoint)?)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("DeepL-Auth-Key {secret}"),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(super::json_body(&body)?),
    )
    .await?;
    parse_response(&bytes, request.texts.len())
}

#[derive(Deserialize)]
struct Response {
    translations: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    text: String,
}

/// 官方保证返回顺序；数量不符时整体拒绝，不把译文错贴到另一个 OCR 框。
fn parse_response(bytes: &[u8], count: usize) -> Result<Vec<String>, String> {
    let response: Response = serde_json::from_slice(bytes).map_err(|_| "翻译响应格式不兼容。")?;
    if response.translations.len() != count
        || response
            .translations
            .iter()
            .any(|item| item.text.trim().is_empty())
    {
        return Err("翻译结果数量不匹配或为空。".into());
    }
    Ok(response
        .translations
        .into_iter()
        .map(|item| item.text)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_hosts_and_order_are_required() {
        assert!(endpoint("https://api-free.deepl.com/v2/translate").is_ok());
        for url in [
            "https://api.deepl.com.evil.test/v2/translate",
            "http://api.deepl.com/v2/translate",
            "https://api.deepl.com/v2/translate?key=x",
        ] {
            assert!(endpoint(url).is_err());
        }
        let bytes = br#"{"translations":[{"text":"first"},{"text":"second"}]}"#;
        assert_eq!(parse_response(bytes, 2).unwrap(), vec!["first", "second"]);
        assert!(parse_response(bytes, 1).is_err());
        assert!(parse_response(br#"{"translations":[{"text":" "}]}"#, 1).is_err());
    }
}
