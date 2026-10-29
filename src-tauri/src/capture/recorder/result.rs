//! 本次运行最近成功导出的录屏；原生持有路径，页面只能按结果 ID 操作。
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingResult {
    id: u64,
    path: String,
    filename: String,
}

#[derive(Default)]
pub struct ResultState(Mutex<Option<RecordingResult>>);

/// 发布成功后更新，失败或取消不覆盖上一次结果；查询不读取整个视频。
pub fn remember(app: &AppHandle, path: &Path) {
    let state = app.state::<ResultState>();
    if let Ok(mut current) = state.0.lock() {
        let id = current.as_ref().map_or(1, |result| result.id + 1);
        *current = Some(RecordingResult {
            id,
            path: path.to_string_lossy().into_owned(),
            filename: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        });
    };
    super::notify_changed(app);
}

#[tauri::command]
pub fn latest_recording_result(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<RecordingResult>, String> {
    if !matches!(window.label(), "main" | "recording-controls") {
        return Err("当前窗口无权访问录屏结果。".into());
    }
    let state = app.state::<ResultState>();
    let result = state.0.lock().map_err(|e| e.to_string())?.clone();
    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultAction {
    OpenDirectory,
    CopyPath,
}

/// 成品只提供打开目录和复制路径，不在应用内读取或重新编码视频。
#[tauri::command]
pub async fn recording_result_action(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    action: ResultAction,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ResultState>();
        let path = {
            let current = state.0.lock().map_err(|e| e.to_string())?;
            let result = current
                .as_ref()
                .filter(|value| value.id == id)
                .ok_or("录屏结果已更新，请刷新后重试。")?;
            PathBuf::from(&result.path)
        };
        match action {
            ResultAction::OpenDirectory => {
                let directory = path
                    .parent()
                    .filter(|p| p.is_dir())
                    .ok_or("录屏目录已被移动或删除。")?;
                crate::shared::platform::open_directory(directory)
            }
            ResultAction::CopyPath => app
                .clipboard()
                .write_text(path.to_string_lossy().into_owned())
                .map_err(|e| format!("复制路径失败：{e}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
