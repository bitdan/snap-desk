//! 设置的唯一来源：启动时读取 JSON，保存时同时更新文件和系统快捷键。
//! 前端只传明确的设置字段，不能指定文件路径。损坏的配置不会在启动时被覆盖。

use crate::capture::{require_window, CaptureState};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::{atomic::Ordering, Mutex},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

/// 普通截图和贴图共用的导出偏好；长截图的自动 PNG 存档保持独立。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub jpeg_quality: u8,
    pub filename_prefix: String,
    pub include_dimensions: bool,
    pub remember_directory: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Png,
            jpeg_quality: 90,
            filename_prefix: "SnapDesk".into(),
            include_dimensions: false,
            remember_directory: true,
        }
    }
}

impl ExportSettings {
    /// 前缀是文件名片段，不能携带路径或 Windows/macOS 不允许的字符。
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err("JPEG 质量必须为 1～100 的整数。".into());
        }
        let prefix = &self.filename_prefix;
        if prefix.is_empty()
            || prefix.chars().count() > 48
            || prefix.trim() != prefix
            || prefix == "."
            || prefix == ".."
            || prefix
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        {
            return Err("文件名前缀须为 1～48 个字符，不能包含路径、控制字符或 <>:\"/\\|?*，首尾不能有空白。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub shortcut: String,
    /// 总开关只影响系统注册，不影响窗口内的编辑快捷键；旧配置默认启用。
    #[serde(default = "enabled_by_default")]
    pub shortcuts_enabled: bool,
    pub delay_seconds: u8,
    // 老版本配置没有此字段时补默认值，不改动原来的快捷键或延时。
    #[serde(default)]
    pub export: ExportSettings,
}
fn enabled_by_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "F1".into(),
            shortcuts_enabled: true,
            delay_seconds: 0,
            export: ExportSettings::default(),
        }
    }
}

impl Settings {
    /// 与前端录入规则一致：F1～F12 可单独作为快捷键；字母、数字需搭配至少一个修饰键。
    /// 后端必须再次校验，不能假设所有 IPC 请求都来自正常表单。
    pub fn validate(&self) -> Result<Shortcut, String> {
        self.export.validate()?;
        if ![0, 3, 5].contains(&self.delay_seconds) {
            return Err("截图延时只能为 0、3 或 5 秒。".into());
        }
        let parts: Vec<_> = self.shortcut.split('+').collect();
        if parts.len() > 5 {
            return Err("快捷键组合过长，最多一个主键加四个修饰键。".into());
        }
        let key = parts[parts.len() - 1];
        if parts.len() == 1 && matches!(key, "F2" | "F3") {
            return Err("F2 已用于剪贴板贴图，F3 已用于录屏，请选择其他截图快捷键。".into());
        }
        let letter = key.len() == 1
            && key
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        let function = (1..=12).any(|n| key == format!("F{n}"));
        if !letter && !function {
            return Err("主键仅支持字母、数字或 F1～F12。".into());
        }
        // 字母和数字是普通输入内容，单独作为全局键会干扰打字；仅 F1～F12 允许不加修饰键。
        if parts.len() == 1 && !function {
            return Err("字母和数字需要搭配至少一个修饰键；F1～F12 可单独使用。".into());
        }
        let mut modifiers = std::collections::HashSet::new();
        for part in &parts[..parts.len() - 1] {
            if !["Ctrl", "Alt", "Shift", "Super"].contains(part) || !modifiers.insert(*part) {
                return Err("快捷键修饰键无效或重复。".into());
            }
        }
        self.shortcut
            .parse::<Shortcut>()
            .map_err(|e| format!("快捷键格式无效：{e}"))
    }
}

/// 固定功能键独立注册；总开关关闭时释放所有实际注册成功的按键。
pub fn pin_shortcut() -> Shortcut {
    "F2".parse().expect("固定 F2 快捷键有效")
}
pub fn recording_shortcut() -> Shortcut {
    "F3".parse().expect("固定 F3 快捷键有效")
}
fn shortcut_keys(value: &Settings) -> Result<Vec<Shortcut>, String> {
    Ok(vec![
        value.validate()?,
        pin_shortcut(),
        recording_shortcut(),
    ])
}

/// Mutex 让设置保存串行执行；注册状态单独记录，以便启动注册失败后重试同一快捷键。
pub struct RuntimeSettings {
    pub value: Settings,
    pub registered: Vec<Shortcut>,
    pub shortcut_error: Option<String>,
    pub config_warning: Option<String>,
}
pub struct SettingsState(pub Mutex<RuntimeSettings>);
pub struct ShortcutMenu(pub tauri::menu::CheckMenuItem<tauri::Wry>);

/// 持锁调用，确保并发保存时托盘勾选状态与磁盘、内存提交顺序一致。
fn notify_settings(app: &AppHandle, value: &Settings) {
    if let Some(menu) = app.try_state::<ShortcutMenu>() {
        if let Err(error) = menu.0.set_checked(value.shortcuts_enabled) {
            log::error!("快捷键托盘状态更新失败：{error}");
        }
    }
    let _ = app.emit_to("main", "settings-changed", value);
}

/// 文件不存在属于首次启动；其他读取/解析错误保留原文件并向界面提示。
pub fn load(path: &Path) -> (Settings, Option<String>) {
    let result = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|value| {
                // 已保存的快捷键原样加载，默认值不用于改写已有配置。
                value.validate()?;
                Ok(value)
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => Err(e.to_string()),
    };
    match result {
        Ok(value) => (value, None),
        Err(error) => (
            Settings::default(),
            Some(format!(
                "配置读取失败，已使用默认设置；原文件保留，点击保存将替换配置：{error}"
            )),
        ),
    }
}

/// 先完整写入同目录临时文件，再替换正式文件，避免中途失败留下半份 JSON。
fn persist(path: &Path, value: &Settings) -> Result<(), String> {
    let parent = path.parent().ok_or("配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn initialize(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let path = app.path().app_config_dir()?.join("settings.json");
    let (value, config_warning) = load(&path);
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
    app.manage(SettingsState(Mutex::new(RuntimeSettings {
        value,
        registered,
        shortcut_error,
        config_warning,
    })));
    Ok(())
}

/// 按顺序更新外部资源；通过闭包传入系统操作，让失败回滚能在测试中模拟。
/// FnMut 表示闭包可修改自己的状态；生产环境操作系统，测试环境记录调用结果。
fn apply_settings(
    old: &Settings,
    next: &Settings,
    registered: &mut Vec<Shortcut>,
    mut register: impl FnMut(Shortcut) -> Result<(), String>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
    mut write: impl FnMut(&Settings) -> Result<(), String>,
) -> Result<Option<String>, String> {
    let old_keys = registered.clone();
    // 每次系统调用成功后同步实际占用，回滚失败时也不虚报注册状态。
    let actual = std::cell::RefCell::new(registered);
    let mut register = |key| {
        register(key)?;
        actual.borrow_mut().push(key);
        Ok::<_, String>(())
    };
    let mut unregister = |key| {
        unregister(key)?;
        actual.borrow_mut().retain(|registered| *registered != key);
        Ok::<_, String>(())
    };
    next.validate()?;
    let new_keys = if next.shortcuts_enabled {
        shortcut_keys(next)?
    } else {
        vec![]
    };
    let mut conflicts = Vec::new();
    let mut added = Vec::new();
    for key in new_keys.iter().filter(|key| !old_keys.contains(key)) {
        if let Err(error) = register(*key) {
            let feature = if *key == pin_shortcut() {
                "剪贴板贴图 (F2)".to_string()
            } else if *key == recording_shortcut() {
                "录屏 (F3)".to_string()
            } else {
                format!("截图 ({})", next.shortcut)
            };
            // 启动时逐项尝试，固定功能冲突不能撤销其他成功的键。
            // 修改截图键冲突时仍保留旧截图键和原设置。
            if *key != next.validate()? || !old_keys.contains(&old.validate()?) {
                conflicts.push(format!("{feature} 注册失败：{error}"));
                continue;
            }
            let cleanup: Vec<_> = added
                .iter()
                .rev()
                .filter_map(|key| unregister(*key).err())
                .collect();
            return Err(format!(
                "快捷键 {key:?} 注册失败，原设置保持不变：{error}{}",
                if cleanup.is_empty() {
                    String::new()
                } else {
                    format!("；快捷键回滚失败，请重启：{}", cleanup.join("；"))
                }
            ));
        }
        added.push(*key);
    }
    if let Err(error) = write(next) {
        let rollback: Vec<_> = added
            .iter()
            .rev()
            .filter_map(|key| unregister(*key).err())
            .collect();
        return Err(format!(
            "配置保存失败，原设置保持不变：{error}{}",
            if rollback.is_empty() {
                String::new()
            } else {
                format!("；新快捷键清理失败，请重启应用：{}", rollback.join("；"))
            }
        ));
    }
    let mut removed = Vec::new();
    for key in old_keys.iter().filter(|key| !new_keys.contains(key)) {
        if let Err(error) = unregister(*key) {
            let disk = write(old).err();
            // 多键操作失败时，恢复已经释放的旧键，并撤销本轮新增键。
            let mut failures: Vec<_> = removed
                .iter()
                .filter_map(|key| register(*key).err())
                .collect();
            failures.extend(added.iter().rev().filter_map(|key| unregister(*key).err()));
            return Err(format!(
                "旧快捷键释放失败：{error}；配置回滚：{}；按键回滚：{}",
                disk.unwrap_or_else(|| "成功".into()),
                if failures.is_empty() {
                    "成功".into()
                } else {
                    failures.join("；")
                }
            ));
        }
        removed.push(*key);
    }
    Ok((!conflicts.is_empty()).then(|| conflicts.join("；")))
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
        let mut current = state.0.lock().map_err(|e| e.to_string())?;
        if app.state::<CaptureState>().active.load(Ordering::SeqCst) {
            return Err("请先完成或取消截图，再修改设置。".into());
        }
        let path = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("settings.json");
        // 新截图键成功前保留旧键，固定功能冲突作为独立警告返回。
        let old = current.value.clone();
        let warning = apply_settings(
            &old,
            &settings,
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
            |value| persist(&path, value),
        )?;
        current.value = settings.clone();
        current.shortcut_error = warning;
        current.config_warning = None;
        app.state::<CaptureState>()
            .hotkey_pressed
            .store(0, Ordering::SeqCst);
        notify_settings(&app, &settings);
        Ok(settings)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 托盘与设置页共用总开关；None 表示在锁内切换，避免快速连点使用过期快照。
pub fn set_shortcuts(app: &AppHandle, enabled: Option<bool>) -> Result<Settings, String> {
    let state = app.state::<SettingsState>();
    let mut current = state.0.lock().map_err(|e| e.to_string())?;
    let mut next = current.value.clone();
    next.shortcuts_enabled = enabled.unwrap_or(!next.shortcuts_enabled);
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("settings.json");
    let old = current.value.clone();
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
        |value| persist(&path, value),
    );
    let warning = match result {
        Ok(warning) => warning,
        Err(error) => {
            // 原生勾选菜单会先切换外观，失败时恢复为真正生效的设置。
            notify_settings(app, &current.value);
            return Err(error);
        }
    };
    current.value = next.clone();
    current.shortcut_error = warning;
    current.config_warning = None;
    app.state::<CaptureState>()
        .hotkey_pressed
        .store(0, Ordering::SeqCst);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_config_adds_export_defaults_without_changing_shortcut_or_delay() {
        let value: Settings =
            serde_json::from_str(r#"{"shortcut":"Ctrl+F8","delaySeconds":5}"#).unwrap();
        assert_eq!(value.shortcut, "Ctrl+F8");
        assert_eq!(value.delay_seconds, 5);
        assert!(value.shortcuts_enabled);
        assert_eq!(value.export, ExportSettings::default());
        assert!(value.validate().is_ok());
    }

    #[test]
    fn export_preferences_round_trip_and_validate_untrusted_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut value = Settings::default();
        value.export = ExportSettings {
            format: ExportFormat::Webp,
            jpeg_quality: 75,
            filename_prefix: "工作截图".into(),
            include_dimensions: true,
            remember_directory: false,
        };
        persist(&path, &value).unwrap();
        assert_eq!(load(&path), (value.clone(), None));
        for prefix in [
            "",
            "../escape",
            "a\\b",
            "a:b",
            " a",
            "a ",
            ".",
            "..",
            "a\n",
            "a\u{0085}b",
        ] {
            value.export.filename_prefix = prefix.into();
            assert!(value.validate().is_err());
        }
        value.export.filename_prefix = "字".repeat(49);
        assert!(value.validate().is_err());
        value.export.filename_prefix = "字".repeat(48);
        for quality in [0, 101, 255] {
            value.export.jpeg_quality = quality;
            assert!(value.validate().is_err());
        }
        for quality in [1, 100] {
            value.export.jpeg_quality = quality;
            assert!(value.validate().is_ok());
        }
    }

    #[test]
    fn disabling_releases_key_and_disabled_edits_never_register() {
        let old = Settings::default();
        let mut disabled = old.clone();
        disabled.shortcuts_enabled = false;
        let removed = std::cell::RefCell::new(Vec::new());
        apply_settings(
            &old,
            &disabled,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| panic!("禁用不能注册快捷键"),
            |key| {
                removed.borrow_mut().push(key);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(*removed.borrow(), shortcut_keys(&old).unwrap());
        let mut changed = disabled.clone();
        changed.shortcut = "F8".into();
        apply_settings(
            &disabled,
            &changed,
            &mut Vec::new(),
            |_| panic!("禁用时编辑不注册"),
            |_| panic!("无注册键可释放"),
            |_| Ok(()),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        persist(&path, &changed).unwrap();
        assert_eq!(load(&path), (changed, None));
    }

    #[test]
    fn toggle_failures_keep_previous_state() {
        let enabled = Settings::default();
        let disabled = Settings {
            shortcuts_enabled: false,
            ..enabled.clone()
        };
        apply_settings(
            &enabled,
            &disabled,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| panic!("不注册"),
            |_| panic!("保存失败不能释放原键"),
            |_| Err("磁盘只读".into()),
        )
        .unwrap_err();
        let warning = apply_settings(
            &disabled,
            &enabled,
            &mut Vec::new(),
            |_| Err("快捷键冲突".into()),
            |_| panic!("无旧键"),
            |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert!(warning.contains("截图 (F1)"));
        assert!(warning.contains("剪贴板贴图 (F2)"));
        assert!(warning.contains("录屏 (F3)"));
        let writes = std::cell::RefCell::new(Vec::new());
        apply_settings(
            &enabled,
            &disabled,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| panic!("不注册"),
            |_| Err("释放失败".into()),
            |value| {
                writes.borrow_mut().push(value.clone());
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(*writes.borrow(), vec![disabled, enabled]);
    }

    #[test]
    fn preserves_saved_shortcuts_and_delay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for key in ["Alt+Shift+S", "Alt+F1", "Alt+F4", "Alt+F12", "Ctrl+F8"] {
            let old = Settings {
                shortcut: key.into(),
                delay_seconds: 5,
                ..Settings::default()
            };
            persist(&path, &old).unwrap();
            let bytes = fs::read(&path).unwrap();
            let (loaded, warning) = load(&path);
            assert!(warning.is_none());
            assert_eq!(loaded.shortcut, key);
            assert_eq!(loaded.delay_seconds, 5);
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        for n in 1..=12 {
            assert!(Settings {
                shortcut: format!("Alt+F{n}"),
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
    }
    #[test]
    fn shortcut_conflict_leaves_old_key_and_disk_untouched() {
        let next = Settings {
            shortcut: "Ctrl+F8".into(),
            ..Settings::default()
        };
        assert!(apply_settings(
            &Settings::default(),
            &next,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| Err("occupied".into()),
            |_| panic!("不应释放旧键"),
            |_| panic!("不应修改文件")
        )
        .is_err());
    }
    #[test]
    fn write_failure_removes_only_new_shortcut() {
        let next = Settings {
            shortcut: "Ctrl+F8".into(),
            ..Settings::default()
        };
        let removed = std::cell::RefCell::new(Vec::new());
        assert!(apply_settings(
            &Settings::default(),
            &next,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| Ok(()),
            |key| {
                removed.borrow_mut().push(key);
                Ok(())
            },
            |_| Err("read-only".into())
        )
        .is_err());
        assert_eq!(*removed.borrow(), vec![next.validate().unwrap()]);
    }
    #[test]
    fn unregister_failure_restores_disk_and_removes_new_key() {
        let old = Settings::default();
        let next = Settings {
            shortcut: "Ctrl+F8".into(),
            ..Settings::default()
        };
        let written = std::cell::RefCell::new(Vec::new());
        let removed = std::cell::RefCell::new(Vec::new());
        assert!(apply_settings(
            &old,
            &next,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| Ok(()),
            |key| {
                removed.borrow_mut().push(key);
                if key == old.validate().unwrap() {
                    Err("system error".into())
                } else {
                    Ok(())
                }
            },
            |value| {
                written.borrow_mut().push(value.clone());
                Ok(())
            }
        )
        .is_err());
        assert_eq!(*written.borrow(), vec![next.clone(), old.clone()]);
        assert_eq!(
            *removed.borrow(),
            vec![old.validate().unwrap(), next.validate().unwrap()]
        );
    }
    #[test]
    fn delay_only_does_not_reregister_but_failed_startup_can_retry() {
        let value = Settings::default();
        apply_settings(
            &value,
            &value,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |_| panic!("无需重新注册"),
            |_| panic!("无需释放"),
            |_| Ok(()),
        )
        .unwrap();
        let registered = std::cell::Cell::new(false);
        apply_settings(
            &value,
            &value,
            &mut Vec::new(),
            |_| {
                registered.set(true);
                Ok(())
            },
            |_| panic!("没有旧键"),
            |_| Ok(()),
        )
        .unwrap();
        assert!(registered.get());
    }

    #[test]
    fn enabling_registers_capture_pin_and_recording() {
        let value = Settings::default();
        let keys = std::cell::RefCell::new(Vec::new());
        apply_settings(
            &value,
            &value,
            &mut Vec::new(),
            |key| {
                keys.borrow_mut().push(key);
                Ok(())
            },
            |_| panic!("启用成功不释放按键"),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(*keys.borrow(), shortcut_keys(&value).unwrap());
    }

    #[test]
    fn conflicts_are_independent_and_only_failed_keys_are_retried() {
        let value = Settings::default();
        for conflict in shortcut_keys(&value).unwrap() {
            let mut registered = Vec::new();
            let warning = apply_settings(
                &value,
                &value,
                &mut registered,
                |key| {
                    if key == conflict {
                        Err("occupied".into())
                    } else {
                        Ok(())
                    }
                },
                |_| panic!("冲突不能释放其他功能"),
                |_| Ok(()),
            )
            .unwrap();
            assert!(warning.is_some());
            assert_eq!(registered.len(), 2);
            assert!(!registered.contains(&conflict));
            assert!(apply_settings(
                &value,
                &value,
                &mut registered,
                |key| {
                    assert_eq!(key, conflict);
                    Ok(())
                },
                |_| panic!("重试不能释放成功键"),
                |_| Ok(()),
            )
            .unwrap()
            .is_none());
            assert_eq!(registered.len(), 3);
        }
    }

    #[test]
    fn disabling_after_conflict_releases_only_registered_keys() {
        let old = Settings::default();
        let next = Settings {
            shortcuts_enabled: false,
            ..old.clone()
        };
        let capture = old.validate().unwrap();
        let mut registered = vec![capture];
        assert!(apply_settings(
            &old,
            &next,
            &mut registered,
            |_| panic!("关闭不能注册"),
            |key| {
                assert_eq!(key, capture);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap()
        .is_none());
        assert!(registered.is_empty());
    }

    #[test]
    fn disabling_failure_restores_previously_released_keys() {
        let old = Settings::default();
        let next = Settings {
            shortcuts_enabled: false,
            ..old.clone()
        };
        let restored = std::cell::RefCell::new(Vec::new());
        let writes = std::cell::RefCell::new(Vec::new());
        assert!(apply_settings(
            &old,
            &next,
            &mut shortcut_keys(&Settings::default()).unwrap(),
            |key| {
                restored.borrow_mut().push(key);
                Ok(())
            },
            |key| if key == recording_shortcut() {
                Err("释放失败".into())
            } else {
                Ok(())
            },
            |value| {
                writes.borrow_mut().push(value.clone());
                Ok(())
            },
        )
        .is_err());
        assert_eq!(
            *restored.borrow(),
            vec![old.validate().unwrap(), pin_shortcut()]
        );
        assert_eq!(*writes.borrow(), vec![next, old]);
    }
    #[test]
    fn missing_corrupt_and_invalid_files_fall_back_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(load(&path), (Settings::default(), None));
        for contents in [
            "broken",
            r#"{"shortcut":"S","delaySeconds":0}"#,
            r#"{"shortcut":"Alt+S","delaySeconds":2}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(load(&path).1.is_some());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }
    #[test]
    fn saved_settings_survive_reload_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        persist(&path, &Settings::default()).unwrap();
        let value = Settings {
            shortcut: "Ctrl+Shift+F8".into(),
            delay_seconds: 5,
            ..Settings::default()
        };
        persist(&path, &value).unwrap();
        assert_eq!(load(&path), (value, None));
    }
    #[test]
    fn validates_modifiers_keys_and_delay() {
        for shortcut in [
            "Ctrl+Ctrl+S",
            "S",
            "Alt+Escape",
            "Alt+F13",
            "Shift+",
            "Alt+SS",
        ] {
            assert!(Settings {
                shortcut: shortcut.into(),
                ..Settings::default()
            }
            .validate()
            .is_err());
        }
        for shortcut in ["Alt+Shift+S", "Super+1", "Ctrl+F12", "F1", "F12"] {
            assert!(Settings {
                shortcut: shortcut.into(),
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
    }

    #[test]
    fn function_keys_are_valid_without_modifiers() {
        // F1～F12 可单独作为全局快捷键，这也是新的默认值 F1 的使用场景。
        for n in (1..=12).filter(|n| ![2, 3].contains(n)) {
            assert!(Settings {
                shortcut: format!("F{n}"),
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
        // F13 及字母、数字仍然需要修饰键。
        for shortcut in ["F2", "F3", "F13", "A", "1"] {
            assert!(Settings {
                shortcut: shortcut.into(),
                ..Settings::default()
            }
            .validate()
            .is_err());
        }
    }
}
