//! “识别与翻译 · 服务管理”页签（还原 Vue `ServiceSettings.vue` 的目录形态）。
//!
//! 本地引擎：PaddleOCR、系统 OCR、WeChat OCR、CTranslate2，显示品牌图标或字母块、说明与就绪状态；“管理”在条目下方
//! 展开该引擎的操作（下载资源、重新检测、语言包）。在线服务：已添加的连接（图标、名称与模型、密钥与用途状态），
//! “管理”打开编辑表单；“添加服务”按用途分组列出预设。服务商图标来自 lobe-icons（MIT，见 assets/brands/README.md）。
use super::super::{col, heading, note, row, SettingsView};
use super::{service_kind, Preset, PRESETS};
use gpui_kit::{component::Icon, *};
use snap_core::{recognition::services::RecognitionService, settings::Settings};
use std::sync::{Arc, OnceLock};

/// 服务商品牌。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Brand {
    Baidu,
    Deepl,
    Deepseek,
    Groq,
    Microsoft,
    Ollama,
    Openai,
}

impl Brand {
    fn svg(self) -> &'static [u8] {
        match self {
            Brand::Baidu => include_bytes!("../assets/brands/baidu-color.svg"),
            Brand::Deepl => include_bytes!("../assets/brands/deepl-color.svg"),
            Brand::Deepseek => include_bytes!("../assets/brands/deepseek-color.svg"),
            Brand::Groq => include_bytes!("../assets/brands/groq.svg"),
            Brand::Microsoft => include_bytes!("../assets/brands/microsoft-color.svg"),
            Brand::Ollama => include_bytes!("../assets/brands/ollama.svg"),
            Brand::Openai => include_bytes!("../assets/brands/openai.svg"),
        }
    }

    /// 单色图标随文字色着色；彩色图标按原色绘制。
    fn monochrome(self) -> bool {
        matches!(self, Brand::Groq | Brand::Ollama | Brand::Openai)
    }

    fn image(self) -> Arc<Image> {
        static CACHE: OnceLock<std::sync::Mutex<Vec<(Brand, Arc<Image>)>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, image)) = cache.iter().find(|(b, _)| *b == self) {
            return image.clone();
        }
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, self.svg().to_vec()));
        cache.push((self, image.clone()));
        image
    }
}

/// 按预设或接口地址识别服务商（Vue `serviceBrand`）。
fn brand_for(preset: &str, endpoint: &str) -> Option<Brand> {
    match preset {
        "deepseek" => return Some(Brand::Deepseek),
        "groq" | "groq-vision" => return Some(Brand::Groq),
        "deepl" => return Some(Brand::Deepl),
        "paddle-ocr" => return Some(Brand::Baidu),
        _ => {}
    }
    let url = url::Url::parse(endpoint).ok()?;
    let host = url.host_str()?.to_owned();
    if host == "api.openai.com" || host.ends_with(".openai.com") {
        Some(Brand::Openai)
    } else if host.ends_with("deepseek.com") {
        Some(Brand::Deepseek)
    } else if host.ends_with("groq.com") {
        Some(Brand::Groq)
    } else if host.ends_with("deepl.com") {
        Some(Brand::Deepl)
    } else if ["localhost", "127.0.0.1", "[::1]"].contains(&host.as_str())
        && url.port() == Some(11434)
    {
        Some(Brand::Ollama)
    } else {
        None
    }
}

/// 已保存服务对应的预设（Vue `servicePresetId`）。
fn preset_id(service: &RecognitionService) -> &'static str {
    if service.protocol != "openai-compatible" {
        return PRESETS
            .iter()
            .find(|p| p.protocol == service.protocol)
            .map_or("custom", |p| p.id);
    }
    PRESETS
        .iter()
        .find(|p| {
            p.protocol == service.protocol
                && p.endpoint == service.endpoint
                && p.vision == service.vision
        })
        .map_or("custom", |p| p.id)
}

/// 28×28 图标：品牌图标在白底细边框里，否则为彩色字母块（Vue `.service-mono`）。
fn tile(brand: Option<Brand>, mono: &str, color: u32) -> Div {
    let base = div()
        .size(px(28.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(7.0));
    match brand {
        Some(brand) if brand.monochrome() => base
            .bg(rgb(col::surface()))
            .border_1()
            .border_color(rgb(col::border()))
            .child(svg_icon(brand)),
        Some(brand) => base
            .bg(rgb(col::surface()))
            .border_1()
            .border_color(rgb(col::border()))
            .child(img(brand.image()).size(px(18.0))),
        None => base
            .bg(rgb(color))
            .text_size(px(11.0))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(0xffffff))
            .child(mono.to_owned()),
    }
}

/// 引擎下拉里的 18×18 图标与连接方式（Vue `ProviderMenu` 的 `.provider-icon` 与连接标记）。
pub(in super::super) fn provider_icon(
    provider: &str,
    service: Option<&RecognitionService>,
    t: &dyn Fn(&str) -> String,
) -> (Div, Option<String>) {
    let base = || div().size(px(18.0)).flex().flex_none().items_center().justify_center().rounded(px(5.0));
    let logo = |brand: Brand| {
        if brand.monochrome() {
            base().child(Icon::default().data(brand.svg()).size(px(16.0)).text_color(rgb(col::text())))
        } else {
            base().child(img(brand.image()).size(px(16.0)))
        }
    };
    let mono = |text: &str, color: u32| {
        base()
            .bg(rgb(color))
            .text_size(px(9.0))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(0xffffff))
            .child(text.to_owned())
    };
    let offline = Some(t("离线"));
    match (provider, service) {
        ("local-ppocr", _) => (logo(Brand::Baidu), offline),
        ("windows-media", _) if cfg!(target_os = "macos") => (mono("AV", 0x1d1d1f), offline),
        ("windows-media", _) => (logo(Brand::Microsoft), offline),
        ("wechat-ocr", _) => (mono("WX", 0x07a35a), offline),
        ("local-ctranslate2", _) => (mono("CT", 0x475569), offline),
        (_, Some(service)) => {
            let preset = preset_id(service);
            let local = url::Url::parse(&service.endpoint)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .is_some_and(|h| ["localhost", "127.0.0.1", "[::1]"].contains(&h.as_str()));
            let icon = match brand_for(preset, &service.endpoint) {
                Some(brand) => logo(brand),
                None => {
                    let (text, color) = monogram(preset, &service.name);
                    mono(&text, color)
                }
            };
            (icon, Some(t(if local { "本机 API" } else { "在线 API" })))
        }
        _ => (mono("?", 0x6b7280), None),
    }
}

fn svg_icon(brand: Brand) -> Icon {
    Icon::default()
        .data(brand.svg())
        .size(px(18.0))
        .text_color(rgb(col::text()))
}

/// 名称首字母块（Vue `monograms` 与回退规则）。
fn monogram(key: &str, label: &str) -> (String, u32) {
    match key {
        "ocr-space" => ("OS".into(), 0x2f7d4f),
        "custom" => ("+".into(), 0x7c5cdb),
        _ => {
            let letters: String = label
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .collect::<String>()
                .to_uppercase();
            (
                if letters.is_empty() {
                    "?".into()
                } else {
                    letters
                },
                0x6b7280,
            )
        }
    }
}

/// 预设的能力（与 `RecognitionService::supports_ocr/supports_translation` 规则一致）。
fn preset_ocr(p: &Preset) -> bool {
    matches!(p.protocol, "ocr-space" | "paddle-ocr")
        || (p.protocol == "openai-compatible" && p.vision)
}
fn preset_translation(p: &Preset) -> bool {
    matches!(p.protocol, "openai-compatible" | "deepl")
}

/// 目录中的一行：图标、名称与说明、状态点与状态文字、右侧“管理”。
fn item(
    icon: Div,
    name: String,
    description: Option<String>,
    ready: bool,
    status: String,
    action: Stateful<Div>,
    first: bool,
) -> Div {
    let line = div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .min_h(px(54.0))
        .px(px(12.0))
        .py(px(8.0))
        .child(icon)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.0))
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name)
                        .children(description.map(|d| {
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(rgb(col::muted()))
                                .truncate()
                                .child(d)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .text_xs()
                        .text_color(rgb(col::muted()))
                        .child(
                            div()
                                .size(px(6.0))
                                .flex_none()
                                .rounded_full()
                                .bg(rgb(if ready { col::success() } else { col::line() })),
                        )
                        .child(div().truncate().child(status)),
                ),
        )
        .child(action);
    if first {
        line
    } else {
        line.border_t_1().border_color(rgb(col::sep()))
    }
}

/// 白底细线框的“管理”按钮（Vue `.service-action.outlined`）。
fn manage(id: impl Into<ElementId>, label: String, enabled: bool) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(24.0))
        .px(px(10.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(col::line()))
        .bg(rgb(col::surface()))
        .text_size(px(13.0))
        .child(label);
    if enabled {
        button.cursor_pointer().hover(|s| s.bg(rgb(col::hover())))
    } else {
        button.opacity(0.45)
    }
}

impl SettingsView {
    pub(in super::super) fn services_tab(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let macos = cfg!(target_os = "macos");
        // ---- 本地引擎 ----
        let unread = t("状态未读取");
        let (ocr, translate) = (t("文字识别"), t("翻译"));
        let paddle_ready = self.resources.as_ref().is_some_and(|r| r.ocr_ready());
        let paddle_status = match &self.resources {
            None => unread.clone(),
            Some(r) if r.ocr_ready() => t("已就绪"),
            Some(_) => t("未下载识别资源"),
        };
        let languages = match &self.recognition.languages {
            Some(Ok(list)) => Some(list.len()),
            _ => None,
        };
        let system_supported = cfg!(any(windows, target_os = "macos"));
        let system_status = match (&self.recognition.languages, system_supported) {
            (_, false) => t("当前平台不支持"),
            (None, _) => unread.clone(),
            (Some(Err(_)), _) => t("未安装语言包"),
            (Some(Ok(list)), _) if macos => {
                t("支持 {0} 种识别语言").replace("{0}", &list.len().to_string())
            }
            (Some(Ok(list)), _) if list.is_empty() => t("未安装语言包"),
            (Some(Ok(list)), _) => {
                t("已安装 {0} 个 OCR 语言包").replace("{0}", &list.len().to_string())
            }
        };
        let wechat_status = match &self.recognition.wechat {
            None => unread.clone(),
            Some(w) if !w.supported => t("当前平台不支持"),
            Some(w) if w.detected => t("已找到 WeChat OCR 组件"),
            Some(_) => t("未找到 WeChat OCR 组件"),
        };
        let ct2_ready = self
            .local
            .as_ref()
            .and_then(|l| l.as_ref().ok())
            .is_some_and(|l| l.runtime_ready && l.models.iter().any(|m| m.ready));
        let ct2_status = match &self.local {
            None => unread.clone(),
            Some(_) if ct2_ready => t("已安装"),
            Some(_) => t("未安装"),
        };
        let engines: [(&str, &str, String, Div, bool, bool, String); 4] = [
            (
                "ppocr",
                "PaddleOCR",
                t("PP-OCRv6 · 离线"),
                tile(Some(Brand::Baidu), "PP", 0x2563a8),
                paddle_ready,
                true,
                format!("{paddle_status} · {ocr}"),
            ),
            (
                "windows",
                if macos { "Apple Vision" } else { "Windows OCR" },
                t("系统自带 · 离线"),
                if macos {
                    tile(None, "AV", 0x1d1d1f)
                } else {
                    tile(Some(Brand::Microsoft), "W", 0x0067b8)
                },
                system_supported && languages.is_some_and(|n| n > 0),
                system_supported,
                format!("{system_status} · {ocr}"),
            ),
            (
                "wechat",
                "WeChat OCR",
                t("实验性 · 离线"),
                tile(None, "WX", 0x07a35a),
                self.recognition.wechat.as_ref().is_some_and(|w| w.detected),
                self.recognition.wechat.as_ref().is_none_or(|w| w.supported),
                format!("{wechat_status} · {ocr}"),
            ),
            (
                "ct2",
                "CTranslate2",
                t("本地中英互译 · 离线"),
                tile(None, "CT", 0x475569),
                ct2_ready,
                true,
                format!("{ct2_status} · {translate}"),
            ),
        ];
        let mut local: Vec<AnyElement> = Vec::new();
        for (index, (_, name, description, icon, ready, enabled, status)) in
            engines.into_iter().enumerate()
        {
            let open = self.managing_local == Some(index);
            let action = manage(
                ("local-manage", index),
                t(if open { "收起" } else { "管理" }),
                enabled,
            )
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.managing_local = if view.managing_local == Some(index) {
                    None
                } else {
                    Some(index)
                };
                match index {
                    0 => view.load_resources(cx),
                    1 | 2 => view.load_recognition(cx),
                    _ => view.load_local(cx),
                }
                cx.notify();
            }));
            local.push(
                item(
                    icon,
                    name.to_owned(),
                    Some(description),
                    ready,
                    status,
                    action,
                    index == 0,
                )
                .into_any_element(),
            );
            if open {
                // 展开的管理区：沿用各引擎已有的操作行。
                let rows: Vec<Div> = match index {
                    0 => vec![self.ocr_resource_row(t, cx)],
                    1 | 2 => {
                        let label = t(if index == 1 {
                            "系统 OCR 语言包"
                        } else {
                            "WeChat OCR 组件"
                        });
                        let hint = if index == 1 {
                            system_status.clone()
                        } else {
                            wechat_status.clone()
                        };
                        let idle = !self.recognition.loading;
                        let mut recheck = self.action_button(
                            if index == 1 {
                                "local-recheck-system"
                            } else {
                                "local-recheck-wechat"
                            },
                            t(if idle {
                                "重新检测"
                            } else {
                                "正在检查…"
                            }),
                            idle,
                        );
                        if idle {
                            recheck =
                                recheck.on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                    view.load_recognition(cx)
                                }));
                        }
                        vec![row(label, Some(hint), recheck)]
                    }
                    _ => self.local_rows(t, cx),
                };
                local.push(
                    div()
                        .mx(px(12.0))
                        .mb(px(10.0))
                        .pl(px(12.0))
                        .rounded(px(8.0))
                        .bg(rgb(col::page_bg()))
                        .children(rows.into_iter().map(|r| r.border_b_0()))
                        .into_any_element(),
                );
            }
        }

        // ---- 在线服务 ----
        let mut online: Vec<AnyElement> = Vec::new();
        for (index, service) in s.recognition_services.iter().enumerate() {
            let preset = preset_id(service);
            let brand = brand_for(preset, &service.endpoint);
            let (mono, color) = monogram(preset, &service.name);
            let configured = self.recognition.credentials.get(&service.id).copied();
            let key_status = match configured {
                Some(0) => t("未配置密钥"),
                Some(n) if n > 1 => t("{0} 个密钥").replace("{0}", &n.to_string()),
                Some(_) => t("已配置密钥"),
                None => t("凭据状态未读取"),
            };
            let active = (s.ocr.provider == "service" && s.ocr.service_id == service.id)
                || (s.ai.provider != "local-ctranslate2" && s.ai.service_id == service.id);
            let uses = [
                service.supports_ocr().then(|| t("文字识别")),
                service.supports_translation().then(|| t("翻译")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let status = [
                Some(key_status),
                active.then(|| t("当前使用")),
                (!uses.is_empty()).then_some(uses),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let chosen = service.clone();
            let action = manage(("service-manage", index), t("管理"), true).on_click(cx.listener(
                move |view, _: &ClickEvent, window, cx| {
                    view.edit_service(None, Some(&chosen), window, cx)
                },
            ));
            let description = (!service.model.is_empty())
                .then(|| service.model.clone())
                .or_else(|| Some(t(service_kind(service))));
            online.push(
                item(
                    tile(brand, &mono, color),
                    service.name.clone(),
                    description,
                    configured.is_some_and(|n| n > 0),
                    status,
                    action,
                    index == 0,
                )
                .into_any_element(),
            );
        }
        if s.recognition_services.is_empty() {
            online.push(
                div()
                    .id("service-empty")
                    .min_h(px(48.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(col::muted()))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(col::accent_fg())))
                    .child(format!("+ {}", t("尚未添加在线服务，点击选择要接入的服务")))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.service_picker = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let add = div()
            .id("service-add")
            .text_xs()
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(col::accent_fg()))
            .cursor_pointer()
            .child(t("添加服务"))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.service_picker = !view.service_picker;
                cx.notify();
            }));

        let mut elements = vec![
            heading(t("本地引擎")).into_any_element(),
            div()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(col::border()))
                .bg(rgb(col::surface()))
                .overflow_hidden()
                .children(local)
                .into_any_element(),
            heading(t("在线服务")).child(add).into_any_element(),
        ];
        elements.push(
            div()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(col::border()))
                .bg(rgb(col::surface()))
                .overflow_hidden()
                .children(online)
                .into_any_element(),
        );
        elements.push(note(t("密钥保存在系统凭据中，不随配置导出。")).into_any_element());
        elements
    }

    /// “添加服务”：按用途分组的预设（Vue 添加服务面板）。
    fn preset_picker(
        &self,
        s: &Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let groups: [(&str, &str, Box<dyn Fn(&Preset) -> bool>); 4] = [
            (
                "文字识别与翻译",
                "支持图片输入的模型",
                Box::new(|p: &Preset| p.id != "custom" && preset_ocr(p) && preset_translation(p)),
            ),
            (
                "文字识别",
                "在线 OCR 接口",
                Box::new(|p: &Preset| preset_ocr(p) && !preset_translation(p)),
            ),
            (
                "翻译",
                "文本翻译接口",
                Box::new(|p: &Preset| p.id != "custom" && !preset_ocr(p) && preset_translation(p)),
            ),
            (
                "自定义",
                "OpenAI 兼容接口或本机模型",
                Box::new(|p: &Preset| p.id == "custom"),
            ),
        ];
        let mut panel = div();
        for (title, _, filter) in groups.iter() {
            let presets: Vec<(usize, &Preset)> = PRESETS
                .iter()
                .enumerate()
                .filter(|(_, p)| filter(p))
                .collect();
            if presets.is_empty() {
                continue;
            }
            let mut grid = div().flex().flex_wrap().gap(px(4.0));
            for (index, preset) in presets {
                let added = s
                    .recognition_services
                    .iter()
                    .any(|x| preset_id(x) == preset.id);
                let label = match preset.id {
                    "custom" => t("自定义"),
                    "paddle-ocr" => "Paddle API".into(),
                    "deepl" => "DeepL".into(),
                    _ => preset.name.into(),
                };
                let (mono, color) = monogram(preset.id, preset.name);
                grid =
                    grid.child(
                        div()
                            .id(("preset", index))
                            .w(px(164.0))
                            .min_h(px(36.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .rounded(px(6.0))
                            .text_size(px(13.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(col::hover())))
                            .child(tile(brand_for(preset.id, preset.endpoint), &mono, color))
                            .child(div().flex_1().truncate().child(label))
                            .children(added.then(|| {
                                div().size(px(6.0)).rounded_full().bg(rgb(col::success()))
                            }))
                            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                                view.service_picker = false;
                                view.edit_service(Some(&PRESETS[index]), None, window, cx)
                            })),
                    );
            }
            panel = panel.child(
                div()
                    .mb(px(14.0))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.0))
                            .mb(px(6.0))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t(title)),
                    )
                    .child(grid),
            );
        }
        panel
    }

    /// “添加服务”与服务编辑器的弹窗（Vue 的服务对话框）；铺满设置窗口，点遮罩或 Esc 关闭。
    pub(in super::super) fn service_dialog(&self, s: &Settings, t: &dyn Fn(&str) -> String, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (title, body) = match &self.service_editor {
            Some(editor) => (
                t(if editor.existing.is_some() { "编辑服务" } else { "添加服务" }),
                self.service_editor_card(editor, t, cx),
            ),
            None if self.service_picker => (t("添加服务"), self.preset_picker(s, t, cx)),
            None => return None,
        };
        let close = div()
            .id("service-dialog-close")
            .size(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(col::hover())))
            .child(Icon::default().data(super::super::icons::X).size(px(14.0)).text_color(rgb(col::muted())))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.service_picker = false;
                view.service_editor = None;
                cx.notify();
            }));
        let dialog = div()
            .id("service-dialog")
            .w(px(560.0))
            .max_w(relative(0.92))
            .max_h(relative(0.88))
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::page_bg()))
            .shadow_lg()
            // 弹窗内的点击不传到遮罩。
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .h(px(46.0))
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(col::border()))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title)
                    .child(close),
            )
            .child(div().id("service-dialog-body").flex_1().min_h_0().overflow_y_scroll().p(px(16.0)).child(body));
        Some(
            deferred(
                div()
                    .id("service-dialog-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x0000004d))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseDownEvent, _, cx| {
                            view.service_picker = false;
                            view.service_editor = None;
                            cx.notify();
                        }),
                    )
                    .child(dialog),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{brand_for, Brand};

    #[test]
    fn brands_follow_preset_then_host() {
        assert_eq!(brand_for("deepl", ""), Some(Brand::Deepl));
        assert_eq!(brand_for("paddle-ocr", ""), Some(Brand::Baidu));
        assert_eq!(
            brand_for("custom", "https://api.openai.com/v1/chat/completions"),
            Some(Brand::Openai)
        );
        assert_eq!(
            brand_for("custom", "http://127.0.0.1:11434/v1/chat/completions"),
            Some(Brand::Ollama)
        );
        assert_eq!(brand_for("custom", "https://example.com/v1"), None);
    }
}
