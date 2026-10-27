//! OCR 的稳定 IPC 契约。坐标始终相对裁剪原图，单位为像素，不能当作屏幕 CSS 坐标。
pub(crate) mod ocr;
pub(crate) mod qr;
pub(crate) mod translation;
pub(crate) mod local_translation;
pub(crate) mod translation_models;
pub(crate) mod translation_runtime;
pub(crate) mod ai_credentials;
#[cfg(windows)]
pub(crate) mod ocr_runtime;

use serde::Serialize;

/// Provider 边界让后续视觉 OCR 复用相同结果；首版只注册本地引擎，不自动联网。
pub trait OcrProvider {
    fn recognize(
        &self,
        app: &tauri::AppHandle,
        image: &image::RgbaImage,
    ) -> Result<OcrResult, String>;
}
pub struct LocalOcrProvider;
impl OcrProvider for LocalOcrProvider {
    fn recognize(
        &self,
        app: &tauri::AppHandle,
        image: &image::RgbaImage,
    ) -> Result<OcrResult, String> {
        crate::recognition::ocr::recognize_structured(app, image)
    }
}
pub fn provider(id: &str) -> Result<&'static dyn OcrProvider, String> {
    match id {
        "local-ppocr" => Ok(&LocalOcrProvider),
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
    fn only_the_maintained_local_engine_is_registered() {
        assert!(super::provider("local-ppocr").is_ok());
        for id in ["ppocrv5", "online", "", "unknown"] {
            assert!(super::provider(id).is_err());
        }
    }
}
