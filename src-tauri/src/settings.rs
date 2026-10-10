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
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub shortcut: String,
    pub delay_seconds: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "Alt+F1".into(),
            delay_seconds: 0,
        }
    }
}

impl Settings {
    /// 与前端录入规则一致：至少一个修饰键，搭配字母、数字或 F1～F12。
    /// 后端必须再次校验，不能假设所有 IPC 请求都来自正常表单。
    pub fn validate(&self) -> Result<Shortcut, String> {
        if ![0, 3, 5].contains(&self.delay_seconds) {
            return Err("截图延时只能为 0、3 或 5 秒。".into());
        }
        let parts: Vec<_> = self.shortcut.split('+').collect();
        if parts.len() < 2 || parts.len() > 5 {
            return Err("快捷键需要修饰键和一个字母、数字或 F1～F12。".into());
        }
        let mut modifiers = std::collections::HashSet::new();
        for part in &parts[..parts.len() - 1] {
            if !["Ctrl", "Alt", "Shift", "Super"].contains(part) || !modifiers.insert(*part) {
                return Err("快捷键修饰键无效或重复。".into());
            }
        }
        let key = parts[parts.len() - 1];
        let letter = key.len() == 1
            && key
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        let function = (1..=12).any(|n| key == format!("F{n}"));
        if !letter && !function {
            return Err("主键仅支持字母、数字或 F1～F12。".into());
        }
        self.shortcut
            .parse::<Shortcut>()
            .map_err(|e| format!("快捷键格式无效：{e}"))
    }
}

/// Mutex 让设置保存串行执行；注册状态单独记录，以便启动注册失败后重试同一快捷键。
pub struct RuntimeSettings {
    pub value: Settings,
    pub registered: bool,
    pub shortcut_error: Option<String>,
    pub config_warning: Option<String>,
}
pub struct SettingsState(pub Mutex<RuntimeSettings>);

/// 文件不存在属于首次启动；其他读取/解析错误保留原文件并向界面提示。
pub fn load(path: &Path) -> (Settings, Option<String>) {
    let result = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|mut value| {
                // 旧版本未记录快捷键来源，只迁移旧默认组合；其他自定义组合不变。
                // 此处只修改内存，用户下次保存设置时才写入磁盘。
                if value.shortcut == "Alt+Shift+S" {
                    value.shortcut = Settings::default().shortcut;
                }
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
    let shortcut_error = app
        .global_shortcut()
        .register(value.shortcut.as_str())
        .err()
        .map(|e| {
            format!(
                "快捷键 {} 注册失败，可使用按钮或托盘，或在设置中更换：{e}",
                value.shortcut
            )
        });
    app.manage(SettingsState(Mutex::new(RuntimeSettings {
        value,
        registered: shortcut_error.is_none(),
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
    registered: bool,
    mut register: impl FnMut(Shortcut) -> Result<(), String>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
    mut write: impl FnMut(&Settings) -> Result<(), String>,
) -> Result<(), String> {
    let old_key = old.validate()?;
    let new_key = next.validate()?;
    let replace = old_key != new_key || !registered;
    if replace {
        register(new_key).map_err(|e| format!("快捷键注册失败，原设置保持不变：{e}"))?;
    }
    if let Err(error) = write(next) {
        let rollback = if replace {
            unregister(new_key).err()
        } else {
            None
        };
        return Err(format!(
            "配置保存失败，原设置保持不变：{error}{}",
            rollback
                .map(|e| format!("；新快捷键清理失败，请重启应用：{e}"))
                .unwrap_or_default()
        ));
    }
    if replace && registered {
        if let Err(error) = unregister(old_key) {
            let disk = write(old).err();
            let key = unregister(new_key).err();
            return Err(format!(
                "旧快捷键释放失败：{error}；配置回滚：{}；新键清理：{}",
                disk.unwrap_or_else(|| "成功".into()),
                key.unwrap_or_else(|| "成功".into())
            ));
        }
    }
    Ok(())
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
        // 新键注册成功前不释放旧键；所有步骤成功后才更新内存中的已生效设置。
        apply_settings(
            &current.value,
            &settings,
            current.registered,
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
        current.registered = true;
        current.shortcut_error = None;
        current.config_warning = None;
        Ok(settings)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_legacy_default_but_keeps_custom_keys_and_delay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for key in ["Alt+Shift+S", "Alt+F12", "Ctrl+F8"] {
            let old = Settings {
                shortcut: key.into(),
                delay_seconds: 5,
            };
            persist(&path, &old).unwrap();
            let bytes = fs::read(&path).unwrap();
            let (loaded, warning) = load(&path);
            assert!(warning.is_none());
            assert_eq!(
                loaded.shortcut,
                if key == "Alt+Shift+S" { "Alt+F1" } else { key }
            );
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
            true,
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
            true,
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
            true,
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
            true,
            |_| panic!("无需重新注册"),
            |_| panic!("无需释放"),
            |_| Ok(()),
        )
        .unwrap();
        let registered = std::cell::Cell::new(false);
        apply_settings(
            &value,
            &value,
            false,
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
        for shortcut in ["Alt+Shift+S", "Super+1", "Ctrl+F12"] {
            assert!(Settings {
                shortcut: shortcut.into(),
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
    }
}
