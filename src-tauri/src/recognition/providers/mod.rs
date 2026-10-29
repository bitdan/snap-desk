//! 在线文本 Provider；只接收识别文字，不接收图片或文件路径。
pub(super) mod deepl;
mod openai_compatible;
mod terminology;

use super::translation::AiSettings;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 数组顺序就是 OCR 单元顺序；目标语言必须与用户确认的配置相同。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationRequest {
    /// OCR 单元原文；整个 IPC 请求合计最多 32 KB。
    pub texts: Vec<String>,
    /// 空值表示由所选服务自动检测。
    pub source_language: Option<String>,
    pub target_language: String,
}

/// JSON 转义与术语标记可能扩大请求；实际线上正文也必须满足 DeepL 的 128 KiB 上限。
fn json_body(body: &serde_json::Value) -> Result<String, String> {
    let value = body.to_string();
    if value.len() > 128 * 1024 {
        return Err("翻译请求超过 128 KB，请缩小选区。".into());
    }
    Ok(value)
}

impl TranslationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.texts.is_empty()
            || self.texts.len() > 50
            || self.texts.iter().any(|text| text.trim().is_empty())
            || self.texts.iter().map(String::len).sum::<usize>() > 32_000
        {
            return Err("一次需要 1～50 段非空文字，合计不超过 32 KB。".into());
        }
        if let Some(source) = &self.source_language {
            language(source)?;
        }
        language(&self.target_language)?;
        Ok(())
    }
}

/// 统一应用语言码与 DeepL 语言码，避免把地区码直接交给服务猜测。
fn language(value: &str) -> Result<&'static str, String> {
    match value {
        "zh-CN" => Ok("ZH"),
        "en" => Ok("EN"),
        "ja" => Ok("JA"),
        "ko" => Ok("KO"),
        "fr" => Ok("FR"),
        "de" => Ok("DE"),
        _ => Err("不支持该目标语言。".into()),
    }
}

pub async fn translate(
    settings: &AiSettings,
    request: &TranslationRequest,
) -> Result<Vec<String>, String> {
    request.validate()?;
    let secret = crate::recognition::ai_credentials::read(&settings.endpoint)?
        .ok_or("请先保存此服务的 API Key。")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(settings.timeout_seconds))
        .build()
        .map_err(|_| "无法初始化翻译网络客户端。")?;
    match settings.provider.as_str() {
        "deepl" => deepl::translate(&client, settings, request, &secret).await,
        "online-llm" => openai_compatible::translate(&client, settings, request, &secret).await,
        _ => Err("不支持此翻译引擎。".into()),
    }
}

/// 错误只保留状态码，不回传可能含原文或凭证的服务正文；禁止重试和重定向。
async fn send(builder: reqwest::RequestBuilder) -> Result<Vec<u8>, String> {
    let mut response = builder.send().await.map_err(network_error)?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "服务拒绝认证，请检查密钥与模型权限。".into(),
            429 => "服务请求受限或额度不足，请稍后重试。".into(),
            456 => "DeepL 翻译额度已用尽，请检查账户用量。".into(),
            code => format!("翻译服务返回 HTTP {code}，请检查地址和模型。"),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > 256 * 1024 {
            return Err("翻译响应超过 256 KB。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "翻译超时，请重试或调整超时时间。"
    } else {
        "连接翻译服务失败，请检查网络与服务地址。"
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_count_utf8_bytes_and_reject_unknown_languages() {
        let mut request = TranslationRequest {
            texts: vec!["hello".into()],
            source_language: None,
            target_language: "zh-CN".into(),
        };
        assert!(request.validate().is_ok());
        request.texts = vec!["中".repeat(10_667)];
        assert!(request.validate().is_err());
        request.texts = vec!["hello".into(); 51];
        assert!(request.validate().is_err());
        request.texts = vec![" ".into()];
        assert!(request.validate().is_err());
        request.texts = vec!["hello".into()];
        request.source_language = Some("invalid".into());
        assert!(request.validate().is_err());
        assert!(json_body(&serde_json::json!({"text": "x".repeat(128 * 1024)})).is_err());
    }
}
