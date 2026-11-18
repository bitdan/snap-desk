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
    /// 同一服务的多个密钥标识，按顺序使用；认证失败或额度不足时换下一个。
    /// 密钥本身只在系统凭据库，配置里只有标识。旧配置缺省为单个 "default"，沿用原凭据条目。
    #[serde(default = "default_keys")]
    pub keys: Vec<String>,
}
pub const DEFAULT_KEY: &str = "default";
fn default_keys() -> Vec<String> {
    vec![DEFAULT_KEY.into()]
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
        let mut keys = std::collections::HashSet::new();
        if self.keys.len() > 16
            || !self.keys.iter().all(|key| {
                !key.is_empty()
                    && key.len() <= 40
                    && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    && keys.insert(key)
            })
        {
            return Err("服务密钥列表无效。".into());
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
/// 单个密钥的凭据键；"default" 与旧版单密钥条目相同，升级后无需搬运。
pub fn secret_key(id: &str, endpoint: &str, key: &str) -> String {
    if key == DEFAULT_KEY {
        credential_key(id, endpoint)
    } else {
        format!("{}#{key}", credential_key(id, endpoint))
    }
}
/// 按顺序读取已保存的密钥，缺失的条目跳过。
pub fn secrets(id: &str, endpoint: &str, keys: &[String]) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for key in keys {
        if let Some(value) = super::ai_credentials::read(&secret_key(id, endpoint, key))? {
            values.push(value);
        }
    }
    Ok(values)
}
/// 认证失败、额度不足或限流时才换下一个密钥；网络、地址等错误换密钥也无济于事。
pub fn key_rejected(error: &str) -> bool {
    ["认证", "额度", "受限"].iter().any(|word| error.contains(word))
}
/// 运行时按服务标识取密钥顺序；旧版直接配置（无服务标识）只有单个条目。
pub fn key_ids(app: &AppHandle, service_id: &str) -> Vec<String> {
    if service_id.is_empty() {
        return default_keys();
    }
    let state = app.state::<crate::settings::SettingsState>();
    let keys = state
        .0
        .lock()
        .ok()
        .and_then(|settings| find(&settings.value, service_id).ok().map(|service| service.keys.clone()));
    keys.unwrap_or_default()
}
/// 只显示首尾少量字符，界面可区分多个密钥而不暴露完整内容。
fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 10 {
        return "••••".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceKeyInfo {
    id: String,
    /// None 表示配置里有此标识但系统凭据库中没有对应密钥（例如修改了地址）。
    hint: Option<String>,
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
    Ok(!secrets(&service.id, &service.endpoint, &service.keys)?.is_empty())
}
#[tauri::command]
pub fn recognition_service_keys(
    app: AppHandle,
    window: WebviewWindow,
    service_id: String,
) -> Result<Vec<ServiceKeyInfo>, String> {
    crate::capture::require_window(&window, "main")?;
    let service = snapshot(&app, &service_id)?;
    service
        .keys
        .iter()
        .map(|key| {
            let hint = super::ai_credentials::read(&secret_key(&service.id, &service.endpoint, key))?
                .map(|value| mask(&value));
            Ok(ServiceKeyInfo { id: key.clone(), hint })
        })
        .collect()
}
#[tauri::command]
pub fn recognition_service_credential(
    app: AppHandle,
    window: WebviewWindow,
    service_id: String,
    expected_endpoint: String,
    key_id: String,
    secret: Option<String>,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let service = snapshot(&app, &service_id)?;
    if service.endpoint != expected_endpoint {
        return Err("服务地址已变更，请重新确认。".into());
    }
    // 只写入已登记在配置中的密钥标识，避免凭据库里留下界面无法管理的条目。
    if !service.keys.contains(&key_id) {
        return Err("此密钥不属于该服务，请重新打开服务管理。".into());
    }
    super::ai_credentials::write(&secret_key(&service.id, &service.endpoint, &key_id), secret)?;
    // 密钥更新后不能命中旧账号的翻译缓存。
    super::translation::clear_cache(&app);
    Ok(())
}

/// 服务管理中的连通性测试：翻译服务发送 "Hello"，仅 OCR 的服务上传一张空白小图；只验证地址、认证和额度可用。
#[tauri::command]
pub async fn recognition_service_test(
    app: AppHandle,
    window: WebviewWindow,
    service_id: String,
    key_id: Option<String>,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let (mut service, ai) = {
        let state = app.state::<crate::settings::SettingsState>();
        let settings = state.0.lock().map_err(|_| "设置正在使用。")?;
        (find(&settings.value, &service_id)?.clone(), settings.value.ai.clone())
    };
    service.validate()?;
    // 指定密钥时只用该密钥测试，不做回退，便于逐个确认哪个密钥可用。
    if let Some(key) = key_id {
        if !service.keys.contains(&key) {
            return Err("密钥不存在。".into());
        }
        service.keys = vec![key];
    }
    if service.supports_translation() {
        let mut config = ai;
        config.service_id = service.id.clone();
        config.provider = if service.protocol == "deepl" { "deepl" } else { "online-llm" }.into();
        config.endpoint = service.endpoint.clone();
        config.model = service.model.clone();
        config.target_language = "zh-CN".into();
        super::providers::translate(
            &config,
            &super::providers::TranslationRequest {
                texts: vec!["Hello".into()],
                source_language: Some("en".into()),
                target_language: "zh-CN".into(),
            },
            &service.keys,
        )
        .await
        .map(|_| ())
    } else {
        let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([255, 255, 255, 255]));
        tauri::async_runtime::spawn_blocking(move || super::online_ocr::recognize(&image, &service))
            .await
            .map_err(|_| "OCR 测试中断。".to_string())?
            .map(|_| ())
    }
}

/// 旧版在翻译页直接填写的在线配置迁移为服务管理中的连接，使翻译页只负责选择。
/// 返回（旧凭据键，新凭据键），由调用方搬运系统凭据；迁移条件不满足时保持原配置。
pub fn migrate_legacy_translation(settings: &mut Settings) -> Option<(String, String)> {
    const ID: &str = "legacy-translation";
    let ai = &settings.ai;
    if ai.provider == "local-ctranslate2" || !ai.service_id.is_empty() || ai.endpoint.is_empty() {
        return None;
    }
    let deepl = ai.provider == "deepl";
    let name = if deepl {
        "DeepL".to_string()
    } else {
        reqwest::Url::parse(&ai.endpoint)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| "翻译服务".into())
    };
    let service = RecognitionService {
        id: ID.into(),
        name,
        protocol: if deepl { "deepl" } else { "openai-compatible" }.into(),
        endpoint: ai.endpoint.clone(),
        model: if deepl { String::new() } else { ai.model.clone() },
        vision: false,
        keys: default_keys(),
    };
    if service.validate().is_err() {
        return None;
    }
    match settings.recognition_services.iter().find(|item| item.id == ID) {
        Some(existing) if *existing != service => return None,
        Some(_) => (),
        None if settings.recognition_services.len() >= 64 => return None,
        None => settings.recognition_services.push(service.clone()),
    }
    settings.ai.service_id = ID.into();
    Some((credential_key("", &service.endpoint), credential_key(ID, &service.endpoint)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_translation_becomes_a_service() {
        let mut settings: Settings = serde_json::from_str(r#"{"ai":{"provider":"deepl","endpoint":"https://api-free.deepl.com/v2/translate","model":""}}"#).unwrap();
        let keys = migrate_legacy_translation(&mut settings).unwrap();
        assert_eq!(keys.0, "https://api-free.deepl.com/v2/translate");
        assert_eq!(keys.1, "service:legacy-translation:https://api-free.deepl.com/v2/translate");
        assert_eq!(settings.ai.service_id, "legacy-translation");
        assert_eq!(settings.recognition_services[0].protocol, "deepl");
        assert!(validate(&settings).is_ok());
        // 已迁移或本地配置不再重复处理。
        assert!(migrate_legacy_translation(&mut settings).is_none());
        let mut local: Settings = serde_json::from_str("{}").unwrap();
        assert!(migrate_legacy_translation(&mut local).is_none());
        assert!(local.recognition_services.is_empty());
    }
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
            keys: default_keys(),
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
            keys: default_keys(),
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
