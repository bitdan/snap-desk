//! 剪贴板组合导出窗口（对照 Vue 版 `ClipboardComposer.vue`）：所选文本与图片按顺序导出为 PDF、长图或分别保存。
//!
//! 素材可上移、下移、移除；参数与 Tauri 版相同（页面 A4/原始比例、长图背景、间距 0～200px、边距 0～30mm），
//! 编码与校验在 core `snap_core::clipboard::export`。文本素材可改写（只作用于本次导出，不修改历史，可还原原文）；
//! Vue 版的分页预览未迁移。
//! 文本排版字体：设置里下载/选择的导出字体，没有时回退到本机中文字体。
use crate::annotate;
use gpui_kit::{
    component::input::{Textarea, TextareaState},
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
            size(px(480.0), px(520.0)),
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
        window_min_size: Some(size(px(400.0), px(360.0))),
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
    busy: bool,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
}

impl Composer {
    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn pill(&self, id: impl Into<ElementId>, label: String, active: bool) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(9.0))
            .py(px(3.0))
            .rounded(px(12.0))
            .text_xs()
            .cursor_pointer()
            .text_color(rgb(if active {
                annotate::active_fg()
            } else {
                annotate::muted()
            }))
            .bg(rgb(if active {
                annotate::active_bg()
            } else {
                annotate::surface()
            }))
            .hover(|s| s.text_color(rgb(annotate::text())))
            .child(label)
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

impl Render for Composer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.language.clone();
        let text =
            move |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let mode = self.options.mode;
        let mut modes = div().flex().gap(px(4.0));
        for (index, (value, label)) in [
            (Mode::Pdf, "PDF"),
            (Mode::LongPng, "长图"),
            (Mode::Separate, "分别保存"),
        ]
        .into_iter()
        .enumerate()
        {
            modes = modes.child(
                self.pill(("export-mode", index), text(label), mode == value)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.options.mode = value;
                        cx.notify();
                    })),
            );
        }
        let mut options = div().flex().flex_col().gap(px(6.0)).child(modes);
        let row = |label: String| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_xs()
                .text_color(rgb(annotate::muted()))
                .child(div().w(px(72.0)).child(label))
        };
        match mode {
            Mode::Pdf => {
                let page = self.options.page_size;
                options = options
                    .child(
                        row(text("页面"))
                            .child(
                                self.pill("export-a4", "A4".into(), page == PageSize::A4)
                                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                        view.options.page_size = PageSize::A4;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                self.pill(
                                    "export-original",
                                    text("原始比例"),
                                    page == PageSize::Original,
                                )
                                .on_click(cx.listener(
                                    |view, _: &ClickEvent, _, cx| {
                                        view.options.page_size = PageSize::Original;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(
                        row(text("边距"))
                            .child(self.pill("export-margin-down", "−".into(), false).on_click(
                                cx.listener(|view, _: &ClickEvent, _, cx| view.step(0, -2.0, cx)),
                            ))
                            .child(format!("{} mm", self.options.margin_mm))
                            .child(self.pill("export-margin-up", "+".into(), false).on_click(
                                cx.listener(|view, _: &ClickEvent, _, cx| view.step(0, 2.0, cx)),
                            )),
                    );
            }
            Mode::LongPng => {
                let background = self.options.background;
                options = options
                    .child(
                        row(text("背景"))
                            .child(
                                self.pill(
                                    "export-white",
                                    text("白色"),
                                    background == Background::White,
                                )
                                .on_click(cx.listener(
                                    |view, _: &ClickEvent, _, cx| {
                                        view.options.background = Background::White;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                self.pill(
                                    "export-transparent",
                                    text("透明"),
                                    background == Background::Transparent,
                                )
                                .on_click(cx.listener(
                                    |view, _: &ClickEvent, _, cx| {
                                        view.options.background = Background::Transparent;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(
                        row(text("间距"))
                            .child(self.pill("export-gap-down", "−".into(), false).on_click(
                                cx.listener(|view, _: &ClickEvent, _, cx| view.step(-8, 0.0, cx)),
                            ))
                            .child(format!("{} px", self.options.gap_px))
                            .child(self.pill("export-gap-up", "+".into(), false).on_click(
                                cx.listener(|view, _: &ClickEvent, _, cx| view.step(8, 0.0, cx)),
                            )),
                    );
            }
            Mode::Separate => {
                options = options.child(div().text_xs().text_color(rgb(annotate::muted())).child(
                    text("在所选目录中新建批次文件夹，文本存为 TXT，图片存为 PNG。"),
                ));
            }
        }
        let count = self.entries.len();
        let mut list = div()
            .id("export-entries")
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .overflow_y_scroll();
        if count == 0 {
            list = list.child(
                div()
                    .p(px(12.0))
                    .text_sm()
                    .text_color(rgb(annotate::muted()))
                    .child(text("组合导出仅支持文本和图片，文件引用请复制后粘贴")),
            );
        }
        for (index, item) in self.entries.iter().enumerate() {
            let id = item.id;
            let edited = self.edits.contains_key(&id);
            let editing = self.editing.as_ref().filter(|(current, _)| *current == id).map(|(_, state)| state.clone());
            let label = match item.kind {
                Kind::Image => format!(
                    "{} {}×{}",
                    text("图片"),
                    item.width.unwrap_or(0),
                    item.height.unwrap_or(0)
                ),
                _ => self
                    .edits
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or(&item.preview)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            };
            let is_text = item.kind == Kind::Text;
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .p(px(6.0))
                    .rounded(px(6.0))
                    .bg(rgb(annotate::surface()))
                    .child(
                        div()
                            .w(px(24.0))
                            .text_xs()
                            .text_color(rgb(annotate::muted()))
                            .child(format!("{}", index + 1)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_sm()
                            .child(label),
                    )
                    .children(edited.then(|| {
                        div().text_xs().text_color(rgb(annotate::active_fg())).child(text("已改写"))
                    }))
                    .children((edited && editing.is_none()).then(|| {
                        self.pill(("export-restore", index), text("还原原文"), false).on_click(cx.listener(
                            move |view, _: &ClickEvent, _, cx| {
                                if !view.busy {
                                    view.edits.remove(&id);
                                    cx.notify();
                                }
                            },
                        ))
                    }))
                    .children(is_text.then(|| {
                        let label = text(if editing.is_some() { "完成" } else { "改写文本" });
                        self.pill(("export-edit", index), label, editing.is_some()).on_click(cx.listener(
                            move |view, _: &ClickEvent, window, cx| view.toggle_edit(id, window, cx),
                        ))
                    }))
                    .child(self.pill(("export-up", index), "↑".into(), false).on_click(
                        cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.move_entry(index, index.saturating_sub(1), cx)
                        }),
                    ))
                    .child(
                        self.pill(("export-down", index), "↓".into(), false)
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                view.move_entry(index, index + 1, cx)
                            })),
                    )
                    .child(
                        self.pill(("export-remove", index), text("移除"), false)
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                if !view.busy && index < view.entries.len() {
                                    let removed = view.entries.remove(index);
                                    view.edits.remove(&removed.id);
                                    if view.editing.as_ref().is_some_and(|(current, _)| *current == removed.id) {
                                        view.editing = None;
                                    }
                                    cx.notify();
                                }
                            })),
                    ),
            );
            if let Some(state) = editing {
                list = list.child(
                    div()
                        .p(px(4.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(rgb(annotate::border()))
                        .child(Textarea::new(&state)),
                );
            }
        }
        let notice = match (&self.error, &self.message) {
            (Some(error), _) => Some(
                div()
                    .text_xs()
                    .text_color(rgb(crate::theme::c().error_text))
                    .child(error.clone()),
            ),
            (None, Some(message)) => Some(
                div()
                    .text_xs()
                    .text_color(rgb(crate::theme::c().ok_text))
                    .child(message.clone()),
            ),
            _ => None,
        };
        let busy = self.busy;
        let action = if busy {
            text("正在导出…")
        } else {
            text("导出…")
        };
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
            .gap(px(10.0))
            .p(px(12.0))
            .bg(rgb(crate::theme::c().panel))
            .text_color(rgb(annotate::text()))
            .child(options)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(annotate::muted()))
                    .child(format!("{} {count}", text("素材"))),
            )
            .child(list)
            .children(notice)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.0))
                    .child(
                        self.pill("export-cancel", text("关闭"), false)
                            .on_click(cx.listener(|view, _: &ClickEvent, window, _| {
                                if !view.busy {
                                    window.remove_window();
                                }
                            })),
                    )
                    .child(
                        self.pill("export-run", action, !busy && count > 0)
                            .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                                view.export(window, cx)
                            })),
                    ),
            )
    }
}
