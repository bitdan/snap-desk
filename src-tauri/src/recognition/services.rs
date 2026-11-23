//! 服务目录与能力绑定。密钥只写系统凭据库，连接修改后凭据按地址重新隔离。
use crate::settings::Settings;
use tauri::{AppHandle, Manager, WebviewWindow};
/// 服务模型、校验与凭据键在 core；这里保留原路径，凭据读写与 IPC 命令仍在本模块。
pub use snap_core::recognition::services::{
    credential_key, find, secret_key, translation_config, RecognitionService,
};
use snap_core::recognition::services::default_keys;
/// 密钥读取、换钥判断与密钥顺序在 core（GPUI 版共用）。
pub use snap_core::recognition::services::secrets;

/// 运行时按服务标识取密钥顺序；旧版直接配置（无服务标识）只有单个条目。
pub fn key_ids(app: &AppHandle, service_id: &str) -> Vec<String> {
    let state = app.state::<crate::settings::SettingsState>();
    let keys = state
        .0
        .lock()
        .ok()
        .map(|settings| snap_core::recognition::services::key_ids(&settings.value, service_id));
    keys.unwrap_or_default()
}
/// 密钥掩码提示与连通性测试在 core（GPUI 版共用）。
use snap_core::recognition::services::{key_infos, ServiceKeyInfo, ServiceTest};
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
    key_infos(&snapshot(&app, &service_id)?)
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
    let (service, ai) = {
        let state = app.state::<crate::settings::SettingsState>();
        let settings = state.0.lock().map_err(|_| "设置正在使用。")?;
        (find(&settings.value, &service_id)?.clone(), settings.value.ai.clone())
    };
    match ServiceTest::new(service, ai, key_id)? {
        ServiceTest::Ocr(service) => tauri::async_runtime::spawn_blocking(move || ServiceTest::run_ocr(&service))
            .await
            .map_err(|_| "OCR 测试中断。".to_string())?,
        test => test.run().await,
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
    use snap_core::recognition::services::validate;
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
