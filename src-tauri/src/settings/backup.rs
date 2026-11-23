//! 完整配置备份的 Tauri 命令：格式、校验与文件快照在 core `snap_core::settings::backup`；
//! 这里负责受限文件选择、快捷键事务和目录/监听状态回滚。
use super::{apply_outside_lock, notify_settings, Settings, SettingsState, ShortcutPressState};
use crate::{
    capture::recording::output,
    capture::{require_window, CaptureState},
    shared::image_io,
};
use snap_core::settings::backup::{
    export_document, guard_destination, parse, read_bounded, restore_secrets, with_rollback,
    write_bytes, Backup, Directories, Files, CONFIG_FILES, VERSION,
};
use std::sync::{atomic::Ordering, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

static TRANSFER: Mutex<()> = Mutex::new(());

fn commit(app: &AppHandle, mut backup: Backup) -> Result<Settings, String> {
    let secrets = std::mem::take(&mut backup.secrets);
    let next = commit_settings(app, backup)?;
    restore_secrets(&next, secrets)?;
    Ok(next)
}
fn commit_settings(app: &AppHandle, backup: Backup) -> Result<Settings, String> {
    // 先验证目录，失效的外置盘或其他机器路径不会触发部分导入或自动创建目录。
    if let Some(path) = &backup.directories.captures {
        image_io::validate_directory(path)?;
    }
    if let Some(path) = &backup.directories.recordings {
        output::validate_directory(path)?;
    }
    let state = app.state::<SettingsState>();
    // 与普通保存共用写闸门；设置快照在短锁内读取，目录快照、剪贴板与快捷键 I/O 都在锁外。
    let _gate = state.1.gate()?;
    let (old, registered) = {
        let current = state.0.lock().map_err(|e| e.to_string())?;
        if app.state::<CaptureState>().active.load(Ordering::SeqCst) {
            return Err("请先完成或取消截图，再修改设置。".into());
        }
        if current.editing {
            return Err("请先结束快捷键录入。".into());
        }
        (current.value.clone(), current.registered.clone())
    };
    let root = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let old_recording = output::configured_directory(app, old.recording_export.remember_directory)?;
    let files = Files::snapshot(&root)?;
    let mut next = backup.settings;
    next.onboarding_version = next.onboarding_version.max(old.onboarding_version);
    let clipboard = app.state::<crate::clipboard::Service>();
    let clipboard_changed = old.clipboard != next.clipboard;
    if clipboard_changed {
        if let Err(error) = clipboard.configure(next.clipboard.clone()) {
            let rollback = clipboard.configure(old.clipboard.clone());
            return Err(with_rollback(error, rollback.err().into_iter().collect()));
        }
    }
    let (registered, result) = apply_outside_lock(app, &old, &next, registered, |value| {
        if value == &next {
            image_io::configure_directory(app, backup.directories.captures.as_deref())?;
            output::configure_directory(
                app,
                backup.directories.recordings.as_deref(),
                next.recording_export.remember_directory,
            )?;
            state.1.persist(&root.join("settings.json"), value)
        } else {
            files.restore()?;
            output::restore_session_directory(old_recording.clone())
        }
    });
    match result {
        Ok(warning) => {
            {
                let mut current = state.0.lock().map_err(|e| e.to_string())?;
                current.registered = registered;
                current.value = next.clone();
                current.shortcut_error = warning;
                current.config_warning = None;
            }
            if let Ok(mut pressed) = app.state::<ShortcutPressState>().0.lock() {
                pressed.clear();
            }
            notify_settings(app, &next);
            // 主界面独立订阅导入完成，切换页面也不能保留导入前的表单草稿。
            let _ = app.emit_to("main", "settings-imported", &next);
            if clipboard_changed {
                crate::clipboard::settings_imported(app);
            }
            log::info!("event=settings.imported module=app schema_version={VERSION}");
            Ok(next)
        }
        Err(error) => {
            // 快捷键事务失败也可能已改变系统注册；先把实际状态提交回内存，再恢复文件。
            if let Ok(mut current) = state.0.lock() {
                current.registered = registered;
            }
            let mut errors = Vec::new();
            if let Err(error) = files.restore() {
                errors.push(error);
            }
            if let Err(error) = output::restore_session_directory(old_recording) {
                errors.push(error);
            }
            if clipboard_changed {
                if let Err(error) = clipboard.configure(old.clipboard) {
                    errors.push(error);
                }
            }
            Err(with_rollback(error, errors))
        }
    }
}

/// 原生命令只接受窗口，不接受任意文件路径或前端未校验的配置。
#[tauri::command]
pub async fn export_settings(
    app: AppHandle,
    window: WebviewWindow,
    include_secrets: bool,
) -> Result<bool, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = TRANSFER
            .try_lock()
            .map_err(|_| "请先完成当前配置导入或导出。".to_string())?;
        // 设置快照在短锁内完成；目录读取可能触盘，放到锁外避免阻塞其他设置入口。
        let (settings, remember_recording) = {
            let state = app.state::<SettingsState>();
            let current = state.0.lock().map_err(|e| e.to_string())?;
            (current.value.clone(), current.value.recording_export.remember_directory)
        };
        let directories = Directories {
            captures: image_io::current_directory(&app)?,
            recordings: output::configured_directory(&app, remember_recording)?,
        };
        let document = export_document(settings, directories, include_secrets)?;
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
        let root = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let active: Vec<_> = CONFIG_FILES.iter().map(|name| root.join(name)).collect();
        guard_destination(&destination, &active)?;
        write_bytes(&destination, &document)?;
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
