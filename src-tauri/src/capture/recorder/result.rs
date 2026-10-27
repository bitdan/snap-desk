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
    replay_layout: Option<super::chrome::ReplayLayout>,
}

#[derive(Default)]
pub struct ResultState(Mutex<Option<RecordingResult>>);

/// 发布成功后更新，失败或取消不覆盖上一次结果；查询不读取整个视频。
pub fn remember(app: &AppHandle, path: &Path) {
    remember_with_layout(app, path, None);
}
/// 只有当前录制携带原选区；恢复历史文件不能误用上次录制的桌面位置。
pub(super) fn remember_with_layout(
    app: &AppHandle,
    path: &Path,
    replay_layout: Option<super::chrome::ReplayLayout>,
) {
    let state = app.state::<ResultState>();
    if let Ok(mut current) = state.0.lock() {
        let id = current.as_ref().map_or(1, |result| result.id + 1);
        *current = Some(RecordingResult {
            id,
            replay_layout,
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
    Preview,
    OpenDirectory,
    CopyPath,
}

fn media_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || !matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("mp4" | "gif")
        )
    {
        return Err("录屏文件路径或格式无效。".into());
    }
    if !path.is_file() {
        return Err("录屏文件已被移动或删除。".into());
    }
    Ok(path.to_path_buf())
}

/// 编辑器只接收结果编号；源路径仍由原生最近成功导出记录解析。
pub(super) fn source_path(app: &AppHandle, id: u64) -> Result<PathBuf, String> {
    let state = app.state::<ResultState>();
    let current = state.0.lock().map_err(|e| e.to_string())?;
    let result = current
        .as_ref()
        .filter(|value| value.id == id)
        .ok_or("录屏结果已更新，请刷新后重试。")?;
    media_path(Path::new(&result.path))
}

/// 预览交给系统默认播放器，避免把大视频整段编码成 Base64 传进 WebView。
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
            ResultAction::Preview => {
                open::that(media_path(&path)?).map_err(|e| format!("无法打开系统播放器：{e}"))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_rejects_missing_files_and_non_media() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clip.mp4");
        assert!(media_path(&path).is_err());
        std::fs::write(&path, b"video").unwrap();
        assert!(media_path(&path).is_ok());
        let executable = directory.path().join("clip.exe");
        std::fs::write(&executable, b"executable").unwrap();
        assert!(media_path(&executable).is_err());
    }
}
