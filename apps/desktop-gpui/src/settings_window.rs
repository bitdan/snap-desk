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
    component::input::{Input, InputEvent, InputState},
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

/// 设置窗口配色，随主题变化（见 `theme.rs`）。
mod col {
    pub fn surface() -> u32 {
        crate::theme::c().surface
    }
    pub fn page_bg() -> u32 {
        crate::theme::c().page
    }
    pub fn border() -> u32 {
        crate::theme::c().border
    }
    pub fn text() -> u32 {
        crate::theme::c().text
    }
    pub fn muted() -> u32 {
        crate::theme::c().muted
    }
    pub fn accent() -> u32 {
        crate::theme::c().accent
    }
    pub fn danger() -> u32 {
        crate::theme::c().danger
    }
    pub fn accent_soft() -> u32 {
        crate::theme::c().accent_soft
    }
    pub fn nav() -> u32 {
        crate::theme::c().nav
    }
    pub fn toggle_off() -> u32 {
        crate::theme::c().toggle_off
    }
    pub fn hover() -> u32 {
        crate::theme::c().hover
    }
    pub fn success() -> u32 {
        crate::theme::c().success
    }
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
    let size = size(px(760.0), px(580.0));
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
        window_min_size: Some(gpui_kit::size(px(600.0), px(420.0))),
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
#[path = "settings_logs.rs"]
mod logs_page;
#[path = "settings_recognition.rs"]
mod recognition_page;

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

impl Page {
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
            Page::About => "关于与更新",
        }
    }
}

/// 防抖保存的文本字段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    CapturePrefix,
    RecordingPrefix,
    ExcludedApps,
    UpdateProxy,
}

impl Field {
    fn apply(self, settings: &mut Settings, value: &str) {
        match self {
            Field::CapturePrefix => settings.export.filename_prefix = value.to_owned(),
            Field::RecordingPrefix => settings.recording_export.filename_prefix = value.to_owned(),
            Field::ExcludedApps => settings.clipboard.excluded_apps = excluded_apps(value),
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
    focus: FocusHandle,
    capture_prefix: Entity<InputState>,
    recording_prefix: Entity<InputState>,
    excluded: Entity<InputState>,
    update_proxy: Entity<InputState>,
    /// 关于页的检查更新结果。
    update: Option<about_page::UpdateState>,
    /// 识别与翻译页：后台读取的语言与密钥状态、正在编辑的服务。
    recognition: recognition_page::RecognitionState,
    service_editor: Option<recognition_page::ServiceEditor>,
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
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(settings: &Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let update_proxy = input(settings.update_network.proxy.clone(), window, cx);
        let mut subscriptions = Vec::new();
        for (state, field) in [
            (&capture_prefix, Field::CapturePrefix),
            (&recording_prefix, Field::RecordingPrefix),
            (&excluded, Field::ExcludedApps),
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
            update_proxy,
            update: None,
            recognition: Default::default(),
            service_editor: None,
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

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = self.capture.as_mut() else {
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

fn row(label: String, hint: Option<String>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(rgb(col::border()))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_sm().child(label))
                .children(hint.map(|h| div().text_xs().text_color(rgb(col::muted())).child(h))),
        )
        .child(div().flex_none().child(control))
}

fn card(rows: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(8.0))
        .bg(rgb(col::surface()))
        .border_1()
        .border_color(rgb(col::border()))
        .overflow_hidden()
        .children(rows)
}

fn heading(text: String) -> Div {
    div()
        .mt(px(18.0))
        .mb(px(8.0))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(col::muted()))
        .child(text)
}

impl SettingsView {
    fn toggle(
        &self,
        id: impl Into<ElementId>,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings, bool) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .w(px(38.0))
            .h(px(22.0))
            .flex_none()
            .rounded_full()
            .p(px(2.0))
            .cursor_pointer()
            .bg(rgb(if on { col::accent() } else { col::toggle_off() }))
            .flex()
            .justify_start()
            .when_on(on)
            .child(div().size(px(18.0)).rounded_full().bg(rgb(0xffffff)))
            .on_click(
                cx.listener(move |view, _: &ClickEvent, _, cx| view.change(cx, |s| change(s, !on))),
            )
    }

    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        options: Vec<(T, String)>,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings, T) + Clone + 'static,
    ) -> Div {
        let mut group = div()
            .flex()
            .p(px(2.0))
            .gap(px(2.0))
            .rounded(px(6.0))
            .bg(rgb(col::page_bg()));
        for (index, (value, label)) in options.into_iter().enumerate() {
            let selected = value == current;
            let change = change.clone();
            group = group.child(
                div()
                    .id((id, index))
                    .h(px(26.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .text_xs()
                    .cursor_pointer()
                    .text_color(rgb(if selected { col::text() } else { col::muted() }))
                    .when_selected(selected)
                    .child(label)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        if !selected {
                            view.change(cx, |s| change(s, value));
                        }
                    })),
            );
        }
        group
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
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .bg(rgb(col::page_bg()))
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

    fn text_input(&self, state: &Entity<InputState>, width: f32) -> Div {
        div()
            .w(px(width))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::surface()))
            .child(Input::new(state))
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
        let small = |id: (&'static str, usize), label: String| {
            div()
                .id(id)
                .h(px(28.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(col::border()))
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(col::page_bg())))
                .child(label)
        };
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
        div()
            .id(id)
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(col::border()))
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(col::page_bg())))
            .child(label)
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
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
            }))
    }
}

/// 让开关/分段控件的选中态写法更紧凑。
trait States: Styled + Sized {
    fn when_on(self, on: bool) -> Self {
        if on {
            self.justify_end()
        } else {
            self
        }
    }
    fn when_selected(self, selected: bool) -> Self {
        if selected {
            self.bg(rgb(col::surface())).shadow_sm()
        } else {
            self
        }
    }
}
impl<T: Styled> States for T {}

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
        export.push(self.folder_row(Folder::Captures, s, t, cx));
        vec![
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
                    None,
                    self.segmented(
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
        let e = &s.recording_export;
        vec![
            heading(t("录屏文件")).into_any_element(),
            card(vec![
                row(
                    t("文件名前缀"),
                    None,
                    self.text_input(&self.recording_prefix, 180.0),
                ),
                row(
                    t("文件名包含视频尺寸"),
                    None,
                    self.toggle("rec-dimensions", e.include_dimensions, cx, |s, v| {
                        s.recording_export.include_dimensions = v
                    }),
                ),
                row(
                    t("记住上次保存目录"),
                    None,
                    self.toggle("rec-remember", e.remember_directory, cx, |s, v| {
                        s.recording_export.remember_directory = v
                    }),
                ),
                self.folder_row(Folder::Recordings, s, t, cx),
            ])
            .into_any_element(),
            div()
                .mt(px(10.0))
                .text_xs()
                .text_color(rgb(col::muted()))
                .child(t("格式、帧率、倒计时与声音在录屏框选时的选项行中设置。"))
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
        let c = &s.clipboard;
        vec![
            heading(t("剪贴板历史")).into_any_element(),
            card(vec![
                row(
                    t("记录剪贴板历史"),
                    None,
                    self.toggle("clip-enabled", c.enabled, cx, |s, v| {
                        s.clipboard.enabled = v
                    }),
                ),
                row(
                    t("唤出后直接粘贴"),
                    Some(t("回车或双击后把内容粘贴到唤出前的窗口")),
                    self.toggle("clip-paste", c.paste_on_activate, cx, |s, v| {
                        s.clipboard.paste_on_activate = v
                    }),
                ),
                row(
                    t("记录文字"),
                    None,
                    self.toggle("clip-text", c.save_copied_text, cx, |s, v| {
                        s.clipboard.save_copied_text = v
                    }),
                ),
                row(
                    t("记录图片"),
                    None,
                    self.toggle("clip-images", c.save_copied_images, cx, |s, v| {
                        s.clipboard.save_copied_images = v
                    }),
                ),
                row(
                    t("记录文件"),
                    None,
                    self.toggle("clip-files", c.save_copied_files, cx, |s, v| {
                        s.clipboard.save_copied_files = v
                    }),
                ),
            ])
            .into_any_element(),
            heading(t("保留与清理")).into_any_element(),
            card(vec![
                row(
                    t("最多保留条数"),
                    None,
                    self.stepper(
                        "clip-items",
                        c.max_items,
                        (100, 5000, 100),
                        "",
                        cx,
                        |s, v| s.clipboard.max_items = v,
                    ),
                ),
                row(
                    t("存储上限"),
                    None,
                    self.stepper(
                        "clip-storage",
                        c.max_storage_mb,
                        (50, 2048, 50),
                        " MiB",
                        cx,
                        |s, v| s.clipboard.max_storage_mb = v,
                    ),
                ),
                row(
                    t("保留天数"),
                    None,
                    self.segmented(
                        "clip-days",
                        c.retain_days,
                        vec![
                            (0, t("永久")),
                            (7, t("7 天")),
                            (30, t("30 天")),
                            (90, t("90 天")),
                        ],
                        cx,
                        |s, v| s.clipboard.retain_days = v,
                    ),
                ),
                row(
                    t("排除应用"),
                    Some(t("不记录这些程序复制的内容，填写程序文件名，用逗号分隔")),
                    self.text_input(&self.excluded, 240.0),
                ),
            ])
            .into_any_element(),
        ]
    }

    fn general_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let language = match s.appearance.language {
            Language::Chinese => 0,
            Language::English => 1,
        };
        let directories: [(&'static str, &'static str, Option<PathBuf>); 4] = [
            (
                "应用日志目录",
                "本机日志，分享前请检查内容。",
                snap_core::paths::log_dir(),
            ),
            (
                "应用配置目录",
                "快捷键与应用设置",
                crate::shell::settings_dir(),
            ),
            (
                "应用数据目录",
                "本地数据与恢复文件",
                snap_core::paths::local_data_dir(),
            ),
            (
                "滚动截图结果目录",
                "滚动截图自动保存的 PNG 结果",
                snap_core::paths::local_data_dir().map(|d| d.join("scroll-results")),
            ),
        ];
        let mut rows = Vec::new();
        for (index, (name, description, path)) in directories.into_iter().enumerate() {
            rows.push(row(
                t(name),
                Some(t(description)),
                self.open_button(("dir", index), t("打开"), path, cx),
            ));
        }
        vec![
            heading(t("外观")).into_any_element(),
            card(vec![
                row(
                    t("主题"),
                    Some(t("跟随系统时随系统浅色/深色切换。")),
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
                    Some(t("新打开的窗口使用新语言。")),
                    self.segmented(
                        "language",
                        language,
                        vec![(0, "中文".into()), (1, "English".into())],
                        cx,
                        |s, v| {
                            s.appearance.language = if v == 1 {
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
                Some(t("与 Tauri 版共用同一登录项；开启后登录时启动当前程序。")),
                match self.autostart {
                    Some(on) => self
                        .plain_toggle("autostart", on, cx, |view, on, cx| {
                            view.set_autostart(on, cx)
                        })
                        .into_any_element(),
                    None => div()
                        .text_xs()
                        .text_color(rgb(col::muted()))
                        .child(t("未知"))
                        .into_any_element(),
                },
            )])
            .into_any_element(),
            heading(t("配置")).into_any_element(),
            card(vec![
                row(
                    t("导出时包含服务密钥"),
                    Some(t("密钥以明文写入导出文件，请妥善保管。")),
                    self.plain_toggle(
                        "include-secrets",
                        self.include_secrets,
                        cx,
                        |view, on, cx| {
                            view.include_secrets = on;
                            cx.notify();
                        },
                    ),
                ),
                row(
                    t("导入导出配置"),
                    Some(t("包含全部设置与保存目录，不含截图、录屏与剪贴板历史。")),
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            self.action_button("config-export", t("导出配置…"), !self.transferring)
                                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                    view.export_config(cx)
                                })),
                        )
                        .child(
                            self.action_button("config-import", t("导入配置…"), !self.transferring)
                                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                    view.import_config(cx)
                                })),
                        ),
                ),
            ])
            .into_any_element(),
            heading(t("目录")).into_any_element(),
            card(rows).into_any_element(),
        ]
    }

    /// 不经设置保存的本地开关（登录项、导出选项）。
    fn plain_toggle(
        &self,
        id: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, bool, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .w(px(38.0))
            .h(px(22.0))
            .flex_none()
            .rounded_full()
            .p(px(2.0))
            .cursor_pointer()
            .bg(rgb(if on { col::accent() } else { col::toggle_off() }))
            .flex()
            .justify_start()
            .when_on(on)
            .child(div().size(px(18.0)).rounded_full().bg(rgb(0xffffff)))
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| change(view, !on, cx)))
    }

    fn action_button(&self, id: &'static str, label: String, enabled: bool) -> Stateful<Div> {
        let button = div()
            .id(id)
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(col::border()))
            .text_xs()
            .child(label);
        if enabled {
            button.cursor_pointer().hover(|s| s.bg(rgb(col::page_bg())))
        } else {
            button.opacity(0.5)
        }
    }

    fn shortcuts_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let unavailable = crate::shell::unavailable_shortcuts(cx);
        let mut rows = vec![row(
            t("启用全局快捷键"),
            None,
            self.toggle("shortcuts-enabled", s.shortcuts_enabled, cx, |s, v| {
                s.shortcuts_enabled = v
            }),
        )];
        for (index, (action, name, value)) in s.shortcuts.entries().into_iter().enumerate() {
            let capturing = self.capture.as_ref().filter(|c| c.action == action);
            let text = match capturing {
                Some(c) if c.preview.is_empty() => t("请按快捷键…"),
                Some(c) => shortcut_label(&c.preview),
                None if value.is_empty() => t("未设置"),
                None => shortcut_label(value),
            };
            let label = if unavailable.contains(&action) {
                format!("{} · {}", t(name), t("已被占用"))
            } else {
                t(name)
            };
            let field = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .id(("shortcut", index))
                        .min_w(px(150.0))
                        .h(px(28.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(rgb(if capturing.is_some() {
                            col::accent()
                        } else {
                            col::border()
                        }))
                        .bg(rgb(if capturing.is_some() {
                            0xeff6ff
                        } else {
                            col::surface()
                        }))
                        .text_xs()
                        .text_color(rgb(if value.is_empty() && capturing.is_none() {
                            col::muted()
                        } else {
                            col::text()
                        }))
                        .cursor_pointer()
                        .child(text)
                        .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                            view.begin_capture(action, window, cx)
                        })),
                )
                .children((!value.is_empty()).then(|| {
                    div()
                        .id(("shortcut-clear", index))
                        .size(px(22.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .text_xs()
                        .text_color(rgb(col::muted()))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(col::page_bg())))
                        .child("×")
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.change(cx, |s| set_shortcut(s, action, String::new()))
                        }))
                }));
            let mut item = row(label, None, field);
            if unavailable.contains(&action) {
                item = item.text_color(rgb(col::danger()));
            }
            rows.push(item);
        }
        let reset = div()
            .id("shortcuts-reset")
            .text_xs()
            .text_color(rgb(col::accent()))
            .cursor_pointer()
            .child(t("恢复默认"))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.change(cx, |s| {
                    s.shortcuts = snap_core::settings::Shortcuts::default()
                })
            }));
        vec![
            div()
                .flex()
                .items_end()
                .justify_between()
                .child(heading(t("全局快捷键")))
                .child(div().mb(px(8.0)).child(reset))
                .into_any_element(),
            card(rows).into_any_element(),
            div()
                .mt(px(10.0))
                .text_xs()
                .text_color(rgb(col::muted()))
                .child(t("点击按键框后按下组合键；Esc 取消，Backspace 禁用。"))
                .into_any_element(),
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
                (&self.update_proxy, settings.update_network.proxy.clone()),
            ] {
                state.update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
        let language = Self::language(cx);
        let words = language.clone();
        let t = move |key: &str| snap_core::i18n::for_language(&words, key).to_owned();
        let mut nav = div()
            .w(px(168.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(10.0))
            .border_r_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::nav()));
        for (index, page) in Page::ALL.into_iter().enumerate() {
            let active = page == self.page;
            nav = nav.child(
                div()
                    .id(("nav", index))
                    .h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .text_sm()
                    .cursor_pointer()
                    .text_color(rgb(if active { col::accent() } else { col::text() }))
                    .bg(rgb(if active {
                        col::accent_soft()
                    } else {
                        col::nav()
                    }))
                    .hover(|s| s.bg(rgb(col::hover())))
                    .child(t(page.label()))
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        if view.capture.is_some() {
                            view.end_capture(None, cx);
                        }
                        view.page = page;
                        if page == Page::Logs {
                            view.load_logs(cx);
                        }
                        if page == Page::Recognition {
                            view.load_recognition(cx);
                        }
                        view.error = None;
                        view.saved = false;
                        view.notice = None;
                        cx.notify();
                    })),
            );
        }
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
        let status = match (&self.error, self.saved) {
            (None, _) if self.notice.is_some() => Some(
                div()
                    .mt(px(12.0))
                    .text_xs()
                    .text_color(rgb(col::success()))
                    .child(t(self.notice.as_deref().unwrap_or_default())),
            ),
            (Some(error), _) => Some(
                div()
                    .mt(px(12.0))
                    .text_xs()
                    .text_color(rgb(col::danger()))
                    .child(snap_core::i18n::for_language(&language, error).to_owned()),
            ),
            (None, true) => Some(
                div()
                    .mt(px(12.0))
                    .text_xs()
                    .text_color(rgb(col::success()))
                    .child(t("已自动保存")),
            ),
            _ => None,
        };
        div()
            .id("settings")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(rgb(col::page_bg()))
            .text_color(rgb(col::text()))
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .child(nav)
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .px(px(24.0))
                    .pb(px(24.0))
                    .child(
                        div()
                            .pt(px(6.0))
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t(self.page.label())),
                    )
                    .children(body)
                    .children(status),
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
