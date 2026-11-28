//! 滚动截图的结果窗口：拼接完成后打开，预览整张长图，拖动两条裁剪线去掉多余的页眉、页脚
//! （横向长图为左右两端），再复制、保存、贴图（贴图后可在贴图上继续标注）或识别文字。
//! 打开时自动把裁剪线放到两端纯色空白的内容边缘（`snap_core::scroll::trim`），可重置或再次自动裁剪。
//!
//! 成图在完成时已复制并存入 `scroll-results`（见 `scroll.rs`）；这里的操作都作用于裁剪后的图片，
//! 不改写已保存的原始结果，直接关闭窗口即放弃裁剪。GPU 纹理有尺寸上限，长图不能整张上传，
//! 预览用缩小的图片；裁剪线按预览在窗口中的实际位置（随滚动变化，每帧记录）换算回原图像素。
//!
//! 普通窗口（不排除在采集之外）：留在屏幕上时再截图也能正常拍到，不会出现黑块。
use crate::annotate::{self, icons};
use crate::recognition::Mode as RecognitionMode;
use gpui_kit::*;
use image::RgbaImage;
use snap_core::settings::Language;
use std::{cell::Cell, rc::Rc, sync::Arc};

/// 预览在垂直于裁剪方向上的逻辑尺寸上限（纵向长图为宽度，横向长图为高度）。
const PREVIEW_CROSS: f32 = 420.0;
/// 预览缩略图的长边上限（像素）。
const MAX_TEXTURE: u32 = 8192;
/// 裁剪后至少保留的长度（原图像素），避免裁成空图。
const MIN_KEEP: u32 = 16;
/// 裁剪线的可拖动范围（逻辑像素，线两侧各一半）。
const LINE_HIT: f32 = 12.0;
/// 预览四周的留白与底部操作栏的高度（逻辑像素），用于计算初始窗口大小。
const PADDING: f32 = 16.0;
const FOOTER: f32 = 56.0;
const MIN_WIDTH: f32 = 560.0;
const MIN_HEIGHT: f32 = 320.0;
const ACCENT: u32 = 0x2563eb;

/// 裁剪线：纵向长图的顶部 / 底部，横向长图的左端 / 右端。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Start,
    End,
}

/// 拼接完成后打开结果窗口；缩略图在后台线程生成后再开窗。
pub fn open(cx: &mut App, image: Arc<RgbaImage>, horizontal: bool, language: Language) {
    cx.spawn(async move |cx| {
        let (preview, auto) = cx
            .background_executor()
            .spawn({
                let image = image.clone();
                async move {
                    let auto = snap_core::scroll::trim::blank_ends(&image, horizontal, MIN_KEEP);
                    (thumbnail(&image, horizontal), auto)
                }
            })
            .await;
        cx.update(|cx| open_window(cx, image, horizontal, preview, auto, language));
    })
    .detach();
}

/// 预览的逻辑尺寸：垂直于裁剪方向的一边不超过 [`PREVIEW_CROSS`]，等比缩放。
fn preview_size(image: &RgbaImage, horizontal: bool) -> (f32, f32) {
    let (w, h) = (image.width() as f32, image.height() as f32);
    if horizontal {
        let ph = h.min(PREVIEW_CROSS);
        (w * ph / h, ph)
    } else {
        let pw = w.min(PREVIEW_CROSS);
        (pw, h * pw / w)
    }
}

/// 预览缩略图（BGRA）：垂直于裁剪方向的一边按两倍预览尺寸采样（高分屏也清晰），长边不超过纹理上限。
fn thumbnail(image: &RgbaImage, horizontal: bool) -> Arc<RenderImage> {
    let (w, h) = image.dimensions();
    let (cross, long) = if horizontal { (h, w) } else { (w, h) };
    let mut k = (2.0 * f64::from(PREVIEW_CROSS) / f64::from(cross.max(1))).min(1.0);
    if f64::from(long) * k > f64::from(MAX_TEXTURE) {
        k = f64::from(MAX_TEXTURE) / f64::from(long);
    }
    let (tw, th) = (((f64::from(w) * k).round() as u32).max(1), ((f64::from(h) * k).round() as u32).max(1));
    let mut frame = if (tw, th) == (w, h) {
        image.clone()
    } else {
        image::imageops::resize(image, tw, th, image::imageops::FilterType::Triangle)
    };
    for pixel in frame.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(frame)]))
}

/// 按裁剪量裁出结果；两端都没裁时返回原图的副本。
fn cropped(image: &RgbaImage, horizontal: bool, start: u32, end: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (x, y, cw, ch) = if horizontal {
        (start, 0, w.saturating_sub(start + end).max(1), h)
    } else {
        (0, start, w, h.saturating_sub(start + end).max(1))
    };
    image::imageops::crop_imm(image, x, y, cw, ch).to_image()
}

fn open_window(
    cx: &mut App,
    image: Arc<RgbaImage>,
    horizontal: bool,
    preview: Arc<RenderImage>,
    auto: (u32, u32),
    language: Language,
) {
    let screen = cx
        .primary_display()
        .map(|d| d.visible_bounds().size)
        .unwrap_or(size(px(1280.0), px(800.0)));
    let (pw, ph) = preview_size(&image, horizontal);
    let width = (pw + PADDING * 2.0).max(MIN_WIDTH).min(f32::from(screen.width) * 0.9);
    let height = (ph + PADDING * 2.0 + FOOTER).max(MIN_HEIGHT).min(f32::from(screen.height) * 0.9);
    let title = snap_core::i18n::for_language(&language, "滚动截图").to_owned();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(width), px(height)), cx))),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
        ..Default::default()
    };
    let (w, h) = image.dimensions();
    let result = cx.open_window(options, |_, cx| {
        cx.new(|cx| ResultView {
            image,
            horizontal,
            preview,
            start: auto.0,
            end: auto.1,
            auto,
            dragging: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            language,
            busy: false,
            notice: None,
            focus: cx.focus_handle(),
            focused: false,
        })
    });
    match result {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            log::info!("event=gpui.scroll_result_opened width={w} height={h} auto_start={} auto_end={}", auto.0, auto.1);
        }
        Err(error) => log::error!("event=gpui.scroll_result_failed message={error}"),
    }
}

/// 关闭结果窗口；延后到当前更新结束，避免在窗口自身的事件处理中移除它。
fn close(handle: AnyWindowHandle, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    });
}

pub struct ResultView {
    image: Arc<RgbaImage>,
    horizontal: bool,
    preview: Arc<RenderImage>,
    /// 从起始端、末端裁掉的原图像素。
    start: u32,
    end: u32,
    /// 自动裁剪的建议值（两端纯色空白），打开时即采用；没有空白时为 (0, 0)。
    auto: (u32, u32),
    dragging: Option<Edge>,
    /// 预览图在窗口中的边界（绘制时写入，随滚动变化），用于把鼠标位置换算成原图像素。
    bounds: Rc<Cell<Bounds<Pixels>>>,
    language: Language,
    /// 复制、保存、贴图、识别进行中（后台裁剪与编码），忽略重复点击。
    busy: bool,
    notice: Option<String>,
    focus: FocusHandle,
    focused: bool,
}

impl ResultView {
    fn t(&self, key: &str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    /// 裁剪方向上的原图长度。
    fn long(&self) -> u32 {
        if self.horizontal { self.image.width() } else { self.image.height() }
    }

    fn cropped_size(&self) -> (u32, u32) {
        let (w, h) = self.image.dimensions();
        let cut = self.start + self.end;
        if self.horizontal { (w.saturating_sub(cut).max(1), h) } else { (w, h.saturating_sub(cut).max(1)) }
    }

    /// 拖动裁剪线：鼠标位置按预览的实际边界换算成原图像素，两端之间至少保留 [`MIN_KEEP`]。
    fn motion(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(edge) = self.dragging else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = None;
            cx.notify();
            return;
        }
        let b = self.bounds.get();
        let (offset, span) = if self.horizontal {
            (event.position.x - b.origin.x, b.size.width)
        } else {
            (event.position.y - b.origin.y, b.size.height)
        };
        let span = f32::from(span);
        if span <= 0.0 {
            return;
        }
        let long = self.long();
        let at = ((f32::from(offset) / span).clamp(0.0, 1.0) * long as f32).round() as u32;
        let room = long.saturating_sub(MIN_KEEP);
        match edge {
            Edge::Start => self.start = at.min(room.saturating_sub(self.end)),
            Edge::End => self.end = (long - at.min(long)).min(room.saturating_sub(self.start)),
        }
        cx.notify();
    }

    fn release(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging.take().is_some() {
            cx.notify();
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;
        match keystroke.key.as_str() {
            "escape" => window.remove_window(),
            "enter" => self.copy(window, cx),
            "c" if command => self.copy(window, cx),
            "s" if command => self.save(window, cx),
            "p" if command => self.pin(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// 在后台裁剪，然后在界面线程交给 `then`；进行中忽略重复触发。
    fn with_cropped(
        &mut self,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, RgbaImage, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let (image, horizontal, start, end) = (self.image.clone(), self.horizontal, self.start, self.end);
        cx.spawn(async move |this, cx| {
            let image = cx
                .background_executor()
                .spawn(async move { cropped(&image, horizontal, start, end) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                then(view, image, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 复制裁剪后的图片并关闭窗口（主操作，Enter / Ctrl+C）。
    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (owner, handle) = (crate::native::hwnd(window), window.window_handle());
        self.with_cropped(cx, move |view, image, cx| {
            let result = crate::capture::png(&image)
                .and_then(|png| crate::capture::write_clipboard(cx, owner, &image, png));
            match result {
                Ok(()) => {
                    log::info!("event=gpui.scroll_result_copied width={} height={}", image.width(), image.height());
                    close(handle, cx);
                }
                Err(error) => {
                    log::error!("event=gpui.scroll_result_copy_failed message={error}");
                    view.notice = Some(error);
                }
            }
        });
    }

    /// 贴到屏幕中央（按工作区大小缩放），可在贴图上继续标注；成功后关闭窗口。
    fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        self.with_cropped(cx, move |view, image, cx| {
            match crate::pins::create(cx, image, Vec::new(), crate::pins::Placement::Centered, None) {
                Ok(()) => {
                    log::info!("event=gpui.scroll_result_pinned");
                    close(handle, cx);
                }
                Err(error) => {
                    log::warn!("event=gpui.scroll_result_pin_failed message={error}");
                    view.notice = Some(error);
                }
            }
        });
    }

    /// 识别文字：结果在单独的识别窗口中显示，本窗口保留，可以接着裁剪或贴图。
    fn recognize(&mut self, cx: &mut Context<Self>) {
        self.with_cropped(cx, |_, image, cx| {
            log::info!("event=gpui.scroll_result_recognize width={} height={}", image.width(), image.height());
            crate::recognition::open(cx, image, RecognitionMode::Ocr);
        });
    }

    /// 保存裁剪后的图片：格式、文件名与目录记忆同截图保存；成功后关闭窗口。
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let export = crate::shell::settings(cx).export.clone();
        let (width, height) = self.cropped_size();
        let name = match snap_core::export::filename(&export, &snap_core::export::timestamp(), width, height) {
            Ok(name) => name,
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.notice = None;
        let record = snap_core::export::directory_record();
        let directory = record
            .as_deref()
            .and_then(|path| snap_core::export::preferred_directory(path, export.remember_directory));
        let dialog = crate::capture::SaveDialog::new(
            self.t("保存图片"),
            name,
            directory,
            export.format,
            &self.language,
            Some(window),
        );
        let handle = window.window_handle();
        let hidden = crate::pins::hide_pins(cx, None);
        let (image, horizontal, start, end) = (self.image.clone(), self.horizontal, self.start, self.end);
        cx.spawn(async move |this, cx| {
            let picked = dialog.pick().await;
            cx.update(|cx| crate::pins::show_pins(cx, hidden));
            let Some(path) = picked else {
                let _ = this.update(cx, |view, cx| {
                    view.busy = false;
                    cx.notify();
                });
                return;
            };
            let written = cx
                .background_executor()
                .spawn({
                    let (path, export) = (path.clone(), export.clone());
                    async move { snap_core::export::write_file(&path, &cropped(&image, horizontal, start, end), &export) }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match written {
                    Ok(()) => {
                        if export.remember_directory
                            && let (Some(record), Some(parent)) = (record, path.parent())
                        {
                            let _ = snap_core::export::remember_directory(&record, parent);
                        }
                        log::info!("event=gpui.scroll_result_saved");
                        close(handle, cx);
                    }
                    Err(error) => {
                        log::error!("event=gpui.scroll_result_save_failed message={error}");
                        view.notice = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 一条可拖动的裁剪线：2 像素的线加中间的小把手，命中范围比线宽。`at` 为线在预览中的位置。
    fn line(&self, edge: Edge, at: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = if edge == Edge::Start { "scroll-crop-start" } else { "scroll-crop-end" };
        let tip = self.t("拖动裁剪");
        let hit = div()
            .id(id)
            .absolute()
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    view.dragging = Some(edge);
                    cx.notify();
                }),
            );
        let half = LINE_HIT / 2.0;
        if self.horizontal {
            hit.top_0()
                .bottom_0()
                .left(px(at - half))
                .w(px(LINE_HIT))
                .cursor_col_resize()
                .child(div().absolute().top_0().bottom_0().left(px(half - 1.0)).w(px(2.0)).bg(rgb(ACCENT)))
                .child(div().w(px(8.0)).h(px(28.0)).rounded(px(4.0)).bg(rgb(ACCENT)))
        } else {
            hit.left_0()
                .right_0()
                .top(px(at - half))
                .h(px(LINE_HIT))
                .cursor_row_resize()
                .child(div().absolute().left_0().right_0().top(px(half - 1.0)).h(px(2.0)).bg(rgb(ACCENT)))
                .child(div().w(px(28.0)).h(px(8.0)).rounded(px(4.0)).bg(rgb(ACCENT)))
        }
    }

    /// 底部操作按钮：图标加文字；主操作（复制）为蓝底。
    fn button(&self, id: &'static str, icon: &'static [u8], label: String, primary: bool) -> Stateful<Div> {
        let c = crate::theme::c();
        let (fg, bg) = if primary { (0xffffff, ACCENT) } else { (c.text, c.surface) };
        let button = div()
            .id(id)
            .h(px(32.0))
            .px(px(12.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(if primary { ACCENT } else { c.border }))
            .bg(rgb(bg))
            .text_sm()
            .text_color(rgb(fg))
            .child(annotate::icon(icon, fg).size(px(15.0)))
            .child(label);
        if self.busy {
            button.opacity(0.5)
        } else if primary {
            button.cursor_pointer().hover(|s| s.bg(rgb(0x1d4ed8)))
        } else {
            button.cursor_pointer().hover(|s| s.bg(rgb(c.hover)))
        }
    }
}

impl Render for ResultView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus, cx);
            self.focused = true;
        }
        let c = crate::theme::c();
        let (pw, ph) = preview_size(&self.image, self.horizontal);
        let long = self.long().max(1) as f32;
        let span = if self.horizontal { pw } else { ph };
        let (start, end) = (self.start as f32 / long * span, (long - self.end as f32) / long * span);
        // 被裁掉的两端盖上半透明遮罩。
        let shade = |from: f32, to: f32| {
            let shade = div().absolute().bg(rgba(0x0f172a99));
            if self.horizontal {
                shade.top_0().bottom_0().left(px(from)).w(px((to - from).max(0.0)))
            } else {
                shade.left_0().right_0().top(px(from)).h(px((to - from).max(0.0)))
            }
        };
        let bounds = self.bounds.clone();
        let picture = div()
            .relative()
            .flex_none()
            .w(px(pw))
            .h(px(ph))
            .child(img(self.preview.clone()).absolute().size_full().object_fit(ObjectFit::Fill))
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            .child(shade(0.0, start))
            .child(shade(end, span))
            .child(self.line(Edge::Start, start, cx))
            .child(self.line(Edge::End, end, cx));
        let preview = div().id("scroll-result-preview").flex_1().min_h_0().min_w_0().p(px(PADDING)).bg(rgb(c.hover)).flex();
        // 纵向长图上下滚动、水平居中；横向长图左右滚动、垂直居中。
        let preview = if self.horizontal {
            preview.overflow_x_scroll().items_center()
        } else {
            preview.overflow_y_scroll().flex_col().items_center()
        }
        .child(picture);
        let (w, h) = self.image.dimensions();
        let (cw, ch) = self.cropped_size();
        let mut info = div()
            .mr_auto()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_xs()
            .text_color(rgb(c.muted))
            .child(div().text_color(rgb(c.text)).child(format!("{cw} × {ch} px")));
        if self.start + self.end > 0 {
            info = info.child(format!("{} {w} × {h}", self.t("原图"))).child(
                div()
                    .id("scroll-crop-reset")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .text_color(rgb(ACCENT))
                    .hover(|s| s.bg(rgb(c.hover)))
                    .child(self.t("重置裁剪"))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.start = 0;
                        view.end = 0;
                        cx.notify();
                    })),
            );
        }
        if self.auto != (0, 0) && (self.start, self.end) != self.auto {
            info = info.child(
                div()
                    .id("scroll-crop-auto")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .text_color(rgb(ACCENT))
                    .hover(|s| s.bg(rgb(c.hover)))
                    .child(self.t("自动裁剪"))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        (view.start, view.end) = view.auto;
                        cx.notify();
                    })),
            );
        }
        if let Some(notice) = &self.notice {
            info = info.child(
                div()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(rgb(c.warn_text))
                    .child(self.t(notice)),
            );
        }
        let footer = div()
            .flex_none()
            .h(px(FOOTER))
            .px(px(PADDING))
            .flex()
            .items_center()
            .gap(px(6.0))
            .border_t_1()
            .border_color(rgb(c.border))
            .child(info)
            .child(
                self.button("scroll-result-ocr", icons::SCAN_TEXT, self.t("识别文字"), false)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.recognize(cx))),
            )
            .child(
                self.button("scroll-result-pin", icons::PIN, self.t("贴图"), false)
                    .tooltip({
                        let tip = self.t("贴到桌面，可继续标注 Ctrl/⌘+P");
                        move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into()
                    })
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| view.pin(window, cx))),
            )
            .child(
                self.button("scroll-result-save", icons::SAVE, self.t("保存"), false)
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| view.save(window, cx))),
            )
            .child(
                self.button("scroll-result-copy", icons::COPY, self.t("复制"), true)
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| view.copy(window, cx))),
            );
        div()
            .id("scroll-result")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(c.surface))
            .text_color(rgb(c.text))
            .on_key_down(cx.listener(Self::key))
            .on_mouse_move(cx.listener(Self::motion))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::release))
            .child(preview)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    // 不用 `super::*`：其中的 `gpui_kit::*` 带有 GPUI 自己的 `test` 宏，会遮住标准的 `#[test]`。
    use super::{cropped, preview_size, thumbnail, MAX_TEXTURE};
    use image::RgbaImage;

    #[test]
    fn crops_along_the_scroll_direction_and_keeps_at_least_one_pixel() {
        let image = RgbaImage::from_fn(4, 10, |_, y| image::Rgba([y as u8, 0, 0, 255]));
        let top = cropped(&image, false, 2, 3);
        assert_eq!(top.dimensions(), (4, 5));
        assert_eq!(top.get_pixel(0, 0).0[0], 2);
        assert_eq!(cropped(&image, false, 0, 0).dimensions(), (4, 10));
        let wide = RgbaImage::new(10, 4);
        assert_eq!(cropped(&wide, true, 1, 2).dimensions(), (7, 4));
        assert_eq!(cropped(&wide, true, 5, 5).dimensions(), (1, 4));
    }

    #[test]
    fn preview_fits_the_cross_axis_and_thumbnail_stays_under_the_texture_limit() {
        let tall = RgbaImage::new(1080, 4_000);
        assert_eq!(preview_size(&tall, false), (420.0, 4_000.0 * 420.0 / 1080.0));
        let narrow = RgbaImage::new(200, 900);
        assert_eq!(preview_size(&narrow, false), (200.0, 900.0));
        let wide = RgbaImage::new(5000, 300);
        assert_eq!(preview_size(&wide, true), (5000.0, 300.0));
        // 100×20000：横向不用缩小，长边超过纹理上限时整体缩到 8192。
        let thumb = thumbnail(&RgbaImage::new(100, 20_000), false);
        let size = thumb.size(0);
        assert_eq!((size.width.0, size.height.0), (41, MAX_TEXTURE as i32));
    }
}
