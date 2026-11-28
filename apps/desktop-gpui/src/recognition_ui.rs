//! 截图识别结果窗口的界面：文字识别、翻译与二维码 / 条形码结果。
//!
//! 文字识别：左侧浅底原图（识别框描边），右侧整理工具条与可编辑文字，底栏为行数/字数与“翻译文字”“复制文字”。
//! 截图翻译：默认只显示原图一栏（译文原地画在图上），底栏提供“返回识别”、显示原文/译文与“复制译文”；
//! 出错、隐藏原图或无法覆盖时才显示原文卡片、语言条与译文卡片。
//! 原图栏可缩放（滚轮 / Ctrl +/-）、放大后拖动平移，双击或点右上角比例还原。
//! 二维码 / 条形码：按原图阅读顺序排列的结果卡片（格式名、链接或文本、内容与操作）与底栏。
use super::{Mode, Panel, TARGETS};
use crate::theme;
use gpui_kit::{
    component::{Icon, input::Textarea},
    *,
};
use snap_core::recognition;

mod icons {
    macro_rules! icon {
        ($name:ident, $file:literal) => {
            pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));
        };
    }
    icon!(WRAP, "wrap-text");
    icon!(ERASER, "eraser");
    icon!(RESTORE, "rotate-ccw");
    icon!(IMAGE, "image");
    icon!(LANGUAGES, "languages");
    icon!(COPY, "copy");
    icon!(CHECK, "check");
    icon!(BACK, "chevron-left");
    icon!(PIN, "pin");
    icon!(PIN_FILLED, "pin-filled");
    icon!(QR, "qr-code");
    icon!(LINK, "link");
    icon!(TYPE, "type");
    icon!(RETRY, "rotate-ccw");
    icon!(EYE, "eye");
}

/// `--ocr-*` 与 `--qr-*` 色值。
struct Colors {
    bg: u32,
    soft: u32,
    border: u32,
    text: u32,
    muted: u32,
    accent: u32,
    box_line: u32,
    danger: u32,
    hover: u32,
    success: u32,
    /// 翻译面板的卡片与条带（`--card-bg` / `--bar-bg`）。
    card: u32,
    bar: u32,
}

fn colors() -> Colors {
    if theme::dark() {
        Colors {
            bg: 0x1b1f24,
            soft: 0x14181d,
            border: 0x39424e,
            text: 0xe6edf3,
            muted: 0x9aa7b4,
            accent: 0x79a6ff,
            box_line: 0x79a6ff99,
            danger: 0xff8a80,
            hover: 0x2b323c,
            success: 0x73d69a,
            card: 0x27272a,
            bar: 0x323237,
        }
    } else {
        Colors {
            bg: 0xffffff,
            soft: 0xf3f5f8,
            border: 0xe1e4e8,
            text: 0x24292f,
            muted: 0x6b7280,
            accent: 0x2563eb,
            box_line: 0x2563eb8c,
            danger: 0xb42318,
            hover: 0xeef1f4,
            success: 0x24834b,
            card: 0xffffff,
            bar: 0xeeeef1,
        }
    }
}

fn icon(data: &'static [u8], size: f32, color: u32) -> Icon {
    Icon::default()
        .data(data)
        .size(px(size))
        .text_color(rgb(color))
}

/// 底栏按钮（34 像素高）：主色或细线框。
fn footer_button(
    id: impl Into<ElementId>,
    data: Option<&'static [u8]>,
    label: String,
    primary: bool,
    enabled: bool,
    c: &Colors,
) -> Stateful<Div> {
    let fg = if primary { 0xffffff } else { c.text };
    let button = div()
        .id(id)
        .h(px(34.0))
        .px(px(14.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(fg))
        .children(data.map(|d| icon(d, 15.0, fg)))
        .child(label);
    let button = if primary {
        button.bg(rgb(0x2563eb))
    } else {
        button.border_1().border_color(rgb(c.border)).bg(rgb(c.bg))
    };
    if !enabled {
        return button.opacity(0.5);
    }
    let hover = c.hover;
    button.cursor_pointer().hover(move |s| {
        if primary {
            s.bg(rgb(0x1d4ed8))
        } else {
            s.bg(rgb(hover))
        }
    })
}

/// 工具条的 30×30 图标按钮。
fn tool_button(
    id: impl Into<ElementId>,
    data: &'static [u8],
    tip: String,
    pressed: bool,
    enabled: bool,
    c: &Colors,
) -> Stateful<Div> {
    let color = if pressed { c.accent } else { c.muted };
    let button = div()
        .id(id)
        .size(px(30.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .child(icon(data, 15.0, color))
        .tooltip(move |_, cx| {
            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                .into()
        });
    if enabled {
        let hover = c.hover;
        button.cursor_pointer().hover(move |s| s.bg(rgb(hover)))
    } else {
        button.opacity(0.4)
    }
}

/// 整理空格：去掉行尾空白、合并连续空格、中文之间的空格。
pub(super) fn tidy_spaces(text: &str) -> String {
    let is_cjk = |c: char| matches!(c as u32, 0x3000..=0x303f | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xff00..=0xffef);
    let mut lines = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.trim_end().chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == ' ' || c == '\t' || c == '\u{3000}' {
                let mut j = i;
                while j < chars.len() && matches!(chars[j], ' ' | '\t' | '\u{3000}') {
                    j += 1;
                }
                let before = out.chars().last();
                let after = chars.get(j).copied();
                let between_cjk = before.is_some_and(is_cjk) && after.is_some_and(is_cjk);
                if !between_cjk && before.is_some() {
                    out.push(' ');
                }
                i = j;
            } else {
                out.push(c);
                i += 1;
            }
        }
        lines.push(out);
    }
    lines.join("\n")
}

impl Panel {
    fn image_pane(&self, width: f32, height: f32, c: &Colors, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let preview = self.preview.clone().filter(|_| self.show_image)?;
        let image = self.image.as_ref()?;
        let (iw, ih) = (image.width() as f32, image.height() as f32);
        let fit = self.fit_scale().unwrap_or(0.0);
        let k = fit * self.zoom;
        let (w, h) = (iw * k, ih * k);
        let (pan_x, pan_y) = self.clamped_pan();
        // 原图居中于原图栏（底部 24 像素留给识别框提示），再按平移量偏移。
        let mut figure = div()
            .absolute()
            .left(px(width / 2.0 + pan_x - w / 2.0))
            .top(px((height - 24.0) / 2.0 + pan_y - h / 2.0))
            .w(px(w))
            .h(px(h))
            .child(
                img(preview)
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            );
        let mut boxes = 0;
        // 覆盖原文：每段先铺取样背景遮住原文，再按框宽逐行排版译文；字号从原文行高起步，放不下时缩小。
        let overlay = self
            .overlay
            .as_ref()
            .filter(|_| self.mode == Mode::Translate && self.show_overlay);
        for item in overlay.into_iter().flatten() {
            let r = item.rect;
            let (w, h) = (r.width * k, r.height * k);
            let (font, lines) = recognition::overlay::fit_text(
                &item.text,
                w,
                h,
                item.line_height * k,
                8.0,
                &recognition::overlay::estimate_width,
            );
            let pad = (item.line_height * k * 0.08).max(1.0);
            let line = font * recognition::overlay::LINE_SPACING;
            let top = ((h - lines.len() as f32 * line) / 2.0).max(0.0);
            let [br, bg, bb] = item.colors.background;
            let [fr, fg, fb] = item.colors.foreground;
            let mut block = div()
                .absolute()
                .left(px(r.x * k - pad))
                .top(px(r.y * k - pad))
                .w(px(w + pad * 2.0))
                .h(px(h + pad * 2.0))
                .pl(px(pad))
                .pt(px(pad + top))
                .flex()
                .flex_col()
                .bg(rgb(u32::from_be_bytes([0, br, bg, bb])))
                .text_color(rgb(u32::from_be_bytes([0, fr, fg, fb])))
                .text_size(px(font))
                .line_height(px(line));
            for text in lines {
                block = block.child(div().whitespace_nowrap().child(text));
            }
            figure = figure.child(block);
        }
        if self.mode == Mode::Ocr {
            for line in self.ocr.iter().flat_map(|r| r.lines.iter()) {
                let Some(points) = line.polygon.as_ref().filter(|p| !p.is_empty()) else {
                    continue;
                };
                let (x0, y0) = points
                    .iter()
                    .fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.x), y.min(p.y)));
                let (x1, y1) = points
                    .iter()
                    .fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.x), y.max(p.y)));
                boxes += 1;
                figure = figure.child(
                    div()
                        .absolute()
                        .left(px(x0 * k))
                        .top(px(y0 * k))
                        .w(px((x1 - x0) * k))
                        .h(px((y1 - y0) * k))
                        .border_1()
                        .border_color(rgba(c.box_line)),
                );
            }
        }
        let zoomed = self.zoom > 1.0;
        let dragging = self.drag.is_some();
        let pane = div()
            .id("recognition-image")
            .relative()
            .w(px(width))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .bg(rgb(c.soft))
            .border_r_1()
            .border_color(rgb(c.border))
            .on_scroll(cx.listener(|p, event: &ScrollEvent, _, cx| {
                let dy = f32::from(event.delta.pixel_delta(px(50.0)).y);
                if dy != 0.0 {
                    // 鼠标一格约 ×1.13，触控板按像素平滑缩放。
                    let factor = 1.0025_f32.powf(dy.clamp(-200.0, 200.0));
                    let at = (f32::from(event.position.x), f32::from(event.position.y));
                    p.zoom_at(factor, Some(at), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|p, event: &MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        p.reset_zoom(cx);
                    } else if p.zoom > 1.0 {
                        p.drag = Some((f32::from(event.position.x), f32::from(event.position.y)));
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|p, event: &MouseMoveEvent, _, cx| {
                let Some((x0, y0)) = p.drag else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    p.drag = None;
                    cx.notify();
                    return;
                }
                let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
                p.pan = (p.pan.0 + x - x0, p.pan.1 + y - y0);
                p.pan = p.clamped_pan();
                p.drag = Some((x, y));
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|p, _: &MouseUpEvent, _, cx| {
                    p.drag = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|p, _: &MouseUpEvent, _, cx| {
                    p.drag = None;
                    cx.notify();
                }),
            )
            .child(figure)
            .children((boxes > 0).then(|| {
                div()
                    .absolute()
                    .bottom(px(6.0))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_size(px(11.0))
                    .text_color(rgb(c.muted))
                    .child(self.t("识别框为识别出的文字位置"))
            }))
            .children(zoomed.then(|| {
                let hover = c.hover;
                let tip = self.t("还原缩放");
                div()
                    .id("recognition-zoom")
                    .absolute()
                    .top(px(8.0))
                    .right(px(8.0))
                    .h(px(24.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(c.border))
                    .bg(rgb(c.bg))
                    .text_size(px(12.0))
                    .text_color(rgb(c.muted))
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgb(hover)))
                    .tooltip(move |_, cx| cx.new(|_| crate::annotate::Tooltip(tip.clone().into())).into())
                    .child(format!("{}%", (k * 100.0).round()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|p, _: &ClickEvent, _, cx| p.reset_zoom(cx)))
            }));
        Some(if dragging {
            pane.cursor_grabbing()
        } else if zoomed {
            pane.cursor_grab()
        } else {
            pane
        })
    }

    fn ocr_main(&self, cx: &mut Context<Self>, c: &Colors) -> Div {
        let has = self.ocr.is_some() && self.busy.is_none();
        let text = self.draft.read(cx).value().to_string();
        let filled = !text.trim().is_empty();
        let changed = self.ocr.as_ref().is_some_and(|r| r.full_text != text);
        let show = self.show_image;
        let tools = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .px(px(8.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(rgb(c.border))
            .child(
                tool_button(
                    "ocr-merge",
                    icons::WRAP,
                    self.t("合并换行"),
                    false,
                    has && filled,
                    c,
                )
                .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                    p.fill = Some(recognition::ocr::merge_lines(&p.draft.read(cx).value()));
                    cx.notify();
                })),
            )
            .child(
                tool_button(
                    "ocr-tidy",
                    icons::ERASER,
                    self.t("整理空格"),
                    false,
                    has && filled,
                    c,
                )
                .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                    p.fill = Some(tidy_spaces(&p.draft.read(cx).value()));
                    cx.notify();
                })),
            )
            .child(
                tool_button(
                    "ocr-restore",
                    icons::RESTORE,
                    self.t("还原识别结果"),
                    false,
                    has && changed,
                    c,
                )
                .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                    p.fill = p.ocr.as_ref().map(|r| r.full_text.clone());
                    cx.notify();
                })),
            )
            .child(div().flex_1())
            .child(
                tool_button(
                    "ocr-image",
                    icons::IMAGE,
                    self.t(if show { "隐藏原图" } else { "显示原图" }),
                    show,
                    true,
                    c,
                )
                .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                    p.show_image = !p.show_image;
                    cx.notify();
                })),
            );
        div()
            .flex_1()
            .min_w(px(320.0))
            .h_full()
            .flex()
            .flex_col()
            .child(tools)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(16.0))
                    .py(px(12.0))
                    .text_size(px(14.0))
                    .line_height(relative(1.75))
                    .child(Textarea::new(&self.draft).appearance(false).h_full()),
            )
            .children(self.error.clone().map(|e| {
                div()
                    .mx(px(12.0))
                    .mb(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(rgba(0xb4231814))
                    .text_xs()
                    .text_color(rgb(c.danger))
                    .child(e)
            }))
    }

    fn translate_main(&self, cx: &mut Context<Self>, c: &Colors) -> Div {
        let idle = self.busy.is_none();
        let has = !self.draft.read(cx).value().trim().is_empty();
        let translated = !self.translated.read(cx).value().trim().is_empty();
        let mut languages = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(2.0))
            .rounded(px(10.0))
            .bg(rgb(c.bar));
        for (index, (code, name)) in TARGETS.into_iter().enumerate() {
            let active = self.target == code;
            let bar = c.card;
            let mut chip = div()
                .id(("translate-target", index))
                .h(px(28.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .child(self.t(name))
                .on_click(cx.listener(move |p, _: &ClickEvent, _, cx| p.set_target(code, cx)));
            chip = if active {
                chip.bg(rgb(c.card))
                    .text_color(rgb(0x2563eb))
                    .font_weight(FontWeight::SEMIBOLD)
            } else {
                chip.text_color(rgb(c.muted)).hover(move |s| s.bg(rgb(bar)))
            };
            languages = languages.child(chip);
        }
        let service = recognition::services::translation_config(&self.settings)
            .ok()
            .map(|ai| match ai.provider.as_str() {
                "local-ctranslate2" => self.t("本地翻译"),
                "deepl" => "DeepL".into(),
                _ => url::Url::parse(&ai.endpoint)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| self.t("兼容服务")),
            })
            .unwrap_or_else(|| self.t("翻译服务"));
        let card = |content: Div| {
            div()
                .flex()
                .flex_col()
                .rounded(px(10.0))
                .overflow_hidden()
                .bg(rgb(c.card))
                .child(content)
        };
        let source = card(
            div()
                .flex_1()
                .min_h(px(120.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p(px(12.0))
                        .text_size(px(14.0))
                        .line_height(relative(1.65))
                        .child(Textarea::new(&self.draft).appearance(false).h_full()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(8.0))
                        .py(px(4.0))
                        .child(
                            tool_button(
                                "translate-copy-source",
                                icons::COPY,
                                self.t("复制原文"),
                                false,
                                has,
                                c,
                            )
                            .on_click(cx.listener(
                                |p, _: &ClickEvent, _, cx| {
                                    let value = p.draft.read(cx).value().to_string();
                                    p.copy(value, "原文已复制。", cx);
                                },
                            )),
                        )
                        .child(div().flex_1())
                        .child(
                            tool_button(
                                "translate-send",
                                icons::LANGUAGES,
                                self.t("翻译"),
                                true,
                                has && idle,
                                c,
                            )
                            .on_click(cx.listener(|p, _: &ClickEvent, _, cx| p.translate(cx))),
                        ),
                ),
        )
        .flex_1();
        let status: Option<AnyElement> = if self.busy.is_some() {
            Some(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(c.muted))
                    .child(self.t("翻译中"))
                    .into_any_element(),
            )
        } else if translated {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(px(11.0))
                    .text_color(rgb(c.success))
                    .child(icon(icons::CHECK, 13.0, c.success))
                    .child(self.t("已完成"))
                    .into_any_element(),
            )
        } else {
            None
        };
        let output = card(
            div()
                .flex_1()
                .min_h(px(140.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .min_h(px(32.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .bg(rgb(c.bar))
                        .child(icon(icons::LANGUAGES, 16.0, 0x2563eb))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(service))
                        .child(div().flex_1())
                        .children(status),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p(px(12.0))
                        .text_size(px(14.0))
                        .line_height(relative(1.65))
                        .child(Textarea::new(&self.translated).appearance(false).h_full()),
                )
                .children(self.error.clone().map(|e| {
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .text_xs()
                        .text_color(rgb(c.danger))
                        .child(e)
                }))
                .child(
                    div().flex().items_center().px(px(8.0)).py(px(4.0)).child(
                        tool_button(
                            "translate-copy",
                            icons::COPY,
                            self.t("复制译文"),
                            false,
                            translated,
                            c,
                        )
                        .on_click(cx.listener(
                            |p, _: &ClickEvent, _, cx| {
                                let value = p.translated.read(cx).value().to_string();
                                p.copy(value, "译文已复制。", cx);
                            },
                        )),
                    ),
                ),
        )
        .flex_1();
        div()
            .flex_1()
            .min_w(px(320.0))
            .h_full()
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .bg(rgb(c.soft))
            .child(source)
            .child(languages)
            .child(
                div()
                    .px(px(4.0))
                    .text_size(px(11.0))
                    .text_color(rgb(c.muted))
                    .child(self.t("在线翻译会发送原文，可能产生费用。")),
            )
            .child(output)
    }

    /// 码多时（如整页商品条码）列表可滚动，卡片不压缩。
    fn codes_body(&self, cx: &mut Context<Self>, c: &Colors) -> Stateful<Div> {
        let mut body = div()
            .id("codes-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(10.0));
        match self.codes.as_ref() {
            None => {
                body = body
                    .items_center()
                    .justify_center()
                    .text_color(rgb(c.muted))
                    .child(self.t("正在识别二维码 / 条形码…"));
            }
            Some(items) if items.is_empty() => {
                body = body
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .size(px(52.0))
                            .mb(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(14.0))
                            .bg(rgb(c.soft))
                            .child(icon(icons::QR, 26.0, c.muted)),
                    )
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("未发现二维码或条形码")),
                    )
                    .child(
                        div()
                            .max_w(px(300.0))
                            .text_center()
                            .line_height(relative(1.6))
                            .text_color(rgb(c.muted))
                            .child(self.t("请完整框选二维码或条形码，然后重试。")),
                    );
            }
            Some(items) => {
                body = body.child(
                    div().text_xs().text_color(rgb(c.muted)).child(
                        self.t("识别到 {0} 个码")
                            .replace("{0}", &items.len().to_string()),
                    ),
                );
                for (index, item) in items.iter().enumerate() {
                    let link = item.url.is_some();
                    let mut actions = div().flex().flex_wrap().justify_end().gap(px(6.0)).child(
                        qr_button(
                            ("qr-copy", index),
                            Some(icons::COPY),
                            self.t("复制内容"),
                            false,
                            c,
                        )
                        .on_click(cx.listener(
                            move |p, _: &ClickEvent, _, cx| {
                                if let Some(text) =
                                    p.codes.as_ref()
                                        .and_then(|items| items.get(index))
                                        .map(|i| i.text.clone())
                                {
                                    p.copy(text, "已复制", cx);
                                }
                            },
                        )),
                    );
                    if link {
                        if self.pending_link == Some(index) {
                            actions = actions
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme::c().warn_text))
                                        .child(self.t("确认在浏览器中打开此链接？")),
                                )
                                .child(
                                    qr_button(
                                        ("qr-open-cancel", index),
                                        None,
                                        self.t("取消"),
                                        false,
                                        c,
                                    )
                                    .on_click(cx.listener(
                                        |p, _: &ClickEvent, _, cx| {
                                            p.pending_link = None;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    qr_button(
                                        ("qr-open-confirm", index),
                                        Some(icons::LINK),
                                        self.t("打开"),
                                        true,
                                        c,
                                    )
                                    .on_click(cx.listener(
                                        move |p, _: &ClickEvent, _, cx| {
                                            if let Some(url) =
                                                p.codes.as_ref()
                                                    .and_then(|items| items.get(index))
                                                    .and_then(|i| i.url.clone())
                                            {
                                                cx.open_url(&url);
                                                p.message = Some(p.t("已请求浏览器打开链接"));
                                            }
                                            p.pending_link = None;
                                            cx.notify();
                                        },
                                    )),
                                );
                        } else {
                            actions = actions.child(
                                qr_button(
                                    ("qr-open", index),
                                    Some(icons::LINK),
                                    self.t("打开"),
                                    true,
                                    c,
                                )
                                .on_click(cx.listener(
                                    move |p, _: &ClickEvent, _, cx| {
                                        p.pending_link = Some(index);
                                        cx.notify();
                                    },
                                )),
                            );
                        }
                    }
                    body = body.child(
                        div()
                            .flex_none()
                            .p(px(12.0))
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(c.border))
                            .bg(rgb(c.soft))
                            .child(
                                // 码制（如 EAN-13，各语言通用写法）在前，内容类型在后。
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .text_xs()
                                    .text_color(rgb(c.muted))
                                    .child(icon(
                                        if link { icons::LINK } else { icons::TYPE },
                                        13.0,
                                        c.muted,
                                    ))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(c.text))
                                            .child(item.format.name()),
                                    )
                                    .child("·")
                                    .child(self.t(if link { "链接" } else { "文本" })),
                            )
                            .child(
                                div()
                                    .max_h(px(110.0))
                                    .overflow_hidden()
                                    .line_height(relative(1.6))
                                    .child(item.text.clone()),
                            )
                            .child(actions),
                    );
                }
            }
        }
        body.children(
            self.error
                .clone()
                .map(|e| div().text_color(rgb(c.danger)).child(e)),
        )
    }
}

fn qr_button(
    id: impl Into<ElementId>,
    data: Option<&'static [u8]>,
    label: String,
    primary: bool,
    c: &Colors,
) -> Stateful<Div> {
    let fg = if primary { 0xffffff } else { c.text };
    let button = div()
        .id(id)
        .h(px(30.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .cursor_pointer()
        .text_color(rgb(fg))
        .children(data.map(|d| icon(d, 14.0, fg)))
        .child(label);
    if primary {
        button
            .border_color(rgb(0x2563eb))
            .bg(rgb(0x2563eb))
            .hover(|s| s.bg(rgb(0x1d4ed8)))
    } else {
        let hover = c.hover;
        button
            .border_color(rgb(c.border))
            .bg(rgb(c.bg))
            .hover(move |s| s.bg(rgb(hover)))
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(value) = self.fill.take() {
            self.draft
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
        if let Some(value) = self.fill_translation.take() {
            self.translated
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
        let c = colors();
        let size = window.viewport_size();
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        let pinned = self.pinned;
        let pin = tool_button(
            "recognition-pin",
            if pinned {
                icons::PIN_FILLED
            } else {
                icons::PIN
            },
            self.t(if pinned {
                "取消置顶"
            } else {
                "置顶窗口"
            }),
            pinned,
            true,
            &c,
        )
        .on_click(cx.listener(move |p, _: &ClickEvent, window, cx| {
            match crate::native::set_topmost(window, !pinned) {
                Ok(()) => p.pinned = !pinned,
                Err(error) => {
                    p.error = Some(
                        snap_core::i18n::for_language(&p.settings.appearance.language, &error)
                            .to_owned(),
                    )
                }
            }
            cx.notify();
        }));
        let message = |p: &Self| -> Option<AnyElement> {
            if let Some(busy) = p.busy {
                Some(
                    div()
                        .text_xs()
                        .text_color(rgb(c.muted))
                        .child(p.t(busy))
                        .into_any_element(),
                )
            } else {
                p.message.clone().map(|m| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_xs()
                        .text_color(rgb(c.success))
                        .child(icon(icons::CHECK, 14.0, c.success))
                        .child(m)
                        .into_any_element()
                })
            }
        };
        let (content, footer) = match self.mode {
            Mode::Qr => {
                let footer = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .gap(px(6.0))
                    .px(px(16.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(rgb(c.border))
                    .child(
                        div().mr_auto().text_xs().text_color(rgb(c.muted)).children(
                            self.busy
                                .map(|b| self.t(b))
                                .or_else(|| self.message.clone()),
                        ),
                    )
                    .child(pin)
                    .child(
                        qr_button(
                            "qr-retry",
                            Some(icons::RETRY),
                            self.t("重新识别"),
                            false,
                            &c,
                        )
                        .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                            p.scan_codes(cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        qr_button("qr-close", None, self.t("关闭"), false, &c)
                            .on_click(|_, window, _| window.remove_window()),
                    );
                (self.codes_body(cx, &c).into_any_element(), footer)
            }
            mode => {
                // 覆盖原文时只留原图一栏（译文画在图上），右侧面板不渲染。
                let only = self.overlay_only();
                let main_width = if only {
                    0.0
                } else {
                    (width * 0.45).max(320.0).min(width)
                };
                let pane_width = (width - main_width).max(0.0);
                self.view = (pane_width, height - 52.0);
                let image = self.image_pane(pane_width, height - 52.0, &c, cx);
                let main = (!only).then(|| {
                    if mode == Mode::Ocr {
                        self.ocr_main(cx, &c)
                    } else {
                        self.translate_main(cx, &c)
                    }
                });
                let content = div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(image)
                    .children(main)
                    .into_any_element();
                let mut footer = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .min_h(px(52.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(rgb(c.border));
                if mode == Mode::Ocr {
                    let text = self.draft.read(cx).value().to_string();
                    let filled = !text.trim().is_empty() && self.busy.is_none();
                    let stats = format!(
                        "{}{}",
                        self.t("{0} 行 · {1} 字")
                            .replace(
                                "{0}",
                                &text
                                    .lines()
                                    .filter(|l| !l.trim().is_empty())
                                    .count()
                                    .to_string()
                            )
                            .replace(
                                "{1}",
                                &text
                                    .chars()
                                    .filter(|c| !c.is_whitespace())
                                    .count()
                                    .to_string()
                            ),
                        self.ocr
                            .as_ref()
                            .map(|r| format!(" · {}", r.provider))
                            .unwrap_or_default()
                    );
                    footer = footer
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(c.muted))
                                .child(
                                    message(self)
                                        .unwrap_or_else(|| div().child(stats).into_any_element()),
                                ),
                        )
                        .child(div().flex_1())
                        .child(pin)
                        .child(
                            footer_button(
                                "ocr-translate",
                                Some(icons::LANGUAGES),
                                self.t("翻译文字"),
                                false,
                                filled,
                                &c,
                            )
                            .on_click(cx.listener(
                                |p, _: &ClickEvent, _, cx| p.enter_translate(cx),
                            )),
                        )
                        .child(
                            footer_button(
                                "ocr-copy",
                                Some(icons::COPY),
                                self.t("复制文字"),
                                true,
                                filled,
                                &c,
                            )
                            .on_click(cx.listener(
                                |p, _: &ClickEvent, _, cx| {
                                    let value = p.draft.read(cx).value().to_string();
                                    p.copy(value, "文字已复制。", cx);
                                },
                            )),
                        );
                } else {
                    let hover = c.hover;
                    footer = footer
                        .child(
                            div()
                                .id("translate-back")
                                .h(px(34.0))
                                .px(px(12.0))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .rounded(px(6.0))
                                .text_color(rgb(c.muted))
                                .cursor_pointer()
                                .hover(move |s| s.bg(rgb(hover)))
                                .child(icon(icons::BACK, 15.0, c.muted))
                                .child(self.t("返回识别"))
                                .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                                    p.mode = Mode::Ocr;
                                    if p.ocr.is_none() && p.busy.is_none() {
                                        p.recognize(cx);
                                    }
                                    cx.notify();
                                })),
                        )
                        .children(message(self))
                        .child(div().flex_1())
                        .children(
                            (self.overlay.is_some() && self.show_image && self.preview.is_some()).then(|| {
                                let shown = self.show_overlay;
                                tool_button(
                                    "translate-overlay",
                                    icons::EYE,
                                    self.t(if shown { "显示原文" } else { "显示译文" }),
                                    shown,
                                    true,
                                    &c,
                                )
                                .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                                    p.show_overlay = !p.show_overlay;
                                    cx.notify();
                                }))
                            }),
                        )
                        .child(pin)
                        // 原地覆盖时不提供隐藏原图（没有侧边面板可切换）；退回文字面板时保留。
                        .children((!only).then(|| {
                            tool_button(
                                "translate-image",
                                icons::IMAGE,
                                self.t(if self.show_image {
                                    "隐藏原图"
                                } else {
                                    "显示原图"
                                }),
                                self.show_image,
                                true,
                                &c,
                            )
                            .on_click(cx.listener(
                                |p, _: &ClickEvent, _, cx| {
                                    p.show_image = !p.show_image;
                                    cx.notify();
                                },
                            ))
                        }))
                        // 只显示原图时右侧面板的复制按钮不在，底栏补上主操作。
                        .children(only.then(|| {
                            let translated = !self.translated.read(cx).value().trim().is_empty();
                            footer_button(
                                "translate-copy-overlay",
                                Some(icons::COPY),
                                self.t("复制译文"),
                                true,
                                translated && self.busy.is_none(),
                                &c,
                            )
                            .on_click(cx.listener(|p, _: &ClickEvent, _, cx| {
                                let value = p.translated.read(cx).value().to_string();
                                p.copy(value, "译文已复制。", cx);
                            }))
                        }));
                }
                (content, footer)
            }
        };
        div()
            .id("recognition")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(c.bg))
            .text_color(rgb(c.text))
            .text_size(px(13.0))
            .child(content)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::tidy_spaces;

    #[test]
    fn tidy_spaces_collapses_runs_and_drops_spaces_between_cjk() {
        assert_eq!(tidy_spaces("a   b  \n中 文  ok"), "a b\n中文 ok");
    }
}
