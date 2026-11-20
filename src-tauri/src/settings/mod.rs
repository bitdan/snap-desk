//! 设置的 Tauri 外壳：启动时读取 JSON，保存时同时更新文件、系统快捷键和各窗口。
//! 设置模型、校验、落盘和快捷键事务在 `snap_core::settings`，这里只接 AppHandle 与事件。
//! 前端只传明确的设置字段，不能指定文件路径。损坏的配置不会在启动时被覆盖。
pub(crate) mod autostart;
pub(crate) mod permissions;
#[cfg(target_os = "linux")]
pub(crate) mod portal_shortcuts;
pub(crate) mod shortcut_probe;
pub(crate) mod storage;
pub(crate) mod backup;

pub use snap_core::settings::*;

use crate::capture::{require_window, CaptureState};
use std::sync::{atomic::Ordering, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

/// 第一项只保护内存快照；第二项串行化快捷键事务并把磁盘写入交给独立线程。
pub struct SettingsState(pub Mutex<RuntimeSettings>, pub(crate) SettingsWriter);
/// 松键事件独立于配置锁处理，避免保存时丢掉松键后一直被视为长按。
#[derive(Default)]
pub struct ShortcutPressState(pub Mutex<Vec<Shortcut>>);
pub struct ShortcutMenu(pub tauri::menu::CheckMenuItem<tauri::Wry>);

/// 持锁调用，确保并发保存时托盘勾选状态与磁盘、内存提交顺序一致。
fn notify_settings(app: &AppHandle, value: &Settings) {
    #[cfg(target_os = "linux")]
    portal_shortcuts::sync(value);
    if let Some(menu) = app.try_state::<ShortcutMenu>() {
        if let Err(_error) = menu.0.set_checked(value.shortcuts_enabled) {
            log::error!("event=app.shortcut_menu_failed module=app code=SHORTCUT_MENU_FAILED");
        }
    }
    // notify_settings 的调用方已持有设置锁，直接传语言快照避免重入死锁。
    if let Some(menu) = app.try_state::<ShortcutMenu>() {
        if menu
            .0
            .set_text(crate::shared::native_i18n::for_language(
                &value.appearance.language,
                "启用全局快捷键",
            ))
            .is_err()
        {
            log::warn!("event=app.tray_language_failed module=app");
        }
    }
    if let Some(tray) = app.try_state::<crate::shared::native_i18n::TrayLabels>() {
        tray.update(&value.appearance.language);
    }
    let _ = app.emit("appearance-changed", &value.appearance);
    let _ = app.emit("pin-appearance-changed", &value.pin_appearance);
    let _ = app.emit_to("main", "settings-changed", value);
}


pub fn initialize(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let path = app.path().app_config_dir()?.join("settings.json");
    let (mut value, config_warning) = load(&path);
    // 旧版翻译页直接填写的在线配置并入服务管理；密钥复制失败时保留连接，用户可在服务管理中重新填写。
    if let Some((old, new)) = crate::recognition::services::migrate_legacy_translation(&mut value) {
        match crate::recognition::ai_credentials::read(&old) {
            Ok(Some(secret)) => {
                if crate::recognition::ai_credentials::write(&new, Some(secret)).is_err() {
                    log::warn!("event=settings.legacy_translation_credential_failed module=settings");
                }
            }
            Ok(None) => (),
            Err(_) => log::warn!("event=settings.legacy_translation_credential_failed module=settings"),
        }
    }
    let mut registered = Vec::new();
    let shortcut_error = if value.shortcuts_enabled {
        apply_settings(
            &value,
            &value,
            &mut registered,
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
            |_| Ok(()),
        )
        .unwrap_or_else(Some)
    } else {
        None
    };
    let writer = SettingsWriter::new().map_err(std::io::Error::other)?;
    app.manage(SettingsState(
        Mutex::new(RuntimeSettings {
            value,
            registered,
            editing: false,
            shortcut_error,
            config_warning,
        }),
        writer,
    ));
    Ok(())
}


/// 在设置锁外完成快捷键注册与磁盘提交；返回系统实际注册列表和事务结果。
/// 调用方必须持有 `SettingsWriter` 闸门，并在返回后只短暂持锁提交内存快照。
pub(super) fn apply_outside_lock(
    app: &AppHandle,
    old: &Settings,
    next: &Settings,
    mut registered: Vec<Shortcut>,
    write: impl Fn(&Settings) -> Result<(), String>,
) -> (Vec<Shortcut>, Result<Option<String>, String>) {
    let result = apply_settings(
        old,
        next,
        &mut registered,
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
        write,
    );
    (registered, result)
}

/// 命令在后台线程执行：系统快捷键注册可能调度主线程，不能在主线程持锁等待。
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    window: WebviewWindow,
    settings: Settings,
) -> Result<Settings, String> {
    require_window(&window, "main")?;
    settings.validate()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SettingsState>();
        // 闸门在设置数据锁之外串行化整笔事务，磁盘 I/O 由 settings-writer 线程完成。
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
        let path = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("settings.json");
        let mut settings = settings;
        // 历史开关仅通过 clipboard_configure 合并，旧页面草稿不得覆盖。
        settings.clipboard = old.clipboard.clone();
        // 并发完成引导后，旧表单不能把版本号退回去。
        settings.onboarding_version = settings
            .onboarding_version
            .max(old.onboarding_version);
        let (registered, outcome) = apply_outside_lock(&app, &old, &settings, registered, |value| {
            state.1.persist(&path, value)
        });
        // 无论成功失败，都把系统实际注册状态提交回内存；失败时保留旧配置。
        {
            let mut current = state.0.lock().map_err(|e| e.to_string())?;
            current.registered = registered;
            if let Ok(warning) = &outcome {
                current.value = settings.clone();
                current.shortcut_error = warning.clone();
                current.config_warning = None;
            }
        }
        outcome?;
        app.state::<ShortcutPressState>()
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .clear();
        // 通知与托盘菜单更新不持有配置锁，避免拖慢截图、贴图等读取设置的入口。
        notify_settings(&app, &settings);
        Ok(settings)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 托盘与设置页共用总开关；None 表示在闸门内基于最新快照切换，避免快速连点使用过期值。
pub fn set_shortcuts(app: &AppHandle, enabled: Option<bool>) -> Result<Settings, String> {
    let state = app.state::<SettingsState>();
    let _gate = state.1.gate()?;
    let (old, registered) = {
        let current = state.0.lock().map_err(|e| e.to_string())?;
        if current.editing {
            return Err("请先结束快捷键录入或当前截图。".into());
        }
        (current.value.clone(), current.registered.clone())
    };
    let mut next = old.clone();
    next.shortcuts_enabled = enabled.unwrap_or(!next.shortcuts_enabled);
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("settings.json");
    let (registered, result) = apply_outside_lock(app, &old, &next, registered, |value| {
        state.1.persist(&path, value)
    });
    let warning = match result {
        Ok(warning) => warning,
        Err(error) => {
            // 快捷键事务可能已部分生效；先提交真实注册状态，再恢复原生勾选外观。
            if let Ok(mut current) = state.0.lock() {
                current.registered = registered;
            }
            // 原生勾选菜单会先切换外观，失败时恢复为真正生效的设置。
            notify_settings(app, &old);
            return Err(error);
        }
    };
    {
        let mut current = state.0.lock().map_err(|e| e.to_string())?;
        current.registered = registered;
        current.value = next.clone();
        current.shortcut_error = warning;
        current.config_warning = None;
    }
    app.state::<ShortcutPressState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .clear();
    notify_settings(app, &next);
    Ok(next)
}

#[tauri::command]
pub async fn set_shortcuts_enabled(
    app: AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> Result<Settings, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || set_shortcuts(&app, Some(enabled)))
        .await
        .map_err(|e| e.to_string())?
}

/// 录入时临时释放系统注册；结束或失焦后恢复，整个过程不改写磁盘。
pub fn set_editing(app: &AppHandle, editing: bool) -> Result<(), String> {
    let state = app.state::<SettingsState>();
    let _gate = state.1.gate()?;
    let (old, registered) = {
        let current = state.0.lock().map_err(|e| e.to_string())?;
        if current.editing == editing {
            return Ok(());
        }
        if editing && app.state::<CaptureState>().active.load(Ordering::SeqCst) {
            return Err("请先结束快捷键录入或当前截图。".into());
        }
        (current.value.clone(), current.registered.clone())
    };
    let mut target = old.clone();
    if editing {
        target.shortcuts_enabled = false;
    }
    app.state::<ShortcutPressState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .clear();
    let (registered, result) =
        apply_outside_lock(app, &old, &target, registered, |_| Ok(()));
    let mut current = state.0.lock().map_err(|e| e.to_string())?;
    current.registered = registered;
    if !editing || result.is_ok() {
        current.editing = editing;
    }
    if let Err(error) = result {
        current.shortcut_error = Some(error.clone());
        return Err(error);
    }
    current.shortcut_error = None;
    Ok(())
}
#[tauri::command]
pub async fn set_shortcut_editing(
    app: AppHandle,
    window: WebviewWindow,
    editing: bool,
) -> Result<(), String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        if editing && !window.is_focused().map_err(|e| e.to_string())? {
            return Err("请在主窗口录入快捷键。".into());
        }
        set_editing(&app, editing)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 跳过也记录引导版本；只合并这一字段，避免覆盖其他页面的设置。
#[tauri::command]
pub async fn complete_onboarding(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Settings, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SettingsState>();
        let _gate = state.1.gate()?;
        let mut next = {
            let current = state.0.lock().map_err(|e| e.to_string())?;
            current.value.clone()
        };
        next.onboarding_version = next.onboarding_version.max(1);
        let path = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("settings.json");
        state.1.persist(&path, &next)?;
        {
            let mut current = state.0.lock().map_err(|e| e.to_string())?;
            current.value = next.clone();
            current.config_warning = None;
        }
        notify_settings(&app, &next);
        Ok(next)
    })
    .await
    .map_err(|e| e.to_string())?
}

