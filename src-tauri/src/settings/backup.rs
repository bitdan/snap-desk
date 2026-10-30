//! 完整配置备份：固定版本格式、受限文件选择、快捷键事务和目录/监听状态回滚。
use super::{
    apply_settings, notify_settings, persist, Settings, SettingsState, ShortcutPressState,
};
use crate::{
    capture::recording::output,
    capture::{require_window, CaptureState},
    shared::image_io,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Mutex},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 512 * 1024;
static TRANSFER: Mutex<()> = Mutex::new(());

/// null 表示使用默认位置；路径只备份目录偏好，不携带其中的图片或视频。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Directories {
    captures: Option<PathBuf>,
    recordings: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Backup {
    schema_version: u32,
    settings: Settings,
    directories: Directories,
}

fn parse(bytes: &[u8]) -> Result<Backup, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("配置文件超过 512 KiB，请检查所选文件。".into());
    }
    let json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "配置格式无效，请选择当前版本导出的配置文件。".to_string())?;
    // 导入要求当前完整结构，不借用启动配置的缺省补齐来悄悄重置遗漏字段。
    let shape = serde_json::to_value(Backup {
        schema_version: VERSION,
        settings: Settings::default(),
        directories: Directories {
            captures: None,
            recordings: None,
        },
    })
    .map_err(|e| e.to_string())?;
    if !complete_shape(&json, &shape) {
        return Err("配置格式无效，请选择当前版本导出的配置文件。".into());
    }
    let value: Backup = serde_json::from_value(json)
        .map_err(|_| "配置格式无效，请选择当前版本导出的配置文件。".to_string())?;
    if value.schema_version != VERSION {
        return Err("配置版本不支持，请使用当前版本重新导出。".into());
    }
    value.settings.validate()?;
    Ok(value)
}
fn complete_shape(value: &serde_json::Value, shape: &serde_json::Value) -> bool {
    match shape.as_object() {
        Some(fields) => value.as_object().is_some_and(|object| {
            fields.iter().all(|(key, expected)| {
                object
                    .get(key)
                    .is_some_and(|value| complete_shape(value, expected))
            })
        }),
        None => true,
    }
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("配置文件超过 512 KiB，请检查所选文件。".into());
    }
    Ok(bytes)
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// 仅快照应用自己的三个固定配置文件；回滚尝试恢复全部文件，绝不删除用户媒体。
struct Files(Vec<(PathBuf, Option<Vec<u8>>)>);
impl Files {
    fn snapshot(root: &Path) -> Result<Self, String> {
        let mut files = Vec::new();
        for name in [
            "settings.json",
            "export-directory.json",
            "recording-directory.json",
        ] {
            let path = root.join(name);
            let bytes = match read_bounded(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if path.try_exists().map_err(|e| e.to_string())? => return Err(error),
                Err(_) => None,
            };
            files.push((path, bytes));
        }
        Ok(Self(files))
    }
    fn restore(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        for (path, bytes) in &self.0 {
            let result = if let Some(bytes) = bytes {
                write_bytes(path, bytes)
            } else {
                match fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                }
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }
}

fn commit(app: &AppHandle, backup: Backup) -> Result<Settings, String> {
    // 先验证目录，失效的外置盘或其他机器路径不会触发部分导入或自动创建目录。
    if let Some(path) = &backup.directories.captures {
        image_io::validate_directory(path)?;
    }
    if let Some(path) = &backup.directories.recordings {
        output::validate_directory(path)?;
    }
    let state = app.state::<SettingsState>();
    let mut current = state.0.lock().map_err(|e| e.to_string())?;
    if app.state::<CaptureState>().active.load(Ordering::SeqCst) {
        return Err("请先完成或取消截图，再修改设置。".into());
    }
    if current.editing {
        return Err("请先结束快捷键录入。".into());
    }
    let root = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let old = current.value.clone();
    let old_recording = output::configured_directory(app, old.recording_export.remember_directory)?;
    let files = Files::snapshot(&root)?;
    let mut next = backup.settings;
    next.onboarding_version = next.onboarding_version.max(old.onboarding_version);
    let clipboard = app.state::<crate::clipboard::Service>();
    let clipboard_changed = old.clipboard != next.clipboard;
    if clipboard_changed {
        if let Err(error) = clipboard.configure(app, next.clipboard.clone()) {
            let rollback = clipboard.configure(app, old.clipboard.clone());
            return Err(with_rollback(error, rollback.err().into_iter().collect()));
        }
    }
    let result = apply_settings(
        &old,
        &next,
        &mut current.registered,
        |key| {
            app.global_shortcut()
                .register(key)
                .map_err(|e| e.to_string())
        },
        |key| {
            app.global_shortcut()
                .unregister(key)
                .map_err(|e| e.to_string())
        },
        |value| {
            if value == &next {
                image_io::configure_directory(app, backup.directories.captures.as_deref())?;
                output::configure_directory(
                    app,
                    backup.directories.recordings.as_deref(),
                    next.recording_export.remember_directory,
                )?;
                persist(&root.join("settings.json"), value)
            } else {
                files.restore()?;
                output::restore_session_directory(old_recording.clone())
            }
        },
    );
    match result {
        Ok(warning) => {
            current.value = next.clone();
            current.shortcut_error = warning;
            current.config_warning = None;
            if let Ok(mut pressed) = app.state::<ShortcutPressState>().0.lock() {
                pressed.clear();
            }
            notify_settings(app, &next);
            // 主界面独立订阅导入完成，切换页面也不能保留导入前的表单草稿。
            let _ = app.emit_to("main", "settings-imported", &next);
            drop(current);
            if clipboard_changed {
                crate::clipboard::settings_imported(app);
            }
            log::info!("event=settings.imported module=app schema_version={VERSION}");
            Ok(next)
        }
        Err(error) => {
            let mut errors = Vec::new();
            if let Err(error) = files.restore() {
                errors.push(error);
            }
            if let Err(error) = output::restore_session_directory(old_recording) {
                errors.push(error);
            }
            if clipboard_changed {
                if let Err(error) = clipboard.configure(app, old.clipboard) {
                    errors.push(error);
                }
            }
            Err(with_rollback(error, errors))
        }
    }
}
fn with_rollback(error: String, failures: Vec<String>) -> String {
    if failures.is_empty() {
        error
    } else {
        format!("配置导入失败：{error}；回滚失败：{}", failures.join("；"))
    }
}

/// 原生命令只接受窗口，不接受任意文件路径或前端未校验的配置。
#[tauri::command]
pub async fn export_settings(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = TRANSFER
            .try_lock()
            .map_err(|_| "请先完成当前配置导入或导出。".to_string())?;
        let value = {
            let state = app.state::<SettingsState>();
            let current = state.0.lock().map_err(|e| e.to_string())?;
            Backup {
                schema_version: VERSION,
                directories: Directories {
                    captures: image_io::current_directory(&app)?,
                    recordings: output::configured_directory(
                        &app,
                        current.value.recording_export.remember_directory,
                    )?,
                },
                settings: current.value.clone(),
            }
        };
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(crate::shared::native_i18n::text(&app, "导出配置"))
            .set_file_name("snap-desk-settings.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let destination = file.into_path().map_err(|e| e.to_string())?;
        if let Ok(canonical) = destination.canonicalize() {
            let root = app.path().app_config_dir().map_err(|e| e.to_string())?;
            if [
                "settings.json",
                "export-directory.json",
                "recording-directory.json",
            ]
            .iter()
            .filter_map(|name| root.join(name).canonicalize().ok())
            .any(|path| path == canonical)
            {
                return Err("不能覆盖正在使用的配置文件，请另选导出位置。".into());
            }
        }
        write_bytes(
            &destination,
            &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn import_settings(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<Settings>, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = TRANSFER
            .try_lock()
            .map_err(|_| "请先完成当前配置导入或导出。".to_string())?;
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(crate::shared::native_i18n::text(&app, "导入配置"))
            .add_filter("JSON", &["json"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let backup = parse(&read_bounded(
            &file.into_path().map_err(|e| e.to_string())?,
        )?)?;
        commit(&app, backup).map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn backup() -> Backup {
        Backup {
            schema_version: VERSION,
            settings: Settings::default(),
            directories: Directories {
                captures: None,
                recordings: None,
            },
        }
    }
    #[test]
    fn complete_backup_round_trips_shortcuts_and_directories() {
        let mut value = backup();
        value.settings.shortcuts.capture = "Ctrl+Shift+A".into();
        value.settings.shortcuts_enabled = false;
        value.directories.captures = Some(PathBuf::from("C:/Pictures"));
        value.directories.recordings = Some(PathBuf::from("D:/Videos"));
        let imported = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(imported.settings, value.settings);
        assert_eq!(imported.directories.captures, value.directories.captures);
        assert_eq!(
            imported.directories.recordings,
            value.directories.recordings
        );
    }
    #[test]
    fn rejects_old_versions_unknown_fields_and_invalid_settings() {
        assert!(parse(&serde_json::to_vec(&Settings::default()).unwrap()).is_err());
        let mut value = serde_json::to_value(backup()).unwrap();
        value["schemaVersion"] = 99.into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value["schemaVersion"] = VERSION.into();
        value["unexpected"] = true.into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value.as_object_mut().unwrap().remove("unexpected");
        value["settings"]["shortcuts"]["pin"] = "F1".into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(parse(&vec![b' '; MAX_BYTES as usize + 1]).is_err());
    }
    #[test]
    fn incomplete_backups_cannot_reset_missing_fields_to_defaults() {
        let mut value = serde_json::to_value(backup()).unwrap();
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("shortcuts");
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(backup()).unwrap();
        value["directories"]
            .as_object_mut()
            .unwrap()
            .remove("captures");
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn rollback_restores_existing_preferences_and_removes_only_new_records() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("settings.json"), b"original").unwrap();
        let snapshot = Files::snapshot(root.path()).unwrap();
        fs::write(root.path().join("settings.json"), b"changed").unwrap();
        fs::write(root.path().join("export-directory.json"), b"new").unwrap();
        fs::write(root.path().join("image.png"), b"keep").unwrap();
        snapshot.restore().unwrap();
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            b"original"
        );
        assert!(!root.path().join("export-directory.json").exists());
        assert_eq!(fs::read(root.path().join("image.png")).unwrap(), b"keep");
    }
}
