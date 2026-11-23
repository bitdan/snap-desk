//! 录屏保存目录的界面命令；临时输出、发布与目录记录在 core（`snap_core::recording::output`）。
//! 路径只能由原生目录选择器产生。
pub use snap_core::recording::output::{restore_session_directory, validate_directory};
use crate::capture::require_window;
use crate::settings::{RecordingExportSettings, SettingsState};
use serde::Serialize;
use snap_core::recording::output as core;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

// 防止多个目录对话框或“恢复默认”同时修改目录记录；不与采集/编码锁相互等待。
static CHANGING: Mutex<()> = Mutex::new(());

pub(super) fn preferences(app: &AppHandle) -> Result<RecordingExportSettings, String> {
    let state = app.state::<SettingsState>();
    let current = state.0.lock().map_err(|e| e.to_string())?;
    Ok(current.value.recording_export.clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDirectory {
    pub path: String,
    pub is_default: bool,
}

fn record(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("recording-directory.json"))
}

fn default_directory(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("recordings"))
}

pub(crate) fn resolve(app: &AppHandle) -> Result<(PathBuf, bool), String> {
    let remember = preferences(app)?.remember_directory;
    core::resolve(&record(app)?, remember, default_directory(app)?)
}

/// 备份在设置锁内使用已读取的记忆开关，避免再次锁定设置状态。
pub(crate) fn configured_directory(
    app: &AppHandle,
    remember: bool,
) -> Result<Option<PathBuf>, String> {
    core::configured_directory(&record(app)?, remember)
}

/// 选择器和配置导入共用目录提交；调用方持有设置写闸门，保证与导入互斥。
pub(crate) fn configure_directory(
    app: &AppHandle,
    directory: Option<&Path>,
    remember: bool,
) -> Result<(), String> {
    core::configure_directory(&record(app)?, directory, remember)
}

fn view(path: &Path, is_default: bool) -> RecordingDirectory {
    RecordingDirectory {
        path: path.to_string_lossy().into_owned(),
        is_default,
    }
}

pub fn prepare(app: &AppHandle) -> Result<PathBuf, String> {
    let remember = preferences(app)?.remember_directory;
    core::prepare(&record(app)?, remember, default_directory(app)?)
}

#[tauri::command]
pub async fn current_recording_directory(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<RecordingDirectory, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (path, is_default) = resolve(&app)?;
        Ok(view(&path, is_default))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取消对话框返回 None，不更新 UI 或配置；reset 只恢复偏好，不移动或删除已有视频。
#[tauri::command]
pub async fn choose_recording_directory(
    app: AppHandle,
    window: WebviewWindow,
    reset: bool,
) -> Result<Option<RecordingDirectory>, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _changing = CHANGING
            .try_lock()
            .map_err(|_| "请先完成当前目录选择。".to_string())?;
        if reset {
            let state = app.state::<SettingsState>();
            // 只在短锁内读取偏好；目录写入与配置导入共用写闸门，对话框不阻塞设置保存。
            let remember = {
                let current = state.0.lock().map_err(|e| e.to_string())?;
                current.value.recording_export.remember_directory
            };
            {
                let _gate = state.1.gate()?;
                configure_directory(&app, None, remember)?;
            }
            let (path, is_default) = resolve(&app)?;
            return Ok(Some(view(&path, is_default)));
        }
        let mut dialog = app
            .dialog()
            .file()
            .set_title(crate::shared::native_i18n::text(&app, "选择录屏保存文件夹"))
            .set_parent(&window);
        if let Ok((path, _)) = resolve(&app) {
            if path.is_dir() {
                dialog = dialog.set_directory(path);
            }
        }
        let Some(selected) = dialog.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(|e| e.to_string())?;
        validate_directory(&path)?;
        let state = app.state::<SettingsState>();
        let remember = {
            let current = state.0.lock().map_err(|e| e.to_string())?;
            current.value.recording_export.remember_directory
        };
        {
            let _gate = state.1.gate()?;
            configure_directory(&app, Some(&path), remember)?;
        }
        Ok(Some(view(&path, false)))
    })
    .await
    .map_err(|e| e.to_string())?
}
