//! 文字翻译窗口。
//!
//! 无边框窗口（380×560）：顶栏为置顶按钮、可拖动的“文字翻译”标题与关闭；下方依次是原文卡片（可编辑，
//! 复制/读取剪贴板/合并换行/清空/截图翻译，检测到的语言，翻译按钮）、语言条（原文语言、交换、目标语言）、
//! “中英自动互译”与译文卡片（服务名、状态、重新读取配置；复制/回译/重新翻译，取消或目标语言）。
//! 回车翻译、Shift+回车换行，Ctrl+回车也翻译，Esc 关闭。中英自动互译按原文主体语言选择方向，
//! 不改写默认目标语言。
use crate::theme;
use gpui_kit::{
    component::{
        Icon,
        input::{InputEvent, Textarea, TextareaState},
    },
    *,
};
use snap_core::{
    recognition::{self, providers::TranslationRequest, translation::AiSettings},
    settings::Settings,
};
use std::time::Duration;

mod icons {
    macro_rules! icon {
        ($name:ident, $file:literal) => {
            pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));
        };
    }
    icon!(PIN, "pin");
    icon!(PIN_FILLED, "pin-filled");
    icon!(LANGUAGES, "languages");
    icon!(X, "x");
    icon!(COPY, "copy");
    icon!(PASTE, "clipboard-paste");
    icon!(WRAP, "wrap-text");
    icon!(TRASH, "trash-2");
    icon!(CAMERA, "camera");
    icon!(SWAP, "arrow-left-right");
    icon!(CHECK, "check");
    icon!(REFRESH, "refresh-cw");
    icon!(RETRY, "rotate-ccw");
    icon!(CHEVRON, "chevron-down");
    icon!(INFO, "info");
}

/// 目标语言；本地翻译只支持前两项。
const LANGUAGES: [(&str, &str); 6] = [
    ("zh-CN", "简体中文"),
    ("en", "英语"),
    ("ja", "日语"),
    ("ko", "韩语"),
    ("fr", "法语"),
    ("de", "德语"),
];

/// 兼容服务的显示名（按接口地址匹配）。
const PRESETS: [(&str, &str); 4] = [
    ("DeepSeek", "https://api.deepseek.com/chat/completions"),
    ("Groq", "https://api.groq.com/openai/v1/chat/completions"),
    ("OpenAI", "https://api.openai.com/v1/chat/completions"),
    ("本机兼容服务", "http://127.0.0.1:11434/v1/chat/completions"),
];

const BLUE: u32 = 0x2563eb;

/// 面板色值（`--panel-*`）。
struct Colors {
    window: u32,
    window_border: u32,
    card: u32,
    bar: u32,
    fg: u32,
    muted: u32,
    border: u32,
    success: u32,
    error: u32,
}

fn colors() -> Colors {
    if theme::dark() {
        Colors {
            window: 0x18181b,
            window_border: 0x3f3f46,
            card: 0x27272a,
            bar: 0x323237,
            fg: 0xf4f4f5,
            muted: 0xa1a1aa,
            border: 0x3f3f46,
            success: 0x73d69a,
            error: 0xfda29b,
        }
    } else {
        Colors {
            window: 0xf5f5f7,
            window_border: 0xe4e4e7,
            card: 0xffffff,
            bar: 0xeeeef1,
            fg: 0x27272a,
            muted: 0x71717a,
            border: 0xe4e4e7,
            success: 0x24834b,
            error: 0xb42318,
        }
    }
}

/// 轻量中英主体语言判断：两个汉字约等于一个英文词。
pub fn detect_language(text: &str) -> &'static str {
    let mut chinese = 0usize;
    let mut english = 0usize;
    let mut in_word = false;
    for word in text.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        // 链接、邮箱与 Windows 路径不参与判断。
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("www.")
        {
            continue;
        }
        if word.contains('@') && word.contains('.') {
            continue;
        }
        let bytes = word.as_bytes();
        if bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
        {
            continue;
        }
        for c in word.chars() {
            if is_han(c) {
                chinese += 1;
                in_word = false;
            } else if c.is_alphabetic() && c.is_ascii() || matches!(c, 'À'..='ɏ') {
                if !in_word {
                    english += 1;
                    in_word = true;
                }
            } else if !matches!(c, '\'' | '’' | '-') {
                in_word = false;
            }
        }
        in_word = false;
    }
    let chinese = chinese as f64 / 2.0;
    let english = english as f64;
    if chinese == 0.0 && english == 0.0 {
        return "unknown";
    }
    let ratio = chinese / (chinese + english);
    if ratio >= 0.6 {
        "zh-CN"
    } else if ratio <= 0.4 {
        "en"
    } else {
        "mixed"
    }
}

fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2ebef)
}

/// 中文译为英文、英文译为中文；混合或无法判断时沿用默认目标。
pub fn automatic_target(language: &str, default: &str) -> String {
    match language {
        "zh-CN" => "en".into(),
        "en" => "zh-CN".into(),
        _ => default.into(),
    }
}

fn label_of(code: &str) -> &'static str {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or("目标语言", |(_, l)| *l)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Select {
    Source,
    Target,
}

pub struct Translator {
    settings: Settings,
    config: Result<AiSettings, String>,
    source: Entity<TextareaState>,
    output: Entity<TextareaState>,
    translated: String,
    /// 原文语言（空为检测语言）与目标语言。
    source_language: String,
    target: String,
    auto_direction: bool,
    busy: bool,
    /// 每次翻译递增；取消或新请求使旧结果作废。
    generation: u64,
    error: Option<String>,
    message: Option<String>,
    pinned: bool,
    open_select: Option<Select>,
    focus: FocusHandle,
    /// 原文变化时的上次文本（Change 事件后判断是否真的改动）。
    last_source: String,
    pending_output: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Translator {
    fn new(
        settings: Settings,
        text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let t = |key: &'static str| {
            snap_core::i18n::for_language(&settings.appearance.language, key).to_owned()
        };
        let source_placeholder = t("输入需要翻译的文字…");
        let output_placeholder = t("译文将在这里显示");
        let source = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(source_placeholder)
                .submit_on_enter(true)
        });
        let output = cx.new(|cx| TextareaState::new(window, cx).placeholder(output_placeholder));
        let mut subscriptions = vec![cx.subscribe_in(
            &source,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.source_changed(cx),
                InputEvent::PressEnter { shift: false, .. } => this.translate(cx),
                _ => {}
            },
        )];
        subscriptions
            .push(cx.on_release(|_, _| log::info!("event=gpui.translation_window_closed")));
        if let Some(text) = &text {
            source.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
        }
        source.update(cx, |state, cx| state.focus(window, cx));
        let mut view = Self {
            config: Err(String::new()),
            settings,
            source,
            output,
            translated: String::new(),
            source_language: String::new(),
            target: "zh-CN".into(),
            auto_direction: true,
            busy: false,
            generation: 0,
            error: None,
            message: None,
            pinned: false,
            open_select: None,
            focus: cx.focus_handle(),
            last_source: text.clone().unwrap_or_default(),
            pending_output: None,
            _subscriptions: subscriptions,
        };
        view.load_config(cx);
        // 带原文打开（划词翻译）即视为确认，立即翻译。
        if text.is_some() {
            view.translate(cx);
        }
        view
    }

    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.settings.appearance.language, key).to_owned()
    }

    fn local(&self) -> bool {
        self.config
            .as_ref()
            .is_ok_and(|c| c.provider == "local-ctranslate2")
    }

    fn languages(&self) -> &'static [(&'static str, &'static str)] {
        if self.local() {
            &LANGUAGES[..2]
        } else {
            &LANGUAGES
        }
    }

    fn source_text(&self, cx: &App) -> String {
        self.source.read(cx).value().to_string()
    }

    fn detected(&self, cx: &App) -> &'static str {
        detect_language(&self.source_text(cx))
    }

    /// 读取翻译配置（设置可能已在设置窗口修改）。
    fn load_config(&mut self, cx: &mut Context<Self>) {
        self.settings = crate::shell::settings(cx);
        self.config = recognition::services::translation_config(&self.settings);
        self.invalidate(cx);
        match self.config.clone() {
            Ok(config) => {
                self.target = config.target_language.clone();
                if !self
                    .languages()
                    .iter()
                    .any(|(c, _)| *c == self.source_language)
                {
                    self.source_language.clear();
                }
                self.auto_direction = self.source_language.is_empty()
                    && ["zh-CN", "en"].contains(&config.target_language.as_str());
                self.apply_direction(cx);
                if config.provider != "local-ctranslate2" && config.ready().is_err() {
                    self.error = Some(self.t("请先在“翻译”中选择服务，或在“服务管理”中添加服务。"));
                }
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn apply_direction(&mut self, cx: &App) {
        if let Ok(config) = &self.config {
            if self.auto_direction && self.source_language.is_empty() && !self.busy {
                self.target = automatic_target(self.detected(cx), &config.target_language);
            }
        }
    }

    fn source_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.source_text(cx);
        if text == self.last_source {
            return;
        }
        self.last_source = text;
        self.apply_direction(cx);
        self.invalidate(cx);
        cx.notify();
    }

    /// 修改原文或方向即清除旧结果。
    fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.translated.clear();
        self.message = None;
        self.error = None;
        let _ = cx;
        self.fill_output(String::new(), cx);
    }

    /// 译文框需要窗口才能写入，留到下次渲染执行。
    fn fill_output(&mut self, value: String, cx: &mut Context<Self>) {
        self.pending_output = Some(value);
        cx.notify();
    }

    fn swap_source(&self, cx: &App) -> Option<String> {
        if !self.source_language.is_empty() {
            return Some(self.source_language.clone());
        }
        let detected = self.detected(cx);
        if self.auto_direction && ["zh-CN", "en"].contains(&detected) {
            return Some(detected.into());
        }
        if self.local() {
            return Some(if self.target == "en" { "zh-CN" } else { "en" }.into());
        }
        None
    }

    fn translate(&mut self, cx: &mut Context<Self>) {
        let source = self.source_text(cx);
        if self.busy || source.trim().is_empty() {
            return;
        }
        let Ok(mut ai) = self.config.clone() else {
            return;
        };
        self.apply_direction(cx);
        if self.source_language == self.target {
            self.error = Some(self.t("原文语言和目标语言不能相同。"));
            cx.notify();
            return;
        }
        if source.len() > 32_000 {
            self.error = Some(self.t("翻译文字超过 32 KB，请减少文字后重试。"));
            cx.notify();
            return;
        }
        ai.target_language = self.target.clone();
        if let Err(error) = ai.ready() {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let request = TranslationRequest {
            texts: vec![source],
            source_language: (!self.source_language.is_empty())
                .then(|| self.source_language.clone()),
            target_language: self.target.clone(),
        };
        if let Err(error) = request.validate() {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let keys = recognition::services::key_ids(&self.settings, &ai.service_id);
        let auto_copy = ai.auto_copy;
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        self.translated.clear();
        self.error = None;
        self.message = None;
        self.fill_output(String::new(), cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // 本地翻译在常驻运行时上复用 worker；在线翻译每次用临时运行时。
                    if ai.provider == "local-ctranslate2" {
                        return crate::local_translate::translate(
                            &request.texts,
                            &request.target_language,
                            ai.timeout_seconds,
                        );
                    }
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| "无法初始化翻译网络客户端。".to_string())?
                        .block_on(recognition::providers::translate(&ai, &request, &keys))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.busy = false;
                match result {
                    Ok(texts) => {
                        log::info!("event=gpui.translation_finished segments={}", texts.len());
                        let text = texts.join("\n\n");
                        view.translated = text.clone();
                        view.fill_output(text.clone(), cx);
                        if auto_copy {
                            view.copy_text(text, "译文已复制。", cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("event=gpui.translation_failed");
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.generation += 1;
            self.busy = false;
            cx.notify();
        }
    }

    fn swap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(previous) = self.swap_source(cx).filter(|_| !self.busy) else {
            return;
        };
        self.auto_direction = false;
        if !self.translated.is_empty() {
            let text = self.translated.clone();
            self.last_source = text.clone();
            self.source
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        self.source_language = self.target.clone();
        self.target = previous;
        self.invalidate(cx);
        cx.notify();
    }

    fn copy_text(&mut self, value: String, success: &'static str, cx: &mut Context<Self>) {
        if value.trim().is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.error = None;
        self.message = Some(self.t(success));
        cx.notify();
    }

    fn set_source(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.source
            .update(cx, |state, cx| state.set_value(text, window, cx));
        self.source_changed(cx);
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "escape" if self.open_select.is_some() => {
                self.open_select = None;
                cx.notify();
            }
            "escape" => window.remove_window(),
            "enter" if keystroke.modifiers.control || keystroke.modifiers.platform => {
                self.translate(cx)
            }
            _ => {}
        }
    }

    /// 面板里的小图标按钮（28×28，6 像素圆角；悬停为条带底色）。
    fn icon_button(
        &self,
        id: impl Into<ElementId>,
        data: &'static [u8],
        size: f32,
        color: u32,
        enabled: bool,
    ) -> Stateful<Div> {
        let c = colors();
        let button = div()
            .id(id)
            .size(px(28.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .child(
                Icon::default()
                    .data(data)
                    .size(px(size))
                    .text_color(rgb(color)),
            );
        if enabled {
            let (bar, fg) = (c.bar, c.fg);
            button
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(bar)).text_color(rgb(fg)))
        } else {
            button.opacity(0.4)
        }
    }

    fn select(&self, which: Select, cx: &mut Context<Self>) -> Div {
        let c = colors();
        let (value, options): (String, Vec<(String, String)>) = match which {
            Select::Source => {
                let mut options = vec![(String::new(), self.t("检测语言"))];
                options.extend(
                    self.languages()
                        .iter()
                        .map(|(code, label)| (code.to_string(), self.t(label))),
                );
                (self.source_language.clone(), options)
            }
            Select::Target => (
                self.target.clone(),
                self.languages()
                    .iter()
                    .map(|(code, label)| (code.to_string(), self.t(label)))
                    .collect(),
            ),
        };
        let label = options
            .iter()
            .find(|(code, _)| *code == value)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let open = self.open_select == Some(which);
        let enabled = !self.busy && self.config.is_ok();
        let right = which == Select::Target;
        let mut trigger = div()
            .id(match which {
                Select::Source => "translate-source-language",
                Select::Target => "translate-target-language",
            })
            .h(px(32.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .rounded(px(6.0))
            .text_color(rgb(c.fg))
            .child(div().min_w_0().truncate().child(label))
            .child(
                Icon::default()
                    .data(icons::CHEVRON)
                    .size(px(14.0))
                    .text_color(rgb(c.muted)),
            );
        if right {
            trigger = trigger.justify_end();
        }
        if enabled {
            trigger = trigger.cursor_pointer().on_click(cx.listener(
                move |view, _: &ClickEvent, _, cx| {
                    view.open_select = if view.open_select == Some(which) {
                        None
                    } else {
                        Some(which)
                    };
                    cx.notify();
                },
            ));
        } else {
            trigger = trigger.opacity(0.5);
        }
        let mut container = div().relative().min_w_0().flex_1().child(trigger);
        if open {
            let mut list = div()
                .id("translate-select-list")
                .absolute()
                .top(px(36.0))
                .min_w(px(140.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(c.border))
                .bg(rgb(c.card))
                .shadow_lg()
                .occlude();
            list = if right { list.right_0() } else { list.left_0() };
            for (index, (code, label)) in options.into_iter().enumerate() {
                let chosen = code == value;
                let bar = c.bar;
                list = list.child(
                    div()
                        .id(("translate-select-option", index))
                        .h(px(30.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .text_color(rgb(if chosen { BLUE } else { c.fg }))
                        .hover(move |s| s.bg(rgb(bar)))
                        .child(label)
                        .children(chosen.then(|| {
                            Icon::default()
                                .data(icons::CHECK)
                                .size(px(14.0))
                                .text_color(rgb(BLUE))
                        }))
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.open_select = None;
                            match which {
                                Select::Source => {
                                    view.source_language = code.clone();
                                    view.auto_direction = view.source_language.is_empty()
                                        && ["zh-CN", "en"].contains(&view.target.as_str());
                                    view.apply_direction(cx);
                                }
                                Select::Target => {
                                    view.target = code.clone();
                                    view.auto_direction = false;
                                }
                            }
                            view.invalidate(cx);
                            cx.notify();
                        })),
                );
            }
            container = container.child(deferred(list).with_priority(1));
        }
        container
    }

    fn provider_label(&self) -> String {
        match &self.config {
            Err(_) => self.t("翻译服务"),
            Ok(c) if c.provider == "local-ctranslate2" => self.t("本地翻译"),
            Ok(c) if c.provider == "deepl" => "DeepL".into(),
            Ok(c) => PRESETS
                .iter()
                .find(|(_, endpoint)| *endpoint == c.endpoint)
                .map_or_else(|| self.t("兼容服务"), |(label, _)| self.t(label)),
        }
    }
}

impl Render for Translator {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(value) = self.pending_output.take() {
            self.output
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
        let c = colors();
        let has_source = !self.source_text(cx).trim().is_empty();
        let has_output = !self.translated.is_empty();
        let ready = self
            .config
            .as_ref()
            .is_ok_and(|cfg| cfg.provider == "local-ctranslate2" || cfg.ready().is_ok());
        let idle = !self.busy;
        let swap = self.swap_source(cx).is_some();
        let detected = self.detected(cx);

        // ---- 顶栏 ----
        let pinned = self.pinned;
        let tools = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .h(px(40.0))
            .mx(px(-6.0))
            .mb(px(-2.0))
            .child(
                self.icon_button(
                    "translate-pin",
                    if pinned {
                        icons::PIN_FILLED
                    } else {
                        icons::PIN
                    },
                    17.0,
                    if pinned { BLUE } else { c.muted },
                    true,
                )
                .tooltip({
                    let tip = self.t(if pinned {
                        "取消置顶"
                    } else {
                        "置顶窗口"
                    });
                    move |_, cx| {
                        cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                            .into()
                    }
                })
                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    match crate::native::set_topmost(window, !pinned) {
                        Ok(()) => view.pinned = !pinned,
                        Err(error) => view.error = Some(error),
                    }
                    cx.notify();
                })),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c.muted))
                    .on_mouse_down(MouseButton::Left, |_, window, _| crate::native::start_move(window))
                    .child(
                        Icon::default()
                            .data(icons::LANGUAGES)
                            .size(px(14.0))
                            .text_color(rgb(c.muted)),
                    )
                    .child(self.t("文字翻译")),
            )
            .child(
                self.icon_button("translate-close", icons::X, 16.0, c.muted, true)
                    .tooltip({
                        let tip = self.t("关闭（Esc）");
                        move |_, cx| {
                            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                .into()
                        }
                    })
                    .on_click(|_, window, _| window.remove_window()),
            );

        // ---- 原文卡片 ----
        let source_tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        self.icon_button(
                            "translate-copy-source",
                            icons::COPY,
                            15.0,
                            c.muted,
                            has_source,
                        )
                        .on_click(cx.listener(
                            |view, _: &ClickEvent, _, cx| {
                                let text = view.source_text(cx);
                                view.copy_text(text, "原文已复制。", cx);
                            },
                        )),
                    )
                    .child(
                        self.icon_button("translate-paste", icons::PASTE, 15.0, c.muted, idle)
                            .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                                if view.busy {
                                    return;
                                }
                                match cx.read_from_clipboard().and_then(|item| item.text()) {
                                    Some(text) => view.set_source(
                                        text.chars().take(16_000).collect(),
                                        window,
                                        cx,
                                    ),
                                    None => view.error = Some(view.t("剪贴板中没有文字。")),
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        self.icon_button(
                            "translate-merge",
                            icons::WRAP,
                            15.0,
                            c.muted,
                            idle && has_source,
                        )
                        .on_click(cx.listener(
                            |view, _: &ClickEvent, window, cx| {
                                let text = view.source_text(cx);
                                view.set_source(merge_lines(&text), window, cx);
                            },
                        )),
                    )
                    .child(
                        self.icon_button(
                            "translate-clear",
                            icons::TRASH,
                            15.0,
                            c.muted,
                            idle && has_source,
                        )
                        .on_click(cx.listener(
                            |view, _: &ClickEvent, window, cx| {
                                view.set_source(String::new(), window, cx);
                            },
                        )),
                    )
                    .child(
                        self.icon_button("translate-capture", icons::CAMERA, 15.0, c.muted, idle)
                            .tooltip({
                                let tip = self.t("截图翻译");
                                move |_, cx| {
                                    cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                        .into()
                                }
                            })
                            .on_click(|_, window, cx| {
                                // 框选完成后在图片翻译窗口显示结果；本窗口保留。
                                let mut settings = crate::shell::settings(cx);
                                settings.delay_seconds = 0;
                                crate::capture::start(
                                    cx,
                                    &settings,
                                    Duration::from_millis(150),
                                    Some(crate::recognition::Mode::Translate),
                                );
                                window.minimize_window();
                            }),
                    ),
            )
            .children(["zh-CN", "en"].contains(&detected).then(|| {
                div()
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(rgb(c.border))
                    .text_size(px(11.0))
                    .text_color(rgb(c.muted))
                    .child(self.t(if detected == "en" {
                        "英语"
                    } else {
                        "简体中文"
                    }))
            }))
            .child(
                div().ml_auto().child(
                    self.icon_button(
                        "translate-run",
                        icons::LANGUAGES,
                        18.0,
                        BLUE,
                        idle && ready && has_source,
                    )
                    .tooltip({
                        let tip = self.t("Enter 翻译，Shift+Enter 换行");
                        move |_, cx| {
                            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                .into()
                        }
                    })
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.translate(cx))),
                ),
            );
        let source_card = div()
            .flex_none()
            .rounded(px(10.0))
            .overflow_hidden()
            .bg(rgb(c.card))
            .child(
                div()
                    .h(px(140.0))
                    .p(px(12.0))
                    .text_size(px(14.0))
                    .line_height(relative(1.65))
                    .child(Textarea::new(&self.source).appearance(false).h_full()),
            )
            .child(source_tools);

        // ---- 语言条 ----
        let source_select = self.select(Select::Source, cx);
        let target_select = self.select(Select::Target, cx);
        let auto = self.auto_direction;
        let language_area = div()
            .flex_none()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(10.0))
                    .bg(rgb(c.bar))
                    .child(source_select)
                    .child(
                        self.icon_button(
                            "translate-swap",
                            icons::SWAP,
                            17.0,
                            c.muted,
                            idle && swap,
                        )
                        .tooltip({
                            let tip = self.t("交换语言和文字");
                            move |_, cx| {
                                cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                    .into()
                            }
                        })
                        .on_click(
                            cx.listener(|view, _: &ClickEvent, window, cx| view.swap(window, cx)),
                        ),
                    )
                    .child(target_select),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .pt(px(6.0))
                    .px(px(4.0))
                    .child(
                        div()
                            .id("translate-auto")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(11.0))
                            .text_color(rgb(c.muted))
                            .cursor_pointer()
                            .child(checkbox(auto, &c))
                            .child(self.t("中英自动互译"))
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                if view.busy || view.config.is_err() {
                                    return;
                                }
                                view.auto_direction = !view.auto_direction;
                                if view.auto_direction {
                                    view.source_language.clear();
                                    view.apply_direction(cx);
                                }
                                view.invalidate(cx);
                                cx.notify();
                            })),
                    ),
            );

        // ---- 译文卡片 ----
        let status: Option<AnyElement> = if self.busy {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(px(11.0))
                    .text_color(rgb(c.muted))
                    .child(self.t("翻译中"))
                    .into_any_element(),
            )
        } else if has_output {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(px(11.0))
                    .text_color(rgb(c.success))
                    .child(
                        Icon::default()
                            .data(icons::CHECK)
                            .size(px(13.0))
                            .text_color(rgb(c.success)),
                    )
                    .child(self.t("已完成"))
                    .into_any_element(),
            )
        } else {
            None
        };
        let online = self
            .config
            .as_ref()
            .is_ok_and(|cfg| cfg.provider != "local-ctranslate2");
        let heading = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(32.0))
            .px(px(8.0))
            .py(px(4.0))
            .bg(rgb(c.bar))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        Icon::default()
                            .data(icons::LANGUAGES)
                            .size(px(16.0))
                            .text_color(rgb(BLUE)),
                    )
                    .child(self.provider_label()),
            )
            .children(online.then(|| {
                div()
                    .id("translate-online-tip")
                    .child(
                        Icon::default()
                            .data(icons::INFO)
                            .size(px(13.0))
                            .text_color(rgb(c.muted)),
                    )
                    .tooltip({
                        let tip = self.t("在线翻译会发送原文，可能产生费用。");
                        move |_, cx| {
                            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                .into()
                        }
                    })
            }))
            .child(div().flex_1())
            .children(status)
            .child(
                self.icon_button("translate-reload", icons::REFRESH, 14.0, c.muted, idle)
                    .tooltip({
                        let tip = self.t("重新读取配置");
                        move |_, cx| {
                            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                .into()
                        }
                    })
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.load_config(cx))),
            );
        let trailing: AnyElement = if self.busy {
            div()
                .id("translate-cancel")
                .h(px(28.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .text_xs()
                .text_color(rgb(c.muted))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(c.bar)).text_color(rgb(c.fg)))
                .child(self.t("取消翻译"))
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.cancel(cx)))
                .into_any_element()
        } else if let Some(message) = self.message.clone() {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_size(px(11.0))
                .text_color(rgb(c.success))
                .child(
                    Icon::default()
                        .data(icons::CHECK)
                        .size(px(13.0))
                        .text_color(rgb(c.success)),
                )
                .child(message)
                .into_any_element()
        } else {
            div()
                .text_size(px(11.0))
                .text_color(rgb(c.muted))
                .child(self.t(label_of(&self.target)))
                .into_any_element()
        };
        let provider_tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        self.icon_button(
                            "translate-copy-output",
                            icons::COPY,
                            15.0,
                            c.muted,
                            has_output,
                        )
                        .on_click(cx.listener(
                            |view, _: &ClickEvent, _, cx| {
                                let text = view.translated.clone();
                                view.copy_text(text, "译文已复制。", cx);
                            },
                        )),
                    )
                    .child(
                        self.icon_button(
                            "translate-reverse",
                            icons::SWAP,
                            15.0,
                            c.muted,
                            idle && has_output && swap,
                        )
                        .tooltip({
                            let tip = self.t("回译");
                            move |_, cx| {
                                cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                    .into()
                            }
                        })
                        .on_click(cx.listener(
                            |view, _: &ClickEvent, window, cx| {
                                if view.busy || view.translated.is_empty() {
                                    return;
                                }
                                view.swap(window, cx);
                                view.translate(cx);
                            },
                        )),
                    )
                    .child(
                        self.icon_button(
                            "translate-retry",
                            icons::RETRY,
                            15.0,
                            c.muted,
                            idle && ready && has_source,
                        )
                        .tooltip({
                            let tip = self.t("重新翻译");
                            move |_, cx| {
                                cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                    .into()
                            }
                        })
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.translate(cx))),
                    ),
            )
            .child(div().ml_auto().child(trailing));
        let provider_card = div()
            .flex_none()
            .rounded(px(10.0))
            .overflow_hidden()
            .bg(rgb(c.card))
            .child(heading)
            .child(
                div()
                    .h(px(180.0))
                    .p(px(12.0))
                    .text_size(px(14.0))
                    .line_height(relative(1.65))
                    .child(Textarea::new(&self.output).appearance(false).h_full()),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_xs()
                    .text_color(rgb(c.error))
                    .child(error)
            }))
            .child(provider_tools);

        let _ = window;
        div()
            .id("translation-window")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    if view.open_select.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(10.0))
            .pb(px(10.0))
            .overflow_y_scroll()
            .border_1()
            .border_color(rgb(c.window_border))
            .bg(rgb(c.window))
            .text_color(rgb(c.fg))
            .text_size(px(13.0))
            .child(tools)
            .child(source_card)
            .child(language_area)
            .child(provider_card)
    }
}

/// 勾选框（control-theme.css，13 像素）。
fn checkbox(checked: bool, c: &Colors) -> Div {
    let (border, fill) = if checked {
        (c.success, if theme::dark() { 0x203d2c } else { 0xedf8f1 })
    } else {
        (if theme::dark() { 0x768390 } else { 0x8c959f }, c.card)
    };
    div()
        .size(px(13.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(fill))
        .children(checked.then(|| {
            Icon::default()
                .data(icons::CHECK)
                .size(px(10.0))
                .text_color(rgb(c.success))
        }))
}

/// 合并换行：连字符换行直接拼接，其余换行换成空格。
pub fn merge_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-' {
            // “-\s*\n\s*” → 删除
            let mut lookahead = chars.clone();
            let mut spaces = 0;
            while matches!(lookahead.peek(), Some(' ' | '\t')) {
                lookahead.next();
                spaces += 1;
            }
            if lookahead.peek() == Some(&'\r') {
                lookahead.next();
            }
            if lookahead.peek() == Some(&'\n') {
                lookahead.next();
                while matches!(lookahead.peek(), Some(' ' | '\t')) {
                    lookahead.next();
                }
                chars = lookahead;
                let _ = spaces;
                continue;
            }
            out.push(c);
        } else if c == '\r' || c == '\n' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            while matches!(chars.peek(), Some(' ' | '\t' | '\r' | '\n')) {
                chars.next();
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// 打开文字翻译窗口（380×560，最小 340×420，无边框）；带原文时立即翻译。
pub fn open(cx: &mut App, text: Option<String>) {
    let settings = crate::shell::settings(cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(380.0), px(560.0)),
            cx,
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: true,
        is_minimizable: true,
        window_min_size: Some(size(px(340.0), px(420.0))),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| Translator::new(settings, text, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            log::info!("event=gpui.translation_window_opened");
        }
        Err(error) => log::error!("event=gpui.translation_window_failed message={error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{automatic_target, detect_language, merge_lines};

    #[test]
    fn detects_the_main_language_like_the_frontend() {
        assert_eq!(detect_language("你好世界，今天天气很好"), "zh-CN");
        assert_eq!(detect_language("Hello world, this is a test"), "en");
        assert_eq!(
            detect_language("打开 https://example.com/very/long/english/path 看看"),
            "zh-CN"
        );
        assert_eq!(detect_language("12345 !!!"), "unknown");
        assert_eq!(automatic_target("zh-CN", "ja"), "en");
        assert_eq!(automatic_target("mixed", "ja"), "ja");
    }

    #[test]
    fn merge_lines_joins_hyphenated_words_and_lines() {
        assert_eq!(merge_lines("trans-\n  lation"), "translation");
        assert_eq!(merge_lines("a\r\n  b\nc"), "a b c");
    }
}
