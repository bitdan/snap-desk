//! OCR 的稳定 IPC 契约。坐标始终相对裁剪原图，单位为像素，不能当作屏幕 CSS 坐标。
pub(crate) mod services;
pub(crate) mod ocr;
pub(crate) mod ocr_config;
pub(crate) mod wechat_ocr;
pub(crate) mod qr;
pub(crate) mod translation;
pub(crate) mod text_input;
mod selected_text;
pub(crate) mod local_translation;
pub(crate) mod translation_models;
pub(crate) mod translation_runtime;

/// 引擎、网络服务、凭据与运行库在 core（GPUI 版共用）；这里保留原路径与 Tauri 命令、窗口状态。
pub(crate) use snap_core::recognition::{ai_credentials, online_ocr, providers};
#[cfg(windows)]
pub(crate) use snap_core::recognition::windows_ocr;
#[cfg(target_os = "macos")]
pub(crate) use snap_core::recognition::vision_ocr;
pub use snap_core::recognition::ocr::OcrResult;

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
        // macOS 的“系统 OCR”由 Apple Vision 提供，沿用同一提供方标识。
        snap_core::recognition::system_ocr(image, &settings.language)
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
    let mut result = match service {
        Some(service) => online_ocr::recognize(image, &service)?,
        None => provider(&settings.provider)?.recognize(app, image, &settings)?,
    };
    // 截图与贴图窗口读不到完整设置，识别后的动作开关随结果一并返回。
    result.auto_copy = settings.auto_copy;
    result.merge_lines = settings.merge_lines;
    Ok(result)
}
pub fn provider(id: &str) -> Result<&'static dyn OcrProvider, String> {
    match id {
        "local-ppocr" => Ok(&LocalOcrProvider),
        "windows-media" => Ok(&WindowsOcrProvider),
        "wechat-ocr" => Ok(&WechatOcrProvider),
        _ => Err("此 OCR 引擎尚未接入。".into()),
    }
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
