//! 剪贴板历史窗口的界面（还原 Vue `ClipboardHistory.vue` 与 `ClipboardItem.vue`）。
//!
//! 无边框窗口：顶栏（搜索框、固定、收起）与底栏可拖动窗口；分类为圆角胶囊；条目为浅灰卡片，标题行显示类型图标、
//! 类型、信息、收藏星标与时间，悬停或选中时操作按钮浮在标题行右侧；底栏未选择时显示容量与帮助、刷新、清空，
//! 选择后原位换成批量操作。右键菜单、帮助与删除确认以窗口内浮层呈现。色值见 `theme::clip`。
use super::{
    Category, CategoryDrag, Confirm, DragChip, HistoryView, MenuItem, can_combine, swatch,
};
use crate::theme::{ClipPalette, clip};
use gpui_kit::{
    component::{Icon, input::Input},
    *,
};
use snap_core::clipboard::{
    classification,
    model::{Item, Kind, TextCategory},
};

pub(super) mod icons {
    macro_rules! icon {
        ($name:ident, $file:literal) => {
            pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));
        };
    }
    icon!(SEARCH, "search");
    icon!(X, "x");
    icon!(PIN, "pin");
    icon!(PIN_OFF, "pin-off");
    icon!(COPY, "copy");
    icon!(EYE, "eye");
    icon!(STAR, "star");
    icon!(STAR_FILLED, "star-filled");
    icon!(TRASH, "trash-2");
    icon!(DOWNLOAD, "download");
    icon!(HELP, "circle-help");
    icon!(REFRESH, "refresh-cw");
    icon!(TYPE, "type");
    icon!(LINK, "link");
    icon!(PALETTE, "palette");
    icon!(IMAGE, "image");
    icon!(FILES, "files");
    icon!(FILE, "file");
    icon!(FILE_TEXT, "file-text");
    icon!(FILE_IMAGE, "file-image");
    icon!(FILE_ARCHIVE, "file-archive");
    icon!(APP_WINDOW, "app-window");
    icon!(FILE_CODE, "file-code");
    icon!(FILE_SPREADSHEET, "file-spreadsheet");
    icon!(FILE_VIDEO, "file-video");
    icon!(FILE_AUDIO, "file-audio");
    icon!(FOLDER, "folder");
    icon!(PANEL_TOP, "panel-top");
    icon!(CHECK, "check");
}

const AMBER_STROKE: u32 = 0xd97706;
const AMBER_FILL: u32 = 0xf59e0b;
/// 图片缩略图最高 120 像素；列表内容区约 418 像素宽（窗口 460，左右边距与卡片内边距各 20）。
const THUMB_MAX_HEIGHT: f32 = 120.0;
const CONTENT_WIDTH: f32 = 418.0;

fn icon(data: &'static [u8], size: f32, color: u32) -> Icon {
    Icon::default()
        .data(data)
        .size(px(size))
        .text_color(rgb(color))
}

/// 条目类型（与 Vue `type` 计算一致）：图片、文件、颜色、链接、文本。
fn kind_of(item: &Item) -> (&'static [u8], &'static str) {
    match item.kind {
        Kind::Image => (icons::IMAGE, "图片"),
        Kind::File => (icons::FILES, "文件"),
        Kind::Text
            if swatch(&item.preview).is_some()
                && classification::classify(&item.preview) == Some(TextCategory::Color) =>
        {
            (icons::PALETTE, "颜色")
        }
        Kind::Text if is_link(&item.preview) => (icons::LINK, "链接"),
        Kind::Text => (icons::TYPE, "文本"),
    }
}

fn is_link(text: &str) -> bool {
    let text = text.trim();
    (text.starts_with("http://") || text.starts_with("https://"))
        && !text.chars().any(char::is_whitespace)
}

/// 按扩展名的文件图标与颜色（Vue `fileTypes`）；以分隔符结尾为文件夹。
pub(super) fn file_icon(path: &str) -> (&'static [u8], u32) {
    if path.ends_with(['/', '\\']) {
        return (icons::FOLDER, 0x71717a);
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let extension = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    const TYPES: [(&[&str], &[u8], u32); 8] = [
        (
            &["txt", "md", "log", "rtf", "doc", "docx", "odt", "pdf"],
            icons::FILE_TEXT,
            0x57606a,
        ),
        (
            &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "heic", "tif", "tiff",
            ],
            icons::FILE_IMAGE,
            0x2563eb,
        ),
        (
            &["zip", "7z", "rar", "tar", "gz", "bz2", "xz"],
            icons::FILE_ARCHIVE,
            0xa16207,
        ),
        (&["exe", "msi", "msix", "appx"], icons::APP_WINDOW, 0x7c3aed),
        (
            &[
                "json", "xml", "yml", "yaml", "toml", "js", "ts", "vue", "html", "css", "py", "rs",
                "java", "c", "cpp", "h", "dll", "sh", "bat", "ps1",
            ],
            icons::FILE_CODE,
            0x526b88,
        ),
        (
            &["xls", "xlsx", "csv", "ods"],
            icons::FILE_SPREADSHEET,
            0x15803d,
        ),
        (
            &["mp4", "mkv", "avi", "mov", "webm"],
            icons::FILE_VIDEO,
            0x9333ea,
        ),
        (
            &["mp3", "wav", "flac", "ogg", "m4a"],
            icons::FILE_AUDIO,
            0xc2410c,
        ),
    ];
    TYPES
        .iter()
        .find(|(list, _, _)| list.contains(&extension.as_str()))
        .map_or((icons::FILE, 0x71717a), |(_, data, color)| (*data, *color))
}

fn file_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// 字节数（Vue `size`）。
pub(super) fn size_label(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    }
}

/// 当天只显示时刻，更早的记录带“月/日”（Vue `time`）。
pub(super) fn time_label(millis: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(date) = Local.timestamp_millis_opt(millis).single() else {
        return String::new();
    };
    let now = Local::now();
    if date.date_naive() == now.date_naive() {
        date.format("%H:%M").to_string()
    } else {
        format!(
            "{}/{} {}",
            date.format("%-m"),
            date.format("%-d"),
            date.format("%H:%M")
        )
    }
}

fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

/// 图标按钮（`.history-icon-button`，32×32；底栏 28×28）。
fn icon_button(
    id: impl Into<ElementId>,
    data: &'static [u8],
    size: f32,
    pressed: bool,
    c: &ClipPalette,
) -> Stateful<Div> {
    let (bg, fg) = if pressed {
        (Some(c.selected), c.accent)
    } else {
        (None, c.muted)
    };
    let mut button = div()
        .id(id)
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .child(icon(data, 16.0, fg));
    if let Some(bg) = bg {
        button = button.bg(rgb(bg));
    } else {
        let (hover, text) = (c.card_hover, c.text);
        button = button.hover(move |s| s.bg(rgb(hover)).text_color(rgb(text)));
    }
    button.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// 文字按钮（Button ghost：底栏“刷新”“清空”等，28 像素高，12 像素字）。
fn ghost_button(
    id: impl Into<ElementId>,
    label: String,
    leading: Option<&'static [u8]>,
    color: u32,
    hover_bg: u32,
    enabled: bool,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(28.0))
        .px(px(8.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .rounded(px(6.0))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .children(leading.map(|data| icon(data, 14.0, color)))
        .child(label)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if enabled {
        button.cursor_pointer().hover(move |s| s.bg(rgb(hover_bg)))
    } else {
        button.opacity(0.5)
    }
}

/// 卡片右上角浮出的操作按钮（24×24）。
fn action_button(
    id: impl Into<ElementId>,
    data: &'static [u8],
    color: u32,
    hover_bg: u32,
    hover_fg: u32,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor_pointer()
        .text_color(rgb(color))
        .hover(move |s| s.bg(rgb(hover_bg)).text_color(rgb(hover_fg)))
        .child(Icon::default().data(data).size(px(14.0)))
}

impl HistoryView {
    pub(super) fn text(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn topbar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let c = clip();
        let focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let keyword = !self.search.read(cx).value().is_empty();
        let search = div()
            .flex()
            .flex_1()
            .min_w_0()
            .h(px(32.0))
            .px(px(8.0))
            .items_center()
            .gap(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(if focused { c.accent } else { c.card }))
            .bg(rgb(if focused { c.bg } else { c.card }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(icon(icons::SEARCH, 16.0, c.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.search).appearance(false)),
            )
            .children(keyword.then(|| {
                div()
                    .id("clipboard-search-clear")
                    .size(px(24.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .child(icon(icons::X, 16.0, c.muted))
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                        view.search.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                            state.focus(window, cx);
                        });
                        view.clear_selection();
                        view.reload(cx);
                    }))
            }));
        let keep_open = self.keep_open;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .pt(px(10.0))
            .px(px(10.0))
            .pb(px(8.0))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(search)
            .child(
                icon_button(
                    "clipboard-keep-open",
                    if keep_open {
                        icons::PIN
                    } else {
                        icons::PIN_OFF
                    },
                    32.0,
                    keep_open,
                    c,
                )
                .tooltip({
                    let tip = self.text("固定窗口，失去焦点时不收起");
                    move |_, cx| {
                        cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                            .into()
                    }
                })
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.keep_open = !view.keep_open;
                    cx.notify();
                })),
            )
            .child(
                icon_button("clipboard-hide", icons::X, 32.0, false, c)
                    .tooltip({
                        let tip = self.text("收起（Esc）");
                        move |_, cx| {
                            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                .into()
                        }
                    })
                    .on_click(|_, window, _| window.remove_window()),
            )
    }

    fn categories(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let c = clip();
        let mut tabs = div()
            .id("clipboard-categories")
            .flex()
            .flex_none()
            .gap(px(4.0))
            .px(px(10.0))
            .pb(px(8.0))
            .overflow_x_scroll();
        for (index, category) in self.order.clone().into_iter().enumerate() {
            let active = self.category == category;
            let label = self.text(category.label());
            let chip = label.clone();
            let (hover, text) = (c.card_hover, c.text);
            let mut tab = div()
                .id(("clipboard-tab", index))
                .h(px(26.0))
                .px(px(9.0))
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(13.0))
                .text_xs()
                .cursor_pointer()
                .on_drag(CategoryDrag(category), move |_, _, _, cx| {
                    cx.new(|_| DragChip(chip.clone()))
                })
                .drag_over::<CategoryDrag>(move |style, _, _, _| {
                    style.border_l_2().border_color(rgb(c.accent))
                })
                .on_drop(cx.listener(move |view, drag: &CategoryDrag, _, cx| {
                    view.reorder(drag.0, category, cx)
                }))
                .on_click(
                    cx.listener(move |view, _: &ClickEvent, _, cx| view.set_category(category, cx)),
                )
                .child(label);
            tab = if active {
                tab.bg(rgb(c.selected))
                    .text_color(rgb(c.accent))
                    .font_weight(FontWeight::SEMIBOLD)
            } else {
                tab.text_color(rgb(c.muted))
                    .hover(move |s| s.bg(rgb(hover)).text_color(rgb(text)))
            };
            tabs = tabs.child(tab);
        }
        tabs
    }

    fn notice(&self, message: String, error: bool) -> Div {
        let c = clip();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .mx(px(10.0))
            .mb(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(rgb(if error { c.danger_bg } else { c.notice }))
            .text_color(rgb(if error { c.danger } else { c.notice_text }))
            .text_xs()
            .line_height(relative(1.5))
            .child(message)
    }

    fn item_card(&mut self, index: usize, item: &Item, cx: &mut Context<Self>) -> Stateful<Div> {
        let c = clip();
        let picked = self.picked.contains(&item.id);
        let focused = self.keyboard && index == self.selected;
        let (type_icon, type_label) = kind_of(item);
        let info = match item.kind {
            Kind::Image => format!("{} × {}", item.width.unwrap_or(0), item.height.unwrap_or(0)),
            Kind::File => self.text("{0} 项 · 文件引用").replace(
                "{0}",
                &item.file_paths.as_ref().map_or(0, Vec::len).to_string(),
            ),
            Kind::Text => size_label(item.byte_size),
        };
        let info = if item.seen_count > 1 {
            format!("{info} · {} {}", item.seen_count, self.text("次"))
        } else {
            info
        };
        let id = item.id;
        let favorite = item.favorite;
        let previewable = item.kind != Kind::File;
        let actions_bg = if picked { c.selected } else { c.actions };
        let group = SharedString::from(format!("clipboard-item-{index}"));
        let mut actions = div()
            .absolute()
            .top(px(5.0))
            .right(px(6.0))
            .flex()
            .gap(px(2.0))
            .pl(px(4.0))
            .rounded(px(4.0))
            .bg(rgb(actions_bg))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                action_button(
                    ("clipboard-copy", index),
                    icons::COPY,
                    c.muted,
                    c.press,
                    c.text,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    view.activate(index, false, window, cx);
                })),
            );
        if previewable {
            let item = item.clone();
            actions = actions.child(
                action_button(
                    ("clipboard-preview", index),
                    icons::EYE,
                    c.muted,
                    c.press,
                    c.text,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    crate::clipboard_preview::open(cx, view.service.clone(), item.clone());
                })),
            );
        }
        actions = actions
            .child(
                action_button(
                    ("clipboard-pin", index),
                    icons::PIN,
                    c.muted,
                    c.press,
                    c.text,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    view.pin(index, cx);
                })),
            )
            .child(
                action_button(
                    ("clipboard-favorite", index),
                    if favorite {
                        icons::STAR_FILLED
                    } else {
                        icons::STAR
                    },
                    if favorite { AMBER_STROKE } else { c.muted },
                    c.press,
                    if favorite { AMBER_STROKE } else { c.text },
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let result = view.service.set_favorite(id, !favorite);
                    view.run(result, cx);
                })),
            )
            .child(
                action_button(
                    ("clipboard-delete", index),
                    icons::TRASH,
                    c.muted,
                    c.danger_bg,
                    c.danger,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    view.picked = vec![id];
                    view.picked_items = [(id, view.items.iter().find(|i| i.id == id).cloned())]
                        .into_iter()
                        .filter_map(|(k, v)| Some((k, v?)))
                        .collect();
                    view.confirm = Some(Confirm::Delete);
                    cx.notify();
                })),
            );
        if !picked && !focused {
            actions = actions
                .invisible()
                .group_hover(group.clone(), |s| s.visible());
        }

        let mut head = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(22.0))
            .mb(px(4.0))
            .text_xs()
            .text_color(rgb(c.muted))
            .child(icon(type_icon, 14.0, c.muted))
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(c.text))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.text(type_label)),
            )
            .child(div().min_w_0().truncate().child(info));
        if favorite {
            head = head.child(
                Icon::default()
                    .data(icons::STAR_FILLED)
                    .size(px(13.0))
                    .text_color(rgb(AMBER_FILL)),
            );
        }
        head = head.child(
            div()
                .ml_auto()
                .flex_none()
                .child(time_label(item.last_seen_at)),
        );

        let content: AnyElement = match item.kind {
            Kind::Image => match self.thumbnail(item.id, cx) {
                Some(thumb) => {
                    let (w, h) = (
                        item.width.unwrap_or(1).max(1) as f32,
                        item.height.unwrap_or(1).max(1) as f32,
                    );
                    let height = THUMB_MAX_HEIGHT.min(CONTENT_WIDTH * h / w).min(h);
                    let width = (height * w / h).min(CONTENT_WIDTH);
                    img(thumb)
                        .w(px(width))
                        .h(px(height))
                        .rounded(px(4.0))
                        .object_fit(ObjectFit::Contain)
                        .into_any_element()
                }
                None => div()
                    .h(px(72.0))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(rgb(c.muted))
                    .child(
                        self.text("图片 {0} × {1}")
                            .replace("{0}", &item.width.unwrap_or(0).to_string())
                            .replace("{1}", &item.height.unwrap_or(0).to_string()),
                    )
                    .into_any_element(),
            },
            Kind::File => {
                let paths = item.file_paths.clone().unwrap_or_default();
                let mut files = div().flex().flex_col();
                for path in paths.iter().take(4) {
                    let (data, color) = file_icon(path);
                    files = files.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .min_h(px(22.0))
                            .child(icon(data, 16.0, color))
                            .child(div().min_w_0().truncate().child(file_name(path))),
                    );
                }
                if paths.len() > 4 {
                    files =
                        files.child(div().ml(px(24.0)).text_xs().text_color(rgb(c.muted)).child(
                            format!(
                                "{} {} {}",
                                self.text("另有"),
                                paths.len() - 4,
                                self.text("项")
                            ),
                        ));
                }
                files.into_any_element()
            }
            Kind::Text => {
                let color = (type_label == "颜色")
                    .then(|| swatch(&item.preview))
                    .flatten();
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.0))
                    .line_height(relative(1.5))
                    .children(color.map(|value| {
                        div()
                            .mt(px(4.0))
                            .size(px(12.0))
                            .flex_none()
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(rgba(0x00000022))
                            .bg(rgb(value))
                    }))
                    .child(div().min_w_0().line_clamp(4).child(item.preview.clone()))
                    .into_any_element()
            }
        };

        let (border, bg, hover) = if picked {
            (c.selected_border, c.selected, c.selected)
        } else if focused {
            (c.accent, c.card, c.card_hover)
        } else {
            (c.card, c.card, c.card_hover)
        };
        div()
            .id(("clipboard-item", index))
            .group(group)
            .relative()
            .pt(px(8.0))
            .px(px(10.0))
            .pb(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(bg))
            .hover(move |s| s.bg(rgb(hover)))
            .text_sm()
            .text_color(rgb(c.text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, _| {
                    view.press = Some((index, event.position));
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, _| view.press = None),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    view.selected = index;
                    let shelf = crate::shelf::shows(cx, id);
                    view.menu = Some((index, event.position, shelf));
                    cx.notify();
                }),
            )
            .on_mouse_move(
                cx.listener(|view, event: &MouseMoveEvent, _, cx| view.motion(event, cx)),
            )
            .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                view.selected = index;
                view.keyboard = false;
                if event.click_count() >= 2 {
                    view.activate(index, view.paste_on_activate, window, cx);
                } else {
                    let modifiers = event.modifiers();
                    view.pick(
                        index,
                        modifiers.control || modifiers.platform,
                        modifiers.shift,
                    );
                }
                cx.notify();
            }))
            .child(head)
            .child(content)
            .child(actions)
    }

    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let c = clip();
        let status = self.service.status();
        let picked = self.picked.len();
        let summary: AnyElement = if picked > 0 {
            div()
                .min_w_0()
                .truncate()
                .text_color(rgb(c.accent))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{} {picked}", self.text("已选")))
                .into_any_element()
        } else if let Some(message) = self.message.clone() {
            div()
                .min_w_0()
                .truncate()
                .text_color(rgb(c.success))
                .child(message)
                .into_any_element()
        } else {
            div()
                .min_w_0()
                .truncate()
                .child(format!(
                    "{} {} {:.1} MiB",
                    status.count,
                    self.text("条 ·"),
                    status.byte_size as f64 / 1_048_576.0
                ))
                .into_any_element()
        };
        let mut actions = div().flex().flex_none().items_center().gap(px(2.0));
        if picked > 0 {
            let kinds = self.picked_kinds();
            let combinable = can_combine(&kinds);
            let exportable = kinds.len() == picked && kinds.iter().all(|k| *k != Kind::File);
            let tip = |key: &'static str| {
                let text = self.text(key);
                move |_: &mut Window, cx: &mut App| {
                    cx.new(|_| crate::annotate::Tooltip(text.clone().into()))
                        .into()
                }
            };
            actions = actions
                .child(
                    icon_button("clipboard-copy-picked", icons::COPY, 28.0, false, c)
                        .opacity(if combinable { 1.0 } else { 0.5 })
                        .tooltip(tip(if combinable {
                            "复制所选"
                        } else {
                            "文本不能与图片或文件一起复制，请分开选择。"
                        }))
                        .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                            if combinable {
                                let ids = view.picked.clone();
                                view.activate_ids(ids, false, window, cx);
                            }
                        })),
                )
                .child(
                    icon_button("clipboard-export-picked", icons::DOWNLOAD, 28.0, false, c)
                        .opacity(if exportable { 1.0 } else { 0.5 })
                        .tooltip(tip(if exportable {
                            "组合导出"
                        } else {
                            "组合导出仅支持文本和图片，文件引用请复制后粘贴"
                        }))
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            if exportable {
                                let items: Vec<Item> = view
                                    .picked
                                    .iter()
                                    .filter_map(|id| view.picked_items.get(id).cloned())
                                    .collect();
                                crate::clipboard_export::open(cx, view.service.clone(), items);
                            }
                        })),
                )
                .child(
                    icon_button("clipboard-pin-picked", icons::PIN, 28.0, false, c)
                        .tooltip(tip("悬浮贴图"))
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.pin_picked(cx))),
                )
                .child(
                    div()
                        .id("clipboard-delete-picked")
                        .size(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgb(c.danger_bg)))
                        .child(icon(icons::TRASH, 16.0, c.danger))
                        .tooltip(tip("删除所选"))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.confirm = Some(Confirm::Delete);
                            cx.notify();
                        })),
                )
                .child(
                    icon_button("clipboard-unpick", icons::X, 28.0, false, c)
                        .tooltip(tip("取消选择"))
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.clear_selection();
                            cx.notify();
                        })),
                );
        } else {
            actions = actions.child(
                icon_button("clipboard-help", icons::HELP, 28.0, false, c)
                    .tooltip({
                        let tip = self.text("帮助");
                        move |_, cx| {
                            cx.new(|_| crate::annotate::Tooltip(tip.clone().into()))
                                .into()
                        }
                    })
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.help = true;
                        cx.notify();
                    })),
            );
        }
        actions = actions.child(
            ghost_button(
                "clipboard-refresh",
                self.text("刷新"),
                Some(icons::REFRESH),
                crate::theme::clip().muted,
                c.card_hover,
                !self.loading,
            )
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.message = None;
                view.reload(cx);
            })),
        );
        if picked == 0 {
            let enabled = status.count > 0;
            let mut clear = ghost_button(
                "clipboard-clear",
                self.text("清空"),
                None,
                c.danger,
                c.danger_bg,
                enabled,
            );
            if enabled {
                clear = clear.on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.include_favorites = false;
                    view.confirm = Some(Confirm::Clear);
                    cx.notify();
                }));
            }
            actions = actions.child(clear);
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .h(px(40.0))
            .pl(px(12.0))
            .pr(px(6.0))
            .border_t_1()
            .border_color(rgb(c.border))
            .text_xs()
            .text_color(rgb(c.muted))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(summary)
            .child(actions)
    }

    /// 窗口内浮层：遮罩 + 居中卡片（帮助、删除确认；Vue 版为独立子窗口）。
    fn overlay(&self, card: Div, cx: &mut Context<Self>) -> Div {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16.0))
            .bg(rgba(0x00000052))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if view.help {
                        view.help = false;
                        cx.notify();
                    }
                }),
            )
            .child(card.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()))
    }

    fn help_card(&self) -> Div {
        let c = clip();
        let modifier = modifier();
        let key = if self.paste_on_activate {
            "回车或双击粘贴到原窗口 · {0}+1～9 粘贴第 N 条 · 单击选中 · {0} 多选 · Shift 连选 · 多选后回车一起粘贴 · {0}+C 仅复制 · 右键更多操作"
        } else {
            "回车或双击复制并收起 · {0}+1～9 复制第 N 条 · 单击选中 · {0} 多选 · Shift 连选 · 多选后回车一起复制 · {0}+C 仅复制 · 右键更多操作"
        };
        div()
            .w_full()
            .max_w(px(420.0))
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(c.menu_border))
            .bg(rgb(c.menu_bg))
            .shadow_lg()
            .text_xs()
            .line_height(relative(1.6))
            .text_color(rgb(c.dialog_muted))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c.dialog_text))
                    .child(self.text("帮助")),
            )
            .child(self.text(key).replace("{0}", modifier))
            .children(
                (self.category == Category::File)
                    .then(|| self.text("仅记录文件路径，原文件移动或删除后失效。")),
            )
    }

    fn confirm_card(&self, confirm: Confirm, cx: &mut Context<Self>) -> Div {
        let c = clip();
        let (title, description) = match confirm {
            Confirm::Clear => (self.text("清空"), self.text("清空历史记录？默认保留收藏。")),
            Confirm::Delete => (
                self.text("删除所选"),
                self.text("删除选中的 {0} 条记录（包含选中的收藏）？")
                    .replace("{0}", &self.picked.len().to_string()),
            ),
        };
        let description = format!("{description} {}", self.text("此操作不可撤销。"));
        let include = self.include_favorites;
        let checkbox = (confirm == Confirm::Clear).then(|| {
            div()
                .id("clipboard-include-favorites")
                .mt(px(12.0))
                .min_h(px(32.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .cursor_pointer()
                .text_sm()
                .text_color(rgb(c.dialog_text))
                .child(
                    div()
                        .size(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.0))
                        .border_1()
                        .border_color(rgb(if include { c.success } else { c.control_border }))
                        .bg(rgb(if include { c.check_fill } else { c.menu_bg }))
                        .children(include.then(|| icon(icons::CHECK, 12.0, c.success))),
                )
                .child(self.text("同时删除收藏"))
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.include_favorites = !view.include_favorites;
                    cx.notify();
                }))
        });
        let error = self.error.clone();
        div()
            .w_full()
            .max_w(px(400.0))
            .p(px(20.0))
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(c.menu_border))
            .bg(rgb(c.menu_bg))
            .shadow_lg()
            .text_sm()
            .text_color(rgb(c.dialog_text))
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .mt(px(12.0))
                    .line_height(relative(1.6))
                    .text_color(rgb(c.dialog_muted))
                    .child(description),
            )
            .children(checkbox)
            .children(error.map(|e| div().mt(px(12.0)).text_color(rgb(c.danger)).child(e)))
            .child(
                div()
                    .mt(px(20.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("clipboard-confirm-cancel")
                            .px(px(16.0))
                            .py(px(10.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(if crate::theme::dark() {
                                c.border
                            } else {
                                0xe4e4e7
                            }))
                            .bg(rgb(c.menu_bg))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(if crate::theme::dark() {
                                c.text
                            } else {
                                0x3f3f46
                            }))
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgb(c.card)))
                            .child(self.text("取消"))
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                view.confirm = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("clipboard-confirm")
                            .px(px(16.0))
                            .py(px(10.0))
                            .rounded(px(6.0))
                            .bg(rgb(0xdc2626))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xb91c1c)))
                            .child(self.text("确认删除"))
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                view.confirm = None;
                                let result = match confirm {
                                    Confirm::Clear => view.service.clear(view.include_favorites),
                                    Confirm::Delete => {
                                        let ids = view.picked.clone();
                                        view.clear_selection();
                                        view.service.delete(&ids)
                                    }
                                };
                                view.run(result, cx);
                            })),
                    ),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn context_menu(
        &self,
        index: usize,
        item: Item,
        at: Point<Pixels>,
        shelf: Option<bool>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let c = clip();
        let mut entries: Vec<Option<(MenuItem, &'static [u8], &'static str)>> =
            vec![Some((MenuItem::Copy, icons::COPY, "复制"))];
        if item.kind != Kind::File {
            entries.push(Some((MenuItem::Preview, icons::EYE, "预览")));
        }
        entries.push(Some((MenuItem::Pin, icons::PIN, "悬浮贴图")));
        if let Some(on_shelf) = shelf {
            entries.push(Some((
                MenuItem::Shelf(on_shelf),
                icons::PANEL_TOP,
                if on_shelf {
                    "从暂存架取下"
                } else {
                    "放到暂存架"
                },
            )));
        }
        entries.push(None);
        entries.push(Some((
            MenuItem::Favorite(item.favorite),
            icons::STAR,
            if item.favorite {
                "取消收藏"
            } else {
                "收藏"
            },
        )));
        entries.push(Some((MenuItem::Delete, icons::TRASH, "删除")));
        const WIDTH: f32 = 172.0;
        const ROW: f32 = 35.0;
        let height = entries
            .iter()
            .map(|e| if e.is_some() { ROW } else { 9.0 })
            .sum::<f32>()
            + 12.0;
        let viewport = window.viewport_size();
        let x = if at.x + px(WIDTH) > viewport.width - px(8.0) {
            (at.x - px(WIDTH)).max(px(8.0))
        } else {
            at.x
        };
        let y = if at.y + px(height) > viewport.height - px(8.0) {
            (at.y - px(height)).max(px(8.0))
        } else {
            at.y
        };
        let mut menu = div()
            .id("clipboard-menu")
            .absolute()
            .left(x)
            .top(y)
            .min_w(px(150.0))
            .p(px(5.0))
            .flex()
            .flex_col()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(c.menu_border))
            .bg(rgb(c.menu_bg))
            .shadow_lg()
            .text_sm()
            .text_color(rgb(c.dialog_text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        for (row, entry) in entries.into_iter().enumerate() {
            let Some((action, data, label)) = entry else {
                menu = menu.child(div().h(px(1.0)).m(px(4.0)).bg(rgb(c.menu_border)));
                continue;
            };
            let item = item.clone();
            let (hover_bg, hover_fg) = (c.menu_hover_bg, c.menu_hover_fg);
            menu = menu.child(
                div()
                    .id(("clipboard-menu-item", row))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(12.0))
                    .py(px(9.0))
                    .rounded(px(3.0))
                    .hover(move |s| s.bg(rgb(hover_bg)).text_color(rgb(hover_fg)))
                    .child(Icon::default().data(data).size(px(15.0)))
                    .child(self.text(label))
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        view.menu_action(index, item.clone(), action, window, cx)
                    })),
            );
        }
        menu
    }

    /// 列表接近底部时加载下一页（Vue `near-end`）。
    fn maybe_load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.next_cursor.is_none() {
            return;
        }
        let offset = -f32::from(self.scroll.offset().y);
        let max = f32::from(self.scroll.max_offset().y);
        if max - offset < 240.0 {
            let cursor = self.next_cursor.clone();
            self.load(cursor, cx);
        }
    }

    fn empty_text(&self, cx: &App) -> String {
        if self.loading {
            return self.text("正在加载…");
        }
        if self.service.data().is_err() {
            return self.text("剪贴板历史不可用。");
        }
        let keyword = !self.search.read(cx).value().is_empty();
        if keyword || self.category != Category::All {
            self.text("没有匹配的记录，试试其他关键词或分类。")
        } else if !self.service.status().enabled {
            self.text("暂无记录。")
        } else {
            self.text("暂无记录，复制文字或图片后会显示在这里。")
        }
    }
}

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 释放已不在列表中的缩略图纹理。
        let ids: std::collections::HashSet<i64> = self.items.iter().map(|i| i.id).collect();
        let stale: Vec<i64> = self
            .thumbs
            .keys()
            .filter(|id| !ids.contains(id))
            .copied()
            .collect();
        for id in stale {
            if let Some(thumb) = self.thumbs.remove(&id) {
                cx.drop_image(thumb, Some(window));
            }
        }
        let c = clip();
        let status = self.service.status();
        let mut list = div()
            .id("clipboard-list")
            .flex_1()
            .min_h_0()
            .px(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll(cx.listener(|view, _: &ScrollEvent, _, cx| view.maybe_load_more(cx)));
        let items = self.items.clone();
        for (index, item) in items.iter().enumerate() {
            list = list.child(self.item_card(index, item, cx));
        }
        if items.is_empty() {
            list = list.child(
                div()
                    .py(px(48.0))
                    .px(px(16.0))
                    .flex()
                    .justify_center()
                    .text_color(rgb(c.muted))
                    .child(self.empty_text(cx)),
            );
        } else if self.loading && self.next_cursor.is_some() {
            list = list.child(
                div()
                    .pt(px(8.0))
                    .pb(px(12.0))
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(rgb(c.muted))
                    .child(self.text("加载中…")),
            );
        } else if self.next_cursor.is_none() {
            list = list.child(div().h(px(4.0)));
        }
        let paused = (status.supported && !status.enabled).then(|| {
            self.notice(self.text("记录已暂停，新复制的内容不会保存。"), false)
                .child(
                    div()
                        .id("clipboard-resume")
                        .flex_none()
                        .h(px(24.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgb(c.card_hover)))
                        .child(self.text("继续记录"))
                        .on_click(
                            cx.listener(|view, _: &ClickEvent, _, cx| view.set_recording(true, cx)),
                        ),
                )
        });
        let unsupported = (!status.supported).then(|| {
            self.notice(
                self.text("当前平台尚未支持自动监听；现有手动贴图不受影响。"),
                false,
            )
        });
        let error = self
            .error
            .clone()
            .or(status.error.clone())
            .filter(|_| self.confirm.is_none())
            .map(|e| self.notice(e, true));
        let menu = self.menu.and_then(|(index, at, shelf)| {
            let item = self.items.get(index)?.clone();
            Some(self.context_menu(index, item, at, shelf, window, cx))
        });
        let overlay = match (self.confirm, self.help) {
            (Some(confirm), _) => Some(self.overlay(self.confirm_card(confirm, cx), cx)),
            (None, true) => Some(self.overlay(self.help_card(), cx)),
            _ => None,
        };
        let topbar = self.topbar(window, cx);
        let categories = self.categories(cx);
        let footer = self.footer(cx);
        div()
            .id("clipboard-history")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    if view.menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    if view.menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(c.border))
            .bg(rgb(c.bg))
            .text_color(rgb(c.text))
            .text_size(px(13.0))
            .child(topbar)
            .child(categories)
            .children(paused)
            .children(unsupported)
            .children(error)
            .child(list)
            .child(footer)
            .children(menu)
            .children(overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::{file_icon, icons, is_link, size_label};

    #[test]
    fn labels_follow_the_vue_item() {
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(2048), "2.0 KB");
        assert_eq!(size_label(3 * 1_048_576), "3.0 MB");
        assert!(is_link(" https://example.com/a "));
        assert!(!is_link("https://example.com/a b"));
        assert_eq!(file_icon(r"C:\a\b.PNG").0, icons::FILE_IMAGE);
        assert_eq!(file_icon(r"C:\a\dir\").0, icons::FOLDER);
        assert_eq!(file_icon("noext").0, icons::FILE);
    }
}
