//! OCR 配置与系统语言包查询；引擎选择独立于文本翻译设置，不做自动回退。
use serde::Serialize;
use tauri::{AppHandle, WebviewWindow};
/// 配置模型在 core，设置校验与识别调度共用。
pub use snap_core::recognition::ocr_config::OcrSettings;

#[derive(Serialize)]
pub struct OcrLanguage {
    pub tag: String,
    pub name: String,
}

/// supported 仅表示平台支持；语言查询失败不伪装成空语言包列表。
#[derive(Serialize)]
pub struct SystemOcrStatus {
    pub supported: bool,
    pub languages: Vec<OcrLanguage>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn system_ocr_status(window: WebviewWindow) -> Result<SystemOcrStatus, String> {
    crate::capture::require_window(&window, "main")?;
    #[cfg(any(windows, target_os = "macos"))]
    return tauri::async_runtime::spawn_blocking(|| match system_languages() {
        Ok(languages) => SystemOcrStatus {
            supported: true,
            languages,
            error: None,
        },
        Err(error) => SystemOcrStatus {
            supported: true,
            languages: vec![],
            error: Some(error),
        },
    })
    .await
    .map_err(|e| e.to_string());
    #[cfg(not(any(windows, target_os = "macos")))]
    Ok(SystemOcrStatus {
        supported: false,
        languages: vec![],
        error: None,
    })
}

#[cfg(windows)]
fn system_languages() -> Result<Vec<OcrLanguage>, String> {
    super::windows_ocr::languages()
}
#[cfg(target_os = "macos")]
fn system_languages() -> Result<Vec<OcrLanguage>, String> {
    super::vision_ocr::languages()
}

/// 同一请求使用同一份引擎与语言快照，识别期间保存设置只影响下次请求。
pub fn snapshot(app: &AppHandle) -> Result<(OcrSettings, Option<super::services::RecognitionService>), String> {
    use tauri::Manager;
    let state = app.state::<crate::settings::SettingsState>();
    let current = state.0.lock().map_err(|_| "无法读取 OCR 设置。")?;
    let value = current.value.ocr.clone();
    value.validate()?;
    let service = if value.provider == "service" { Some(super::services::find(&current.value, &value.service_id)?.clone()) } else { None };
    Ok((value, service))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_engine_and_language_without_changing_user_selection() {
        let value = OcrSettings::default();
        assert_eq!(value.provider, "local-ppocr");
        assert!(OcrSettings {
            provider: "wechat-ocr".into(),
            ..value.clone()
        }.validate().is_ok());
        for language in ["auto", "en-US", "zh-Hans-CN"] {
            assert!(OcrSettings {
                provider: "windows-media".into(),
                language: language.into(),
                ..value.clone()
            }
            .validate()
            .is_ok());
        }
        for language in ["", "zh_CN", "en/US"] {
            assert!(OcrSettings {
                language: language.into(),
                ..value.clone()
            }
            .validate()
            .is_err());
        }
        assert!(OcrSettings {
            provider: "auto".into(),
            ..value
        }
        .validate()
        .is_err());
    }
}
