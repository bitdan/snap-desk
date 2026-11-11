//! 设置的唯一来源：启动时读取 JSON，保存时同时更新文件和系统快捷键。
//! 前端只传明确的设置字段，不能指定文件路径。损坏的配置不会在启动时被覆盖。
pub(crate) mod autostart;
pub(crate) mod shortcut_probe;
pub(crate) mod storage;
pub(crate) mod backup;
mod writer;

pub(crate) use writer::SettingsWriter;
#[cfg(test)]
pub(crate) use writer::persist;

use crate::capture::{require_window, CaptureState};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::{atomic::Ordering, Mutex},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

/// 普通截图和贴图共用的导出偏好；滚动截图的自动 PNG 存档保持独立。
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

/// 录屏文件命名与目录记忆独立于图片，格式和画质仍由录制配置决定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct RecordingExportSettings {
    pub filename_prefix: String,
    pub include_dimensions: bool,
    pub remember_directory: bool,
}
impl Default for RecordingExportSettings {
    fn default() -> Self {
        Self {
            filename_prefix: "SnapDesk".into(),
            include_dimensions: false,
            remember_directory: true,
        }
    }
}
impl RecordingExportSettings {
    pub fn validate(&self) -> Result<(), String> {
        ExportSettings {
            filename_prefix: self.filename_prefix.clone(),
            ..ExportSettings::default()
        }
        .validate()
    }
}

/// 按动作分发，不再根据固定 F2/F3 推断功能。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    Capture,
    Pin,
    Record,
    History,
    Translate,
    Shelf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "ShortcutsWire")]
pub struct Shortcuts {
    pub capture: String,
    pub pin: String,
    pub record: String,
    pub history: String,
    pub translate: String,
    pub shelf: String,
}
/// 区分缺失字段与显式禁用；兼容旧翻译映射，保留已自定义的快捷键。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct ShortcutsWire {
    capture: String,
    pin: String,
    record: String,
    history: Option<String>,
    translate: Option<String>,
    shelf: String,
    // 只读取旧版本的两个翻译映射，保存时统一输出 translate。
    translate_selection: Option<String>,
    translate_clipboard: Option<String>,
}
impl Default for ShortcutsWire {
    fn default() -> Self {
        let keys = Shortcuts::default();
        Self {
            capture: keys.capture,
            pin: keys.pin,
            record: keys.record,
            history: None,
            translate: None,
            shelf: keys.shelf,
            translate_selection: None,
            translate_clipboard: None,
        }
    }
}
impl From<ShortcutsWire> for Shortcuts {
    fn from(wire: ShortcutsWire) -> Self {
        let missing = wire.history.is_none();
        let translation_missing = wire.translate.is_none();
        // 旧默认组合升级为 Alt+T；已自定义或禁用的翻译键继续保留。
        let translate = wire.translate
            .or_else(|| wire.translate_selection.filter(|value| value != "Ctrl+Alt+T"))
            .or_else(|| wire.translate_clipboard.filter(|value| value != "Ctrl+Alt+Y"))
            .unwrap_or_else(|| Self::default().translate);
        let mut keys = Self {
            capture: wire.capture,
            pin: wire.pin,
            record: wire.record,
            history: wire.history.unwrap_or_else(|| Self::default().history),
            translate,
            shelf: wire.shelf,
        };
        if missing {
            keys.preserve_legacy_mapping();
        }
        // 旧配置已有相同组合时保留原动作，只禁用新加入的默认快捷键。
        if translation_missing { keys.preserve_translation_mapping(); }
        keys
    }
}
impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            capture: "F1".into(),
            pin: "F2".into(),
            record: "F3".into(),
            history: "F4".into(),
            translate: "Alt+T".into(),
            shelf: String::new(),
        }
    }
}
impl Shortcuts {
    fn preserve_legacy_mapping(&mut self) {
        let history = parse_shortcut(&self.history).ok().flatten();
        if history.is_some()
            && [&self.capture, &self.pin, &self.record]
                .iter()
                .any(|key| parse_shortcut(key).ok().flatten() == history)
        {
            self.history.clear();
        }
    }
    fn preserve_translation_mapping(&mut self) {
        let translate = parse_shortcut(&self.translate).ok().flatten();
        if translate.is_some() && [&self.capture, &self.pin, &self.record, &self.history]
            .iter().any(|key| parse_shortcut(key).ok().flatten() == translate)
        { self.translate.clear(); }
    }
    pub fn entries(&self) -> [(ShortcutAction, &str, &str); 6] {
        [
            (ShortcutAction::Capture, "区域截图", &self.capture),
            (ShortcutAction::Pin, "剪贴板贴图", &self.pin),
            (ShortcutAction::Record, "开始 / 停止录屏", &self.record),
            (ShortcutAction::History, "剪贴板历史", &self.history),
            (ShortcutAction::Translate, "翻译", &self.translate),
            (ShortcutAction::Shelf, "展开 / 收起暂存架", &self.shelf),
        ]
    }
}

/// 外观配置拒绝未知值；旧配置缺少字段时自动使用默认值。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    English,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Appearance {
    pub theme: Theme,
    pub language: Language,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "SettingsWire")]
pub struct Settings {
    /// 默认只接收正式版；开启后也接收 alpha、beta 和 rc 应用更新。
    pub receive_prerelease_updates: bool,
    pub appearance: Appearance,
    pub recognition_services: Vec<crate::recognition::services::RecognitionService>,
    pub ai: crate::recognition::translation::AiSettings,
    pub ocr: crate::recognition::ocr_config::OcrSettings,
    pub shortcuts: Shortcuts,
    pub shortcuts_enabled: bool,
    pub delay_seconds: u8,
    /// 普通截图包含系统光标，与录屏光标设置独立。
    pub screenshot_cursor: bool,
    pub screenshot_snap: bool,
    /// 框选完成后的动作：none 显示工具栏，copy / pin / save 直接执行并结束截图。
    pub screenshot_after: String,
    pub export: ExportSettings,
    pub recording_export: RecordingExportSettings,
    pub pin_appearance: crate::pins::PinAppearance,
    pub clipboard: crate::clipboard::model::ClipboardSettings,
    /// 只在首次或引导大版本变化时展示，与应用版本号无关。
    pub onboarding_version: u32,
}

/// 显式接收旧字段，继续拒绝未知字段和新旧格式混用。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsWire {
    #[serde(default)]
    receive_prerelease_updates: bool,
    #[serde(default)]
    appearance: Appearance,
    #[serde(default)]
    recognition_services: Vec<crate::recognition::services::RecognitionService>,
    #[serde(default)]
    ai: crate::recognition::translation::AiSettings,
    #[serde(default)]
    ocr: crate::recognition::ocr_config::OcrSettings,
    shortcut: Option<String>,
    shortcuts: Option<Shortcuts>,
    #[serde(default = "enabled_by_default")]
    shortcuts_enabled: bool,
    #[serde(default)]
    delay_seconds: u8,
    #[serde(default)]
    screenshot_cursor: bool,
    #[serde(default = "enabled_by_default")]
    screenshot_snap: bool,
    #[serde(default = "no_after_action")]
    screenshot_after: String,
    #[serde(default)]
    export: ExportSettings,
    #[serde(default)]
    recording_export: RecordingExportSettings,
    #[serde(default)]
    pin_appearance: crate::pins::PinAppearance,
    #[serde(default)]
    clipboard: crate::clipboard::model::ClipboardSettings,
    #[serde(default)]
    onboarding_version: u32,
}
impl TryFrom<SettingsWire> for Settings {
    type Error = String;
    fn try_from(wire: SettingsWire) -> Result<Self, String> {
        if wire.shortcut.is_some() && wire.shortcuts.is_some() {
            return Err("不能同时指定 shortcut 和 shortcuts。".into());
        }
        let mut shortcuts = wire.shortcuts.unwrap_or_default();
        if let Some(capture) = wire.shortcut {
            shortcuts.capture = capture;
            shortcuts.preserve_legacy_mapping();
            shortcuts.preserve_translation_mapping();
        }
        Ok(Self {
            receive_prerelease_updates: wire.receive_prerelease_updates,
            shortcuts,
            shortcuts_enabled: wire.shortcuts_enabled,
            delay_seconds: wire.delay_seconds,
            screenshot_cursor: wire.screenshot_cursor,
            screenshot_snap: wire.screenshot_snap,
            screenshot_after: wire.screenshot_after,
            export: wire.export,
            recording_export: wire.recording_export,
            pin_appearance: wire.pin_appearance,
            clipboard: wire.clipboard,
            onboarding_version: wire.onboarding_version,
            recognition_services: wire.recognition_services,
            ai: wire.ai,
            ocr: wire.ocr,
            appearance: wire.appearance,
        })
    }
}
fn enabled_by_default() -> bool {
    true
}
fn no_after_action() -> String {
    "none".into()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            receive_prerelease_updates: false,
            shortcuts: Shortcuts::default(),
            shortcuts_enabled: true,
            delay_seconds: 0,
            screenshot_cursor: false,
            screenshot_snap: true,
            screenshot_after: no_after_action(),
            export: ExportSettings::default(),
            recording_export: RecordingExportSettings::default(),
            pin_appearance: crate::pins::PinAppearance::default(),
            clipboard: crate::clipboard::model::ClipboardSettings::default(),
            onboarding_version: 0,
            recognition_services: Vec::new(),
            ai: crate::recognition::translation::AiSettings::default(),
            ocr: crate::recognition::ocr_config::OcrSettings::default(),
            appearance: Appearance::default(),
        }
    }
}

/// 空字符串禁用单个动作，其余输入必须校验。
fn parse_shortcut(value: &str) -> Result<Option<Shortcut>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let parts: Vec<_> = value.split('+').collect();
    if parts.len() > 5 {
        return Err("快捷键组合过长。".into());
    }
    // 按系统库支持的主键解析，不额外限制为 F1～F12，也不强制字母数字带修饰键。
    let mut modifiers = std::collections::HashSet::new();
    for part in &parts[..parts.len() - 1] {
        if !["Ctrl", "Alt", "Shift", "Super"].contains(part) || !modifiers.insert(*part) {
            return Err("快捷键修饰键无效或重复。".into());
        }
    }
    value
        .parse::<Shortcut>()
        .map(Some)
        .map_err(|e| format!("快捷键格式无效：{e}"))
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        crate::recognition::services::validate(self)?;
        self.ai.validate()?;
        self.ocr.validate()?;
        self.export.validate()?;
        self.recording_export.validate()?;
        self.pin_appearance.validate()?;
        self.clipboard.validate()?;
        if !["none", "copy", "pin", "save"].contains(&self.screenshot_after.as_str()) {
            return Err("截图后动作无效。".into());
        }
        if ![0, 3, 5].contains(&self.delay_seconds) {
            return Err("截图延时只能为 0、3 或 5 秒。".into());
        }
        let mut keys = Vec::new();
        for (_, name, value) in self.shortcuts.entries() {
            if let Some(key) = parse_shortcut(value).map_err(|e| format!("{name}：{e}"))? {
                if let Some((_, other)) = keys.iter().find(|(existing, _)| *existing == key) {
                    return Err(format!("{name}与{other}的快捷键冲突。"));
                }
                keys.push((key, name));
            }
        }
        Ok(())
    }
    pub fn action_for(&self, key: &Shortcut) -> Option<ShortcutAction> {
        self.shortcuts
            .entries()
            .into_iter()
            .find_map(|(action, _, value)| {
                (parse_shortcut(value).ok().flatten().as_ref() == Some(key)).then_some(action)
            })
    }
}
fn shortcut_keys(value: &Settings) -> Result<Vec<Shortcut>, String> {
    value.validate()?;
    value
        .shortcuts
        .entries()
        .into_iter()
        .filter_map(|(_, _, key)| parse_shortcut(key).transpose())
        .collect()
}

/// Mutex 让设置保存串行执行；注册状态单独记录，以便启动注册失败后重试同一快捷键。
pub struct RuntimeSettings {
    pub value: Settings,
    pub registered: Vec<Shortcut>,
    pub editing: bool,
    pub shortcut_error: Option<String>,
    pub config_warning: Option<String>,
}
/// 第一项只保护内存快照；第二项串行化快捷键事务并把磁盘写入交给独立线程。
pub struct SettingsState(pub Mutex<RuntimeSettings>, pub(crate) SettingsWriter);
/// 松键事件独立于配置锁处理，避免保存时丢掉松键后一直被视为长按。
#[derive(Default)]
pub struct ShortcutPressState(pub Mutex<Vec<Shortcut>>);
pub struct ShortcutMenu(pub tauri::menu::CheckMenuItem<tauri::Wry>);

/// 持锁调用，确保并发保存时托盘勾选状态与磁盘、内存提交顺序一致。
fn notify_settings(app: &AppHandle, value: &Settings) {
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
    let mut added = Vec::new();
    for key in new_keys.iter().filter(|key| !old_keys.contains(key)) {
        if let Err(error) = register(*key) {
            // 任意动作注册失败都撤销新增键，保留原配置。
            let feature = next
                .shortcuts
                .entries()
                .into_iter()
                .find(|(_, _, value)| parse_shortcut(value).ok().flatten() == Some(*key))
                .map(|(_, name, value)| format!("{name} ({value})"))
                .unwrap_or_default();
            let cleanup: Vec<_> = added
                .iter()
                .rev()
                .filter_map(|key| unregister(*key).err())
                .collect();
            return Err(format!(
                "{feature} 注册失败，原设置保持不变：{error}{}",
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
    Ok(None)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_legacy_defaults_roundtrip_and_rejects_unknown_values() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.appearance, Appearance::default());
        let configured: Settings =
            serde_json::from_str(r#"{"appearance":{"theme":"dark","language":"en"}}"#).unwrap();
        assert_eq!(configured.appearance.theme, Theme::Dark);
        assert_eq!(configured.appearance.language, Language::English);
        let restored: Settings =
            serde_json::from_value(serde_json::to_value(&configured).unwrap()).unwrap();
        assert_eq!(restored.appearance, configured.appearance);
        for invalid in [
            r#"{"appearance":{"theme":"invalid"}}"#,
            r#"{"appearance":{"language":"invalid"}}"#,
            r#"{"appearance":{"extra":true}}"#,
        ] {
            assert!(serde_json::from_str::<Settings>(invalid).is_err());
        }
    }
    #[test]
    fn maps_actions_clears_keys_and_migrates_strictly() {
        let mut value = Settings::default();
        value.shortcuts.capture = "".into();
        value.shortcuts.pin = "F1".into();
        value.shortcuts.record = "Ctrl+Shift+S".into();
        assert!(value.validate().is_ok());
        assert_eq!(shortcut_keys(&value).unwrap().len(), 4);
        assert_eq!(
            value.action_for(&"F1".parse().unwrap()),
            Some(ShortcutAction::Pin)
        );
        value.shortcuts.capture = "Shift+Ctrl+S".into();
        assert!(value.validate().unwrap_err().contains("录屏"));
        for json in [
            r#"{"shortcut":"F1","shortcuts":{}}"#,
            r#"{"unexpected":1}"#,
            r#"{"shortcuts":{"unexpected":"F8"}}"#,
        ] {
            assert!(serde_json::from_str::<Settings>(json).is_err());
        }
        let old: Settings =
            serde_json::from_str(r#"{"shortcut":"Ctrl+S","delaySeconds":3}"#).unwrap();
        assert_eq!(old.shortcuts.pin, "F2");
        assert_eq!(old.shortcuts.record, "F3");
        assert_eq!(old.shortcuts.history, "F4");
        assert!(old.clipboard.enabled);
        assert!(!old.receive_prerelease_updates);
        for json in [
            r#"{"shortcut":"F4"}"#,
            r#"{"shortcuts":{"capture":"F4","pin":"F2","record":"F3"}}"#,
        ] {
            let migrated: Settings = serde_json::from_str(json).unwrap();
            assert!(migrated.shortcuts.history.is_empty());
            assert!(migrated.validate().is_ok());
            assert_eq!(migrated.shortcuts.capture, "F4");
        }
        assert!(!serde_json::to_value(old)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("shortcut"));
    }

    #[test]
    fn translation_shortcuts_migrate_without_replacing_existing_actions() {
        for json in [
            r#"{"shortcut":"Alt+T"}"#,
            r#"{"shortcuts":{"capture":"Alt+T"}}"#,
        ] {
            let settings: Settings = serde_json::from_str(json).unwrap();
            assert!(settings.shortcuts.translate.is_empty());
            assert!(settings.validate().is_ok());
        }
        let explicit: Settings = serde_json::from_str(
            r#"{"shortcuts":{"capture":"Alt+T","translate":"Alt+T"}}"#,
        ).unwrap();
        assert!(explicit.validate().is_err());
        let old: Settings = serde_json::from_str(
            r#"{"shortcuts":{"translateSelection":"Ctrl+Alt+T","translateClipboard":"Ctrl+Alt+Y"},"ai":{"selectionContextMenu":true}}"#,
        ).unwrap();
        assert_eq!(old.shortcuts.translate, "Alt+T");
        let old_saved = serde_json::to_value(old).unwrap();
        assert!(old_saved["shortcuts"].get("translateSelection").is_none());
        assert!(old_saved["shortcuts"].get("translateClipboard").is_none());
        assert!(old_saved["ai"].get("selectionContextMenu").is_none());
        for (legacy, expected) in [
            (r#"{"translateSelection":"Alt+R"}"#, "Alt+R"),
            (r#"{"translateClipboard":"Alt+Y"}"#, "Alt+Y"),
            (r#"{"translateSelection":""}"#, ""),
            (r#"{"translate":"Alt+G","translateSelection":"Alt+R"}"#, "Alt+G"),
        ] {
            let keys: Shortcuts = serde_json::from_str(legacy).unwrap();
            assert_eq!(keys.translate, expected);
        }
        let defaults = serde_json::to_value(Shortcuts::default()).unwrap();
        assert_eq!(defaults["translate"], "Alt+T");
    }

    #[test]
    fn pin_and_record_conflicts_roll_back_all_additions() {
        let old = Settings::default();
        for conflict in ["F5", "F6", "F7"] {
            let next = Settings {
                shortcuts: Shortcuts {
                    capture: "F8".into(),
                    pin: "F5".into(),
                    record: "F6".into(),
                    history: "F7".into(),
                    ..Shortcuts::default()
                },
                ..old.clone()
            };
            let mut registered = shortcut_keys(&old).unwrap();
            assert!(apply_settings(
                &old,
                &next,
                &mut registered,
                |key| if key == conflict.parse::<Shortcut>().unwrap() {
                    Err("occupied".into())
                } else {
                    Ok(())
                },
                |_| Ok(()),
                |_| panic!("冲突不能写配置")
            )
            .is_err());
            assert_eq!(registered, shortcut_keys(&old).unwrap());
        }
    }

    #[test]
    fn legacy_config_adds_export_defaults_without_changing_shortcut_or_delay() {
        let value: Settings =
            serde_json::from_str(r#"{"shortcut":"Ctrl+F8","delaySeconds":5}"#).unwrap();
        assert_eq!(value.shortcuts.capture, "Ctrl+F8");
        assert_eq!(value.delay_seconds, 5);
        assert!(!value.screenshot_cursor);
        assert!(value.screenshot_snap);
        assert!(value.shortcuts_enabled);
        assert_eq!(value.ocr, crate::recognition::ocr_config::OcrSettings::default());
        assert_eq!(value.export, ExportSettings::default());
        assert_eq!(
            value.ai,
            crate::recognition::translation::AiSettings::default()
        );
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
        changed.shortcuts.capture = "F8".into();
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
        assert!(apply_settings(
            &disabled,
            &enabled,
            &mut Vec::new(),
            |_| Err("快捷键冲突".into()),
            |_| Ok(()),
            |_| panic!("冲突不能保存")
        )
        .is_err());
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
                shortcuts: Shortcuts {
                    capture: key.into(),
                    ..Shortcuts::default()
                },
                delay_seconds: 5,
                ..Settings::default()
            };
            persist(&path, &old).unwrap();
            let bytes = fs::read(&path).unwrap();
            let (loaded, warning) = load(&path);
            assert!(warning.is_none());
            assert_eq!(loaded.shortcuts.capture, key);
            assert_eq!(loaded.delay_seconds, 5);
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        for n in 1..=12 {
            assert!(Settings {
                shortcuts: Shortcuts {
                    capture: format!("Alt+F{n}"),
                    ..Shortcuts::default()
                },
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
    }
    #[test]
    fn shortcut_conflict_leaves_old_key_and_disk_untouched() {
        let next = Settings {
            shortcuts: Shortcuts {
                capture: "Ctrl+F8".into(),
                ..Shortcuts::default()
            },
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
            shortcuts: Shortcuts {
                capture: "Ctrl+F8".into(),
                ..Shortcuts::default()
            },
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
        assert_eq!(
            *removed.borrow(),
            vec![parse_shortcut(&next.shortcuts.capture).unwrap().unwrap()]
        );
    }
    #[test]
    fn unregister_failure_restores_disk_and_removes_new_key() {
        let old = Settings::default();
        let next = Settings {
            shortcuts: Shortcuts {
                capture: "Ctrl+F8".into(),
                ..Shortcuts::default()
            },
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
                if key == parse_shortcut(&old.shortcuts.capture).unwrap().unwrap() {
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
            vec![
                parse_shortcut(&old.shortcuts.capture).unwrap().unwrap(),
                parse_shortcut(&next.shortcuts.capture).unwrap().unwrap()
            ]
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
    fn disabling_after_conflict_releases_only_registered_keys() {
        let old = Settings::default();
        let next = Settings {
            shortcuts_enabled: false,
            ..old.clone()
        };
        let capture = parse_shortcut(&old.shortcuts.capture).unwrap().unwrap();
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
            |key| if key == "F3".parse::<Shortcut>().unwrap() {
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
            vec![
                parse_shortcut(&old.shortcuts.capture).unwrap().unwrap(),
                "F2".parse::<Shortcut>().unwrap()
            ]
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
            // 新旧格式混用由 SettingsWire 明确拒绝；单独旧的 shortcut 与 delaySeconds 仍属合法配置。
            r#"{"shortcut":"S","shortcuts":{}}"#,
            r#"{"shortcut":"S","delaySeconds":2}"#,
            r#"{"unknownField":1}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(load(&path).1.is_some(), "无效配置应回退并给出提示：{contents}");
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }
    #[test]
    fn saved_settings_survive_reload_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        persist(&path, &Settings::default()).unwrap();
        let value = Settings {
            shortcuts: Shortcuts {
                capture: "Ctrl+Shift+F8".into(),
                ..Shortcuts::default()
            },
            delay_seconds: 5,
            ..Settings::default()
        };
        persist(&path, &value).unwrap();
        assert_eq!(load(&path), (value, None));
    }
    #[test]
    fn validates_modifiers_keys_and_delay() {
        for shortcut in ["Ctrl+Ctrl+S", "Shift+", "Alt+SS", "Ctrl+A+B", "Ctrl", "F25"] {
            assert!(Settings {
                shortcuts: Shortcuts {
                    capture: shortcut.into(),
                    ..Shortcuts::default()
                },
                ..Settings::default()
            }
            .validate()
            .is_err());
        }
        for shortcut in [
            "Alt+Shift+S",
            "Super+1",
            "Ctrl+F12",
            "F1",
            "F24",
            "A",
            "1",
            "Enter",
            "Space",
            "Ctrl+ArrowLeft",
            "Alt+Escape",
            "Ctrl+Delete",
            "Shift+Tab",
            "Ctrl+Shift+Equal",
            "Alt+NumpadAdd",
            "MediaPlayPause",
        ] {
            assert!(Settings {
                shortcuts: Shortcuts {
                    capture: shortcut.into(),
                    ..Shortcuts::default()
                },
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
    }

    #[test]
    fn function_keys_are_valid_without_modifiers() {
        // F1～F24 均可无修饰键；动作之间的占用冲突另行校验。
        for n in 1..=24 {
            assert!(parse_shortcut(&format!("F{n}")).unwrap().is_some());
        }
        // 默认 F2/F3/F4 已分别被贴图、录屏和历史占用，不能重复用作截图键。
        for n in (1..=24).filter(|n| ![2, 3, 4].contains(n)) {
            assert!(Settings {
                shortcuts: Shortcuts {
                    capture: format!("F{n}"),
                    ..Shortcuts::default()
                },
                ..Settings::default()
            }
            .validate()
            .is_ok());
        }
        // 重复动作映射及不支持的主键不能通过完整配置校验。
        for shortcut in ["F2", "F3", "F4", "F25", "Ctrl+A+B"] {
            assert!(Settings {
                shortcuts: Shortcuts {
                    capture: shortcut.into(),
                    ..Shortcuts::default()
                },
                ..Settings::default()
            }
            .validate()
            .is_err());
        }
    }
}
