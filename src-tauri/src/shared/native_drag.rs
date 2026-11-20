//! 系统拖出共用设施：Windows 使用 OLE DoDragDrop，macOS 使用 NSDraggingSession，Linux 使用 GTK 拖拽。
//! 调用方准备受信任内容并保管临时文件。
use tauri::{AppHandle, WebviewWindow};
#[cfg(windows)]
use tauri::Manager;

#[cfg(target_os = "macos")]
#[path = "native_drag_macos.rs"]
mod macos_drag;
#[cfg(target_os = "linux")]
#[path = "native_drag_linux.rs"]
mod linux_drag;

/// 当前平台能否把内容拖到其他应用；不支持时调用方应在准备临时文件前直接拒绝。
pub(crate) const SUPPORTED: bool = cfg!(any(windows, target_os = "macos", target_os = "linux"));

/// 内容类型、临时 PNG 与 Windows OLE 拖出在 core（GPUI 版共用）。
pub(crate) use snap_core::drag::Payload;

/// 只编码受信任的像素；临时文件由 core 保留到进程退出。
pub(crate) fn image_file(_app: &AppHandle, image: &image::RgbaImage) -> Result<String, String> {
    snap_core::drag::image_file(image)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// 必须从后台线程调用：Windows 的 OLE 会话调度至 UI STA 线程并阻塞到放下，
/// macOS 与 Linux 的会话在主线程发起后异步进行。
pub(crate) fn start(window: &WebviewWindow, payload: Payload) -> Result<(), String> {
    #[cfg(windows)]
    {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        window
            .app_handle()
            .run_on_main_thread(move || {
                let _ = sender.send(snap_core::drag::start(payload));
            })
            .map_err(err)?;
        receiver.recv().map_err(err)?
    }
    #[cfg(target_os = "macos")]
    return macos_drag::start(window, payload);
    #[cfg(target_os = "linux")]
    return linux_drag::start(window, payload);
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (window, payload);
        Err("当前平台尚未支持内容拖出。".into())
    }
}
