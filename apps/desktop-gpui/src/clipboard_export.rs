//! 剪贴板组合导出窗口（对照 Vue 版 `ClipboardComposer.vue`）：所选文本与图片按顺序导出为 PDF、长图或分别保存。
//!
//! 素材可上移、下移、移除；参数与 Tauri 版相同（页面 A4/原始比例、长图背景、间距 0～200px、边距 0～30mm），
//! 编码与校验在 core `snap_core::clipboard::export`。文本素材可改写（只作用于本次导出，不修改历史，可还原原文）；
//! Vue 版的分页预览未迁移。
//! 文本排版字体：设置里下载/选择的导出字体，没有时回退到本机中文字体。
use crate::annotate;
use gpui_kit::{
    component::{
        input::{Textarea, TextareaState},
        Icon,
    },
    *,
};
use snap_core::clipboard::{
    export::{self, Background, Mode, Options, PageSize},
    model::{Item, Kind},
    service::Service,
};
use std::{collections::HashMap, sync::atomic::Ordering};

struct ExportState {
    window: Option<AnyWindowHandle>,
}
impl Global for ExportState {}

/// 打开导出窗口；已打开时替换为新的素材并激活。只保留文本与图片（文件条目仅是引用，不能导出）。
pub fn open(cx: &mut App, service: Service, items: Vec<Item>) {
    let entries: Vec<Item> = items
        .into_iter()
        .filter(|item| item.kind != Kind::File)
        .collect();
    if let Some(handle) = cx.try_global::<ExportState>().and_then(|s| s.window) {
        let replaced = handle.update(cx, |root, window, cx| {
            if let Ok(view) = root.downcast::<Composer>() {
                view.update(cx, |view, cx| {
                    if !view.busy {
                        view.entries = entries.clone();
                        view.edits.clear();
                        view.editing = None;
                        view.error = None;
                        view.message = None;
                        cx.notify();
                    }
                });
            }
            window.activate_window();
        });
        if replaced.is_ok() {
            return;
        }
    }
    let language = crate::shell::settings(cx).appearance.language;
    let title = snap_core::i18n::for_language(&language, "组合导出").to_owned();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(960.0), px(640.0)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(size(px(640.0), px(420.0))),
        ..Default::default()
    };
    match cx.open_window(options, |_, cx| {
        cx.new(|cx| Composer {
            service,
            language,
            entries,
            options: Options {
                ids: Vec::new(),
                texts: Vec::new(),
                mode: Mode::Pdf,
                page_size: PageSize::A4,
                gap_px: 16,
                margin_mm: 12.0,
                background: Background::White,
            },
            edits: HashMap::new(),
            editing: None,
            thumbs: HashMap::new(),
            thumbs_pending: Default::default(),
            busy: false,
            error: None,
            message: None,
            focus: cx.focus_handle(),
        })
    }) {
        Ok(handle) => {
            cx.set_global(ExportState {
                window: Some(handle.into()),
            });
            log::info!("event=gpui.clipboard_export_opened");
        }
        Err(error) => log::error!("event=gpui.clipboard_export_open_failed message={error}"),
    }
}

/// 条目的完整原文（改写与判断是否改动时读取）。
fn original_text(service: &Service, id: i64) -> Result<String, String> {
    match service.contents(&[id])?.pop() {
        Some(snap_core::clipboard::service::Content::Text(text)) => Ok(text),
        _ => Err("只能改写文本素材。".into()),
    }
}

/// 导出窗口是否打开（历史窗口失焦时据此不收起）。
pub fn is_open(cx: &App) -> bool {
    cx.try_global::<ExportState>()
        .and_then(|s| s.window)
        .is_some_and(|handle| {
            cx.windows()
                .iter()
                .any(|w| w.window_id() == handle.window_id())
        })
}

struct Composer {
    service: Service,
    language: snap_core::settings::Language,
    entries: Vec<Item>,
    options: Options,
    /// 本次导出改写过的文本（条目 ID → 新文本）。
    edits: HashMap<i64, String>,
    /// 正在改写的条目与输入框。
    editing: Option<(i64, Entity<TextareaState>)>,
    /// 图片素材的缩略图（列表与预览共用）。
    thumbs: HashMap<i64, std::sync::Arc<RenderImage>>,
    thumbs_pending: std::collections::HashSet<i64>,
    busy: bool,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
}

impl Composer {
    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn step(&mut self, gap: i32, margin: f32, cx: &mut Context<Self>) {
        self.options.gap_px = (self.options.gap_px as i32 + gap).clamp(0, 200) as u32;
        self.options.margin_mm = (self.options.margin_mm + margin).clamp(0.0, 30.0);
        cx.notify();
    }

    fn move_entry(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from < self.entries.len() && to < self.entries.len() && from != to {
            let item = self.entries.remove(from);
            self.entries.insert(to, item);
        }
        cx.notify();
    }

    /// 快照、字体与排版在后台；对话框为异步（独立线程弹出），期间置顶贴图临时隐藏。
    /// 结束改写：把输入框内容记为改写（与原文相同则视为未改写）。
    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some((id, state)) = self.editing.take() else {
            return;
        };
        let value = state.read(cx).value().to_string();
        let original = original_text(&self.service, id).ok();
        if original.as_deref() == Some(value.as_str()) {
            self.edits.remove(&id);
        } else {
            self.edits.insert(id, value);
        }
    }

    /// 改写文本：读取完整原文（或已改写的文本）放进输入框；再次点击结束改写。
    fn toggle_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let same = self.editing.as_ref().is_some_and(|(current, _)| *current == id);
        self.commit_edit(cx);
        if same || self.busy {
            cx.notify();
            return;
        }
        let text = match self.edits.get(&id) {
            Some(text) => Ok(text.clone()),
            None => original_text(&self.service, id),
        };
        match text {
            Ok(text) => {
                let state = cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx);
                    state.set_auto_grow(3, 12, cx);
                    state.set_value(text, window, cx);
                    state
                });
                state.update(cx, |state, cx| state.focus(window, cx));
                self.editing = Some((id, state));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.entries.is_empty() {
            return;
        }
        self.commit_edit(cx);
        // 改写后的文本不能为空（core 也会拒绝，这里提前给出对应条目的提示）。
        if self.entries.iter().any(|item| self.edits.get(&item.id).is_some_and(|t| t.trim().is_empty())) {
            self.error = Some(self.t("改写后的文本不能为空。"));
            cx.notify();
            return;
        }
        if self
            .service
            .exporting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.error = Some(self.t("已有素材正在导出，请完成或取消当前保存对话框。"));
            cx.notify();
            return;
        }
        let mut options = self.options.clone();
        options.ids = self.entries.iter().map(|item| item.id).collect();
        options.texts = self.entries.iter().map(|item| self.edits.get(&item.id).cloned()).collect();
        if options.texts.iter().all(Option::is_none) {
            options.texts.clear();
        }
        let service = self.service.clone();
        let (title, filter) = (self.t("导出剪贴板素材"), self.t("导出文件"));
        let remember = crate::shell::settings(cx).export.remember_directory;
        self.busy = true;
        self.error = None;
        self.message = None;
        cx.notify();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = run(
                cx,
                service.clone(),
                options,
                remember,
                title,
                filter,
                handle,
            )
            .await;
            service.exporting.store(false, Ordering::Release);
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(Some(count)) => {
                        log::info!("event=gpui.clipboard_exported files={count}");
                        view.message = Some(format!("{} {count}", view.t("已保存文件")));
                    }
                    Ok(None) => view.message = Some(view.t("已取消导出。")),
                    Err(error) => {
                        log::warn!("event=gpui.clipboard_export_failed message={error}");
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 返回写出的文件数；`None` 表示取消。
async fn run(
    cx: &mut AsyncApp,
    service: Service,
    options: Options,
    remember: bool,
    title: String,
    filter: String,
    owner: AnyWindowHandle,
) -> Result<Option<usize>, String> {
    let staged = cx
        .background_executor()
        .spawn({
            let options = options.clone();
            async move {
                let (dir, items) = export::stage(&service, &options)?;
                // 缺字体时在打开保存对话框之前提示。
                let font = if export::needs_font(&items, options.mode) {
                    snap_core::pins::text::font_bytes(annotate::user_font())?
                } else {
                    Vec::new()
                };
                Ok::<_, String>((dir, items, font))
            }
        })
        .await?;
    let record = snap_core::export::directory_record();
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(directory) = record
        .as_deref()
        .and_then(|path| snap_core::export::preferred_directory(path, remember))
    {
        dialog = dialog.set_directory(directory);
    }
    let separate = options.mode == Mode::Separate;
    let ext = if options.mode == Mode::Pdf {
        "pdf"
    } else {
        "png"
    };
    if !separate {
        dialog = dialog.add_filter(filter, &[ext]).set_file_name(format!(
            "SnapDesk-{}.{ext}",
            snap_core::clipboard::service::now()
        ));
    }
    let hidden = cx.update(|cx| crate::pins::hide_pins(cx, Some(owner)));
    let picked = if separate {
        dialog.pick_folder().await
    } else {
        dialog.save_file().await
    };
    cx.update(|cx| crate::pins::show_pins(cx, hidden));
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.path().to_path_buf();
    let (_dir, items, font) = staged;
    let (count, directory) = cx
        .background_executor()
        .spawn(async move {
            if separate {
                let written =
                    export::write_separate(&items, &path, &snap_core::export::timestamp())?;
                Ok::<_, String>((written.len(), path))
            } else {
                export::write_combined(&items, &options, &path, &font)?;
                Ok((
                    1,
                    path.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
                ))
            }
        })
        .await?;
    if remember {
        if let Some(record) = record {
            // 文件已保存；目录记忆失败只记录。
            if snap_core::export::remember_directory(&record, &directory).is_err() {
                log::warn!("event=gpui.directory_memory_failed");
            }
        }
    }
    Ok(Some(count))
}

/// `--cmp-*` 色值（Vue `ClipboardComposer.vue`）。
struct Colors {
    bg: u32,
    soft: u32,
    card: u32,
    border: u32,
    text: u32,
    muted: u32,
    accent: u32,
    selected: u32,
    danger: u32,
    success: u32,
    hover: u32,
}

fn colors() -> Colors {
    if crate::theme::dark() {
        Colors { bg: 0x1b1f24, soft: 0x14181d, card: 0x252b33, border: 0x39424e, text: 0xe6edf3, muted: 0x9aa7b4, accent: 0x79a6ff, selected: 0x1f3352, danger: 0xff8a80, success: 0x73d69a, hover: 0x2b323c }
    } else {
        Colors { bg: 0xffffff, soft: 0xf3f5f8, card: 0xffffff, border: 0xe1e4e8, text: 0x24292f, muted: 0x6b7280, accent: 0x2563eb, selected: 0xeff6ff, danger: 0xb42318, success: 0x24834b, hover: 0xeef1f4 }
    }
}

impl Composer {
    fn thumbnail(&mut self, id: i64, cx: &mut Context<Self>) -> Option<std::sync::Arc<RenderImage>> {
        if let Some(thumb) = self.thumbs.get(&id) {
            return Some(thumb.clone());
        }
        if self.thumbs_pending.insert(id) {
            let service = self.service.clone();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        let bytes = service.thumbnail(id)?;
                        let mut image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
                        for pixel in image.pixels_mut() {
                            pixel.0.swap(0, 2);
                        }
                        Ok::<_, String>(std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
                    })
                    .await;
                let _ = this.update(cx, |view, cx| {
                    if let Ok(thumb) = decoded {
                        view.thumbs.insert(id, thumb);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        None
    }

    fn entry_text(&self, item: &Item) -> String {
        self.edits.get(&item.id).cloned().unwrap_or_else(|| item.preview.clone())
    }

    fn icon_button(&self, id: impl Into<ElementId>, data: &'static [u8], tip: String, pressed: bool, danger: bool, c: &Colors) -> Stateful<Div> {
        let (hover_fg, bg) = (if danger { c.danger } else { c.text }, if pressed { Some(c.selected) } else { None });
        let mut button = div()
            .id(id)
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .cursor_pointer()
            .text_color(rgb(if pressed { c.accent } else { c.muted }))
            .child(Icon::default().data(data).size(px(14.0)))
            .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into());
        if let Some(bg) = bg {
            button = button.bg(rgb(bg));
        }
        let soft = c.soft;
        button.hover(move |s| s.bg(rgb(soft)).text_color(rgb(hover_fg)))
    }

    /// 数值输入（−/+ 与数值，样式同 Vue 的 64 像素数字框）。
    fn stepper(&self, id: &'static str, value: String, unit: &'static str, down: fn(&mut Self, &mut Context<Self>), up: fn(&mut Self, &mut Context<Self>), c: &Colors, cx: &mut Context<Self>) -> Div {
        let small = |index: usize, label: &'static str, action: fn(&mut Self, &mut Context<Self>)| {
            let hover = c.hover;
            div()
                .id((id, index))
                .w(px(20.0))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(hover)))
                .child(label)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| action(view, cx)))
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(c.border))
                    .bg(rgb(c.bg))
                    .overflow_hidden()
                    .text_color(rgb(c.text))
                    .child(small(0, "−", down))
                    .child(div().w(px(32.0)).flex().justify_center().child(value))
                    .child(small(1, "+", up)),
            )
            .child(unit)
    }
}

impl Render for Composer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors();
        let language = self.language.clone();
        let text = move |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let mode = self.options.mode;
        let busy = self.busy;

        // ---- 工具栏：导出方式 + 参数 ----
        let mut modes = div().flex().p(px(2.0)).rounded(px(8.0)).bg(rgb(c.soft));
        for (index, (value, label)) in [(Mode::Pdf, "PDF"), (Mode::LongPng, "长图"), (Mode::Separate, "分别保存")].into_iter().enumerate() {
            let checked = mode == value;
            let mut button = div()
                .id(("export-mode", index))
                .h(px(28.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .child(text(label))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    if !view.busy {
                        view.options.mode = value;
                        cx.notify();
                    }
                }));
            button = if checked {
                button.bg(rgb(c.bg)).text_color(rgb(c.accent)).font_weight(FontWeight::SEMIBOLD).shadow_sm()
            } else {
                button.text_color(rgb(c.muted))
            };
            modes = modes.child(button);
        }
        let choice = |id: &'static str, label: String, on: bool, action: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>| {
            let hover = c.hover;
            div()
                .id(id)
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(if on { c.accent } else { c.border }))
                .bg(rgb(if on { c.selected } else { c.bg }))
                .text_color(rgb(if on { c.accent } else { c.text }))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(hover)))
                .child(label)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| action(view, cx)))
        };
        let mut params = div().flex().flex_wrap().items_center().gap(px(10.0)).text_color(rgb(c.muted));
        match mode {
            Mode::Pdf => {
                let a4 = self.options.page_size == PageSize::A4;
                params = params
                    .child(choice("export-a4", "A4".into(), a4, |v, cx| {
                        v.options.page_size = PageSize::A4;
                        cx.notify();
                    }, cx))
                    .child(choice("export-original", text("图片原始比例"), !a4, |v, cx| {
                        v.options.page_size = PageSize::Original;
                        cx.notify();
                    }, cx))
                    .child(text("边距"))
                    .child(self.stepper("export-margin", format!("{}", self.options.margin_mm), "mm", |v, cx| v.step(0, -2.0, cx), |v, cx| v.step(0, 2.0, cx), &c, cx));
            }
            Mode::LongPng => {
                let white = self.options.background == Background::White;
                params = params
                    .child(text("间距"))
                    .child(self.stepper("export-gap", format!("{}", self.options.gap_px), "px", |v, cx| v.step(-8, 0.0, cx), |v, cx| v.step(8, 0.0, cx), &c, cx))
                    .child(choice("export-white", text("白底"), white, |v, cx| {
                        v.options.background = Background::White;
                        cx.notify();
                    }, cx))
                    .child(choice("export-transparent", text("透明背景"), !white, |v, cx| {
                        v.options.background = Background::Transparent;
                        cx.notify();
                    }, cx));
            }
            Mode::Separate => {
                params = params.child(div().text_xs().child(text("文本存为 TXT，图片按原图存为 PNG，放进新建的批次文件夹。")));
            }
        }
        let toolbar = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(16.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(rgb(c.border))
            .child(modes)
            .child(params);

        // ---- 左侧素材卡片 ----
        let mut list = div()
            .id("export-entries")
            .w(px(300.0))
            .flex_none()
            .h_full()
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .overflow_y_scroll()
            .border_r_1()
            .border_color(rgb(c.border))
            .child(div().mx(px(2.0)).mb(px(2.0)).text_xs().line_height(relative(1.5)).text_color(rgb(c.muted)).child(text("用 ↑ ↓ 调整顺序，改写只影响本次导出。")));
        if self.entries.is_empty() {
            list = list.child(div().mx(px(2.0)).text_xs().text_color(rgb(c.muted)).child(text("素材已全部移除，关闭后重新选择。")));
        }
        let entries = self.entries.clone();
        for (index, item) in entries.iter().enumerate() {
            let id = item.id;
            let edited = self.edits.contains_key(&id);
            let editing = self.editing.as_ref().filter(|(current, _)| *current == id).map(|(_, state)| state.clone());
            let is_text = item.kind == Kind::Text;
            let mut actions = div().ml_auto().flex().gap(px(2.0));
            if index > 0 {
                actions = actions.child(self.icon_button(("export-up", index), annotate::icons::CHEVRON_UP, text("上移"), false, false, &c).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.move_entry(index, index - 1, cx))));
            }
            if index + 1 < entries.len() {
                actions = actions.child(self.icon_button(("export-down", index), annotate::icons::CHEVRON_DOWN, text("下移"), false, false, &c).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.move_entry(index, index + 1, cx))));
            }
            if edited && editing.is_none() {
                actions = actions.child(self.icon_button(("export-restore", index), annotate::icons::UNDO, text("还原原文"), false, false, &c).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    if !view.busy {
                        view.edits.remove(&id);
                        cx.notify();
                    }
                })));
            }
            if is_text {
                actions = actions.child(self.icon_button(("export-edit", index), annotate::icons::PENCIL, text("改写文本"), editing.is_some(), false, &c).on_click(cx.listener(move |view, _: &ClickEvent, window, cx| view.toggle_edit(id, window, cx))));
            }
            actions = actions.child(self.icon_button(("export-remove", index), annotate::icons::CLOSE, text("从本次导出移除"), false, true, &c).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                if !view.busy && index < view.entries.len() {
                    let removed = view.entries.remove(index);
                    view.edits.remove(&removed.id);
                    if view.editing.as_ref().is_some_and(|(current, _)| *current == removed.id) {
                        view.editing = None;
                    }
                    cx.notify();
                }
            })));
            let head = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_h(px(24.0))
                .child(
                    div()
                        .min_w(px(18.0))
                        .h(px(18.0))
                        .px(px(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.0))
                        .bg(rgb(c.selected))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c.accent))
                        .child(format!("{}", index + 1)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_xs()
                        .text_color(rgb(c.muted))
                        .child(Icon::default().data(if is_text { annotate::icons::TYPE } else { annotate::icons::SQUARE }).size(px(13.0)))
                        .child(if is_text { text("文本") } else { format!("{} × {}", item.width.unwrap_or(0), item.height.unwrap_or(0)) }),
                )
                .children(edited.then(|| div().px(px(6.0)).rounded(px(4.0)).bg(rgb(c.selected)).text_size(px(11.0)).text_color(rgb(c.accent)).child(text("已改写"))))
                .child(actions);
            let body: AnyElement = if let Some(state) = editing {
                div()
                    .mt(px(6.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .min_h(px(120.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(c.border))
                    .bg(rgb(c.bg))
                    .text_xs()
                    .child(Textarea::new(&state).appearance(false))
                    .into_any_element()
            } else if is_text {
                div().mt(px(4.0)).text_xs().line_height(relative(1.5)).line_clamp(3).child(self.entry_text(item)).into_any_element()
            } else {
                div()
                    .mt(px(4.0))
                    .h(px(72.0))
                    .flex()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(rgb(c.soft))
                    .overflow_hidden()
                    .children(self.thumbnail(id, cx).map(|t| img(t).h_full().object_fit(ObjectFit::Contain)))
                    .into_any_element()
            };
            list = list.child(
                div()
                    .pt(px(6.0))
                    .px(px(8.0))
                    .pb(px(8.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(if self.editing.as_ref().is_some_and(|(e, _)| *e == id) { c.accent } else { c.border }))
                    .bg(rgb(c.card))
                    .child(head)
                    .child(body),
            );
        }

        // ---- 右侧近似预览 ----
        let mut preview = div()
            .id("export-preview")
            .flex_1()
            .min_w_0()
            .h_full()
            .p(px(20.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(18.0))
            .overflow_y_scroll()
            .bg(rgb(c.soft));
        match mode {
            Mode::Pdf => {
                let a4 = self.options.page_size == PageSize::A4;
                let pad = (self.options.margin_mm as f32 / 210.0 * 320.0).max(0.0);
                for (index, item) in entries.iter().enumerate() {
                    let (w, h) = if a4 || item.kind == Kind::Text {
                        (320.0, 320.0 * 297.0 / 210.0)
                    } else {
                        let (iw, ih) = (item.width.unwrap_or(1).max(1) as f32, item.height.unwrap_or(1).max(1) as f32);
                        (320.0, (320.0 * ih / iw).min(600.0))
                    };
                    let sheet_content: AnyElement = if item.kind == Kind::Text {
                        div().size_full().text_size(px(9.0)).line_height(relative(1.6)).text_color(rgb(0x111111)).overflow_hidden().child(self.entry_text(item)).into_any_element()
                    } else {
                        div().size_full().flex().items_center().justify_center().children(self.thumbnail(item.id, cx).map(|t| img(t).size_full().object_fit(ObjectFit::Contain))).into_any_element()
                    };
                    preview = preview.child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(div().w(px(w)).h(px(h)).p(px(pad)).bg(rgb(0xffffff)).shadow_sm().overflow_hidden().child(sheet_content))
                            .child(div().mt(px(6.0)).text_size(px(11.0)).text_color(rgb(c.muted)).child(format!("{} · {}", text("第 {0} 页").replace("{0}", &(index + 1).to_string()), text("素材 {0}").replace("{0}", &(index + 1).to_string())))),
                    );
                }
            }
            Mode::LongPng => {
                let mut long = div()
                    .w(px(420.0))
                    .flex()
                    .flex_col()
                    .gap(px(self.options.gap_px as f32 * 420.0 / 1000.0))
                    .bg(rgb(if self.options.background == Background::White { 0xffffff } else { 0xe6e6e6 }))
                    .shadow_sm();
                for item in &entries {
                    long = long.child(if item.kind == Kind::Text {
                        div().w_full().p(px(8.0)).bg(rgb(0xffffff)).text_size(px(6.0)).line_height(relative(2.0)).text_color(rgb(0x111111)).child(self.entry_text(item)).into_any_element()
                    } else {
                        let (iw, ih) = (item.width.unwrap_or(1).max(1) as f32, item.height.unwrap_or(1).max(1) as f32);
                        div().w_full().h(px(420.0 * ih / iw)).children(self.thumbnail(item.id, cx).map(|t| img(t).size_full().object_fit(ObjectFit::Fill))).into_any_element()
                    });
                }
                preview = preview.child(long);
            }
            Mode::Separate => {
                let mut grid = div().w_full().flex().flex_wrap().gap(px(12.0));
                for (index, item) in entries.iter().enumerate() {
                    let tile: AnyElement = if item.kind == Kind::Text {
                        div().size_full().p(px(8.0)).text_size(px(11.0)).line_height(relative(1.4)).line_clamp(5).child(self.entry_text(item)).into_any_element()
                    } else {
                        div().size_full().flex().items_center().justify_center().children(self.thumbnail(item.id, cx).map(|t| img(t).size_full().object_fit(ObjectFit::Contain))).into_any_element()
                    };
                    grid = grid.child(
                        div()
                            .w(px(140.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(div().w_full().h(px(105.0)).rounded(px(6.0)).border_1().border_color(rgb(c.border)).bg(rgb(c.bg)).overflow_hidden().child(tile))
                            .child(div().text_xs().text_color(rgb(c.muted)).child(format!("{:02}.{}", index + 1, if item.kind == Kind::Text { "txt" } else { "png" }))),
                    );
                }
                preview = preview.items_start().child(grid);
            }
        }
        preview = preview.child(div().text_size(px(11.0)).text_color(rgb(c.muted)).child(text("预览为近似排版，文字换行以导出结果为准。")));

        // ---- 底栏 ----
        let count = entries.len();
        let images = entries.iter().filter(|i| i.kind != Kind::Text).count();
        let status: AnyElement = match (&self.error, &self.message) {
            (Some(error), _) => div().text_color(rgb(c.danger)).child(error.clone()).into_any_element(),
            (None, Some(message)) => div().text_color(rgb(c.success)).child(message.clone()).into_any_element(),
            _ => div().child(format!("{} {count} · {} {} · {} {}", text("素材"), text("图片"), images, text("文本"), count - images)).into_any_element(),
        };
        let can_export = !busy && count > 0;
        let hover = c.hover;
        let footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(rgb(c.border))
            .child(div().flex_1().min_w_0().truncate().text_xs().text_color(rgb(c.muted)).child(status))
            .child(
                div()
                    .id("export-cancel")
                    .h(px(32.0))
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(c.border))
                    .bg(rgb(c.bg))
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgb(hover)))
                    .child(text("关闭"))
                    .on_click(cx.listener(|view, _: &ClickEvent, window, _| {
                        if !view.busy {
                            window.remove_window();
                        }
                    })),
            )
            .child({
                let button = div()
                    .id("export-run")
                    .h(px(32.0))
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .bg(rgb(0x2563eb))
                    .text_color(rgb(0xffffff))
                    .font_weight(FontWeight::MEDIUM)
                    .child(text(if busy { "正在导出…" } else { "选择位置并导出" }));
                if can_export {
                    button.cursor_pointer().hover(|s| s.bg(rgb(0x1d4ed8))).on_click(cx.listener(|view, _: &ClickEvent, window, cx| view.export(window, cx)))
                } else {
                    button.opacity(0.5)
                }
            });

        div()
            .id("clipboard-export")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" && !view.busy {
                    window.remove_window();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(c.bg))
            .text_color(rgb(c.text))
            .text_size(px(13.0))
            .child(toolbar)
            .child(div().flex_1().min_h_0().flex().child(list).child(preview))
            .child(footer)
    }
}

