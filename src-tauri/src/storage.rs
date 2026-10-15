//! 通用设置的目录入口：前端只能传固定种类，实际路径由后端解析。
use crate::{capture::require_window, settings::SettingsState};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AppDirectory {
    Config,
    Captures,
    Recordings,
    ScrollResults,
    Clipboard,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryView {
    kind: AppDirectory,
    path: Option<String>,
    error: Option<String>,
}
/// 第二项表示应用自己管理的目录，只有这些目录可以按需创建。
fn resolve(app: &AppHandle, kind: AppDirectory) -> Result<(Option<PathBuf>, bool), String> {
    match kind {
        AppDirectory::Clipboard => Ok((
            Some(
                app.path()
                    .app_local_data_dir()
                    .map_err(|e| e.to_string())?
                    .join("clipboard"),
            ),
            true,
        )),
        AppDirectory::Config => Ok((
            Some(app.path().app_config_dir().map_err(|e| e.to_string())?),
            true,
        )),
        AppDirectory::Captures => Ok((crate::image_io::current_directory(app)?, false)),
        AppDirectory::Recordings => {
            let (path, is_default) = crate::capture::recording::output::resolve(app)?;
            Ok((Some(path), is_default))
        }
        AppDirectory::ScrollResults => Ok((
            Some(
                app.path()
                    .app_local_data_dir()
                    .map_err(|e| e.to_string())?
                    .join("scroll-results"),
            ),
            true,
        )),
    }
}
#[tauri::command]
pub async fn app_directories(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<DirectoryView>, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok([
            AppDirectory::Config,
            AppDirectory::Captures,
            AppDirectory::Recordings,
            AppDirectory::ScrollResults,
            AppDirectory::Clipboard,
        ]
        .into_iter()
        .map(|kind| match resolve(&app, kind) {
            Ok((path, _)) => DirectoryView {
                kind,
                path: path.map(|p| p.to_string_lossy().into_owned()),
                error: None,
            },
            Err(error) => DirectoryView {
                kind,
                path: None,
                error: Some(error),
            },
        })
        .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn open_app_directory(
    app: AppHandle,
    window: WebviewWindow,
    kind: AppDirectory,
) -> Result<(), String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (path, managed) = resolve(&app, kind)?;
        let path = path.ok_or("尚未记录截图保存目录，请先保存一张截图。")?;
        if managed {
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        }
        if !path.is_absolute() || !path.is_dir() {
            return Err("目录不存在或当前不可用，请重新选择。".into());
        }
        crate::platform::open_directory(&path)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn copy_app_directory(
    app: AppHandle,
    window: WebviewWindow,
    kind: AppDirectory,
) -> Result<(), String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = resolve(&app, kind)?.0.ok_or("尚未记录目录。")?;
        app.clipboard()
            .write_text(path.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
/// 示例只包含快捷键契约，复制不保存、不改变当前绑定。
#[tauri::command]
pub async fn copy_shortcut_example(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let example = serde_json::json!({ "shortcutsEnabled": true, "shortcuts": crate::settings::Shortcuts::default() });
        app.clipboard().write_text(serde_json::to_string_pretty(&example).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

/// 导出已生效的主配置；输出位置来自原生对话框，不开放任意路径写入 IPC。
#[tauri::command]
pub async fn export_settings(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let value = app
            .state::<SettingsState>()
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .value
            .clone();
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("导出主配置")
            .set_file_name("snap-desk-settings.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file()
        else {
            return Ok(false);
        };
        crate::settings::persist(&file.into_path().map_err(|e| e.to_string())?, &value)?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_kinds_reject_arbitrary_paths() {
        for value in ["config", "captures", "recordings", "scrollResults"] {
            assert!(serde_json::from_str::<AppDirectory>(&format!("\"{value}\"")).is_ok());
        }
        for value in [
            r#""../../etc""#,
            r#""C:\\Windows""#,
            r#"{"path":"anything"}"#,
        ] {
            assert!(serde_json::from_str::<AppDirectory>(value).is_err());
        }
    }
}
