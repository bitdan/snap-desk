//! 主窗口状态与应用级命令，窗口权限检查在命令入口执行。
use crate::{capture::{self, require_window, CaptureState}, settings};
use crate::shared::platform;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppStatus {
    settings: settings::Settings,
    shortcut_error: Option<String>,
    config_warning: Option<String>,
    delay_remaining: u8,
    platform: &'static str,
    version: String,
}

#[tauri::command]
pub(super) async fn app_status(app: AppHandle, window: WebviewWindow) -> Result<AppStatus, String> {
    require_window(&window, "main")?;
    // 设置保存会调用主线程注册快捷键/更新菜单，查询不能在主线程等待同一把锁。
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<settings::SettingsState>();
        let current = state.0.lock().map_err(|e| e.to_string())?;
        Ok(AppStatus {
            settings: current.value.clone(),
            shortcut_error: current.shortcut_error.clone(),
            config_warning: current.config_warning.clone(),
            delay_remaining: app
                .state::<CaptureState>()
                .delay_remaining
                .load(Ordering::SeqCst),
            platform: std::env::consts::OS,
            version: app.package_info().version.to_string(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) fn hide_main(window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    window.hide().map_err(|e| e.to_string())
}

/// 关于页点击 GitHub 图标后交给系统浏览器；仅允许主窗口打开固定项目地址。
#[tauri::command]
pub(super) async fn open_project_homepage(window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(platform::open_project_homepage)
        .await
        .map_err(|e| e.to_string())?
}

/// 主窗口的录屏按钮直接进入录屏框选，复用全局录屏动作的采集入口。
#[tauri::command]
pub(super) async fn select_recording_area(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    capture::begin_recording(app).await
}
