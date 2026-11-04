//! OCR 的稳定 IPC 契约。坐标始终相对裁剪原图，单位为像素，不能当作屏幕 CSS 坐标。
pub(crate) mod services;
mod online_ocr;
mod paddle_ocr;
pub(crate) mod ocr;
pub(crate) mod ocr_config;
#[cfg(windows)]
mod windows_ocr;
pub(crate) mod wechat_ocr;
pub(crate) mod qr;
pub(crate) mod translation;
pub(crate) mod text_input;
mod selected_text;
mod providers;
pub(crate) mod local_translation;
pub(crate) mod translation_models;
pub(crate) mod translation_runtime;
pub(crate) mod ai_credentials;
#[cfg(windows)]
pub(crate) mod ocr_runtime;

use serde::Serialize;

/// 引擎共享结果契约；严格使用已保存配置，不跨引擎重试或自动联网。
pub trait OcrProvider {
    fn recognize(
        &self,
        app: &tauri::AppHandle,
        image: &image::RgbaImage,
        settings: &ocr_config::OcrSettings,
    ) -> Result<OcrResult, String>;
}
pub struct LocalOcrProvider;
impl OcrProvider for LocalOcrProvider {
    fn recognize(
        &self,
        app: &tauri::AppHandle,
        image: &image::RgbaImage,
        _settings: &ocr_config::OcrSettings,
    ) -> Result<OcrResult, String> {
        crate::recognition::ocr::recognize_structured(app, image)
    }
}
pub struct WindowsOcrProvider;
impl OcrProvider for WindowsOcrProvider {
    fn recognize(&self, _app: &tauri::AppHandle, image: &image::RgbaImage, settings: &ocr_config::OcrSettings) -> Result<OcrResult, String> {
        #[cfg(windows)]
        return windows_ocr::recognize(image, &settings.language);
        #[cfg(not(windows))]
        { let _ = (image, settings); Err("Windows 系统 OCR 仅支持 Windows。".into()) }
    }
}

pub struct WechatOcrProvider;
impl OcrProvider for WechatOcrProvider {
    fn recognize(&self, app: &tauri::AppHandle, image: &image::RgbaImage, _settings: &ocr_config::OcrSettings) -> Result<OcrResult, String> {
        wechat_ocr::recognize(app, image)
    }
}

/// 截图文字、截图翻译及贴图使用同一入口，设置快照只读取一次。
pub fn recognize_selected(app: &tauri::AppHandle, image: &image::RgbaImage) -> Result<OcrResult, String> {
    let (settings, service) = ocr_config::snapshot(app)?;
    if let Some(service) = service { return online_ocr::recognize(image, &service); }
    provider(&settings.provider)?.recognize(app, image, &settings)
}
pub fn provider(id: &str) -> Result<&'static dyn OcrProvider, String> {
    match id {
        "local-ppocr" => Ok(&LocalOcrProvider),
        "windows-media" => Ok(&WindowsOcrProvider),
        "wechat-ocr" => Ok(&WechatOcrProvider),
        _ => Err("此 OCR 引擎尚未接入。".into()),
    }
}

#[derive(Debug, Serialize)]
pub struct PolygonPoint {
    pub x: f32,
    pub y: f32,
}

/// 保留引擎阅读顺序；没有可靠坐标或语言判断时返回 null。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub id: String,
    pub text: String,
    pub confidence: Option<f32>,
    pub polygon: Option<Vec<PolygonPoint>>,
    pub language: Option<String>,
    pub order: usize,
    pub paragraph_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub request_id: String,
    pub provider: String,
    pub image_width: u32,
    pub image_height: u32,
    pub full_text: String,
    pub lines: Vec<OcrLine>,
    pub elapsed_ms: u64,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_explicit_ocr_engines_are_registered() {
        assert!(super::provider("local-ppocr").is_ok());
        assert!(super::provider("windows-media").is_ok());
        assert!(super::provider("wechat-ocr").is_ok());
        for id in ["ppocrv5", "online", "", "unknown"] {
            assert!(super::provider(id).is_err());
        }
    }
}
