//! 划词复制（前台窗口、注入复制并等待剪贴板更新、X11 PRIMARY 选区）在 core；
//! 这里用剪贴板插件读取文字，行为与移入前一致。
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

pub(super) use snap_core::recognition::selected_text::foreground;

pub(super) fn copy(app: &AppHandle, target: usize) -> Result<String, String> {
    snap_core::recognition::selected_text::copy(target, || {
        app.clipboard().read_text().map_err(|e| e.to_string())
    })
}
