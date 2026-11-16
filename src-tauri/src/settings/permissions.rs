//! 系统隐私权限的查询与引导。目前只有 macOS 需要：截图依赖“屏幕录制”，
//! 后续的窗口吸附、直接粘贴等依赖“辅助功能”。其他平台返回 supported=false，前端隐藏入口。
use crate::capture::require_window;
use serde::Serialize;
use tauri::WebviewWindow;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPermissions {
    /// 当前平台是否需要在应用内引导授权。
    supported: bool,
    screen_capture: bool,
    accessibility: bool,
}

fn current() -> SystemPermissions {
    #[cfg(target_os = "macos")]
    return SystemPermissions {
        supported: true,
        screen_capture: crate::shared::macos::screen_capture_granted(),
        accessibility: crate::shared::macos::accessibility_trusted(),
    };
    #[cfg(not(target_os = "macos"))]
    SystemPermissions {
        supported: false,
        screen_capture: true,
        accessibility: true,
    }
}

/// 设置页读取授权状态；系统查询很快，不另开阻塞线程。
#[tauri::command]
pub fn system_permissions(window: WebviewWindow) -> Result<SystemPermissions, String> {
    require_window(&window, "main")?;
    Ok(current())
}

/// 请求授权并打开对应的系统设置面板。屏幕录制先调用系统请求，让应用出现在授权列表里；
/// 系统要求重启应用后新授权才生效，因此返回的状态可能仍为未授权。
#[tauri::command]
pub async fn request_system_permission(
    window: WebviewWindow,
    kind: String,
) -> Result<SystemPermissions, String> {
    require_window(&window, "main")?;
    let screen_capture = match kind.as_str() {
        "screenCapture" => true,
        "accessibility" => false,
        _ => return Err("未知的系统权限。".into()),
    };
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            if screen_capture && !crate::shared::macos::request_screen_capture() {
                crate::shared::macos::open_privacy_pane(true)?;
            } else if !screen_capture {
                crate::shared::macos::open_privacy_pane(false)?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = screen_capture;
    Ok(current())
}
