//! 设置窗口“识别与翻译”页（对照 Vue `OcrSettings.vue`、`AiProviderSettings.vue`、`ServiceSettings.vue`）。
//!
//! - 文字识别：引擎（本地 PP-OCR、系统 OCR、支持图片识别的在线服务）、系统 OCR 语言、识别后自动复制/合并换行。
//! - 翻译：引擎（支持翻译的在线服务）、目标语言、超时、自动复制译文、截图翻译覆盖原文。
//! - 服务管理：服务列表（能力、地址、密钥状态）、按预设新增、编辑名称/地址/模型/图片能力/密钥、删除。
//!   密钥只写入系统凭据库（core `ai_credentials`，键随服务标识与地址变化），配置里只保存密钥标识。
//!
//! 多密钥的列表、逐个测试与删除在 `settings_keys.rs`。与 Tauri 版的差异：没有打开服务商密钥页面的入口。
use super::{card, col, heading, row, SettingsView};
use gpui_kit::{
    component::input::{Input, InputState},
    *,
};
use snap_core::{
    recognition::{
        ai_credentials,
        ocr_config::OcrLanguage,
        services::{self, RecognitionService},
    },
    settings::Settings,
};
use std::collections::HashMap;

#[path = "settings_keys.rs"]
mod keys;

/// 新增服务的预设（与 Vue `servicePresets` 一致）。
struct Preset {
    /// 与 Vue 预设标识一致（测试中用于定位失败的预设）。
    #[cfg_attr(not(test), allow(dead_code))]
    id: &'static str,
    name: &'static str,
    protocol: &'static str,
    endpoint: &'static str,
    model: &'static str,
    vision: bool,
}

const PRESETS: [Preset; 7] = [
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        protocol: "openai-compatible",
        endpoint: "https://api.deepseek.com/chat/completions",
        model: "deepseek-flash",
        vision: true,
    },
    Preset {
        id: "groq",
        name: "Groq",
        protocol: "openai-compatible",
        endpoint: "https://api.groq.com/openai/v1/chat/completions",
        model: "openai/gpt-oss-120b",
        vision: false,
    },
    Preset {
        id: "groq-vision",
        name: "Groq Vision",
        protocol: "openai-compatible",
        endpoint: "https://api.groq.com/openai/v1/chat/completions",
        model: "qwen/qwen3.8-27b",
        vision: true,
    },
    Preset {
        id: "ocr-space",
        name: "OCR.space",
        protocol: "ocr-space",
        endpoint: "https://api.ocr.space/parse/image",
        model: "2",
        vision: true,
    },
    Preset {
        id: "paddle-ocr",
        name: "PaddleOCR API",
        protocol: "paddle-ocr",
        endpoint: "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs",
        model: "PP-OCRv6",
        vision: true,
    },
    Preset {
        id: "deepl",
        name: "DeepL API Free",
        protocol: "deepl",
        endpoint: "https://api-free.deepl.com/v2/translate",
        model: "",
        vision: false,
    },
    Preset {
        id: "custom",
        name: "自定义兼容服务",
        protocol: "openai-compatible",
        endpoint: "",
        model: "",
        vision: false,
    },
];

const TARGETS: [(&str, &str); 6] = [
    ("zh-CN", "中文"),
    ("en", "英语"),
    ("ja", "日语"),
    ("ko", "韩语"),
    ("fr", "法语"),
    ("de", "德语"),
];

/// 正在编辑（或新增）的服务。
pub(super) struct ServiceEditor {
    /// 已有服务的标识；None 为新增。
    existing: Option<String>,
    protocol: &'static str,
    vision: bool,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    model: Entity<InputState>,
    keys: keys::Keys,
}

/// 页面后台读取的状态：系统 OCR 语言、WeChat OCR 组件与各服务已保存的密钥数。
#[derive(Default)]
pub(super) struct RecognitionState {
    languages: Option<Result<Vec<OcrLanguage>, String>>,
    wechat: Option<snap_core::recognition::wechat_ocr::WechatOcrStatus>,
    credentials: HashMap<String, usize>,
    pub(super) loading: bool,
}

/// 服务能力说明（与 Vue `serviceKind` 一致）。
fn service_kind(service: &RecognitionService) -> &'static str {
    match service.protocol.as_str() {
        "deepl" => "在线翻译",
        "ocr-space" | "paddle-ocr" => "在线 OCR",
        _ if url::Url::parse(&service.endpoint)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .is_some_and(|h| ["localhost", "127.0.0.1", "[::1]"].contains(&h.as_str())) =>
        {
            "本机 AI"
        }
        _ if service.vision => "AI 视觉 / 翻译",
        _ => "AI 翻译",
    }
}

fn host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| endpoint.to_owned())
}

/// 新服务标识：时间 + 序号，只含字母数字与连字符。
fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("svc-{nanos:x}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// 删除服务后，引用它的识别/翻译选择回到不依赖服务的状态，保证整份设置仍然合法。
fn remove_service(settings: &mut Settings, id: &str) {
    settings.recognition_services.retain(|s| s.id != id);
    if settings.ocr.provider == "service" && settings.ocr.service_id == id {
        settings.ocr.provider = "local-ppocr".into();
        settings.ocr.service_id.clear();
    }
    if settings.ai.service_id == id {
        settings.ai.service_id.clear();
    }
}

impl SettingsView {
    /// 后台读取系统 OCR 语言与服务密钥状态。
    pub(super) fn load_recognition(&mut self, cx: &mut Context<Self>) {
        if self.recognition.loading {
            return;
        }
        self.recognition.loading = true;
        cx.notify();
        let services = crate::shell::settings(cx).recognition_services;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (languages, wechat, credentials) = cx
                .background_executor()
                .spawn(async move {
                    let languages = snap_core::recognition::system_ocr_languages();
                    let wechat = snap_core::recognition::wechat_ocr::status();
                    let credentials = services
                        .iter()
                        .map(|s| {
                            let count = services::secrets(&s.id, &s.endpoint, &s.keys)
                                .map_or(0, |v| v.len());
                            (s.id.clone(), count)
                        })
                        .collect::<HashMap<_, _>>();
                    (languages, wechat, credentials)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.recognition.loading = false;
                view.recognition.languages = Some(languages);
                view.recognition.wechat = Some(wechat);
                view.recognition.credentials = credentials;
                cx.notify();
            });
        })
        .detach();
    }

    fn edit_service(
        &mut self,
        preset: Option<&Preset>,
        existing: Option<&RecognitionService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = crate::shell::settings(cx).appearance.language;
        let input = |value: String, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .masked(masked)
            })
        };
        let (protocol, vision, name, endpoint, model) = match (existing, preset) {
            (Some(s), _) => (
                PRESETS
                    .iter()
                    .find(|p| p.protocol == s.protocol)
                    .map_or("openai-compatible", |p| p.protocol),
                s.vision,
                s.name.clone(),
                s.endpoint.clone(),
                s.model.clone(),
            ),
            (None, Some(p)) => (
                p.protocol,
                p.vision,
                snap_core::i18n::for_language(&language, p.name).to_owned(),
                p.endpoint.to_owned(),
                p.model.to_owned(),
            ),
            (None, None) => return,
        };
        self.service_editor = Some(ServiceEditor {
            existing: existing.map(|s| s.id.clone()),
            protocol,
            vision,
            name: input(name, false, window, cx),
            endpoint: input(endpoint, false, window, cx),
            model: input(model, false, window, cx),
            keys: keys::Keys::new(existing, window, cx),
        });
        self.error = None;
        self.load_keys(cx);
        cx.notify();
    }

    fn delete_service(&mut self, id: String, cx: &mut Context<Self>) {
        let current = crate::shell::settings(cx);
        let Some(service) = current
            .recognition_services
            .iter()
            .find(|s| s.id == id)
            .cloned()
        else {
            return;
        };
        let target = id.clone();
        if let Err(error) = crate::shell::update_settings(cx, move |s| remove_service(s, &target)) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        // 删除凭据失败只提示，不恢复服务。
        for key in &service.keys {
            if let Err(error) = ai_credentials::write(
                &services::secret_key(&service.id, &service.endpoint, key),
                None,
            ) {
                self.error = Some(error);
            }
        }
        self.service_editor = None;
        self.saved = self.error.is_none();
        log::info!("event=gpui.recognition_service_deleted");
        self.load_recognition(cx);
        cx.notify();
    }

    fn option(
        &self,
        id: (&'static str, usize),
        label: String,
        hint: Option<String>,
        selected: bool,
        enabled: bool,
    ) -> Stateful<Div> {
        let option = div()
            .id(id)
            .px(px(12.0))
            .py(px(8.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .border_b_1()
            .border_color(rgb(col::border()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(if enabled { col::text() } else { col::muted() }))
                            .child(label),
                    )
                    .children(hint.map(|h| div().text_xs().text_color(rgb(col::muted())).child(h))),
            )
            .child(
                div()
                    .size(px(16.0))
                    .flex_none()
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(if selected {
                        col::accent()
                    } else {
                        col::border()
                    }))
                    .bg(rgb(if selected {
                        col::accent()
                    } else {
                        col::surface()
                    })),
            );
        if enabled {
            option.cursor_pointer().hover(|s| s.bg(rgb(col::page_bg())))
        } else {
            option.opacity(0.6)
        }
    }

    pub(super) fn recognition_page(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let _ = window;
        let credentials = &self.recognition.credentials;
        let key_hint = |service: &RecognitionService| -> String {
            match credentials.get(&service.id) {
                Some(0) => t("未配置密钥，请在服务管理中填写。"),
                Some(_) => t("已配置密钥"),
                None => t("凭据状态未读取"),
            }
        };
        // ---- 文字识别 ----
        let mut engines = div().flex().flex_col();
        let system_label = if cfg!(target_os = "macos") {
            "macOS 系统 OCR"
        } else {
            "Windows 系统 OCR"
        };
        let fixed: [(&'static str, &'static str, Option<String>, bool); 3] = [
            (
                "local-ppocr",
                "本地 PP-OCR",
                Some(t("离线识别，需先下载识别资源")),
                true,
            ),
            (
                "windows-media",
                system_label,
                Some(t("使用系统自带的识别语言包")),
                cfg!(any(windows, target_os = "macos")),
            ),
            (
                "wechat-ocr",
                "WeChat OCR（实验性）",
                Some(t("实验性 · 离线")),
                cfg!(all(windows, target_arch = "x86_64")),
            ),
        ];
        for (index, (provider, label, hint, enabled)) in fixed.into_iter().enumerate() {
            let selected = s.ocr.provider == provider;
            let mut option = self.option(("ocr-engine", index), t(label), hint, selected, enabled);
            if enabled && !selected {
                option = option.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.change(cx, |s| {
                        s.ocr.provider = provider.into();
                        s.ocr.service_id.clear();
                    })
                }));
            }
            engines = engines.child(option);
        }
        for (index, service) in s
            .recognition_services
            .iter()
            .filter(|x| x.supports_ocr())
            .enumerate()
        {
            let selected = s.ocr.provider == "service" && s.ocr.service_id == service.id;
            let id = service.id.clone();
            let mut option = self.option(
                ("ocr-service", index),
                service.name.clone(),
                Some(format!(
                    "{} · {}",
                    t(service_kind(service)),
                    key_hint(service)
                )),
                selected,
                true,
            );
            if !selected {
                option = option.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    let id = id.clone();
                    view.change(cx, move |s| {
                        s.ocr.provider = "service".into();
                        s.ocr.service_id = id;
                    })
                }));
            }
            engines = engines.child(option);
        }
        let mut ocr_rows = Vec::new();
        if s.ocr.provider == "local-ppocr" {
            ocr_rows.push(self.ocr_resource_row(t, cx));
        }
        if s.ocr.provider == "wechat-ocr" {
            let status = match (&self.recognition.wechat, self.recognition.loading) {
                (_, true) => t("正在检测 WeChat OCR…"),
                (None, _) => t("状态未读取"),
                (Some(w), _) => match &w.error {
                    Some(error) => {
                        snap_core::i18n::for_language(&s.appearance.language, error).to_owned()
                    }
                    None if w.detected => t("已找到 WeChat OCR 组件"),
                    None => t("未找到 WeChat OCR 组件"),
                },
            };
            let idle = !self.recognition.loading;
            let mut recheck = self.action_button("wechat-recheck", t("重新检测"), idle);
            if idle {
                recheck = recheck.on_click(
                    cx.listener(|view, _: &ClickEvent, _, cx| view.load_recognition(cx)),
                );
            }
            ocr_rows.push(row(t("WeChat OCR"), Some(status), recheck));
        }
        if s.ocr.provider == "windows-media" {
            let mut languages = div().flex().flex_wrap().gap(px(4.0)).max_w(px(360.0));
            let mut options: Vec<(String, String)> = vec![("auto".into(), t("跟随系统"))];
            let status = match &self.recognition.languages {
                Some(Ok(list)) => {
                    options.extend(list.iter().map(|l| (l.tag.clone(), l.name.clone())));
                    if list.is_empty() {
                        t("未安装 Windows OCR 语言包，请在 Windows 语言设置中安装。")
                    } else {
                        format!("{} {}", list.len(), t("个可用识别语言"))
                    }
                }
                Some(Err(error)) => {
                    snap_core::i18n::for_language(&s.appearance.language, error).to_owned()
                }
                None => t("正在检查系统语言包…"),
            };
            for (index, (tag, name)) in options.into_iter().enumerate() {
                let selected = s.ocr.language == tag;
                languages = languages.child(
                    div()
                        .id(("ocr-language", index))
                        .h(px(26.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .rounded(px(5.0))
                        .text_xs()
                        .cursor_pointer()
                        .text_color(rgb(if selected { col::accent() } else { col::text() }))
                        .bg(rgb(if selected {
                            col::accent_soft()
                        } else {
                            col::page_bg()
                        }))
                        .child(name)
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            let tag = tag.clone();
                            view.change(cx, move |s| s.ocr.language = tag)
                        })),
                );
            }
            ocr_rows.push(row(t("识别语言"), Some(status), languages));
        }
        ocr_rows.push(row(
            t("识别后自动复制文字"),
            None,
            self.toggle("ocr-auto-copy", s.ocr.auto_copy, cx, |s, v| {
                s.ocr.auto_copy = v
            }),
        ));
        ocr_rows.push(row(
            t("识别后自动合并换行"),
            None,
            self.toggle("ocr-merge", s.ocr.merge_lines, cx, |s, v| {
                s.ocr.merge_lines = v
            }),
        ));
        // ---- 翻译 ----
        let mut translators = div().flex().flex_col();
        let local_selected = s.ai.provider == "local-ctranslate2";
        let local_supported = snap_core::recognition::local_runtime::supported();
        let mut local = self.option(
            ("tr-local", 0),
            t("本地翻译"),
            Some(t(if local_supported {
                "离线翻译，文字不上传（仅中英互译）"
            } else {
                "翻译引擎下载和导入目前仅支持 Windows x64。"
            })),
            local_selected,
            local_supported,
        );
        if local_supported && !local_selected {
            local = local.on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.change(cx, |s| {
                    s.ai.provider = "local-ctranslate2".into();
                    s.ai.service_id.clear();
                    // 本地语言包只支持中英互译。
                    if !["zh-CN", "en"].contains(&s.ai.target_language.as_str()) {
                        s.ai.target_language = "zh-CN".into();
                    }
                });
                view.load_local(cx);
            }));
        }
        translators = translators.child(local);
        for (index, service) in s
            .recognition_services
            .iter()
            .filter(|x| x.supports_translation())
            .enumerate()
        {
            let selected = s.ai.service_id == service.id && s.ai.provider != "local-ctranslate2";
            let chosen = service.clone();
            let mut option = self.option(
                ("tr-service", index),
                service.name.clone(),
                Some(format!(
                    "{} · {}",
                    t(service_kind(service)),
                    key_hint(service)
                )),
                selected,
                true,
            );
            if !selected {
                option = option.on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    let chosen = chosen.clone();
                    view.change(cx, move |s| {
                        s.ai.service_id = chosen.id.clone();
                        s.ai.provider = if chosen.protocol == "deepl" {
                            "deepl"
                        } else {
                            "online-llm"
                        }
                        .into();
                        s.ai.endpoint = chosen.endpoint.clone();
                        s.ai.model = chosen.model.clone();
                    })
                }));
            }
            translators = translators.child(option);
        }
        let targets = self.segmented(
            "tr-target",
            TARGETS
                .iter()
                .position(|(code, _)| *code == s.ai.target_language)
                .unwrap_or(0),
            TARGETS
                .iter()
                .enumerate()
                .map(|(index, (_, label))| (index, t(label)))
                .collect(),
            cx,
            |s, v| s.ai.target_language = TARGETS[v].0.into(),
        );
        let mut translation_rows = vec![
            row(t("默认目标语言"), None, targets),
            row(
                t("超时（秒）"),
                None,
                self.stepper(
                    "tr-timeout",
                    s.ai.timeout_seconds as u32,
                    (5, 120, 5),
                    "",
                    cx,
                    |s, v| s.ai.timeout_seconds = u64::from(v),
                ),
            ),
            row(
                t("翻译后自动复制译文"),
                None,
                self.toggle("tr-auto-copy", s.ai.auto_copy, cx, |s, v| {
                    s.ai.auto_copy = v
                }),
            ),
            row(
                t("截图翻译覆盖原文"),
                None,
                self.toggle("tr-overlay", s.ai.overlay, cx, |s, v| s.ai.overlay = v),
            ),
        ];
        if local_selected {
            translation_rows.extend(self.local_rows(t, cx));
        }
        // ---- 服务管理 ----
        let mut list = div().flex().flex_col();
        for (index, service) in s.recognition_services.iter().enumerate() {
            let editing = self
                .service_editor
                .as_ref()
                .is_some_and(|e| e.existing.as_deref() == Some(service.id.as_str()));
            let item = service.clone();
            list = list.child(
                div()
                    .id(("service", index))
                    .px(px(12.0))
                    .py(px(8.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(rgb(col::border()))
                    .bg(rgb(if editing {
                        col::accent_soft()
                    } else {
                        col::surface()
                    }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(col::page_bg())))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_sm().child(service.name.clone()))
                            .child(div().text_xs().text_color(rgb(col::muted())).child(format!(
                                "{} · {} · {}",
                                t(service_kind(service)),
                                host(&service.endpoint),
                                key_hint(service)
                            ))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(col::accent()))
                            .child(t("编辑")),
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        view.edit_service(None, Some(&item), window, cx)
                    })),
            );
        }
        if s.recognition_services.is_empty() {
            list = list.child(
                div()
                    .px(px(12.0))
                    .py(px(14.0))
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .child(t("尚未添加服务。")),
            );
        }
        let mut presets = div().flex().flex_wrap().gap(px(6.0));
        for (index, preset) in PRESETS.iter().enumerate() {
            presets = presets.child(
                div()
                    .id(("preset", index))
                    .h(px(28.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(col::border()))
                    .bg(rgb(col::surface()))
                    .text_xs()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(col::page_bg())))
                    .child(format!("+ {}", t(preset.name)))
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        view.edit_service(Some(&PRESETS[index]), None, window, cx)
                    })),
            );
        }
        let mut elements = vec![
            heading(t("文字识别")).into_any_element(),
            div()
                .rounded(px(8.0))
                .bg(rgb(col::surface()))
                .border_1()
                .border_color(rgb(col::border()))
                .overflow_hidden()
                .child(engines)
                .into_any_element(),
            div().mt(px(8.0)).child(card(ocr_rows)).into_any_element(),
            heading(t("翻译")).into_any_element(),
            div()
                .rounded(px(8.0))
                .bg(rgb(col::surface()))
                .border_1()
                .border_color(rgb(col::border()))
                .overflow_hidden()
                .child(translators)
                .into_any_element(),
            div()
                .mt(px(8.0))
                .child(card(translation_rows))
                .into_any_element(),
            heading(t("服务管理")).into_any_element(),
            div()
                .rounded(px(8.0))
                .bg(rgb(col::surface()))
                .border_1()
                .border_color(rgb(col::border()))
                .overflow_hidden()
                .child(list)
                .into_any_element(),
            div().mt(px(8.0)).child(presets).into_any_element(),
        ];
        if let Some(editor) = &self.service_editor {
            elements.push(self.service_editor_card(editor, t, cx).into_any_element());
        }
        elements.push(
            div()
                .mt(px(10.0))
                .text_xs()
                .text_color(rgb(col::muted()))
                .child(t("在线服务会上传图片或文字；密钥只保存在系统凭据库。"))
                .into_any_element(),
        );
        elements
    }

    fn service_editor_card(
        &self,
        editor: &ServiceEditor,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let field = |label: String, state: &Entity<InputState>| {
            row(
                label,
                None,
                div()
                    .w(px(300.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(col::border()))
                    .bg(rgb(col::surface()))
                    .child(Input::new(state)),
            )
        };
        let protocol_label = match editor.protocol {
            "deepl" => "DeepL",
            "ocr-space" => "OCR.space",
            "paddle-ocr" => "PaddleOCR",
            _ => "OpenAI 兼容",
        };
        let mut rows = vec![
            row(
                t("协议"),
                None,
                div()
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .child(t(protocol_label)),
            ),
            field(t("名称"), &editor.name),
            field(t("接口地址"), &editor.endpoint),
        ];
        if editor.protocol != "deepl" {
            rows.push(field(t("模型"), &editor.model));
        }
        if editor.protocol == "openai-compatible" {
            let vision = editor.vision;
            rows.push(row(
                t("支持图片识别"),
                Some(t("模型能读取图片时可用于截图识字")),
                self.plain_toggle("service-vision", vision, cx, |view, on, cx| {
                    if let Some(editor) = view.service_editor.as_mut() {
                        editor.vision = on;
                    }
                    cx.notify();
                }),
            ));
        }
        rows.push(self.key_rows(editor, t, cx));
        let button = |id: &'static str, primary: bool, danger: bool| {
            div()
                .id(id)
                .h(px(28.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .text_xs()
                .cursor_pointer()
                .text_color(rgb(if primary {
                    0xffffff
                } else if danger {
                    col::danger()
                } else {
                    col::text()
                }))
                .bg(rgb(if primary {
                    col::accent()
                } else {
                    col::surface()
                }))
                .border_1()
                .border_color(rgb(if primary {
                    col::accent()
                } else {
                    col::border()
                }))
        };
        let mut actions = div()
            .flex()
            .gap(px(6.0))
            .child(
                button("service-save", true, false)
                    .child(t("保存"))
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                        view.save_service(false, window, cx);
                    })),
            )
            .child(
                button("service-cancel", false, false)
                    .child(t("取消"))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.service_editor = None;
                        cx.notify();
                    })),
            );
        let (test, result) = self.test_controls(editor, t, cx);
        actions = actions.child(test);
        if let Some(id) = editor.existing.clone() {
            actions = actions.child(
                button("service-delete", false, true)
                    .child(t("删除"))
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.delete_service(id.clone(), cx)
                    })),
            );
        }
        rows.push(row(
            String::new(),
            Some(t("先保存当前配置，再发送一次测试请求，可能产生少量费用。")),
            actions,
        ));
        if let Some(result) = result {
            rows.push(div().px(px(14.0)).pb(px(10.0)).child(result));
        }
        div()
            .mt(px(10.0))
            .child(heading(t(if editor.existing.is_some() {
                "编辑服务"
            } else {
                "新增服务"
            })))
            .child(card(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::{new_id, remove_service, service_kind, PRESETS};
    use snap_core::{recognition::services::RecognitionService, settings::Settings};

    fn service(protocol: &str, endpoint: &str, vision: bool) -> RecognitionService {
        RecognitionService {
            id: new_id(),
            name: "x".into(),
            protocol: protocol.into(),
            endpoint: endpoint.into(),
            model: "m".into(),
            vision,
            keys: vec!["default".into()],
        }
    }

    #[test]
    fn presets_and_ids_pass_core_validation() {
        for preset in PRESETS.iter().filter(|p| !p.endpoint.is_empty()) {
            let mut s = service(preset.protocol, preset.endpoint, preset.vision);
            s.model = preset.model.into();
            assert!(s.validate().is_ok(), "{}", preset.id);
        }
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn kinds_follow_protocol_and_location() {
        assert_eq!(
            service_kind(&service(
                "deepl",
                "https://api-free.deepl.com/v2/translate",
                false
            )),
            "在线翻译"
        );
        assert_eq!(
            service_kind(&service(
                "openai-compatible",
                "http://127.0.0.1:11434/v1/chat/completions",
                false
            )),
            "本机 AI"
        );
        assert_eq!(
            service_kind(&service(
                "openai-compatible",
                "https://api.example.com/v1",
                true
            )),
            "AI 视觉 / 翻译"
        );
    }

    #[test]
    fn removing_a_used_service_keeps_settings_valid() {
        let mut settings = Settings::default();
        let s = service(
            "openai-compatible",
            "https://api.example.com/v1/chat/completions",
            true,
        );
        settings.recognition_services.push(s.clone());
        settings.ocr.provider = "service".into();
        settings.ocr.service_id = s.id.clone();
        settings.ai.service_id = s.id.clone();
        settings.ai.provider = "online-llm".into();
        settings.ai.endpoint = s.endpoint.clone();
        settings.ai.model = s.model.clone();
        assert!(settings.validate().is_ok());
        remove_service(&mut settings, &s.id);
        assert!(settings.recognition_services.is_empty());
        assert_eq!(settings.ocr.provider, "local-ppocr");
        assert!(settings.validate().is_ok());
    }
}
