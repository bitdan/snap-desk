//! 用户级登录自启：系统注册项是唯一状态源，不在 JSON 中重复保存布尔值。
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;

/// 自启入口携带固定参数，让程序初始化托盘和快捷键但不弹出主窗口。
pub const STARTUP_ARG: &str = "--autostart";

/// 将系统查询、启用和禁用串行化，避免并发请求返回相互矛盾的状态。
#[derive(Default)]
pub struct AutostartState(Mutex<()>);

/// 首次启动和单实例回调共用精确参数匹配，避免把相似参数当作登录启动。
pub fn is_autostart(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> bool {
    args.into_iter().any(|arg| arg.as_ref() == STARTUP_ARG)
}

/// 通用设置进入或刷新时读取系统状态，系统侧手动删除注册项后也能反映实际值。
#[tauri::command]
pub async fn autostart_enabled(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AutostartState>();
        let _guard = state.0.lock().map_err(|e| e.to_string())?;
        app.autolaunch()
            .is_enabled()
            .map_err(|e| format!("读取开机自启状态失败：{e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 只允许主窗口切换当前程序的注册项；不接收可执行文件路径或启动参数。
/// 系统操作完成后重新读取，前端仅使用确认后的状态，不提前展示保存成功。
#[tauri::command]
pub async fn set_autostart_enabled(
    app: AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> Result<bool, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AutostartState>();
        let _guard = state.0.lock().map_err(|e| e.to_string())?;
        let manager = app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|e| format!("设置开机自启失败：{e}"))?;
        let actual = manager
            .is_enabled()
            .map_err(|e| format!("设置已提交，但读取自启状态失败，请刷新确认：{e}"))?;
        if actual != enabled {
            return Err("系统未应用开机自启设置，请刷新状态后重试。".into());
        }
        Ok(actual)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_startup_flag_hides_main_window() {
        assert!(is_autostart(["snap-desk", "--autostart"]));
        assert!(!is_autostart(["snap-desk"]));
        assert!(!is_autostart(["snap-desk", "--autostart=false"]));
    }
}
