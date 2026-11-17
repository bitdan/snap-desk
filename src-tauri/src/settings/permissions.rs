//! 系统隐私权限的查询与引导。目前只有 macOS 需要：截图与录屏依赖“屏幕录制”，
//! 控件吸附、直接粘贴、划词翻译依赖“辅助功能”，录屏键鼠特效依赖“输入监控”。
//! 其他平台返回 supported=false，前端隐藏入口。
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
    input_monitoring: bool,
}

fn current() -> SystemPermissions {
    #[cfg(target_os = "macos")]
    return SystemPermissions {
        supported: true,
        screen_capture: crate::shared::macos::screen_capture_granted(),
        accessibility: crate::shared::macos::accessibility_trusted(),
        input_monitoring: crate::shared::macos::input_monitoring_granted(),
    };
    #[cfg(not(target_os = "macos"))]
    SystemPermissions {
        supported: false,
        screen_capture: true,
        accessibility: true,
        input_monitoring: true,
    }
}

/// 设置页读取授权状态；系统查询很快，不另开阻塞线程。
#[tauri::command]
pub fn system_permissions(window: WebviewWindow) -> Result<SystemPermissions, String> {
    require_window(&window, "main")?;
    Ok(current())
}

/// 请求授权并打开对应的系统设置面板。屏幕录制与输入监控先调用系统请求，让应用出现在授权列表里，
/// 系统弹窗已处理时不再重复打开设置；新授权通常需要重启应用才生效，因此返回的状态可能仍为未授权。
#[tauri::command]
pub async fn request_system_permission(
    window: WebviewWindow,
    kind: String,
) -> Result<SystemPermissions, String> {
    require_window(&window, "main")?;
    if !matches!(kind.as_str(), "screenCapture" | "accessibility" | "inputMonitoring") {
        return Err("未知的系统权限。".into());
    }
    #[cfg(target_os = "macos")]
    {
        use crate::shared::macos::{self, PrivacyPane};
        tauri::async_runtime::spawn_blocking(move || {
            let (granted, pane) = match kind.as_str() {
                "screenCapture" => (macos::request_screen_capture(), PrivacyPane::ScreenCapture),
                "inputMonitoring" => (macos::request_input_monitoring(), PrivacyPane::InputMonitoring),
                _ => (false, PrivacyPane::Accessibility),
            };
            if !granted {
                macos::open_privacy_pane(pane)?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = kind;
    Ok(current())
}
