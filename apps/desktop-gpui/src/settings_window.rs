//! 设置窗口（对照 Vue 主界面 `App.vue` 的设置页与 `ShortcutInput.vue`）。
//!
//! 托盘“设置…”打开。左侧导航，右侧为页面：截图、录屏、贴图、剪贴板、通用、快捷键。每个控件修改后立即
//! 经 [`crate::shell::update_settings`] 写盘（先重读磁盘再只改对应字段，快捷键注册与写盘为一笔事务）；
//! 文本框停顿 500 ms 后保存，内容无效时只显示原因、不写盘。
//!
//! 快捷键录入：点击按键框后注销全部全局快捷键（否则组合会被系统热键吞掉），按下组合即预览，
//! 全部按键松开后保存；Esc 取消，Backspace/Delete 禁用该动作；窗口失焦也会取消并恢复全局快捷键。
//!
//! 截图、录屏的默认保存目录可更改或恢复默认（与 Tauri 版共用目录记录文件）。
//! 通用页：开机自启（core `autostart`，与 Tauri 版共用登录项）、配置导出/导入（core `settings::backup`，
//! 导入事务见 `shell::import_backup`）。
use gpui_kit::{
    component::{
        Icon, Sizable,
        input::{Input, InputEvent, InputState},
    },
    *,
};
use snap_core::{
    pins::PinShadow,
    settings::{ExportFormat, Language, Settings, ShortcutAction, Theme},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// 设置窗口配色（Vue `.app-shell` 的 `--desk-*` 变量，见 `theme::desk`），随主题变化。
mod col {
    fn d() -> &'static crate::theme::DeskPalette {
        crate::theme::desk()
    }
    /// 卡片、输入框底色。
    pub fn surface() -> u32 {
        d().surface
    }
    /// 内容区底色。
    pub fn page_bg() -> u32 {
        d().bg
    }
    pub fn border() -> u32 {
        d().border
    }
    /// 控件细线框。
    pub fn line() -> u32 {
        d().line
    }
    /// 卡片内行分隔线。
    pub fn sep() -> u32 {
        d().sep
    }
    pub fn text() -> u32 {
        d().text
    }
    pub fn muted() -> u32 {
        d().muted
    }
    pub fn accent() -> u32 {
        d().accent
    }
    /// 强调色文字（深色主题下更亮）。
    pub fn accent_fg() -> u32 {
        d().accent_fg
    }
    pub fn danger() -> u32 {
        d().danger
    }
    pub fn accent_soft() -> u32 {
        d().soft
    }
    pub fn nav() -> u32 {
        d().nav
    }
    pub fn toggle_off() -> u32 {
        d().line
    }
    pub fn hover() -> u32 {
        d().hover
    }
    pub fn hover_nav() -> u32 {
        d().hover_nav
    }
    pub fn success() -> u32 {
        d().success
    }
    pub fn notice() -> u32 {
        d().notice
    }
    pub fn notice_error() -> u32 {
        d().notice_error
    }
}
/// 设置窗口用到的 Lucide 图标（ISC，见 assets/icons/LICENSE-LUCIDE）。
mod icons {
    macro_rules! icon {
        ($name:ident, $file:literal) => {
            pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));
        };
    }
    icon!(SCAN, "scan");
    icon!(SEARCH, "search");
    icon!(CAMERA, "camera");
    icon!(VIDEO, "video");
    icon!(PIN, "pin");
    icon!(CLIPBOARD, "clipboard-list");
    icon!(SETTINGS, "settings");
    icon!(KEYBOARD, "keyboard");
    icon!(LANGUAGES, "languages");
    icon!(LOGS, "file-text");
    icon!(INFO, "info");
    icon!(CHEVRON, "chevron-down");
    icon!(CHECK, "check");
    icon!(X, "x");
}

/// 文本框停顿多久后保存。
const DEBOUNCE: Duration = Duration::from_millis(500);
/// 与截图标注色板一致的贴图边框预设。
const BORDER_COLORS: [&str; 8] = [
    "#a1a1aa", "#3f3f46", "#ffffff", "#ef4444", "#f59e0b", "#22c55e", "#3b82f6", "#a855f7",
];

#[derive(Default)]
struct SettingsState {
    window: Option<AnyWindowHandle>,
}
impl Global for SettingsState {}

/// 打开（或激活已打开的）设置窗口。
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsState>().and_then(|s| s.window) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let settings = crate::shell::settings(cx);
    let title = snap_core::i18n::for_language(&settings.appearance.language, "设置").to_owned();
    // 与 Tauri 主窗口一致：820×580，最小 680×500。
    let size = size(px(820.0), px(580.0));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(gpui_kit::size(px(680.0), px(500.0))),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| SettingsView::new(&settings, window, cx))
    }) {
        Ok(handle) => {
            cx.default_global::<SettingsState>().window = Some(handle.into());
            log::info!("event=gpui.settings_opened");
        }
        Err(error) => log::error!("event=gpui.settings_open_failed message={error}"),
    }
}

#[path = "settings_about.rs"]
mod about_page;
#[path = "settings_local.rs"]
mod local_page;
#[path = "settings_logs.rs"]
mod logs_page;
#[path = "settings_probe.rs"]
pub(crate) mod probe;
#[path = "settings_recognition.rs"]
mod recognition_page;
#[path = "settings_resources.rs"]
mod resources_page;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Capture,
    Recording,
    Pins,
    Clipboard,
    General,
    Shortcuts,
    Recognition,
    Logs,
    About,
}

/// 导航分组（Vue `groups`）：名称与页面；关键词让搜索也能命中页面内的设置项。
const GROUPS: [(&str, &[Page]); 3] = [
    (
        "工具",
        &[Page::Capture, Page::Recording, Page::Pins, Page::Clipboard],
    ),
    ("设置", &[Page::General, Page::Shortcuts, Page::Recognition]),
    ("支持", &[Page::Logs, Page::About]),
];

impl Page {
    fn icon(self) -> &'static [u8] {
        match self {
            Page::Capture => icons::CAMERA,
            Page::Recording => icons::VIDEO,
            Page::Pins => icons::PIN,
            Page::Clipboard => icons::CLIPBOARD,
            Page::General => icons::SETTINGS,
            Page::Shortcuts => icons::KEYBOARD,
            Page::Recognition => icons::LANGUAGES,
            Page::Logs => icons::LOGS,
            Page::About => icons::INFO,
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Page::Capture => "区域 延时 光标 吸附 保存 格式 文件名 capture screenshot delay format",
            Page::Recording => "mp4 gif 帧率 画质 码率 编码 倒计时 键鼠 特效 record video fps",
            Page::Pins => "阴影 边框 圆角 外观 pin shadow border",
            Page::Clipboard => "历史 记录 清理 粘贴 排除 暂存架 clipboard history shelf",
            Page::General => {
                "外观 主题 深色 语言 开机 自启 字体 目录 导入 导出 配置 备份 theme language startup font directory import export"
            }
            Page::Shortcuts => "按键 冲突 检测 shortcut hotkey",
            Page::Recognition => {
                "ocr 文字识别 翻译 服务 模型 语言包 密钥 deepl openai deepseek groq paddle translate service"
            }
            Page::Logs => "记录级别 诊断 排错 logs diagnostics",
            Page::About => "版本 更新 测试版 引导 帮助 许可 about help update version",
        }
    }

    /// 搜索：名称（含译文）或关键词包含查询（不区分大小写）。
    fn matches(self, query: &str, label: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.label().to_lowercase().contains(&query)
            || label.to_lowercase().contains(&query)
            || self.keywords().to_lowercase().contains(&query)
    }

    const ALL: [Page; 9] = [
        Page::Capture,
        Page::Recording,
        Page::Pins,
        Page::Clipboard,
        Page::General,
        Page::Shortcuts,
        Page::Recognition,
        Page::Logs,
        Page::About,
    ];
    fn label(self) -> &'static str {
        match self {
            Page::Capture => "截图",
            Page::Recording => "录屏",
            Page::Pins => "贴图",
            Page::Clipboard => "剪贴板",
            Page::General => "通用",
            Page::Shortcuts => "快捷键",
            Page::Recognition => "识别与翻译",
            Page::Logs => "日志",
            Page::About => "关于与帮助",
        }
    }
}

/// 防抖保存的文本字段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    CapturePrefix,
    RecordingPrefix,
    ExcludedApps,
    UpdateMirror,
    UpdateProxy,
}

impl Field {
    fn apply(self, settings: &mut Settings, value: &str) {
        match self {
            Field::CapturePrefix => settings.export.filename_prefix = value.to_owned(),
            Field::RecordingPrefix => settings.recording_export.filename_prefix = value.to_owned(),
            Field::ExcludedApps => settings.clipboard.excluded_apps = excluded_apps(value),
            Field::UpdateMirror => settings.update_network.mirror = value.trim().to_owned(),
            Field::UpdateProxy => settings.update_network.proxy = value.trim().to_owned(),
        }
    }
}

/// 排除应用：逗号、分号或换行分隔，去掉首尾空白与空项。
fn excluded_apps(value: &str) -> Vec<String> {
    value
        .split([',', ';', '\n', '，', '；'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 正在录入的快捷键。
struct Capture {
    action: ShortcutAction,
    /// 已按下的有效组合（等全部松开再保存）。
    candidate: Option<String>,
    preview: String,
}

struct SettingsView {
    page: Page,
    /// 侧栏搜索框与当前打开的下拉选择。
    nav_search: Entity<InputState>,
    open_select: Option<&'static str>,
    /// 识别与翻译页的页签（0 文字识别、1 翻译、2 服务管理）。
    recognition_tab: usize,
    _nav_search: Subscription,
    focus: FocusHandle,
    capture_prefix: Entity<InputState>,
    recording_prefix: Entity<InputState>,
    excluded: Entity<InputState>,
    update_mirror: Entity<InputState>,
    update_proxy: Entity<InputState>,
    /// 关于页的检查更新结果。
    /// 识别与翻译页：后台读取的语言与密钥状态、正在编辑的服务。
    recognition: recognition_page::RecognitionState,
    service_editor: Option<recognition_page::ServiceEditor>,
    /// 服务管理页：展开管理区的本地引擎（0 PaddleOCR、1 系统 OCR、2 WeChat OCR、3 CTranslate2）。
    managing_local: Option<usize>,
    /// 服务管理页：“添加服务”预设面板是否展开。
    service_picker: bool,
    /// 录屏页在录屏进行中每秒刷新一次（阶段由录屏线程推进）。
    recording_poll: bool,
    /// 资源状态、下载中标记与进度（识别页、通用页共用）。
    resources: Option<snap_core::resources::Status>,
    resource_busy: bool,
    resource_progress: resources_page::Progress,
    /// 本地翻译引擎与语言包状态。
    local: Option<Result<snap_core::recognition::local_models::LocalStatus, String>>,
    /// 快捷键兼容测试。
    probe: probe::Probe,
    /// 每次文本修改加一；防抖任务只保存最新一次。
    generation: u64,
    capture: Option<Capture>,
    error: Option<String>,
    /// 最近一次保存成功（显示“已自动保存”）。
    saved: bool,
    /// 系统登录项状态；None 为尚未读取或读取失败。
    autostart: Option<bool>,
    /// 导出配置时包含服务密钥（明文）。
    include_secrets: bool,
    /// 配置导入、导出进行中。
    transferring: bool,
    /// 操作成功提示（导出/导入完成）。
    notice: Option<String>,
    /// 导入后文本框需要按新设置重填（需要窗口，在渲染时执行）。
    reload_inputs: bool,
    /// 日志页的最近一次读取结果与筛选。
    logs: Option<logs_page::LogsView>,
    logs_loading: bool,
    log_filter: Option<snap_core::diagnostics::logs::Level>,
    /// 日志页：模块筛选、搜索框与自动刷新。
    log_module: Option<snap_core::diagnostics::logs::Module>,
    log_search: Entity<InputState>,
    log_live: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(settings: &Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_placeholder =
            snap_core::i18n::for_language(&settings.appearance.language, "搜索").to_owned();
        let nav_search = cx.new(|cx| InputState::new(window, cx).placeholder(search_placeholder));
        let search_changed =
            cx.subscribe(&nav_search, |_: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let input = |value: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let capture_prefix = input(settings.export.filename_prefix.clone(), window, cx);
        let recording_prefix = input(
            settings.recording_export.filename_prefix.clone(),
            window,
            cx,
        );
        let excluded = input(settings.clipboard.excluded_apps.join(", "), window, cx);
        let placeholder = |key: &str| snap_core::i18n::for_language(&settings.appearance.language, key).to_owned();
        let (mirror_hint, proxy_hint) = (placeholder("可选，如 https://mirror.example/"), placeholder("可选，如 http://127.0.0.1:7890"));
        let update_mirror = cx.new(|cx| {
            InputState::new(window, cx).placeholder(mirror_hint).default_value(settings.update_network.mirror.clone())
        });
        let update_proxy = cx.new(|cx| {
            InputState::new(window, cx).placeholder(proxy_hint).default_value(settings.update_network.proxy.clone())
        });
        let log_search = {
            let placeholder = snap_core::i18n::for_language(&settings.appearance.language, "搜索事件名、错误码").to_owned();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(&log_search, |_: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        for (state, field) in [
            (&capture_prefix, Field::CapturePrefix),
            (&recording_prefix, Field::RecordingPrefix),
            (&excluded, Field::ExcludedApps),
            (&update_mirror, Field::UpdateMirror),
            (&update_proxy, Field::UpdateProxy),
        ] {
            subscriptions.push(cx.subscribe(
                state,
                move |view: &mut Self, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = state.read(cx).value().to_string();
                        view.schedule(field, value, cx);
                    }
                },
            ));
        }
        // 窗口失焦时取消快捷键录入，恢复全局快捷键。
        subscriptions.push(
            cx.observe_window_activation(window, |view: &mut Self, window, cx| {
                if !window.is_window_active() && view.capture.is_some() {
                    view.end_capture(None, cx);
                }
            }),
        );
        Self {
            page: Page::Capture,
            focus: cx.focus_handle(),
            capture_prefix,
            recording_prefix,
            excluded,
            update_mirror,
            update_proxy,
            recognition: Default::default(),
            service_editor: None,
            managing_local: None,
            service_picker: false,
            recording_poll: false,
            resources: None,
            resource_busy: false,
            resource_progress: Default::default(),
            local: None,
            probe: Default::default(),
            nav_search,
            open_select: None,
            recognition_tab: 0,
            _nav_search: search_changed,
            generation: 0,
            capture: None,
            error: None,
            saved: false,
            autostart: None,
            include_secrets: false,
            transferring: false,
            notice: None,
            reload_inputs: false,
            logs: None,
            logs_loading: false,
            log_filter: None,
            log_module: None,
            log_search,
            log_live: false,
            _subscriptions: subscriptions,
        }
        .with_autostart(cx)
    }

    /// 后台读取登录项状态。
    fn with_autostart(self, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let state = cx
                .background_executor()
                .spawn(async { snap_core::autostart::enabled() })
                .await;
            let _ = this.update(cx, |view, cx| {
                match state {
                    Ok(on) => view.autostart = Some(on),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        self
    }

    fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { snap_core::autostart::set(on) })
                .await;
            let fresh = cx
                .background_executor()
                .spawn(async { snap_core::autostart::enabled() })
                .await;
            let _ = this.update(cx, |view, cx| {
                // 写入失败也可能已部分生效，以重新查询的状态为准。
                view.autostart = fresh.ok();
                match result {
                    Ok(_) => {
                        view.error = None;
                        view.saved = true;
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 导出完整配置（可选包含服务密钥）。
    fn export_config(&mut self, cx: &mut Context<Self>) {
        if self.transferring {
            return;
        }
        self.transferring = true;
        self.notice = None;
        let settings = crate::shell::settings(cx);
        let include = self.include_secrets;
        let active = crate::shell::active_config_files(cx);
        let title =
            snap_core::i18n::for_language(&settings.appearance.language, "导出配置").to_owned();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = async {
                let remember = settings.recording_export.remember_directory;
                let document = cx
                    .background_executor()
                    .spawn(async move {
                        let directories = snap_core::settings::backup::Directories {
                            captures: snap_core::export::directory_record()
                                .and_then(|record| snap_core::export::read_directory(&record)),
                            recordings: match crate::shell::recording_record() {
                                Some(record) => snap_core::recording::output::configured_directory(
                                    &record, remember,
                                )?,
                                None => None,
                            },
                        };
                        snap_core::settings::backup::export_document(settings, directories, include)
                    })
                    .await?;
                let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
                let picked = rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .set_file_name("snap-desk-settings.json")
                    .add_filter("JSON", &["json"])
                    .save_file()
                    .await;
                cx.update(|cx| crate::pins::show_pins(cx, hidden));
                let Some(file) = picked else {
                    return Ok(false);
                };
                let destination = file.path().to_path_buf();
                cx.background_executor()
                    .spawn(async move {
                        snap_core::settings::backup::guard_destination(&destination, &active)?;
                        snap_core::settings::backup::write_bytes(&destination, &document)
                    })
                    .await?;
                Ok::<_, String>(true)
            }
            .await;
            let _ = this.update(cx, |view, cx| {
                view.transferring = false;
                match result {
                    Ok(true) => {
                        view.error = None;
                        view.notice = Some("已导出配置。".into());
                    }
                    Ok(false) => {}
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 导入完整配置：校验后作为一笔事务替换设置与目录记录，失败时原样恢复。
    fn import_config(&mut self, cx: &mut Context<Self>) {
        if self.transferring || self.capture.is_some() {
            return;
        }
        self.transferring = true;
        self.notice = None;
        let language = crate::shell::settings(cx).appearance.language;
        let title = snap_core::i18n::for_language(&language, "导入配置").to_owned();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
            let picked = rfd::AsyncFileDialog::new()
                .set_title(title)
                .add_filter("JSON", &["json"])
                .pick_file()
                .await;
            cx.update(|cx| crate::pins::show_pins(cx, hidden));
            let result = match picked {
                None => Ok(None),
                Some(file) => {
                    let path = file.path().to_path_buf();
                    let parsed = cx
                        .background_executor()
                        .spawn(async move {
                            snap_core::settings::backup::parse(
                                &snap_core::settings::backup::read_bounded(&path)?,
                            )
                        })
                        .await;
                    match parsed {
                        Ok(backup) => cx
                            .update(|cx| crate::shell::import_backup(cx, backup))
                            .map(Some),
                        Err(error) => Err(error),
                    }
                }
            };
            let _ = this.update(cx, |view, cx| {
                view.transferring = false;
                match result {
                    Ok(Some(_)) => {
                        view.error = None;
                        view.notice = Some("已导入配置。".into());
                        view.reload_inputs = true;
                    }
                    Ok(None) => {}
                    // 导入本身成功但密钥写回失败时，设置已更新，同样重填文本框。
                    Err(error) => {
                        view.reload_inputs = true;
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn language(cx: &App) -> Language {
        crate::shell::settings(cx).appearance.language
    }

    /// 修改并保存；失败时显示原因（设置保持原状）。
    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        match crate::shell::update_settings(cx, change) {
            Ok(_) => {
                self.error = None;
                self.saved = true;
            }
            Err(error) => {
                log::warn!("event=gpui.settings_rejected message={error}");
                self.error = Some(error);
                self.saved = false;
            }
        }
        cx.notify();
    }

    fn schedule(&mut self, field: Field, value: String, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(DEBOUNCE).await;
            let _ = this.update(cx, |view, cx| {
                if view.generation == generation {
                    view.change(cx, |s| field.apply(s, &value));
                }
            });
        })
        .detach();
    }

    fn begin_capture(
        &mut self,
        action: ShortcutAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.capture.is_some() {
            self.end_capture(None, cx);
        }
        crate::shell::suspend_shortcuts(cx, true);
        self.capture = Some(Capture {
            action,
            candidate: None,
            preview: String::new(),
        });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// 结束录入：先恢复全局快捷键，再保存新组合（`Some("")` 为禁用）。
    fn end_capture(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        crate::shell::suspend_shortcuts(cx, false);
        if let Some(value) = value {
            self.change(cx, |s| set_shortcut(s, capture.action, value));
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = self.capture.as_mut() else {
            // Esc：先收起展开的下拉/预设面板，否则关闭设置窗口。
            let k = &event.keystroke;
            if k.key == "escape" && !event.is_held && !k.modifiers.modified() {
                cx.stop_propagation();
                if self.open_select.take().is_some() || std::mem::take(&mut self.service_picker) {
                    cx.notify();
                } else {
                    window.remove_window();
                }
            }
            return;
        };
        cx.stop_propagation();
        if event.is_held {
            return;
        }
        match shortcut_from_keystroke(&event.keystroke) {
            KeyResult::Cancel => self.end_capture(None, cx),
            KeyResult::Disable => self.end_capture(Some(String::new()), cx),
            KeyResult::Modifiers(preview) => {
                capture.preview = preview;
                capture.candidate = None;
                cx.notify();
            }
            KeyResult::Unsupported => {
                self.error = Some("不支持此按键，请使用其他按键。".into());
                capture.candidate = None;
                cx.notify();
            }
            KeyResult::Shortcut(value) => {
                // 先按完整设置校验（格式、与其他动作冲突），通过后等松开再保存。
                let mut draft = crate::shell::settings(cx);
                set_shortcut(&mut draft, capture.action, value.clone());
                match draft.validate() {
                    Ok(()) => {
                        self.error = None;
                        capture.preview = value.clone();
                        capture.candidate = Some(value);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        capture.candidate = None;
                    }
                }
                cx.notify();
            }
        }
    }

    /// 组合键全部松开后保存，避免把仍按住的按键当成新的全局触发。
    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let released = !(event.modifiers.control
            || event.modifiers.alt
            || event.modifiers.shift
            || event.modifiers.platform);
        if let Some(value) = self
            .capture
            .as_ref()
            .and_then(|c| c.candidate.clone())
            .filter(|_| released)
        {
            self.end_capture(Some(value), cx);
        }
    }

    fn key_up(&mut self, _: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = self.capture.as_ref() else {
            return;
        };
        cx.stop_propagation();
        // 主键松开且没有修饰键仍按住时保存；仍按住修饰键时等 ModifiersChanged。
        if let Some(value) = capture.candidate.clone() {
            let modifiers = window.modifiers();
            if !(modifiers.control || modifiers.alt || modifiers.shift || modifiers.platform) {
                self.end_capture(Some(value), cx);
            }
        }
    }
}

fn set_shortcut(settings: &mut Settings, action: ShortcutAction, value: String) {
    let keys = &mut settings.shortcuts;
    let slot = match action {
        ShortcutAction::Capture => &mut keys.capture,
        ShortcutAction::Pin => &mut keys.pin,
        ShortcutAction::Record => &mut keys.record,
        ShortcutAction::History => &mut keys.history,
        ShortcutAction::Translate => &mut keys.translate,
        ShortcutAction::Shelf => &mut keys.shelf,
        ShortcutAction::Ocr => &mut keys.ocr,
    };
    *slot = value;
}

#[derive(Debug, PartialEq)]
enum KeyResult {
    Cancel,
    Disable,
    /// 只按下了修饰键：显示已按下的部分。
    Modifiers(String),
    Unsupported,
    Shortcut(String),
}

/// GPUI 按键 → 设置中的快捷键写法（与 Vue `captureShortcut` 一致：`Ctrl+Alt+Shift+Super+主键`）。
fn shortcut_from_keystroke(keystroke: &Keystroke) -> KeyResult {
    let m = &keystroke.modifiers;
    let modifiers: Vec<&str> = [
        (m.control, "Ctrl"),
        (m.alt, "Alt"),
        (m.shift, "Shift"),
        (m.platform, "Super"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    let key = keystroke.key.as_str();
    if modifiers.is_empty() && key == "escape" {
        return KeyResult::Cancel;
    }
    if modifiers.is_empty() && matches!(key, "backspace" | "delete") {
        return KeyResult::Disable;
    }
    if matches!(
        key,
        "control" | "alt" | "shift" | "platform" | "cmd" | "win" | "super" | "fn" | ""
    ) {
        return KeyResult::Modifiers(modifiers.join(" + "));
    }
    let Some(main) = main_key(key) else {
        return KeyResult::Unsupported;
    };
    let value = modifiers
        .into_iter()
        .chain([main.as_str()])
        .collect::<Vec<_>>()
        .join("+");
    // 最终以 core 的解析为准（与全局快捷键库支持的主键一致）。
    match snap_core::settings::parse_shortcut(&value) {
        Ok(Some(_)) => KeyResult::Shortcut(value),
        _ => KeyResult::Unsupported,
    }
}

fn main_key(key: &str) -> Option<String> {
    let named = match key {
        "space" => "Space",
        "enter" => "Enter",
        "tab" => "Tab",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "insert" => "Insert",
        "-" => "Minus",
        "=" => "Equal",
        "," => "Comma",
        "." => "Period",
        "/" => "Slash",
        ";" => "Semicolon",
        "'" => "Quote",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "\\" => "Backslash",
        "`" => "Backquote",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_owned());
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase().to_string()),
        _ if key.len() <= 3
            && key.starts_with('f')
            && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) =>
        {
            Some(key.to_ascii_uppercase())
        }
        _ => None,
    }
}

/// 快捷键的显示写法（Ctrl + Shift + A）。
fn shortcut_label(value: &str) -> String {
    value.split('+').collect::<Vec<_>>().join(" + ")
}

// ---- 控件 ----

/// 设置行（`.setting-row`）：左侧标签与一行说明，右侧控件；最小高 44，行间细分隔线（最后一行由卡片裁掉）。
fn row(label: String, hint: Option<String>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .min_h(px(44.0))
        .py(px(7.0))
        .pr(px(12.0))
        .border_b_1()
        .border_color(rgb(col::sep()))
        .text_size(px(13.0))
        .child(
            div()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(2.0))
                .flex_1()
                .min_w_0()
                .child(div().child(label))
                .children(hint.map(|h| {
                    div()
                        .text_size(px(11.0))
                        .line_height(relative(1.5))
                        .text_color(rgb(col::muted()))
                        .child(h)
                })),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .child(control),
        )
}

/// 分组卡片（`.settings-card`）：左内边距 12，1px 边框，8 像素圆角；最后一行不画分隔线。
fn card(rows: Vec<Div>) -> Div {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .pl(px(12.0))
        .rounded(px(8.0))
        .bg(rgb(col::surface()))
        .border_1()
        .border_color(rgb(col::border()))
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(move |(index, row)| {
            if index + 1 == count {
                row.border_b_0()
            } else {
                row
            }
        }))
}

/// 分组标题（`.section-heading`）：12 像素灰色粗体。
fn heading(text: String) -> Div {
    div()
        .flex()
        .items_baseline()
        .justify_between()
        .gap(px(8.0))
        .mt(px(20.0))
        .mx(px(4.0))
        .mb(px(6.0))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(col::muted()))
        .child(text)
}

/// 分组下的说明文字（`.section-note`）。
fn note(text: String) -> Div {
    div()
        .mt(px(6.0))
        .mx(px(4.0))
        .text_size(px(11.0))
        .line_height(relative(1.5))
        .text_color(rgb(col::muted()))
        .child(text)
}

/// 按钮样式（Vue `Button`：default / outline / ghost / destructive，28 像素高）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Primary,
    Outline,
    Ghost,
}

fn button(
    id: impl Into<ElementId>,
    label: String,
    kind: ButtonKind,
    enabled: bool,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .h(px(28.0))
        .px(px(12.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .text_size(px(13.0))
        .child(label);
    let base = match kind {
        ButtonKind::Primary => base
            .bg(rgb(col::accent()))
            .text_color(rgb(0xffffff))
            .font_weight(FontWeight::MEDIUM),
        ButtonKind::Outline => base
            .border_1()
            .border_color(rgb(col::line()))
            .bg(rgb(col::surface()))
            .text_color(rgb(col::text())),
        ButtonKind::Ghost => base.text_color(rgb(col::muted())),
    };
    if !enabled {
        return base.opacity(0.45);
    }
    let hover = col::hover();
    let text = col::text();
    base.cursor_pointer().hover(move |s| match kind {
        ButtonKind::Primary => s.opacity(0.9),
        ButtonKind::Ghost => s.bg(rgb(hover)).text_color(rgb(text)),
        _ => s.bg(rgb(hover)),
    })
}

/// 快捷键标签（`.app-shell kbd`）。
fn kbd(text: String) -> Div {
    div()
        .h(px(20.0))
        .min_w(px(20.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .bg(rgb(col::hover()))
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(col::text()))
        .child(text)
}

/// 工具页顶部卡片（Vue `ToolHero`）：图标、名称与可选说明、快捷键与操作。
fn hero(
    icon: &'static [u8],
    title: String,
    meta: Option<String>,
    shortcut: Option<String>,
    actions: Vec<AnyElement>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.0))
        .mt(px(18.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(rgb(col::border()))
        .bg(rgb(col::surface()))
        .child(
            div()
                .size(px(40.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .bg(rgb(col::accent_soft()))
                .child(
                    Icon::default()
                        .data(icon)
                        .size(px(22.0))
                        .text_color(rgb(col::accent_fg())),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .children(meta.map(|m| div().text_xs().text_color(rgb(col::muted())).child(m))),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .children(
                    shortcut
                        .filter(|s| !s.is_empty())
                        .map(|s| kbd(shortcut_label(&s))),
                )
                .children(actions),
        )
}

impl SettingsView {
    /// 开关（`.switch-control`，34×20）。
    fn toggle(
        &self,
        id: impl Into<ElementId>,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings, bool) + 'static,
    ) -> Stateful<Div> {
        switch(id, on, false).on_click(
            cx.listener(move |view, _: &ClickEvent, _, cx| view.change(cx, |s| change(s, !on))),
        )
    }

    /// 分段控件（`.segmented-control`）：细线框，选中项淡色强调底。修改写入设置。
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        options: Vec<(T, String)>,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings, T) + Clone + 'static,
    ) -> Div {
        self.segmented_with(id, current, options, cx, move |view, value, cx| {
            let change = change.clone();
            view.change(cx, move |s| change(s, value));
        })
    }

    /// 分段控件本体：选中新值时调用 `pick`。
    fn segmented_with<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        options: Vec<(T, String)>,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Self, T, &mut Context<Self>) + Clone + 'static,
    ) -> Div {
        let mut group = div()
            .flex()
            .flex_wrap()
            .p(px(2.0))
            .gap(px(2.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(col::line()))
            .bg(rgb(col::surface()));
        for (index, (value, label)) in options.into_iter().enumerate() {
            let selected = value == current;
            let pick = pick.clone();
            let text = col::text();
            let mut item = div()
                .id((id, index))
                .min_h(px(22.0))
                .px(px(11.0))
                .flex()
                .items_center()
                .rounded(px(4.0))
                .text_xs()
                .cursor_pointer()
                .child(label);
            item = if selected {
                item.bg(rgb(col::accent_soft())).text_color(rgb(col::accent_fg()))
            } else {
                item.text_color(rgb(col::muted())).hover(move |s| s.text_color(rgb(text)))
            };
            group = group.child(item.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                if !selected {
                    pick(view, value, cx);
                }
            })));
        }
        group
    }

    /// 修改录制偏好（与录屏框选选项行共用 recording-preferences.json）。
    fn change_recording(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut snap_core::recording::preferences::Preferences)) {
        let mut preferences = crate::recording::preferences();
        change(&mut preferences);
        crate::recording::save_preferences(&preferences);
        self.error = None;
        self.saved = true;
        cx.notify();
    }

    fn rec_segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        options: Vec<(T, String)>,
        cx: &mut Context<Self>,
        change: impl Fn(&mut snap_core::recording::preferences::Preferences, T) + Clone + 'static,
    ) -> Div {
        self.segmented_with(id, current, options, cx, move |view, value, cx| {
            let change = change.clone();
            view.change_recording(cx, move |p| change(p, value));
        })
    }

    fn rec_toggle(
        &self,
        id: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut snap_core::recording::preferences::Preferences, bool) + 'static,
    ) -> Stateful<Div> {
        switch(id, on, false).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.change_recording(cx, |p| change(p, !on))))
    }

    /// 下拉选择（Vue `SelectControl`）：细线框触发器 + 浮层列表，选中项强调色。
    fn select<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        options: Vec<(T, String)>,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings, T) + Clone + 'static,
    ) -> Div {
        self.select_with(id, current, options, cx, move |view, value, cx| {
            let change = change.clone();
            view.change(cx, move |s| change(s, value));
        })
    }

    /// 下拉选择本体：选中新值时调用 `pick`（可修改视图状态而非设置）。
    fn select_with<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        options: Vec<(T, String)>,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Self, T, &mut Context<Self>) + Clone + 'static,
    ) -> Div {
        let open = self.open_select == Some(id);
        let label = options
            .iter()
            .find(|(v, _)| *v == current)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let trigger = div()
            .id(id)
            .min_w(px(150.0))
            .min_h(px(28.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(if open { col::accent() } else { col::line() }))
            .bg(rgb(col::surface()))
            .text_size(px(13.0))
            .cursor_pointer()
            .hover(|s| s.border_color(rgb(col::muted())))
            .child(div().truncate().child(label))
            .child(
                Icon::default()
                    .data(icons::CHEVRON)
                    .size(px(14.0))
                    .text_color(rgb(col::muted())),
            )
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.open_select = if view.open_select == Some(id) {
                    None
                } else {
                    Some(id)
                };
                cx.notify();
            }));
        let mut container = div().relative().child(trigger);
        if open {
            let mut list = div()
                .id((id, 1000usize))
                .absolute()
                .top(px(32.0))
                .right_0()
                .min_w(px(150.0))
                .p(px(5.0))
                .flex()
                .flex_col()
                .rounded(px(9.0))
                .border_1()
                .border_color(rgb(col::line()))
                .bg(rgb(col::surface()))
                .shadow_lg()
                .occlude();
            for (index, (value, label)) in options.into_iter().enumerate() {
                let selected = value == current;
                let pick = pick.clone();
                let hover = col::hover();
                let mut item = div()
                    .id((id, index))
                    .min_h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(20.0))
                    .rounded(px(5.0))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgb(hover)))
                    .child(label);
                if selected {
                    item = item
                        .bg(rgb(col::accent_soft()))
                        .text_color(rgb(col::accent_fg()))
                        .font_weight(FontWeight::MEDIUM)
                        .child(
                            Icon::default()
                                .data(icons::CHECK)
                                .size(px(14.0))
                                .text_color(rgb(col::accent_fg())),
                        );
                }
                list = list.child(item.on_click(cx.listener(
                    move |view, _: &ClickEvent, _, cx| {
                        view.open_select = None;
                        if !selected {
                            pick(view, value, cx);
                        } else {
                            cx.notify();
                        }
                    },
                )));
            }
            container = container.child(deferred(list).with_priority(1));
        }
        container
    }

    fn stepper(
        &self,
        id: &'static str,
        value: u32,
        (min, max, step): (u32, u32, u32),
        suffix: &str,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings, u32) + Clone + 'static,
    ) -> Div {
        let button = |index: usize,
                      label: &'static str,
                      next: u32,
                      enabled: bool,
                      change: Box<dyn Fn(&mut Settings, u32)>| {
            div()
                .id((id, index))
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(col::line()))
                .bg(rgb(col::surface()))
                .text_sm()
                .text_color(rgb(if enabled {
                    col::text()
                } else {
                    col::toggle_off()
                }))
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    if enabled {
                        view.change(cx, |s| change(s, next));
                    }
                }))
        };
        let down = value.saturating_sub(step).max(min);
        let up = value.saturating_add(step).min(max);
        let (c1, c2) = (change.clone(), change);
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(button(
                0,
                "−",
                down,
                value > min,
                Box::new(move |s, v| c1(s, v)),
            ))
            .child(
                div()
                    .min_w(px(64.0))
                    .text_center()
                    .text_sm()
                    .child(format!("{value}{suffix}")),
            )
            .child(button(
                1,
                "+",
                up,
                value < max,
                Box::new(move |s, v| c2(s, v)),
            ))
    }

    /// 文本框（1px 细线框，6 像素圆角，28 像素高）。
    fn text_input(&self, state: &Entity<InputState>, width: f32) -> Div {
        div()
            .w(px(width))
            .min_h(px(28.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(col::line()))
            .bg(rgb(col::surface()))
            .text_size(px(13.0))
            .hover(|s| s.border_color(rgb(col::muted())))
            .child(Input::new(state).appearance(false).small())
    }

    /// 默认保存目录：当前位置 + 更改…/恢复默认/打开。
    fn folder_row(
        &self,
        folder: Folder,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let (path, is_default) = folder.current(s);
        let location = match (&path, is_default) {
            (Some(path), false) => path.display().to_string(),
            (Some(path), true) => format!("{} · {}", t("默认"), path.display()),
            (None, _) => t("系统默认位置（保存对话框决定）"),
        };
        let (label, hint, id) = match folder {
            Folder::Captures => ("截图保存目录", "保存对话框的默认位置", 0usize),
            Folder::Recordings => ("录屏保存目录", "视频输出位置，恢复默认不会移动已有视频", 1),
        };
        let small =
            |id: (&'static str, usize), label: String| button(id, label, ButtonKind::Outline, true);
        let controls = div()
            .flex()
            .gap(px(6.0))
            .child(small(("folder-change", id), t("更改…")).on_click(
                cx.listener(move |view, _: &ClickEvent, _, cx| view.choose_folder(folder, cx)),
            ))
            .children((!is_default).then(|| {
                small(("folder-reset", id), t("恢复默认")).on_click(cx.listener(
                    move |view, _: &ClickEvent, _, cx| {
                        let settings = crate::shell::settings(cx);
                        let result = folder.configure(None, &settings);
                        view.report(result, cx);
                    },
                ))
            }))
            .child(self.open_button(("folder-open", id), t("打开"), path, cx));
        row(
            t(label),
            Some(format!("{} · {location}", t(hint))),
            controls,
        )
    }

    /// 系统文件夹选择器（期间隐藏置顶贴图，避免盖住对话框）。
    fn choose_folder(&mut self, folder: Folder, cx: &mut Context<Self>) {
        let settings = crate::shell::settings(cx);
        let language = settings.appearance.language.clone();
        let title = snap_core::i18n::for_language(
            &language,
            match folder {
                Folder::Captures => "选择截图保存文件夹",
                Folder::Recordings => "选择录屏保存文件夹",
            },
        )
        .to_owned();
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
        if let Some(current) = folder.current(&settings).0.filter(|p| p.is_dir()) {
            dialog = dialog.set_directory(current);
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
            let picked = dialog.pick_folder().await;
            cx.update(|cx| crate::pins::show_pins(cx, hidden));
            let Some(picked) = picked else {
                return;
            };
            let path = picked.path().to_path_buf();
            let _ = this.update(cx, |view, cx| {
                let settings = crate::shell::settings(cx);
                let result = folder.configure(Some(&path), &settings);
                view.report(result, cx);
            });
        })
        .detach();
    }

    fn report(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.error = None;
                self.saved = true;
            }
            Err(error) => {
                self.error = Some(error);
                self.saved = false;
            }
        }
        cx.notify();
    }

    fn open_button(
        &self,
        id: impl Into<ElementId>,
        label: String,
        path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        button(id, label, ButtonKind::Outline, true).on_click(cx.listener(
            move |view, _: &ClickEvent, _, cx| {
                let result = path
                    .clone()
                    .ok_or_else(|| "目录不可用。".to_owned())
                    .and_then(|p| {
                        std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                        snap_core::platform::open_directory(&p)
                    });
                if let Err(error) = result {
                    view.error = Some(error);
                    cx.notify();
                }
            },
        ))
    }
}

/// 开关（`.switch-control`）：34×20，`small` 为顶栏的 28×16；关闭时轨道为细线色。
fn switch(id: impl Into<ElementId>, on: bool, small: bool) -> Stateful<Div> {
    let (w, h, knob) = if small {
        (28.0, 16.0, 12.0)
    } else {
        (34.0, 20.0, 16.0)
    };
    let track = div()
        .id(id)
        .w(px(w))
        .h(px(h))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(h / 2.0))
        .p(px(2.0))
        .cursor_pointer()
        .bg(rgb(if on { col::accent() } else { col::line() }));
    let track = if on {
        track.justify_end()
    } else {
        track.justify_start()
    };
    track.child(
        div()
            .size(px(knob))
            .rounded_full()
            .bg(rgb(0xffffff))
            .shadow_sm(),
    )
}

// ---- 页面 ----

impl SettingsView {
    fn capture_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let jpeg = s.export.format == ExportFormat::Jpeg;
        let mut export = vec![row(
            t("保存格式"),
            None,
            self.segmented(
                "export-format",
                s.export.format,
                vec![
                    (ExportFormat::Png, "PNG".into()),
                    (ExportFormat::Jpeg, "JPEG".into()),
                    (ExportFormat::Webp, "WebP".into()),
                ],
                cx,
                |s, v| s.export.format = v,
            ),
        )];
        if jpeg {
            export.push(row(
                t("JPEG 质量"),
                None,
                self.stepper(
                    "jpeg-quality",
                    u32::from(s.export.jpeg_quality),
                    (1, 100, 5),
                    "",
                    cx,
                    |s, v| s.export.jpeg_quality = v as u8,
                ),
            ));
        }
        export.push(row(
            t("文件名前缀"),
            snap_core::export::filename(&s.export, "20260101-120000", 1920, 1080)
                .ok()
                .map(|name| format!("{}{name}", t("示例："))),
            self.text_input(&self.capture_prefix, 180.0),
        ));
        export.push(row(
            t("文件名包含图片尺寸"),
            None,
            self.toggle(
                "export-dimensions",
                s.export.include_dimensions,
                cx,
                |s, v| s.export.include_dimensions = v,
            ),
        ));
        export.push(row(
            t("记住上次保存目录"),
            None,
            self.toggle(
                "export-remember",
                s.export.remember_directory,
                cx,
                |s, v| s.export.remember_directory = v,
            ),
        ));
        vec![
            hero(
                icons::CAMERA,
                t("区域截图"),
                None,
                s.shortcuts_enabled.then(|| s.shortcuts.capture.clone()),
                vec![
                    button("hero-capture", t("开始截图"), ButtonKind::Primary, true)
                        .on_click(|_, _, cx| {
                            crate::shell::run_action(cx, ShortcutAction::Capture, "settings")
                        })
                        .into_any_element(),
                ],
            )
            .into_any_element(),
            heading(t("截图")).into_any_element(),
            card(vec![
                row(
                    t("延时"),
                    None,
                    self.segmented(
                        "delay",
                        s.delay_seconds,
                        vec![(0, t("无延时")), (3, t("3 秒")), (5, t("5 秒"))],
                        cx,
                        |s, v| s.delay_seconds = v,
                    ),
                ),
                row(
                    t("截图包含光标"),
                    None,
                    self.toggle("cursor", s.screenshot_cursor, cx, |s, v| {
                        s.screenshot_cursor = v
                    }),
                ),
                row(
                    t("窗口 / 元素吸附"),
                    None,
                    self.toggle("snap", s.screenshot_snap, cx, |s, v| s.screenshot_snap = v),
                ),
                row(
                    t("沿用上次选区"),
                    None,
                    self.toggle("reuse", s.screenshot_reuse_last, cx, |s, v| {
                        s.screenshot_reuse_last = v
                    }),
                ),
                row(
                    t("框选完成后"),
                    (s.screenshot_after != "none").then(|| {
                        t("按住 {0} 框选可保留工具栏").replace(
                            "{0}",
                            if cfg!(target_os = "macos") {
                                "⌘"
                            } else {
                                "Ctrl"
                            },
                        )
                    }),
                    self.select(
                        "after",
                        after_index(&s.screenshot_after),
                        vec![
                            (0, t("显示工具栏")),
                            (1, t("直接复制")),
                            (2, t("直接贴图")),
                            (3, t("直接保存")),
                        ],
                        cx,
                        |s, v| s.screenshot_after = AFTER[v].into(),
                    ),
                ),
            ])
            .into_any_element(),
            heading(t("文件与导出")).into_any_element(),
            card(export).into_any_element(),
        ]
    }

    fn recording_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        use snap_core::recording::config::{EncodingPreference, KeyPosition, OutputFormat, RecordingQuality};
        let e = &s.recording_export;
        let p = crate::recording::preferences();
        let mp4 = p.format == OutputFormat::Mp4;
        // 视频与导出、录制行为（Vue `RecordingPreferences.vue`）。
        let mut video = vec![
            row(
                t("保存格式"),
                None,
                self.rec_segmented("rec-format", p.format, vec![(OutputFormat::Mp4, "MP4".into()), (OutputFormat::Gif, "GIF".into())], cx, |p, v| p.format = v),
            ),
            row(
                t("帧率"),
                None,
                if mp4 {
                    self.rec_segmented("rec-fps", p.mp4_fps, vec![(30, "30 FPS".into()), (60, "60 FPS".into())], cx, |p, v| p.mp4_fps = v)
                } else {
                    self.rec_segmented("rec-fps", p.gif_fps, vec![(10, "10 FPS".into()), (20, "20 FPS".into())], cx, |p, v| p.gif_fps = v)
                },
            ),
        ];
        if mp4 {
            video.push(row(
                t("画质"),
                None,
                self.rec_segmented(
                    "rec-quality",
                    p.quality,
                    vec![(RecordingQuality::Economy, t("节省空间")), (RecordingQuality::Balanced, t("均衡")), (RecordingQuality::High, t("高清"))],
                    cx,
                    |p, v| {
                        p.quality = v;
                        p.mbps = None;
                    },
                ),
            ));
            video.push(row(
                t("编码方式"),
                None,
                self.rec_segmented(
                    "rec-encoding",
                    p.encoding,
                    vec![(EncodingPreference::Auto, t("自动编码")), (EncodingPreference::Software, t("软件兼容"))],
                    cx,
                    |p, v| p.encoding = v,
                ),
            ));
        }
        let behavior = vec![
            row(
                t("倒计时"),
                None,
                self.rec_segmented("rec-countdown", p.countdown, vec![(0, t("无延时")), (3, t("{0} 秒").replace("{0}", "3")), (5, t("{0} 秒").replace("{0}", "5"))], cx, |p, v| p.countdown = v),
            ),
            row(t("录制光标"), None, self.rec_toggle("rec-cursor", p.capture_cursor, cx, |p, v| p.capture_cursor = v)),
        ];
        // 键鼠特效（Vue `RecordingInputSettings.vue` 的设置行形态）。
        let fx = &p.effects;
        let mut effects = Vec::new();
        for (index, (label, on)) in [("点击光圈", fx.clicks), ("鼠标移动尾迹", fx.movement), ("按住拖动轨迹", fx.drag), ("滚轮方向提示", fx.wheel)].into_iter().enumerate() {
            let id: &'static str = ["fx-clicks", "fx-movement", "fx-drag", "fx-wheel"][index];
            effects.push(row(
                t(label),
                None,
                self.rec_toggle(id, on, cx, move |p, v| match index {
                    0 => p.effects.clicks = v,
                    1 => p.effects.movement = v,
                    2 => p.effects.drag = v,
                    _ => p.effects.wheel = v,
                }),
            ));
        }
        effects.push(row(t("按键提示"), Some(t("不记录普通文字输入")), self.rec_toggle("fx-keyboard", fx.keyboard, cx, |p, v| p.effects.keyboard = v)));
        if fx.keyboard {
            effects.push(row(
                t("按键位置"),
                None,
                self.rec_segmented(
                    "fx-position",
                    fx.position,
                    vec![(KeyPosition::TopLeft, t("左上角")), (KeyPosition::BottomLeft, t("左下角")), (KeyPosition::BottomRight, t("右下角"))],
                    cx,
                    |p, v| p.effects.position = v,
                ),
            ));
        }
        if fx.keyboard || fx.clicks || fx.movement || fx.drag || fx.wheel {
            effects.push(row(
                t("效果尺寸"),
                None,
                self.rec_segmented("fx-size", fx.size, [16, 24, 32, 48, 64].into_iter().map(|v| (v, format!("{v} px"))).collect(), cx, |p, v| p.effects.size = v),
            ));
            effects.push(row(
                t("持续时间"),
                None,
                self.rec_segmented("fx-duration", fx.duration_ms, [400, 800, 1200, 2000].into_iter().map(|v| (v, format!("{v} ms"))).collect(), cx, |p, v| p.effects.duration_ms = v),
            ));
        }
        // 特效颜色（Vue 用系统取色器；GPUI 没有原生取色控件，改为色板，含各自的默认色）。
        const FX_COLORS: [[u8; 3]; 8] = [
            [64, 160, 255],
            [255, 112, 80],
            [190, 100, 255],
            [34, 197, 94],
            [250, 204, 21],
            [236, 72, 153],
            [255, 255, 255],
            [24, 24, 27],
        ];
        let colors: [(&str, [u8; 3], bool); 3] = [
            ("左键 / 移动 / 滚轮", fx.left_color, fx.clicks || fx.movement || fx.wheel),
            ("右键 / 拖动", fx.right_color, fx.clicks || fx.drag),
            ("双击", fx.double_color, fx.clicks),
        ];
        for (field, (label, current, shown)) in colors.into_iter().enumerate() {
            if !shown {
                continue;
            }
            let mut swatches = div().flex().items_center().gap(px(6.0));
            for (index, color) in FX_COLORS.into_iter().enumerate() {
                let selected = color == current;
                let value = (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]);
                swatches = swatches.child(
                    div()
                        .id(("fx-color", field * 16 + index))
                        .size(px(20.0))
                        .rounded_full()
                        .bg(rgb(value))
                        .border_2()
                        .border_color(rgb(if selected { col::accent() } else { col::border() }))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.change_recording(cx, |p| match field {
                                0 => p.effects.left_color = color,
                                1 => p.effects.right_color = color,
                                _ => p.effects.double_color = color,
                            })
                        })),
                );
            }
            let hex = format!("#{:02X}{:02X}{:02X}", current[0], current[1], current[2]);
            effects.push(row(
                t(label),
                None,
                swatches.child(div().min_w(px(56.0)).text_xs().text_color(rgb(col::muted())).child(hex)),
            ));
        }
        let reset = div()
            .id("fx-reset")
            .text_xs()
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(col::accent_fg()))
            .cursor_pointer()
            .child(t("恢复默认"))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.change_recording(cx, |p| p.effects = Default::default())));
        // 录屏进行中：主操作换成开始/暂停/恢复/停止并导出/取消并清理（Vue `RecordingPanel.vue`）。
        use snap_core::recording::{Phase, RecordingAction};
        let control = |id: &'static str, label: &str, kind: ButtonKind, action: RecordingAction, cx: &mut Context<Self>| {
            button(id, t(label), kind, true)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    let result = crate::recording::control(cx, action);
                    view.report(result, cx);
                }))
                .into_any_element()
        };
        let record_actions = match crate::recording::phase(cx) {
            None => vec![button("hero-record", t("选择录制范围"), ButtonKind::Primary, true)
                .on_click(|_, _, cx| crate::shell::run_action(cx, ShortcutAction::Record, "settings"))
                .into_any_element()],
            Some(phase) => {
                let mut actions = Vec::new();
                if phase == Phase::Ready {
                    actions.push(control("rec-start", "开始录制", ButtonKind::Primary, RecordingAction::Start, cx));
                }
                if phase == Phase::Recording {
                    actions.push(control("rec-pause", "暂停", ButtonKind::Outline, RecordingAction::Pause, cx));
                }
                if phase == Phase::Paused {
                    actions.push(control("rec-resume", "恢复", ButtonKind::Outline, RecordingAction::Resume, cx));
                }
                if matches!(phase, Phase::Recording | Phase::Paused) {
                    actions.push(control("rec-stop", "停止并导出", ButtonKind::Primary, RecordingAction::Stop, cx));
                }
                if phase != Phase::Encoding {
                    actions.push(control("rec-cancel", "取消并清理", ButtonKind::Outline, RecordingAction::Cancel, cx));
                }
                actions
            }
        };
        vec![
            hero(
                icons::VIDEO,
                t("录屏"),
                None,
                s.shortcuts_enabled.then(|| s.shortcuts.record.clone()),
                record_actions,
            )
            .into_any_element(),
            heading(t("视频与导出")).into_any_element(),
            card(video).into_any_element(),
            heading(t("录制行为")).into_any_element(),
            card(behavior).into_any_element(),
            heading(t("键鼠特效")).child(reset).into_any_element(),
            card(effects).into_any_element(),
            heading(t("文件")).into_any_element(),
            card(vec![
                row(t("文件名前缀"), None, self.text_input(&self.recording_prefix, 180.0)),
                row(
                    t("文件名包含视频尺寸"),
                    None,
                    self.toggle("rec-dimensions", e.include_dimensions, cx, |s, v| s.recording_export.include_dimensions = v),
                ),
                row(
                    t("记住上次保存目录"),
                    None,
                    self.toggle("rec-remember", e.remember_directory, cx, |s, v| s.recording_export.remember_directory = v),
                ),
                self.folder_row(Folder::Recordings, s, t, cx),
            ])
            .into_any_element(),
        ]
    }

    fn pins_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let a = &s.pin_appearance;
        let mut swatches = div().flex().gap(px(6.0));
        for (index, color) in BORDER_COLORS.iter().enumerate() {
            let selected = a.border_color.eq_ignore_ascii_case(color);
            let value = u32::from_str_radix(&color[1..], 16).unwrap_or(0);
            swatches = swatches.child(
                div()
                    .id(("pin-color", index))
                    .size(px(20.0))
                    .rounded_full()
                    .bg(rgb(value))
                    .border_2()
                    .border_color(rgb(if selected {
                        col::accent()
                    } else {
                        col::border()
                    }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.change(cx, |s| s.pin_appearance.border_color = (*color).to_owned())
                    })),
            );
        }
        vec![
            hero(
                icons::PIN,
                t("剪贴板贴图"),
                None,
                s.shortcuts_enabled.then(|| s.shortcuts.pin.clone()),
                vec![
                    button(
                        "hero-pin-file",
                        t("打开图片贴图"),
                        ButtonKind::Outline,
                        true,
                    )
                    .on_click(|_, _, cx| crate::pins::open_file(cx))
                    .into_any_element(),
                    button("hero-pin", t("剪贴板贴图"), ButtonKind::Primary, true)
                        .on_click(|_, _, cx| {
                            crate::shell::run_action(cx, ShortcutAction::Pin, "settings")
                        })
                        .into_any_element(),
                ],
            )
            .into_any_element(),
            heading(t("贴图外观")).into_any_element(),
            card(vec![
                row(
                    t("阴影"),
                    None,
                    self.segmented(
                        "pin-shadow",
                        shadow_index(&a.shadow),
                        vec![(0, t("自动")), (1, t("柔和")), (2, t("无"))],
                        cx,
                        |s, v| {
                            s.pin_appearance.shadow =
                                [PinShadow::Auto, PinShadow::Soft, PinShadow::None][v].clone()
                        },
                    ),
                ),
                row(
                    t("圆角"),
                    None,
                    self.stepper(
                        "pin-radius",
                        u32::from(a.corner_radius),
                        (0, 24, 2),
                        " px",
                        cx,
                        |s, v| s.pin_appearance.corner_radius = v as u8,
                    ),
                ),
                row(t("边框颜色"), None, swatches),
            ])
            .into_any_element(),
            div()
                .mt(px(10.0))
                .text_xs()
                .text_color(rgb(col::muted()))
                .child(t("新外观用于之后创建的贴图。"))
                .into_any_element(),
        ]
    }

    fn clipboard_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        use snap_core::shelf::ShelfCategory;
        let c = &s.clipboard;
        let status = crate::clipboard::service(cx).map(|service| service.status());
        let meta = status.as_ref().map(|status| {
            format!("{} {} {:.1} MiB", status.count, t("条 ·"), status.byte_size as f64 / 1_048_576.0)
        });
        // 记录状态（Vue：正在记录 / 已暂停 / 当前平台不支持）。
        let recording_status = match &status {
            Some(st) if st.supported && st.enabled => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_xs()
                .text_color(rgb(col::success()))
                .child(Icon::default().data(icons::CHECK).size(px(14.0)).text_color(rgb(col::success())))
                .child(t("正在记录")),
            Some(st) if !st.supported => div().text_xs().text_color(rgb(col::muted())).child(t("当前平台不支持")),
            Some(_) => div().text_xs().text_color(rgb(col::muted())).child(t("已暂停")),
            None => div().text_xs().text_color(rgb(col::muted())).child(t("状态未读取")),
        };
        // 顶部暂存架（Vue `ShelfSettings.vue`）。
        let shelf = crate::shelf::settings(cx);
        let mut shelf_rows = Vec::new();
        if let Some(shelf) = shelf {
            let enabled = shelf.enabled;
            let hover = shelf.top_hover;
            let toggle_enabled = shelf.clone();
            let toggle_hover = shelf.clone();
            shelf_rows.push(row(
                t("开启顶部暂存架"),
                None,
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(button("shelf-open", t("展开暂存架"), ButtonKind::Outline, enabled).on_click(|_, _, cx| crate::shelf::open(cx)))
                    .child(switch("shelf-enabled", enabled, false).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        let mut next = toggle_enabled.clone();
                        next.enabled = !enabled;
                        let result = crate::shelf::set_settings(cx, next);
                        view.report(result, cx);
                    }))),
            ));
            let mut hover_switch = switch("shelf-hover", hover, false);
            hover_switch = if enabled {
                hover_switch.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    let mut next = toggle_hover.clone();
                    next.top_hover = !hover;
                    let result = crate::shelf::set_settings(cx, next);
                    view.report(result, cx);
                }))
            } else {
                hover_switch.opacity(0.45)
            };
            shelf_rows.push(row(t("鼠标停留屏幕顶部中央时展开"), None, hover_switch));
            const CATEGORIES: [(ShelfCategory, &str); 7] = [
                (ShelfCategory::Text, "文本"),
                (ShelfCategory::Image, "图片"),
                (ShelfCategory::File, "文件"),
                (ShelfCategory::Link, "链接"),
                (ShelfCategory::Path, "文件路径"),
                (ShelfCategory::Color, "颜色"),
                (ShelfCategory::Favorites, "收藏"),
            ];
            let mut options = div().flex().flex_wrap().justify_end().gap(px(10.0)).max_w(px(420.0));
            for (index, (category, label)) in CATEGORIES.into_iter().enumerate() {
                let checked = shelf.categories.contains(&category);
                let current = shelf.clone();
                let (border, fill) = if checked { (col::success(), col::notice()) } else { (col::line(), col::surface()) };
                options = options.child(
                    div()
                        .id(("shelf-category", index))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_xs()
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(14.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.0))
                                .border_1()
                                .border_color(rgb(border))
                                .bg(rgb(fill))
                                .children(checked.then(|| Icon::default().data(icons::CHECK).size(px(10.0)).text_color(rgb(col::success())))),
                        )
                        .child(t(label))
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            let mut next = current.clone();
                            if checked {
                                next.categories.retain(|c| *c != category);
                            } else {
                                next.categories.push(category);
                            }
                            let result = crate::shelf::set_settings(cx, next);
                            view.report(result, cx);
                        })),
                );
            }
            shelf_rows.push(row(t("放置类别"), None, options));
        }
        let mut elements = vec![
            hero(
                icons::CLIPBOARD,
                t("剪贴板历史"),
                meta,
                s.shortcuts_enabled.then(|| s.shortcuts.history.clone()),
                vec![button("hero-clipboard", t("打开剪贴板"), ButtonKind::Primary, true)
                    .on_click(|_, _, cx| crate::shell::run_action(cx, ShortcutAction::History, "settings"))
                    .into_any_element()],
            )
            .into_any_element(),
            heading(t("记录")).into_any_element(),
            card(vec![
                row(
                    t("记录剪贴板历史"),
                    None,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(recording_status)
                        .child(self.toggle("clip-enabled", c.enabled, cx, |s, v| s.clipboard.enabled = v)),
                ),
                row(t("记录文本"), None, self.toggle("clip-text", c.save_copied_text, cx, |s, v| s.clipboard.save_copied_text = v)),
                row(t("记录图片"), None, self.toggle("clip-images", c.save_copied_images, cx, |s, v| s.clipboard.save_copied_images = v)),
                row(t("记录文件"), None, self.toggle("clip-files", c.save_copied_files, cx, |s, v| s.clipboard.save_copied_files = v)),
            ])
            .into_any_element(),
            heading(t("自动清理")).into_any_element(),
            card(vec![
                row(t("最多条目"), None, self.stepper("clip-items", c.max_items, (100, 5000, 100), "", cx, |s, v| s.clipboard.max_items = v)),
                row(t("容量上限（MiB）"), None, self.stepper("clip-storage", c.max_storage_mb, (50, 2048, 50), "", cx, |s, v| s.clipboard.max_storage_mb = v)),
                row(t("保留天数（0 为不限）"), None, self.stepper("clip-days", c.retain_days, (0, 3650, 1), "", cx, |s, v| s.clipboard.retain_days = v)),
            ])
            .into_any_element(),
            heading(t("使用")).into_any_element(),
            card(vec![
                row(
                    t("回车或双击后直接粘贴到原窗口"),
                    None,
                    self.toggle("clip-paste", c.paste_on_activate, cx, |s, v| s.clipboard.paste_on_activate = v),
                ),
                row(
                    t("不记录这些程序复制的内容"),
                    Some(t("填写程序文件名，用逗号分隔")),
                    self.text_input(&self.excluded, 240.0),
                ),
            ])
            .into_any_element(),
        ];
        if !shelf_rows.is_empty() {
            elements.push(heading(t("顶部暂存架")).into_any_element());
            elements.push(card(shelf_rows).into_any_element());
        }
        elements
    }

    fn general_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // 文件与存储（与 Tauri 版 `app_directories` 顺序一致，日志目录在日志页）。
        let mut directories = Vec::new();
        let data = snap_core::paths::local_data_dir();
        let fixed: [(&'static str, Option<PathBuf>); 2] = [
            ("应用配置目录", crate::shell::settings_dir()),
            ("应用数据目录", data.clone()),
        ];
        for (index, (name, path)) in fixed.into_iter().enumerate() {
            directories.push(self.directory_row(("dir", index), t(name), path, None, t, cx));
        }
        for (index, folder) in [Folder::Captures, Folder::Recordings]
            .into_iter()
            .enumerate()
        {
            let path = folder.current(s).0;
            let name = match folder {
                Folder::Captures => "截图保存目录",
                Folder::Recordings => "录屏保存目录",
            };
            directories.push(self.directory_row(
                ("dir", 10 + index),
                t(name),
                path,
                Some(folder),
                t,
                cx,
            ));
        }
        let clipboard = std::env::var_os("SNAP_DESK_CLIPBOARD_DIR")
            .map(PathBuf::from)
            .or_else(|| data.as_ref().map(|d| d.join("clipboard")));
        for (index, (name, path)) in [
            (
                "滚动截图结果目录",
                data.as_ref().map(|d| d.join("scroll-results")),
            ),
            ("剪贴板历史目录", clipboard),
        ]
        .into_iter()
        .enumerate()
        {
            directories.push(self.directory_row(("dir", 20 + index), t(name), path, None, t, cx));
        }
        let autostart_status = match self.autostart {
            Some(true) => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_xs()
                .text_color(rgb(col::success()))
                .child(
                    Icon::default()
                        .data(icons::CHECK)
                        .size(px(14.0))
                        .text_color(rgb(col::success())),
                )
                .child(t("已开启")),
            Some(false) => div()
                .text_xs()
                .text_color(rgb(col::muted()))
                .child(t("已关闭")),
            None => div()
                .text_xs()
                .text_color(rgb(col::muted()))
                .child(t("状态未读取")),
        };
        let include = self.include_secrets;
        vec![
            heading(t("外观与语言")).into_any_element(),
            card(vec![
                row(
                    t("主题"),
                    None,
                    self.segmented(
                        "theme",
                        match s.appearance.theme {
                            Theme::System => 0,
                            Theme::Light => 1,
                            Theme::Dark => 2,
                        },
                        vec![(0, t("跟随系统")), (1, t("浅色")), (2, t("深色"))],
                        cx,
                        |s, v| {
                            s.appearance.theme =
                                [Theme::System, Theme::Light, Theme::Dark][v].clone()
                        },
                    ),
                ),
                row(
                    t("语言"),
                    None,
                    self.select(
                        "language",
                        matches!(s.appearance.language, Language::English),
                        vec![(false, t("简体中文")), (true, "English".into())],
                        cx,
                        |s, english| {
                            s.appearance.language = if english {
                                Language::English
                            } else {
                                Language::Chinese
                            }
                        },
                    ),
                ),
            ])
            .into_any_element(),
            heading(t("启动")).into_any_element(),
            card(vec![row(
                t("开机自启"),
                None,
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(autostart_status)
                    .children(self.autostart.map(|on| {
                        self.plain_toggle("autostart", on, cx, |view, on, cx| {
                            view.set_autostart(on, cx)
                        })
                    })),
            )])
            .into_any_element(),
            heading(t("导出字体")).into_any_element(),
            card(vec![self.font_resource_row(t, cx)]).into_any_element(),
            heading(t("文件与存储")).into_any_element(),
            card(directories).into_any_element(),
            heading(t("配置备份")).into_any_element(),
            card(vec![
                row(
                    t("导入或导出全部设置"),
                    None,
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(
                            self.action_button("config-import", t("导入配置"), !self.transferring)
                                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                    view.import_config(cx)
                                })),
                        )
                        .child(
                            self.action_button("config-export", t("导出配置"), !self.transferring)
                                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                    view.export_config(cx)
                                })),
                        ),
                ),
                row(
                    t("导出时包含服务密钥"),
                    Some(t("备份文件将含明文密钥，仅在自己的设备间迁移时使用。")),
                    switch("include-secrets", include, true).on_click(cx.listener(
                        move |view, _: &ClickEvent, _, cx| {
                            view.include_secrets = !include;
                            cx.notify();
                        },
                    )),
                ),
            ])
            .into_any_element(),
            note(t(if include {
                "导出包含服务密钥；不包含历史记录和图片视频。"
            } else {
                "默认不包含密钥、历史记录和图片视频。"
            }))
            .into_any_element(),
        ]
    }

    /// 文件与存储的一行：名称、等宽小字路径；打开、复制路径，保存目录另有选择与恢复默认。
    fn directory_row(
        &self,
        id: (&'static str, usize),
        name: String,
        path: Option<PathBuf>,
        folder: Option<Folder>,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let text = path.as_ref().map(|p| p.display().to_string());
        let has = path.is_some();
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap(px(6.0))
            .child(self.open_button((id.0, id.1 * 4), t("打开"), path.clone(), cx));
        let mut copy = button((id.0, id.1 * 4 + 1), t("复制路径"), ButtonKind::Ghost, has);
        if let Some(value) = text.clone() {
            copy = copy.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
                view.error = None;
                view.notice = Some("目录路径已复制。".into());
                cx.notify();
            }));
        }
        actions = actions.child(copy);
        if let Some(folder) = folder {
            actions =
                actions
                    .child(
                        button((id.0, id.1 * 4 + 2), t("选择"), ButtonKind::Outline, true)
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                view.choose_folder(folder, cx)
                            })),
                    )
                    .child(
                        button((id.0, id.1 * 4 + 3), t("恢复默认"), ButtonKind::Ghost, true)
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                let settings = crate::shell::settings(cx);
                                let result = folder.configure(None, &settings);
                                view.report(result, cx);
                            })),
                    );
        }
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .min_h(px(44.0))
            .py(px(7.0))
            .pr(px(12.0))
            .border_b_1()
            .border_color(rgb(col::sep()))
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.0))
                    .flex()
                    .flex_col()
                    .child(name)
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(11.0))
                            .line_height(relative(1.5))
                            .font_family("Consolas")
                            .text_color(rgb(col::muted()))
                            .child(text.unwrap_or_else(|| t("系统默认位置"))),
                    ),
            )
            .child(actions)
    }

    /// 不经设置保存的本地开关（登录项、导出选项）。
    fn plain_toggle(
        &self,
        id: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, bool, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        switch(id, on, false)
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| change(view, !on, cx)))
    }

    fn action_button(&self, id: &'static str, label: String, enabled: bool) -> Stateful<Div> {
        button(id, label, ButtonKind::Outline, enabled)
    }

    fn shortcuts_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let unavailable = crate::shell::unavailable_shortcuts(cx);
        let mut rows = Vec::new();
        for (index, (action, name, value)) in s.shortcuts.entries().into_iter().enumerate() {
            let capturing = self.capture.as_ref().filter(|c| c.action == action);
            let editing = capturing.is_some();
            // 按键框（Vue `ShortcutInput`）：每个键一个 kbd；录入中显示预览与帮助。
            let mut field = div()
                .id(("shortcut", index))
                .min_w(px(120.0))
                .h(px(26.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .gap(px(3.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(if editing { col::accent() } else { col::line() }))
                .bg(rgb(if editing {
                    col::accent_soft()
                } else {
                    col::surface()
                }))
                .cursor_pointer()
                .hover(|s| s.border_color(rgb(col::muted())))
                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    view.begin_capture(action, window, cx)
                }));
            let placeholder = |text: String| {
                div()
                    .px(px(4.0))
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .child(text)
            };
            field = match capturing {
                Some(c) if c.preview.is_empty() => field.child(placeholder(t("请按快捷键…"))),
                Some(c) => field.child(placeholder(shortcut_label(&c.preview))),
                None if value.is_empty() => field.child(placeholder(t("未设置"))),
                None => field.children(value.split('+').map(|key| {
                    div()
                        .h(px(18.0))
                        .min_w(px(18.0))
                        .px(px(5.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .bg(rgb(col::hover()))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(key.to_owned())
                })),
            };
            let control = div()
                .flex()
                .flex_col()
                .items_end()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(field)
                        .children((!value.is_empty()).then(|| {
                            let text = col::text();
                            div()
                                .id(("shortcut-clear", index))
                                .size(px(18.0))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rgb(col::hover()))
                                .text_color(rgb(col::muted()))
                                .cursor_pointer()
                                .hover(move |s| s.text_color(rgb(text)))
                                .child(Icon::default().data(icons::X).size(px(12.0)))
                                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                    view.change(cx, |s| set_shortcut(s, action, String::new()))
                                }))
                        })),
                )
                .children(editing.then(|| {
                    div()
                        .mt(px(4.0))
                        .text_size(px(11.0))
                        .text_color(rgb(col::muted()))
                        .child(t("单按 Esc 取消 · 单按 Backspace / Delete 禁用"))
                }));
            let label = div().flex().items_center().child(t(name)).children(
                unavailable.contains(&action).then(|| {
                    div()
                        .ml(px(8.0))
                        .text_xs()
                        .text_color(rgb(col::danger()))
                        .child(t("已被占用"))
                }),
            );
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.0))
                    .min_h(px(44.0))
                    .py(px(7.0))
                    .pr(px(12.0))
                    .border_b_1()
                    .border_color(rgb(col::sep()))
                    .child(label)
                    .child(control),
            );
        }
        let reset = div()
            .id("shortcuts-reset")
            .text_xs()
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(col::accent_fg()))
            .cursor_pointer()
            .child(t("恢复默认"))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.change(cx, |s| {
                    s.shortcuts = snap_core::settings::Shortcuts::default()
                })
            }));
        vec![
            heading(t("全局快捷键")).child(reset).into_any_element(),
            card(rows).into_any_element(),
            heading(t("排查")).into_any_element(),
            card(self.probe_rows(t, cx)).into_any_element(),
        ]
    }
}

const AFTER: [&str; 4] = ["none", "copy", "pin", "save"];

fn after_index(value: &str) -> usize {
    AFTER.iter().position(|v| *v == value).unwrap_or(0)
}

fn shadow_index(value: &PinShadow) -> usize {
    match value {
        PinShadow::Auto => 0,
        PinShadow::Soft => 1,
        PinShadow::None => 2,
    }
}

/// 可更改的默认保存目录。记录文件与 Tauri 版共用（配置目录下）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Folder {
    Captures,
    Recordings,
}

impl Folder {
    /// 当前目录与是否为默认。截图没有固定默认目录（交给保存对话框），返回 None。
    fn current(self, s: &Settings) -> (Option<PathBuf>, bool) {
        match self {
            Folder::Captures => {
                let path = snap_core::export::directory_record().and_then(|record| {
                    snap_core::export::preferred_directory(&record, s.export.remember_directory)
                });
                let is_default = path.is_none();
                (path, is_default)
            }
            Folder::Recordings => {
                let (Some(record), Some(data)) = (
                    crate::shell::recording_record(),
                    snap_core::paths::local_data_dir(),
                ) else {
                    return (None, true);
                };
                match snap_core::recording::output::resolve(
                    &record,
                    s.recording_export.remember_directory,
                    data.join("recordings"),
                ) {
                    Ok((path, is_default)) => (Some(path), is_default),
                    Err(_) => (None, true),
                }
            }
        }
    }

    /// 设为 `directory`（None 为恢复默认）；目录须可写，失败不改记录。
    fn configure(self, directory: Option<&Path>, s: &Settings) -> Result<(), String> {
        match self {
            Folder::Captures => {
                let record = snap_core::export::directory_record().ok_or("无法定位配置目录。")?;
                snap_core::export::configure_directory_record(&record, directory)
            }
            Folder::Recordings => {
                let record = crate::shell::recording_record().ok_or("无法定位配置目录。")?;
                snap_core::recording::output::configure_directory(
                    &record,
                    directory,
                    s.recording_export.remember_directory,
                )
            }
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // “跟随系统”时系统外观可能已切换；其他窗口在保存设置时统一重绘。
        crate::theme::refresh(cx);
        let settings = crate::shell::settings(cx);
        if std::mem::take(&mut self.reload_inputs) {
            // set_value 不触发 Change，不会把重填当成编辑再次保存。
            for (state, value) in [
                (
                    &self.capture_prefix,
                    settings.export.filename_prefix.clone(),
                ),
                (
                    &self.recording_prefix,
                    settings.recording_export.filename_prefix.clone(),
                ),
                (&self.excluded, settings.clipboard.excluded_apps.join(", ")),
                (&self.update_mirror, settings.update_network.mirror.clone()),
                (&self.update_proxy, settings.update_network.proxy.clone()),
            ] {
                state.update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
        let language = Self::language(cx);
        let words = language.clone();
        let has_update = crate::updater::view(cx).has_update();
        if self.page == Page::Recording && !self.recording_poll && crate::recording::phase(cx).is_some() {
            self.recording_poll = true;
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep = this
                    .update(cx, |view, cx| {
                        cx.notify();
                        let keep = view.page == Page::Recording && crate::recording::phase(cx).is_some();
                        view.recording_poll = keep;
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            })
            .detach();
        }
        let t = move |key: &str| snap_core::i18n::for_language(&words, key).to_owned();
        let query = self.nav_search.read(cx).value().to_string();
        let mut nav = div()
            .id("settings-sidebar")
            .w(px(196.0))
            .flex_none()
            .flex()
            .flex_col()
            .pt(px(14.0))
            .px(px(10.0))
            .pb(px(12.0))
            .border_r_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::nav()))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .pt(px(2.0))
                    .px(px(8.0))
                    .pb(px(12.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .size(px(22.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(rgb(col::accent()))
                            .child(
                                Icon::default()
                                    .data(icons::SCAN)
                                    .size(px(14.0))
                                    .text_color(rgb(0xffffff)),
                            ),
                    )
                    .child("Snap Desk"),
            )
            .child(
                div()
                    .h(px(30.0))
                    .mb(px(6.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(col::line()))
                    .bg(rgb(col::surface()))
                    .text_size(px(13.0))
                    .child(
                        Icon::default()
                            .data(icons::SEARCH)
                            .size(px(14.0))
                            .text_color(rgb(col::muted())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.nav_search).appearance(false).small()),
                    ),
            );
        let mut any = false;
        for (group, pages) in GROUPS {
            let visible: Vec<Page> = pages
                .iter()
                .copied()
                .filter(|p| p.matches(&query, &t(p.label())))
                .collect();
            if visible.is_empty() {
                continue;
            }
            any = true;
            nav = nav.child(
                div()
                    .mt(px(12.0))
                    .mx(px(8.0))
                    .mb(px(4.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(col::muted()))
                    .child(t(group)),
            );
            for page in visible {
                let active = page == self.page;
                let index = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
                let hover = col::hover_nav();
                let mut item = div()
                    .id(("nav", index))
                    .min_h(px(30.0))
                    .mb(px(1.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .child(
                        Icon::default()
                            .data(page.icon())
                            .size(px(16.0))
                            .text_color(rgb(if active {
                                col::accent_fg()
                            } else {
                                col::muted()
                            })),
                    )
                    .child(t(page.label()))
                    // 有新版本时“关于与帮助”带红点（Vue `.nav-dot`）。
                    .children((page == Page::About && has_update).then(|| {
                        div().ml_auto().size(px(6.0)).flex_none().rounded_full().bg(rgb(0xdc2626))
                    }));
                item = if active {
                    item.bg(rgb(col::accent_soft()))
                        .text_color(rgb(col::accent_fg()))
                        .font_weight(FontWeight::MEDIUM)
                } else {
                    item.text_color(rgb(col::text()))
                        .hover(move |s| s.bg(rgb(hover)))
                };
                nav = nav.child(
                    item.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        if view.capture.is_some() {
                            view.end_capture(None, cx);
                        }
                        view.page = page;
                        view.open_select = None;
                        if page == Page::Logs {
                            view.load_logs(cx);
                        }
                        if page == Page::Recognition {
                            view.load_recognition(cx);
                            view.load_local(cx);
                        }
                        if matches!(page, Page::Recognition | Page::General) {
                            view.load_resources(cx);
                        }
                        view.error = None;
                        view.saved = false;
                        view.notice = None;
                        cx.notify();
                    })),
                );
            }
        }
        if !any {
            nav = nav.child(
                div()
                    .m(px(12.0))
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .child(t("没有匹配的页面")),
            );
        }
        let version = crate::updater::current_version();
        nav = nav.child(
            div()
                .mt_auto()
                .pt(px(12.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(11.0))
                .text_color(rgb(col::muted()))
                .child(
                    div()
                        .size(px(6.0))
                        .flex_none()
                        .rounded_full()
                        .bg(rgb(col::success())),
                )
                .child(format!("v{version}")),
        );
        let body = match self.page {
            Page::Capture => self.capture_page(&settings, &t, cx),
            Page::Recording => self.recording_page(&settings, &t, cx),
            Page::Pins => self.pins_page(&settings, &t, cx),
            Page::Clipboard => self.clipboard_page(&settings, &t, cx),
            Page::General => self.general_page(&settings, &t, cx),
            Page::Shortcuts => self.shortcuts_page(&settings, &t, cx),
            Page::Recognition => self.recognition_page(&settings, &t, window, cx),
            Page::Logs => self.logs_page(&t, cx),
            Page::About => self.about_page(&settings, &t, cx),
        };
        let notice = |text: String, error: bool| {
            div()
                .mt(px(16.0))
                .px(px(14.0))
                .py(px(10.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(if error { col::danger() } else { col::line() }))
                .bg(rgb(if error {
                    col::notice_error()
                } else {
                    col::notice()
                }))
                .text_xs()
                .line_height(relative(1.6))
                .text_color(rgb(if error { col::danger() } else { col::text() }))
                .child(text)
        };
        let status = match (&self.error, self.saved) {
            (Some(error), _) => Some(notice(
                snap_core::i18n::for_language(&language, error).to_owned(),
                true,
            )),
            (None, _) if self.notice.is_some() => Some(
                div()
                    .mt(px(6.0))
                    .mx(px(4.0))
                    .text_xs()
                    .text_color(rgb(col::success()))
                    .child(t(self.notice.as_deref().unwrap_or_default())),
            ),
            (None, true) => Some(
                div()
                    .mt(px(6.0))
                    .mx(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_xs()
                    .text_color(rgb(col::success()))
                    .child(
                        Icon::default()
                            .data(icons::CHECK)
                            .size(px(14.0))
                            .text_color(rgb(col::success())),
                    )
                    .child(t("已自动保存")),
            ),
            _ => None,
        };
        let shortcuts_on = settings.shortcuts_enabled;
        let unavailable = !crate::shell::unavailable_shortcuts(cx).is_empty();
        let topbar = div()
            .h(px(52.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .px(px(24.0))
            .border_b_1()
            .border_color(rgb(col::border()))
            .text_xs()
            .text_color(rgb(col::muted()))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(col::text()))
                    .child(t(self.page.label())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    // 有新版本：顶部“有更新”，打开“关于与帮助”的更新面板（Vue `UpdateNotice.vue`）。
                    .children((has_update && self.page != Page::About).then(|| {
                        about_page::dot_button("update-notice", t("有更新"), true)
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                view.page = Page::About;
                                view.open_select = None;
                                cx.notify();
                            }))
                    }))
                    .children(
                        unavailable
                            .then(|| div().text_color(rgb(col::danger())).child(t("注册异常"))),
                    )
                    .child(t("全局快捷键"))
                    .child(
                        switch("shortcuts-enabled-top", shortcuts_on, true).on_click(cx.listener(
                            move |view, _: &ClickEvent, _, cx| {
                                view.change(cx, |s| s.shortcuts_enabled = !shortcuts_on)
                            },
                        )),
                    ),
            );
        div()
            .id("settings")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(rgb(col::page_bg()))
            .text_color(rgb(col::text()))
            .text_size(px(13.0))
            .line_height(relative(1.5))
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    if view.open_select.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(nav)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(topbar)
                    .child(
                        div()
                            .id("settings-page")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .pt(px(2.0))
                            .px(px(24.0))
                            .pb(px(28.0))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(760.0))
                                    .mx_auto()
                                    .children(body)
                                    .children(status),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{excluded_apps, main_key, shortcut_label};

    #[test]
    fn keys_map_to_settings_names() {
        assert_eq!(main_key("a").as_deref(), Some("A"));
        assert_eq!(main_key("7").as_deref(), Some("7"));
        assert_eq!(main_key("f12").as_deref(), Some("F12"));
        assert_eq!(main_key("space").as_deref(), Some("Space"));
        assert_eq!(main_key("up").as_deref(), Some("ArrowUp"));
        assert_eq!(main_key("f25"), None);
        assert_eq!(main_key("é"), None);
        assert_eq!(shortcut_label("Ctrl+Shift+A"), "Ctrl + Shift + A");
    }

    #[test]
    fn excluded_apps_split_and_trim() {
        assert_eq!(
            excluded_apps(" KeePass.exe, 1Password.exe；\nfoo.exe ,, "),
            ["KeePass.exe", "1Password.exe", "foo.exe"]
        );
        assert!(excluded_apps("  ").is_empty());
    }
}
