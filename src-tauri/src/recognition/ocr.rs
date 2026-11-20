//! 截图文字识别：本地 PP-OCR 引擎在 core（`snap_core::recognition::ocr::LocalOcr`），
//! 这里持有应用级引擎状态，并保留复制识别结果的 IPC 命令。
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[cfg(windows)]
pub(super) use snap_core::recognition::ocr::next_request;
pub(super) use snap_core::recognition::ocr::validate_image;

/// 模型首次使用时加载；空闲五分钟后释放（见 `LocalOcr`）。
#[derive(Default)]
pub struct OcrState(pub snap_core::recognition::ocr::LocalOcr);

pub fn release_idle(app: &AppHandle) {
    app.state::<OcrState>().0.release_idle();
}

/// 新旧入口共享模型锁与过滤规则，结构化识别不会额外加载模型。
pub fn recognize_structured(
    app: &AppHandle,
    image: &image::RgbaImage,
) -> Result<super::OcrResult, String> {
    app.state::<OcrState>()
        .0
        .recognize(image, || crate::resources::ocr_paths(app))
}

/// 结果允许用户编辑；主窗口、翻译、截图或仍存在的贴图可复制文字，且限制消息大小。
#[tauri::command]
pub fn copy_ocr_text(app: AppHandle, window: WebviewWindow, text: String) -> Result<(), String> {
    if !matches!(window.label(), "main" | "capture" | "translation") {
        crate::pins::require_pin(&app, &window)?;
    }
    if text.is_empty() || text.len() > 1024 * 1024 {
        return Err("复制文本为空或超过 1 MiB。".into());
    }
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}
