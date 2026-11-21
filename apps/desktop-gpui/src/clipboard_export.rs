//! 剪贴板组合导出窗口（对照 Vue 版 `ClipboardComposer.vue`）：所选文本与图片按顺序导出为 PDF、长图或分别保存。
//!
//! 素材可上移、下移、移除；参数与 Tauri 版相同（页面 A4/原始比例、长图背景、间距 0～200px、边距 0～30mm），
//! 编码与校验在 core `snap_core::clipboard::export`。Vue 版的“改写文本”与分页预览未迁移，导出始终使用历史原文。
//! 文本排版字体：设置里下载/选择的导出字体，没有时回退到本机中文字体。
use crate::annotate;
use gpui_kit::*;
use snap_core::clipboard::{
    export::{self, Background, Mode, Options, PageSize},
    model::{Item, Kind},
    service::Service,
};
use std::sync::atomic::Ordering;

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
                annotate::ACTIVE_FG
            } else {
                annotate::MUTED
            }))
            .bg(rgb(if active {
                annotate::ACTIVE_BG
            } else {
                annotate::SURFACE
            }))
            .hover(|s| s.text_color(rgb(annotate::TEXT)))
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
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.entries.is_empty() {
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
                .text_color(rgb(annotate::MUTED))
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
                options = options.child(div().text_xs().text_color(rgb(annotate::MUTED)).child(
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
                    .text_color(rgb(annotate::MUTED))
                    .child(text("组合导出仅支持文本和图片，文件引用请复制后粘贴")),
            );
        }
        for (index, item) in self.entries.iter().enumerate() {
            let label = match item.kind {
                Kind::Image => format!(
                    "{} {}×{}",
                    text("图片"),
                    item.width.unwrap_or(0),
                    item.height.unwrap_or(0)
                ),
                _ => item.preview.lines().next().unwrap_or_default().to_string(),
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .p(px(6.0))
                    .rounded(px(6.0))
                    .bg(rgb(annotate::SURFACE))
                    .child(
                        div()
                            .w(px(24.0))
                            .text_xs()
                            .text_color(rgb(annotate::MUTED))
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
                                    view.entries.remove(index);
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        let notice = match (&self.error, &self.message) {
            (Some(error), _) => Some(
                div()
                    .text_xs()
                    .text_color(rgb(0xfb7185))
                    .child(error.clone()),
            ),
            (None, Some(message)) => Some(
                div()
                    .text_xs()
                    .text_color(rgb(0x86efac))
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
            .bg(rgb(0x181b20))
            .text_color(rgb(annotate::TEXT))
            .child(options)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(annotate::MUTED))
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
