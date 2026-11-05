//! 服务目录与能力绑定。密钥只写系统凭据库，连接修改后凭据按地址重新隔离。
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecognitionService {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: String,
    pub model: String,
    /// 兼容协议的文本模型不能仅凭服务商名称推断支持图片。
    pub vision: bool,
}
impl RecognitionService {
    pub fn supports_ocr(&self) -> bool {
        matches!(self.protocol.as_str(), "ocr-space" | "paddle-ocr")
            || (self.protocol == "openai-compatible" && self.vision)
    }
    pub fn supports_translation(&self) -> bool {
        matches!(self.protocol.as_str(), "openai-compatible" | "deepl")
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 80
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("服务名称或标识无效。".into());
        }
        super::translation::endpoint_url(&self.endpoint)?;
        if self.model.len() > 200 || self.model.chars().any(char::is_control) {
            return Err("模型名称无效。".into());
        }
        match self.protocol.as_str() {
            "openai-compatible" if !self.model.trim().is_empty() => (),
            "ocr-space" if ["1", "2", "3"].contains(&self.model.as_str()) => (),
            "paddle-ocr" if super::paddle_ocr::supports_model(&self.endpoint, &self.model) => (),
            "deepl" => {
                super::providers::deepl::endpoint(&self.endpoint)?;
            }
            _ => return Err("服务协议或模型无效。".into()),
        }
        Ok(())
    }
}

pub fn find<'a>(settings: &'a Settings, id: &str) -> Result<&'a RecognitionService, String> {
    settings
        .recognition_services
        .iter()
        .find(|service| service.id == id)
        .ok_or_else(|| "所选服务不存在，请重新选择。".into())
}
/// 服务和功能偏好分别保存；运行时合并为原有翻译契约，保留单次上传确认。
pub fn translation_config(settings: &Settings) -> Result<super::translation::AiSettings, String> {
    let mut value = settings.ai.clone();
    if value.provider != "local-ctranslate2" && !value.service_id.is_empty() {
        let service = find(settings, &value.service_id)?;
        if !service.supports_translation() {
            return Err("所选服务不支持翻译。".into());
        }
        value.provider = if service.protocol == "deepl" {
            "deepl"
        } else {
            "online-llm"
        }
        .into();
        value.endpoint = service.endpoint.clone();
        value.model = service.model.clone();
    }
    value.validate()?;
    Ok(value)
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    if settings.recognition_services.len() > 64 {
        return Err("最多保存 64 个服务。".into());
    }
    let mut ids = std::collections::HashSet::new();
    for service in &settings.recognition_services {
        service.validate()?;
        if !ids.insert(&service.id) {
            return Err("服务标识重复。".into());
        }
    }
    translation_config(settings)?;
    if settings.ocr.provider == "service"
        && !find(settings, &settings.ocr.service_id)?.supports_ocr()
    {
        return Err("所选服务不支持图片识别。".into());
    }
    Ok(())
}
/// 地址参与凭据键，编辑地址后不会把原账号密钥发送给另一个站点。
pub fn credential_key(id: &str, endpoint: &str) -> String {
    if id.is_empty() {
        endpoint.into()
    } else {
        format!("service:{id}:{endpoint}")
    }
}
fn snapshot(app: &AppHandle, id: &str) -> Result<RecognitionService, String> {
    let state = app.state::<crate::settings::SettingsState>();
    let settings = state.0.lock().map_err(|_| "设置正在使用。")?;
    Ok(find(&settings.value, id)?.clone())
}
/// 仅打开内置官方入口，不接收任意 URL 或账号信息。
#[tauri::command]
pub async fn open_recognition_portal(window: WebviewWindow, preset: String) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let url = match preset.as_str() {
        "groq" | "groq-vision" => "https://console.groq.com/keys",
        "deepseek" => "https://platform.deepseek.com/api_keys",
        "ocr-space" => "https://ocr.space/ocrapi",
        "paddle-ocr" => "https://aistudio.baidu.com/paddleocr/task",
        "deepl" => "https://www.deepl.com/your-account/keys",
        _ => return Err("此服务没有内置凭据入口。".into()),
    };
    tauri::async_runtime::spawn_blocking(move || crate::shared::platform::open_web_page(url))
        .await
        .map_err(|_| "无法打开浏览器。".to_string())?
}
#[tauri::command]
pub fn recognition_service_status(
    app: AppHandle,
    window: WebviewWindow,
    service_id: String,
) -> Result<bool, String> {
    crate::capture::require_window(&window, "main")?;
    let service = snapshot(&app, &service_id)?;
    Ok(super::ai_credentials::read(&credential_key(&service.id, &service.endpoint))?.is_some())
}
#[tauri::command]
pub fn recognition_service_credential(
    app: AppHandle,
    window: WebviewWindow,
    service_id: String,
    expected_endpoint: String,
    secret: Option<String>,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let service = snapshot(&app, &service_id)?;
    if service.endpoint != expected_endpoint {
        return Err("服务地址已变更，请重新确认。".into());
    }
    super::ai_credentials::write(&credential_key(&service.id, &service.endpoint), secret)?;
    // 密钥更新后不能命中旧账号的翻译缓存。
    super::translation::clear_cache(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_and_binding_resolution() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.recognition_services.is_empty());
        assert!(settings.ai.service_id.is_empty());
        assert!(settings.ocr.service_id.is_empty());
        assert_eq!(
            translation_config(&settings).unwrap().provider,
            "local-ctranslate2"
        );
        settings.recognition_services.push(RecognitionService {
            id: "account-a".into(),
            name: "Account A".into(),
            protocol: "openai-compatible".into(),
            endpoint: "https://example.com/v1/chat/completions".into(),
            model: "new-model".into(),
            vision: false,
        });
        settings.ai.provider = "online-llm".into();
        settings.ai.service_id = "account-a".into();
        settings.ai.endpoint = "https://old.example/v1".into();
        settings.ai.model = "old-model".into();
        let actual = translation_config(&settings).unwrap();
        assert_eq!(actual.model, "new-model");
        assert_eq!(actual.endpoint, "https://example.com/v1/chat/completions");
        assert_eq!(settings.ai.model, "old-model");
        settings.ai.service_id = "missing".into();
        assert!(validate(&settings).is_err());
    }
    #[test]
    fn capability_and_credential_boundaries() {
        let service = RecognitionService {
            id: "test".into(),
            name: "test".into(),
            protocol: "openai-compatible".into(),
            endpoint: "https://example.com/v1/chat/completions".into(),
            model: "text".into(),
            vision: false,
        };
        assert!(service.validate().is_ok());
        assert!(!service.supports_ocr());
        assert!(service.supports_translation());
        assert_ne!(
            credential_key("a", &service.endpoint),
            credential_key("b", &service.endpoint)
        );
        assert_ne!(
            credential_key("a", &service.endpoint),
            credential_key("a", "https://other.example/v1")
        );
        let mut settings = Settings::default();
        settings.recognition_services.push(service.clone());
        settings.ocr.provider = "service".into();
        settings.ocr.service_id = service.id.clone();
        assert!(validate(&settings).is_err());
        settings.recognition_services[0].vision = true;
        assert!(validate(&settings).is_ok());
        settings.recognition_services.push(service);
        assert!(validate(&settings).is_err());
    }
}
