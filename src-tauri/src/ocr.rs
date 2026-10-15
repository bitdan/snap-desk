//! 截图文字识别：复用 OAR-OCR 的检测、透视裁剪及 CTC 解码，通过 ort 在 CPU 上推理。
use oar_ocr::prelude::*;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// 模型首次使用时加载，后续复用；互斥锁限制同时只有一个识别任务，控制内存峰值。
#[derive(Default)]
pub struct OcrState(Mutex<Option<OAROCR>>);

pub fn recognize(app: &AppHandle, image: &image::RgbaImage) -> Result<String, String> {
    if u64::from(image.width()) * u64::from(image.height()) > 16_000_000 {
        return Err("OCR 选区超过 1600 万像素，请缩小范围。".into());
    }
    let state = app.state::<OcrState>();
    let mut engine = state.0.try_lock().map_err(|_| "已有文字识别正在进行。")?;
    if engine.is_none() {
        let dir = crate::resources::ocr_paths(app)?;
        *engine = Some(
            OAROCRBuilder::new(
                dir.join("pp-ocrv5_mobile_det.onnx"),
                dir.join("pp-ocrv5_mobile_rec.onnx"),
                dir.join("ppocrv5_dict.txt"),
            )
            .build()
            .map_err(|e| format!("OCR 模型加载失败：{e}"))?,
        );
    }
    let rgb = crate::image_io::white_background(image);
    let results = engine
        .as_ref()
        .ok_or("OCR 未初始化。")?
        .predict(vec![rgb])
        .map_err(|e| format!("文字识别失败：{e}"))?;
    Ok(results
        .iter()
        .flat_map(|r| r.text_regions.iter())
        .filter_map(|r| r.text_with_confidence())
        .filter(|(text, confidence)| !text.trim().is_empty() && *confidence >= 0.5)
        .map(|(text, _)| text.to_string())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 结果允许用户编辑；仅截图窗口可写文本剪贴板，且限制消息大小。
#[tauri::command]
pub fn copy_ocr_text(app: AppHandle, window: WebviewWindow, text: String) -> Result<(), String> {
    crate::capture::require_window(&window, "capture")?;
    if text.is_empty() || text.len() > 1024 * 1024 {
        return Err("复制文本为空或超过 1 MiB。".into());
    }
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}
