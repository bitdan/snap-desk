//! OCR 配置与系统语言包查询；引擎选择独立于文本翻译设置，不做自动回退。
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, WebviewWindow};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct OcrSettings {
    pub provider: String,
    /// 空值保留旧本地配置；在线识别通过服务 ID 解析连接。
    pub service_id: String,
    /// auto 使用 Windows 用户语言优先级；其他值是系统已安装 OCR 语言的 BCP-47 标签。
    pub language: String,
}
impl Default for OcrSettings {
    fn default() -> Self {
        Self {
            provider: "local-ppocr".into(),
            service_id: String::new(),
            language: "auto".into(),
        }
    }
}
impl OcrSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !["local-ppocr", "windows-media", "service"].contains(&self.provider.as_str()) {
            return Err("此 OCR 引擎尚未接入。".into());
        }
        if self.language.is_empty()
            || self.language.len() > 85
            || !self
                .language
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("OCR 语言标签无效。".into());
        }
        Ok(())
    }
}

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
    #[cfg(windows)]
    return tauri::async_runtime::spawn_blocking(|| match super::windows_ocr::languages() {
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
    #[cfg(not(windows))]
    Ok(SystemOcrStatus {
        supported: false,
        languages: vec![],
        error: None,
    })
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
        for language in ["auto", "en-US", "zh-Hans-CN"] {
            assert!(OcrSettings {
                provider: "windows-media".into(),
                service_id: String::new(),
                language: language.into()
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
