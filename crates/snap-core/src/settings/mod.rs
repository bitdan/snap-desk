//! 设置模型、校验与磁盘读写：沿用旧版的 `settings.json`。
//! 这里只描述“设置是什么、如何合法、如何落盘”，不注册系统快捷键、不通知界面；
//! 快捷键事务通过闭包传入注册/释放动作，由各前端外壳接到自己的快捷键管理器上。
pub mod backup;
pub mod shortcut_probe;
mod writer;

pub use writer::{persist, SettingsWriter};

use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

/// 系统快捷键类型（global-hotkey 的 `HotKey`）；设置中的写法解析成它后，
/// 可直接比较和注册。
pub use global_hotkey::hotkey::HotKey as Shortcut;

/// 设置文件路径：与旧版（Tauri `app_config_dir()`）相同（系统配置目录 / 应用标识），
/// 两个可执行程序读写同一份配置。
pub fn default_path() -> Option<PathBuf> {
    crate::paths::config_dir().map(|dir| dir.join("settings.json"))
}

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

impl ExportFormat {
    /// 保存对话框默认扩展名；JPEG 统一用 `.jpg`。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    /// 保存对话框的类型筛选：显示名称与可接受的扩展名。
    pub fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Png => ("PNG 图片（无损）", &["png"]),
            Self::Jpeg => ("JPEG 图片（白底）", &["jpg", "jpeg"]),
            Self::Webp => ("WebP 图片（无损）", &["webp"]),
        }
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
    Ocr,
}

impl ShortcutAction {
    pub const ALL: [Self; 7] = [
        Self::Capture,
        Self::Pin,
        Self::Record,
        Self::History,
        Self::Translate,
        Self::Shelf,
        Self::Ocr,
    ];
    /// 与设置字段、前端动作键以及命令行 `--action` 参数一致的名称。
    pub fn id(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Pin => "pin",
            Self::Record => "record",
            Self::History => "history",
            Self::Translate => "translate",
            Self::Shelf => "shelf",
            Self::Ocr => "ocr",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
}

/// 命令行 `--action <名称>`、`--action=<名称>` 或简写 `--<名称>`（名称同设置字段，如 `--capture`）。
/// 应用无法注册全局快捷键时（如 Linux Wayland），用户可把系统快捷键或 macOS 快捷指令绑定到这条命令；
/// 已有实例运行时由单实例机制转交参数，不会启动第二个进程。
pub fn action_from_args<I, S>(args: I) -> Option<ShortcutAction>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string_lossy().into_owned())
        .collect();
    // 简写只接受已知动作名，`--autostart` 等其他开关不受影响。
    if let Some(action) = args.iter().find_map(|arg| {
        arg.strip_prefix("--").and_then(ShortcutAction::from_id)
    }) {
        return Some(action);
    }
    let name = args.iter().enumerate().find_map(|(index, arg)| {
        if arg == "--action" {
            args.get(index + 1).cloned()
        } else {
            arg.strip_prefix("--action=").map(str::to_owned)
        }
    })?;
    let action = ShortcutAction::from_id(&name);
    if action.is_none() {
        log::warn!("event=app.cli_action_unknown module=app");
    }
    action
}

/// 默认快捷键所属的平台；备份跨平台导入时据此换算默认键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        Self::from_id(std::env::consts::OS)
    }
    /// 与 `std::env::consts::OS` 一致；其他类 Unix 系统按 Linux 处理。
    pub fn from_id(id: &str) -> Self {
        match id {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            _ => Self::Linux,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }
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
    pub ocr: String,
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
    shelf: Option<String>,
    ocr: Option<String>,
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
            shelf: None,
            ocr: None,
            translate_selection: None,
            translate_clipboard: None,
        }
    }
}
impl From<ShortcutsWire> for Shortcuts {
    fn from(wire: ShortcutsWire) -> Self {
        let missing = wire.history.is_none();
        let translation_missing = wire.translate.is_none();
        let shelf_missing = wire.shelf.is_none();
        let ocr_missing = wire.ocr.is_none();
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
            shelf: wire.shelf.unwrap_or_else(|| Self::default().shelf),
            ocr: wire.ocr.unwrap_or_else(|| Self::default().ocr),
        };
        if missing {
            keys.preserve_legacy_mapping();
        }
        // 旧配置已有相同组合时保留原动作，只禁用新加入的默认快捷键。
        if translation_missing { keys.preserve_translation_mapping(); }
        if shelf_missing { keys.preserve_default_mapping(ShortcutAction::Shelf); }
        if ocr_missing { keys.preserve_default_mapping(ShortcutAction::Ocr); }
        keys
    }
}
impl Default for Shortcuts {
    fn default() -> Self {
        Self::for_platform(Platform::current())
    }
}
impl Shortcuts {
    /// 默认快捷键的唯一来源；前端通过应用状态读取，不另行维护。
    /// 截图、贴图、录屏三平台一致用 F1–F3（mac 上需按 Fn，或在系统设置中把 F 键设为标准功能键）。
    /// 其余功能沿用同一助记字母；macOS 的 ⌥+字母会输入 √、ç 等字符，改用 ⌃⌥+字母。
    pub fn for_platform(platform: Platform) -> Self {
        let [history, translate, shelf, ocr] = match platform {
            Platform::Macos => ["Ctrl+Alt+V", "Ctrl+Alt+T", "Ctrl+Alt+Z", "Ctrl+Alt+C"],
            Platform::Windows | Platform::Linux => ["Alt+V", "Alt+T", "Alt+Z", "Alt+C"],
        };
        let [capture, pin, record] = ["F1", "F2", "F3"];
        Self {
            capture: capture.into(),
            pin: pin.into(),
            record: record.into(),
            history: history.into(),
            translate: translate.into(),
            shelf: shelf.into(),
            ocr: ocr.into(),
        }
    }
}
impl Shortcuts {
    /// 备份来自其他平台时，把仍等于来源平台默认值的按键换成本机默认值；自定义的按键保持不变。
    pub fn adopt_platform_defaults(&mut self, from: Platform) {
        let (source, target) = (Self::for_platform(from), Self::default());
        let pairs = [
            (&mut self.capture, &source.capture, &target.capture),
            (&mut self.pin, &source.pin, &target.pin),
            (&mut self.record, &source.record, &target.record),
            (&mut self.history, &source.history, &target.history),
            (&mut self.translate, &source.translate, &target.translate),
            (&mut self.shelf, &source.shelf, &target.shelf),
            (&mut self.ocr, &source.ocr, &target.ocr),
        ];
        for (value, source, target) in pairs {
            if value == source {
                value.clone_from(target);
            }
        }
    }
    fn preserve_legacy_mapping(&mut self) {
        self.preserve_default_mapping(ShortcutAction::History);
    }
    fn preserve_translation_mapping(&mut self) {
        self.preserve_default_mapping(ShortcutAction::Translate);
    }
    /// 缺失字段补默认值时让已有动作优先；显式配置的冲突仍由 validate 报错。
    fn preserve_default_mapping(&mut self, action: ShortcutAction) {
        let collision = {
            let entries = self.entries();
            let key = entries.iter().find(|(id, _, _)| *id == action)
                .and_then(|(_, _, value)| parse_shortcut(value).ok().flatten());
            key.is_some() && entries.iter().any(|(id, _, value)| {
                *id != action && parse_shortcut(value).ok().flatten() == key
            })
        };
        if collision {
            match action {
                ShortcutAction::History => self.history.clear(),
                ShortcutAction::Translate => self.translate.clear(),
                ShortcutAction::Shelf => self.shelf.clear(),
                ShortcutAction::Ocr => self.ocr.clear(),
                _ => {}
            }
        }
    }
    pub fn entries(&self) -> [(ShortcutAction, &str, &str); 7] {
        [
            (ShortcutAction::Capture, "区域截图", &self.capture),
            (ShortcutAction::Pin, "剪贴板贴图", &self.pin),
            (ShortcutAction::Record, "开始 / 停止录屏", &self.record),
            (ShortcutAction::History, "剪贴板历史", &self.history),
            (ShortcutAction::Translate, "翻译", &self.translate),
            (ShortcutAction::Shelf, "展开 / 收起暂存架", &self.shelf),
            (ShortcutAction::Ocr, "识别文字", &self.ocr),
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
impl Language {
    /// 首次启动按系统语言选择界面语言：中文系统（zh、zh-CN、zh-Hans-CN、zh_TW.UTF-8 等）用中文，
    /// 其他语言或无法识别时用英文。
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(tag) if tag.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("zh")) => Self::Chinese,
            _ => Self::English,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Appearance {
    pub theme: Theme,
    pub language: Language,
}

/// 应用更新的网络选项，均可留空。下载镜像是放在 github.com 地址前的前缀（如 `https://mirror.example/`）；
/// 镜像不受信任：版本列表仍从 GitHub 接口读取，安装包下载后必须通过签名校验。
/// 代理只用于检查和下载更新，留空时沿用系统代理。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct UpdateNetwork {
    pub mirror: String,
    pub proxy: String,
}

impl UpdateNetwork {
    pub fn validate(&self) -> Result<(), String> {
        if !self.mirror.is_empty() {
            update_mirror(&self.mirror)?;
        }
        if !self.proxy.is_empty() {
            update_proxy(&self.proxy)?;
        }
        Ok(())
    }
}

/// 镜像前缀必须是 HTTPS，且不带查询、片段或凭证；末尾补 `/` 以便直接拼接 github.com 地址。
pub fn update_mirror(value: &str) -> Result<url::Url, String> {
    const INVALID: &str = "下载镜像需填写完整的 https:// 地址，如 https://mirror.example/，不能包含空格、查询参数或账号密码。";
    let url = url::Url::parse(value).map_err(|_| INVALID)?;
    if value.len() > 512
        || value.trim() != value
        || value.contains(char::is_whitespace)
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(INVALID.into());
    }
    let mut url = url;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// 代理支持 http、https、socks5 与 socks5h（SOCKS 必须写端口）；允许账号密码（只保存在本机设置中）。
pub fn update_proxy(value: &str) -> Result<url::Url, String> {
    const INVALID: &str = "代理地址格式应为 http://127.0.0.1:7890 或 socks5://127.0.0.1:1080。";
    let url = url::Url::parse(value).map_err(|_| INVALID)?;
    if value.len() > 512
        || value.contains(char::is_whitespace)
        || !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host_str().is_none_or(str::is_empty)
        || url.port_or_known_default().is_none()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(INVALID.into());
    }
    Ok(url)
}

/// 去掉代理地址中的账号密码；空值或无法解析时原样返回。
pub fn proxy_without_credentials(value: &str) -> String {
    match url::Url::parse(value) {
        Ok(mut url) if !url.username().is_empty() || url.password().is_some() => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.as_str().trim_end_matches('/').to_string()
        }
        _ => value.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "SettingsWire")]
pub struct Settings {
    /// 默认只接收正式版；开启后也接收 alpha、beta 和 rc 应用更新。
    /// 测试版构建不受此项限制，始终接收测试版（见 app::updater::preview）。
    pub receive_prerelease_updates: bool,
    /// 后台复查新版本的间隔（分钟），只允许 UPDATE_CHECK_MINUTES 中的档位；手动检查不受影响。
    pub update_check_minutes: u32,
    pub update_network: UpdateNetwork,
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
    /// 打开普通截图时预先选中上次成功导出的选区；屏幕布局或分辨率变化时不恢复。
    pub screenshot_reuse_last: bool,
    /// 框选完成后的动作：none 显示工具栏，copy / pin / save 直接执行并结束截图。
    pub screenshot_after: String,
    pub export: ExportSettings,
    pub recording_export: RecordingExportSettings,
    pub pin_appearance: crate::pins::PinAppearance,
    /// 退出时保存仍打开的贴图，下次启动时恢复。
    pub pin_restore_on_startup: bool,
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
    #[serde(default = "hourly_update_check")]
    update_check_minutes: u32,
    #[serde(default)]
    update_network: UpdateNetwork,
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
    #[serde(default)]
    screenshot_reuse_last: bool,
    #[serde(default = "no_after_action")]
    screenshot_after: String,
    #[serde(default)]
    export: ExportSettings,
    #[serde(default)]
    recording_export: RecordingExportSettings,
    #[serde(default)]
    pin_appearance: crate::pins::PinAppearance,
    #[serde(default)]
    pin_restore_on_startup: bool,
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
            shortcuts.preserve_default_mapping(ShortcutAction::Shelf);
            shortcuts.preserve_default_mapping(ShortcutAction::Ocr);
        }
        Ok(Self {
            receive_prerelease_updates: wire.receive_prerelease_updates,
            update_check_minutes: wire.update_check_minutes,
            update_network: wire.update_network,
            shortcuts,
            shortcuts_enabled: wire.shortcuts_enabled,
            delay_seconds: wire.delay_seconds,
            screenshot_cursor: wire.screenshot_cursor,
            screenshot_snap: wire.screenshot_snap,
            screenshot_reuse_last: wire.screenshot_reuse_last,
            screenshot_after: wire.screenshot_after,
            export: wire.export,
            recording_export: wire.recording_export,
            pin_appearance: wire.pin_appearance,
            pin_restore_on_startup: wire.pin_restore_on_startup,
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
/// 可选的后台检查更新间隔；最短 30 分钟，每次只请求一次发布列表，远低于 GitHub 匿名接口每小时 60 次的限额。
pub const UPDATE_CHECK_MINUTES: [u32; 4] = [30, 60, 360, 1440];
fn hourly_update_check() -> u32 {
    60
}
fn no_after_action() -> String {
    "none".into()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            receive_prerelease_updates: false,
            update_check_minutes: hourly_update_check(),
            update_network: UpdateNetwork::default(),
            shortcuts: Shortcuts::default(),
            shortcuts_enabled: true,
            delay_seconds: 0,
            screenshot_cursor: false,
            screenshot_snap: true,
            screenshot_reuse_last: false,
            screenshot_after: no_after_action(),
            export: ExportSettings::default(),
            recording_export: RecordingExportSettings::default(),
            pin_appearance: crate::pins::PinAppearance::default(),
            pin_restore_on_startup: false,
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
pub fn parse_shortcut(value: &str) -> Result<Option<Shortcut>, String> {
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
        if !UPDATE_CHECK_MINUTES.contains(&self.update_check_minutes) {
            return Err("检查更新间隔无效。".into());
        }
        self.update_network.validate()?;
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
pub fn shortcut_keys(value: &Settings) -> Result<Vec<Shortcut>, String> {
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

/// 按顺序更新外部资源；通过闭包传入系统操作，让失败回滚能在测试中模拟。
/// FnMut 表示闭包可修改自己的状态；生产环境操作系统，测试环境记录调用结果。
pub fn apply_settings(
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
    // 原配置里就有、这次没有改动的按键（启动时或此前被其他程序占用而未注册成功）只尽力重试：
    // 失败时保持未生效并提示，不拖累其他设置的保存。新设置的按键仍按事务处理，失败即整体回滚。
    let configured = if old.shortcuts_enabled {
        shortcut_keys(old).unwrap_or_default()
    } else {
        vec![]
    };
    let mut added = Vec::new();
    let mut unavailable = Vec::new();
    for key in new_keys.iter().filter(|key| !old_keys.contains(key)) {
        if configured.contains(key) {
            if register(*key).is_err() {
                unavailable.push(*key);
            } else {
                added.push(*key);
            }
            continue;
        }
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
    if unavailable.is_empty() {
        return Ok(None);
    }
    let names: Vec<_> = next
        .shortcuts
        .entries()
        .into_iter()
        .filter(|(_, _, value)| {
            parse_shortcut(value)
                .ok()
                .flatten()
                .is_some_and(|key| unavailable.contains(&key))
        })
        .map(|(_, name, value)| format!("{name} ({value})"))
        .collect();
    Ok(Some(format!(
        "以下快捷键已被系统或其他程序占用，暂未生效：{}。请在“快捷键”页更换按键。",
        names.join("、")
    )))
}

/// 已配置却没有注册成功的动作（被系统或其他程序占用）；总开关关闭或录入中时不计。
pub fn unavailable_actions(current: &RuntimeSettings) -> Vec<&'static str> {
    if !current.value.shortcuts_enabled || current.editing {
        return Vec::new();
    }
    current
        .value
        .shortcuts
        .entries()
        .into_iter()
        .filter(|(_, _, value)| {
            parse_shortcut(value)
                .ok()
                .flatten()
                .is_some_and(|key| !current.registered.contains(&key))
        })
        .map(|(action, _, _)| action.id())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_language_follows_system_locale() {
        for tag in ["zh", "zh-CN", "zh-Hans-CN", "zh_TW.UTF-8", "ZH-cn"] {
            assert_eq!(Language::from_locale(Some(tag)), Language::Chinese, "{tag}");
        }
        for tag in ["en-US", "C.UTF-8", "ja-JP", "z", ""] {
            assert_eq!(Language::from_locale(Some(tag)), Language::English, "{tag}");
        }
        assert_eq!(Language::from_locale(None), Language::English);
    }

    #[test]
    fn update_network_defaults_empty_and_validates_mirror_and_proxy() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.update_network, UpdateNetwork::default());
        let configured: Settings = serde_json::from_str(
            r#"{"updateNetwork":{"mirror":"https://mirror.example/gh","proxy":"socks5://127.0.0.1:1080"}}"#,
        )
        .unwrap();
        configured.validate().unwrap();
        assert!(serde_json::from_str::<Settings>(r#"{"updateNetwork":{"other":""}}"#).is_err());
        // 镜像末尾补 `/`，便于直接拼接 github.com 地址。
        assert_eq!(
            update_mirror("https://mirror.example/gh").unwrap().as_str(),
            "https://mirror.example/gh/"
        );
        for bad in [
            "http://mirror.example/",
            "https://user:pw@mirror.example/",
            "https://mirror.example/?a=1",
            "https://mirror.example/#x",
            " https://mirror.example/",
            "mirror.example",
        ] {
            assert!(update_mirror(bad).is_err(), "{bad}");
        }
        for good in [
            "http://127.0.0.1:7890",
            "https://proxy.example",
            "socks5://127.0.0.1:1080",
            "socks5h://user:pw@127.0.0.1:1080",
        ] {
            assert!(update_proxy(good).is_ok(), "{good}");
        }
        for bad in [
            "127.0.0.1:7890",
            "ftp://127.0.0.1:21",
            "socks5://127.0.0.1",
            "http://127.0.0.1:7890/path",
            "socks4://127.0.0.1:1080",
        ] {
            assert!(update_proxy(bad).is_err(), "{bad}");
        }
        let invalid = Settings {
            update_network: UpdateNetwork {
                mirror: "http://mirror.example/".into(),
                proxy: String::new(),
            },
            ..Settings::default()
        };
        assert!(invalid.validate().is_err());
        assert_eq!(
            proxy_without_credentials("socks5h://user:pw@127.0.0.1:1080"),
            "socks5h://127.0.0.1:1080"
        );
        assert_eq!(
            proxy_without_credentials("http://127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(proxy_without_credentials(""), "");
    }

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
    // 以下迁移用例以 Windows / Linux 的默认键为基准；macOS 默认键见 macos_defaults_avoid_option_letter_keys。
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn maps_actions_clears_keys_and_migrates_strictly() {
        let mut value = Settings::default();
        value.shortcuts.capture = "".into();
        value.shortcuts.pin = "F1".into();
        value.shortcuts.record = "Ctrl+Shift+S".into();
        assert!(value.validate().is_ok());
        assert_eq!(shortcut_keys(&value).unwrap().len(), 6);
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
        assert_eq!(old.shortcuts.history, "Alt+V");
        assert!(old.clipboard.enabled);
        assert!(!old.receive_prerelease_updates);
        for json in [
            r#"{"shortcut":"Alt+V"}"#,
            r#"{"shortcuts":{"capture":"Alt+V","pin":"F2","record":"F3"}}"#,
        ] {
            let migrated: Settings = serde_json::from_str(json).unwrap();
            assert!(migrated.shortcuts.history.is_empty());
            assert!(migrated.validate().is_ok());
            assert_eq!(migrated.shortcuts.capture, "Alt+V");
        }
        assert!(!serde_json::to_value(old)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("shortcut"));
    }

    #[test]
    fn update_check_interval_defaults_to_hourly_and_rejects_other_values() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.update_check_minutes, 60);
        for minutes in UPDATE_CHECK_MINUTES {
            let json = format!(r#"{{"updateCheckMinutes":{minutes}}}"#);
            let settings: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.update_check_minutes, minutes);
            assert!(settings.validate().is_ok());
        }
        for minutes in [0, 1, 45, 100_000] {
            let settings = Settings { update_check_minutes: minutes, ..Settings::default() };
            assert!(settings.validate().is_err());
        }
    }

    // 以下迁移用例以 Windows / Linux 的默认键为基准；macOS 默认键见 macos_defaults_avoid_option_letter_keys。
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ocr_shelf_and_history_defaults_preserve_existing_shortcuts() {
        let defaults = Settings::default();
        assert_eq!(defaults.shortcuts.history, "Alt+V");
        assert_eq!(defaults.shortcuts.shelf, "Alt+Z");
        assert_eq!(defaults.shortcuts.ocr, "Alt+C");
        assert_eq!(defaults.action_for(&"Alt+C".parse().unwrap()), Some(ShortcutAction::Ocr));
        assert!(defaults.validate().is_ok());
        for (key, action) in [("Alt+C", ShortcutAction::Ocr), ("Alt+Z", ShortcutAction::Shelf)] {
            for json in [format!(r#"{{"shortcut":"{key}"}}"#), format!(r#"{{"shortcuts":{{"capture":"{key}"}}}}"#)] {
                let migrated: Settings = serde_json::from_str(&json).unwrap();
                assert!(migrated.validate().is_ok());
                assert_eq!(migrated.action_for(&key.parse().unwrap()), Some(ShortcutAction::Capture));
                assert!(migrated.shortcuts.entries().iter().find(|(id, _, _)| *id == action).unwrap().2.is_empty());
            }
        }
        let old: Settings = serde_json::from_str(r#"{"shortcuts":{"history":"F4","shelf":"","ocr":""}}"#).unwrap();
        assert_eq!(old.shortcuts.history, "F4");
        assert!(old.shortcuts.shelf.is_empty());
        assert!(old.shortcuts.ocr.is_empty());
        let conflicting: Settings = serde_json::from_str(r#"{"shortcuts":{"capture":"Alt+C","ocr":"Alt+C"}}"#).unwrap();
        assert!(conflicting.validate().is_err());
        let restored: Settings = serde_json::from_value(serde_json::to_value(defaults.clone()).unwrap()).unwrap();
        assert_eq!(restored.shortcuts, defaults.shortcuts);
    }

    // 以下迁移用例以 Windows / Linux 的默认键为基准；macOS 默认键见 macos_defaults_avoid_option_letter_keys。
    #[cfg(not(target_os = "macos"))]
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_defaults_avoid_option_letter_keys() {
        let defaults = Settings::default();
        assert!(defaults.validate().is_ok());
        let keys = &defaults.shortcuts;
        assert_eq!([&*keys.capture, &*keys.pin, &*keys.record], ["F1", "F2", "F3"]);
        for key in [&keys.history, &keys.translate, &keys.shelf, &keys.ocr] {
            assert!(key.starts_with("Ctrl+Alt+"), "{key}");
        }
        assert_eq!(
            defaults.action_for(&"Ctrl+Alt+V".parse().unwrap()),
            Some(ShortcutAction::History)
        );
        // 旧配置把新增默认键占给其他动作时，保留原动作，只禁用新加入的快捷键。
        let migrated: Settings =
            serde_json::from_str(r#"{"shortcuts":{"capture":"Ctrl+Alt+V"}}"#).unwrap();
        assert!(migrated.shortcuts.history.is_empty());
        assert!(migrated.validate().is_ok());
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
        assert!(!value.screenshot_reuse_last);
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
        // 默认 F2/F3 被贴图和录屏占用，其余默认键都带修饰键，F4 起可重新分配。
        for n in (1..=24).filter(|n| ![2, 3].contains(n)) {
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
        // 重复动作映射及不支持的主键不能通过完整配置校验；其余默认键因平台而异，取实际值。
        let keys = Shortcuts::default();
        for shortcut in [
            "F2",
            "F3",
            keys.history.as_str(),
            keys.shelf.as_str(),
            keys.ocr.as_str(),
            "F25",
            "Ctrl+A+B",
        ] {
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

    #[test]
    fn occupied_configured_keys_do_not_block_startup_or_other_saves() {
        let value = Settings::default();
        let capture = parse_shortcut(&value.shortcuts.capture).unwrap().unwrap();
        let occupied = |key: Shortcut| if key == capture { Err("occupied".to_string()) } else { Ok(()) };
        let mut registered = Vec::new();
        let warning = apply_settings(&value, &value, &mut registered, occupied, |_| Ok(()), |_| Ok(()))
            .unwrap()
            .unwrap();
        assert!(warning.contains("区域截图 (F1)"), "{warning}");
        assert!(!registered.contains(&capture));
        assert_eq!(registered.len(), shortcut_keys(&value).unwrap().len() - 1);
        let current = RuntimeSettings {
            value: value.clone(),
            registered: registered.clone(),
            editing: false,
            shortcut_error: Some(warning),
            config_warning: None,
        };
        assert_eq!(unavailable_actions(&current), vec!["capture"]);
        // 仍被占用时修改其他设置照常保存，只重试并继续提示该按键。
        let next = Settings { delay_seconds: 3, ..value.clone() };
        let written = std::cell::Cell::new(false);
        let warning = apply_settings(&value, &next, &mut registered, occupied, |_| Ok(()), |_| {
            written.set(true);
            Ok(())
        })
        .unwrap();
        assert!(written.get() && warning.is_some());
        // 改成新的被占用按键仍整体回滚。
        let changed = Settings {
            shortcuts: Shortcuts { pin: "F8".into(), ..value.shortcuts.clone() },
            ..value.clone()
        };
        let f8 = parse_shortcut("F8").unwrap().unwrap();
        let before = registered.clone();
        assert!(apply_settings(
            &value,
            &changed,
            &mut registered,
            |key| if key == f8 || key == capture { Err("occupied".into()) } else { Ok(()) },
            |_| Ok(()),
            |_| panic!("冲突不能写配置"),
        )
        .is_err());
        assert_eq!(registered, before);
    }

    #[test]
    fn platform_defaults_share_function_keys_and_map_across_backups() {
        for platform in [Platform::Windows, Platform::Macos, Platform::Linux] {
            let keys = Shortcuts::for_platform(platform);
            assert_eq!([&*keys.capture, &*keys.pin, &*keys.record], ["F1", "F2", "F3"]);
            assert!(Settings { shortcuts: keys, ..Settings::default() }.validate().is_ok());
            assert_eq!(Platform::from_id(platform.id()), platform);
        }
        assert_eq!(Shortcuts::for_platform(Platform::Macos).history, "Ctrl+Alt+V");
        assert_eq!(Shortcuts::for_platform(Platform::Windows).history, "Alt+V");
        // 来源平台的默认键换成本机默认键，自定义按键保持不变。
        for from in [Platform::Windows, Platform::Macos] {
            let mut keys = Shortcuts { capture: "Ctrl+F8".into(), ..Shortcuts::for_platform(from) };
            keys.adopt_platform_defaults(from);
            assert_eq!(keys, Shortcuts { capture: "Ctrl+F8".into(), ..Shortcuts::default() });
        }
        for action in ShortcutAction::ALL {
            assert_eq!(ShortcutAction::from_id(action.id()), Some(action));
        }
        assert_eq!(ShortcutAction::from_id("unknown"), None);
    }

    #[test]
    fn command_line_actions_accept_both_forms_and_ignore_unknown_names() {
        assert_eq!(action_from_args(["snap-desk", "--action", "capture"]), Some(ShortcutAction::Capture));
        assert_eq!(action_from_args(["snap-desk", "--action=ocr"]), Some(ShortcutAction::Ocr));
        assert_eq!(action_from_args(["snap-desk", "--autostart"]), None);
        assert_eq!(action_from_args(["snap-desk", "--capture"]), Some(ShortcutAction::Capture));
        assert_eq!(action_from_args(["snap-desk", "--record"]), Some(ShortcutAction::Record));
        assert_eq!(action_from_args(["snap-desk", "--autostart", "--history"]), Some(ShortcutAction::History));
        assert_eq!(action_from_args(["snap-desk", "capture"]), None);
        assert_eq!(action_from_args(["snap-desk", "--update-failed"]), None);
        assert_eq!(action_from_args(["snap-desk", "--action"]), None);
        assert_eq!(action_from_args(["snap-desk", "--action", "rm"]), None);
    }
}
