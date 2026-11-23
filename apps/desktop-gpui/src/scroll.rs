//! 滚动截图。
//!
//! 采集、稳定检测、匹配、拼接、恢复与输出都在 core（`snap_core::scroll`），由采集线程驱动；
//! 全局监听（首次滚轮锁定方向、Esc 取消、Ctrl+C 完成）也在 core。这里负责：
//! 1. 截图浮层点“滚动截图”后，以冻结画面中的选区为首帧建立会话，收起浮层；
//! 2. 选区紫色边框（鼠标穿透）、侧边预览窗（拼接缩略图 + 当前画面位置）与下方控制条
//!    （状态、完成/保存已确认部分/重试、取消），三者置顶且不进入采集（Windows 用 WDA_EXCLUDEFROMCAPTURE）；
//! 3. 采集线程通过 [`GpuiHost`] 写入共享状态，控制条每 100 ms 读取并执行窗口操作；
//!    完成后全部窗口立即收起（结果已复制并保存），不再显示结果条。
//!
//! 结果 PNG 存在数据目录 `scroll-results`（不经导出暂存记录）；预览/控制条遮挡选区时
//! 只在 Windows 上由捕获排除保证不进入画面（macOS/Linux 未实现采样时临时隐藏）；结果不进主界面。
use gpui_kit::*;
use image::RgbaImage;
use snap_core::{
    capture::{Display, Rect},
    scroll::{self as core, ExportBackground, Host, Screen, ScrollState, ScrollView},
    settings::Language,
};
use std::{
    any::Any,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const POLL: Duration = Duration::from_millis(100);
const ACCENT: u32 = 0x7c3aed;
const BAR_WIDTH: f32 = 400.0;
const BAR_HEIGHT: f32 = 44.0;
const PREVIEW_WIDTH: f32 = 240.0;
const GAP: f32 = 8.0;

/// 采集线程写入、界面轮询读取的共享状态。
#[derive(Default)]
struct Shared {
    view: Option<ScrollView>,
    /// 请求隐藏全部窗口（取消、开始输出）。
    hidden: bool,
    /// 输出失败后请求重新显示。
    show: bool,
    /// 会话已结束（`close_session`）。
    closed: Option<u64>,
    /// 输出完成：保存路径（失败为 None）与是否已复制。
    result: Option<(Option<PathBuf>, bool)>,
    saved: Option<PathBuf>,
    copied: bool,
    error: Option<String>,
}

#[derive(Default)]
struct Scroller {
    state: Arc<ScrollState>,
    /// 与采集线程共用的操作互斥（GPUI 截图在界面线程同步完成，这里只防采集与输出交叠）。
    operation: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    host: Option<Arc<GpuiHost>>,
    border: Option<AnyWindowHandle>,
    preview: Option<AnyWindowHandle>,
    controls: Option<AnyWindowHandle>,
    next_id: u64,
}

impl Global for Scroller {}

/// 正在滚动截图（采集或输出中）。
pub fn active(cx: &App) -> bool {
    cx.try_global::<Scroller>().is_some_and(|s| {
        s.state.collecting.load(Ordering::SeqCst)
            || s.state
                .session
                .lock()
                .is_ok_and(|session| session.is_some())
    })
}

/// 截图/滚动截图快捷键或托盘在采集中触发：结束采集并输出。
pub fn stop(cx: &App) -> bool {
    cx.try_global::<Scroller>()
        .is_some_and(|s| core::stop(&s.state))
}

pub struct Request {
    /// 冻结的整张截图（物理像素）与其中的选区。
    pub image: Arc<RgbaImage>,
    pub rect: Rect,
    pub origin: (i32, i32),
    pub displays: Vec<Display>,
    pub language: Language,
}

/// 以截图选区为首帧开始滚动截图。成功时调用方应收起截图浮层（暂存架保持收起，结束后恢复）。
pub fn start(cx: &mut App, request: Request) -> Result<(), String> {
    if crate::recording::active(cx) || active(cx) {
        return Err("正在录屏或滚动截图，请先结束。".into());
    }
    let (display, local) = request
        .displays
        .iter()
        .find_map(|d| {
            snap_core::capture::within_display(request.rect, request.origin, d)
                .map(|l| (d.clone(), l))
        })
        .ok_or("请选择单个显示器内的区域；跨屏录制与滚动截图暂不支持。")?;
    let screen = core::screen_at(display.x, display.y)?;
    // 冻结后显示器分辨率改变时，选区与首帧已不对应。
    if (screen.width, screen.height) != (display.width, display.height) {
        return Err("显示器布局、尺寸或 DPI 发生变化，请重新截图。".into());
    }
    let seed = image::imageops::crop_imm(
        request.image.as_ref(),
        request.rect.x,
        request.rect.y,
        request.rect.width,
        request.rect.height,
    )
    .to_image();
    let scroller = cx.default_global::<Scroller>();
    scroller.next_id += 1;
    let id = scroller.next_id;
    let (state, operation) = (scroller.state.clone(), scroller.operation.clone());
    let shared = Arc::new(Mutex::new(Shared::default()));
    scroller.shared = shared.clone();
    let session = core::new_session(id, seed, local, screen, None, ExportBackground::White)?;
    let first = core::install(&state, session)?;
    shared.lock().map_err(|e| e.to_string())?.view = Some(first);
    let host = Arc::new(GpuiHost {
        state: state.clone(),
        operation,
        shared,
    });
    // 窗口位置用 GPUI 逻辑坐标（物理像素 ÷ 显示器缩放）。
    let region = crate::recording::logical_region(cx, &display, local);
    let screen_bounds = crate::recording::display_bounds(cx, &display);
    let border = open_border(cx, region);
    let preview = open_preview(
        cx,
        preview_bounds(screen_bounds, region),
        request.language.clone(),
    );
    let controls = open_controls(cx, bar_bounds(screen_bounds, region), id, request.language);
    {
        let scroller = cx.global_mut::<Scroller>();
        scroller.host = Some(host.clone());
        scroller.border = border;
        scroller.preview = preview;
        scroller.controls = controls;
    }
    if let Err(error) = core::launch(host, id, 120) {
        let _ = core::abort(&state);
        close_windows(cx, true);
        return Err(error);
    }
    log::info!(
        "event=gpui.scroll_started id={id} width={} height={}",
        local.width,
        local.height
    );
    Ok(())
}

/// 控制条放在选区下方；放不下时放在上方，再不行放在选区内底部。
fn bar_bounds(screen: Bounds<Pixels>, region: Bounds<Pixels>) -> Bounds<Pixels> {
    let (width, height, gap) = (px(BAR_WIDTH), px(BAR_HEIGHT), px(GAP));
    let below = region.origin.y + region.size.height + gap;
    let bottom = screen.origin.y + screen.size.height;
    let y = if below + height <= bottom {
        below
    } else if region.origin.y - gap - height >= screen.origin.y {
        region.origin.y - gap - height
    } else {
        (region.origin.y + region.size.height - height - gap).max(screen.origin.y)
    };
    let x = (region.origin.x + region.size.width - width)
        .max(screen.origin.x)
        .min(screen.origin.x + screen.size.width - width);
    Bounds::new(point(x, y), size(width, height))
}

/// 预览窗优先放在选区右侧，其次左侧；两侧都放不下时放在屏幕右边缘（遮挡选区的部分不进入采集）。
fn preview_bounds(screen: Bounds<Pixels>, region: Bounds<Pixels>) -> Bounds<Pixels> {
    let (width, gap) = (px(PREVIEW_WIDTH), px(GAP));
    let height = region
        .size
        .height
        .max(px(220.0))
        .min(px(560.0))
        .min(screen.size.height);
    let right = region.origin.x + region.size.width + gap;
    let screen_right = screen.origin.x + screen.size.width;
    let x = if right + width <= screen_right {
        right
    } else if region.origin.x - gap - width >= screen.origin.x {
        region.origin.x - gap - width
    } else {
        screen_right - width
    };
    let y = region
        .origin
        .y
        .max(screen.origin.y)
        .min(screen.origin.y + screen.size.height - height);
    Bounds::new(point(x, y), size(width, height))
}

/// 选区外侧的紫色边框：鼠标穿透（滚轮交给下面的页面），不进入采集。
struct Border;

impl Render for Border {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().border_2().border_color(rgb(ACCENT))
    }
}

fn open_border(cx: &mut App, region: Bounds<Pixels>) -> Option<AnyWindowHandle> {
    let inset = px(2.0);
    let bounds = Bounds::new(
        point(region.origin.x - inset, region.origin.y - inset),
        size(
            region.size.width + inset * 2.0,
            region.size.height + inset * 2.0,
        ),
    );
    match cx.open_window(crate::recording::popup(bounds), |window, cx| {
        if let Err(error) = crate::native::set_click_through(window, true) {
            log::warn!("event=gpui.scroll_border_click_through_failed message={error}");
        }
        crate::recording::protect(window);
        cx.new(|_| Border)
    }) {
        Ok(handle) => Some(handle.into()),
        Err(error) => {
            log::warn!("event=gpui.scroll_border_failed message={error}");
            None
        }
    }
}

fn open_preview(
    cx: &mut App,
    bounds: Bounds<Pixels>,
    language: Language,
) -> Option<AnyWindowHandle> {
    let result = cx.open_window(crate::recording::popup(bounds), |window, cx| {
        crate::recording::protect(window);
        cx.new(|cx| {
            cx.spawn(
                async move |this: WeakEntity<Preview>, cx: &mut AsyncApp| loop {
                    cx.background_executor().timer(POLL).await;
                    if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                        break;
                    }
                },
            )
            .detach();
            Preview {
                language,
                view: None,
                image: None,
            }
        })
    });
    match result {
        Ok(handle) => Some(handle.into()),
        Err(error) => {
            log::warn!("event=gpui.scroll_preview_failed message={error}");
            None
        }
    }
}

fn open_controls(
    cx: &mut App,
    bounds: Bounds<Pixels>,
    id: u64,
    language: Language,
) -> Option<AnyWindowHandle> {
    let result = cx.open_window(crate::recording::popup(bounds), |window, cx| {
        crate::recording::protect(window);
        cx.new(|cx| {
            cx.spawn(
                async move |this: WeakEntity<Controls>, cx: &mut AsyncApp| loop {
                    cx.background_executor().timer(POLL).await;
                    if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                        break;
                    }
                },
            )
            .detach();
            Controls {
                id,
                language,
                view: None,
                busy: None,
                hidden: false,
                waiting: 0,
                notice: None,
            }
        })
    });
    match result {
        Ok(handle) => Some(handle.into()),
        Err(error) => {
            log::error!("event=gpui.scroll_controls_failed message={error}");
            None
        }
    }
}

/// 关闭边框与预览；`all` 时连控制条一起关闭并恢复暂存架。
fn close_windows(cx: &mut App, all: bool) {
    let Some(scroller) = cx.try_global::<Scroller>() else {
        return;
    };
    let mut handles: Vec<AnyWindowHandle> = [scroller.border, scroller.preview]
        .into_iter()
        .flatten()
        .collect();
    if all {
        handles.extend(scroller.controls);
    }
    let scroller = cx.global_mut::<Scroller>();
    scroller.border = None;
    scroller.preview = None;
    if all {
        scroller.controls = None;
        scroller.host = None;
    }
    cx.defer(move |cx| {
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    });
    if all {
        crate::shelf::resume(cx);
    }
}

/// 隐藏或重新显示全部滚动截图窗口（不销毁，输出失败后还要用）。
fn set_visible(cx: &mut App, visible: bool) {
    let Some(scroller) = cx.try_global::<Scroller>() else {
        return;
    };
    let handles: Vec<AnyWindowHandle> = [scroller.border, scroller.preview, scroller.controls]
        .into_iter()
        .flatten()
        .collect();
    cx.defer(move |cx| {
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| {
                if let Err(error) = crate::native::set_visible_async(window, visible) {
                    log::warn!("event=gpui.scroll_visibility_failed message={error}");
                }
            });
        }
    });
}

fn t(language: &Language, key: &'static str) -> String {
    snap_core::i18n::for_language(language, key).to_owned()
}

/// PNG 预览解码为 GPUI 图像（BGRA）。
fn decode_preview(png: &[u8]) -> Option<Arc<RenderImage>> {
    let mut frame = image::load_from_memory(png).ok()?.to_rgba8();
    for pixel in frame.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(frame)])))
}

/// 侧边预览：拼接缩略图与当前采集画面位置。
struct Preview {
    language: Language,
    view: Option<ScrollView>,
    image: Option<Arc<RenderImage>>,
}

impl Preview {
    fn poll(&mut self, cx: &mut Context<Self>) {
        let Some(next) = cx
            .try_global::<Scroller>()
            .and_then(|s| s.shared.lock().ok().and_then(|shared| shared.view.clone()))
        else {
            return;
        };
        if self
            .view
            .as_ref()
            .is_some_and(|v| v.id == next.id && v.revision >= next.revision)
        {
            return;
        }
        let changed = self.view.as_ref().is_none_or(|v| v.preview != next.preview);
        if changed && !next.preview.is_empty() {
            if let Some(old) = self.image.take() {
                cx.drop_image(old, None);
            }
            self.image = decode_preview(&next.preview);
        }
        self.view = Some(next);
        cx.notify();
    }
}

/// 视口在画布中的位置（0～1 比例，裁到画布内）；画布坐标可以为负，先减原点。
fn marker(view: &ScrollView) -> (f32, f32, f32, f32) {
    let (b, v) = (view.canvas_bounds, view.viewport_rect);
    if b.width == 0 || b.height == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (bw, bh) = (f64::from(b.width), f64::from(b.height));
    let left = ((v.x - b.x) as f64).clamp(0.0, bw);
    let top = ((v.y - b.y) as f64).clamp(0.0, bh);
    let right = ((v.x + i64::from(v.width) - b.x) as f64).clamp(left, bw);
    let bottom = ((v.y + i64::from(v.height) - b.y) as f64).clamp(top, bh);
    (
        (left / bw) as f32,
        (top / bh) as f32,
        ((right - left) / bw) as f32,
        ((bottom - top) / bh) as f32,
    )
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = |key| t(&self.language, key);
        let bounds = window.bounds().size;
        // 画布区域：窗口减去内边距、标题与底部两行文字。
        let (area_w, area_h) = (
            f32::from(bounds.width) - 12.0,
            f32::from(bounds.height) - 76.0,
        );
        let mut canvas = div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(px(3.0))
            .bg(rgb(0xe4e4e7));
        match (self.view.as_ref(), self.image.clone()) {
            (Some(view), Some(image)) if view.width > 0 && view.height > 0 => {
                let k = (area_w / view.width as f32)
                    .min(area_h / view.height as f32)
                    .max(0.0);
                let (w, h) = (view.width as f32 * k, view.height as f32 * k);
                let (mx, my, mw, mh) = marker(view);
                canvas = canvas.child(
                    div()
                        .relative()
                        .w(px(w))
                        .h(px(h))
                        .child(
                            img(image)
                                .absolute()
                                .size_full()
                                .object_fit(ObjectFit::Fill),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(mx * w))
                                .top(px(my * h))
                                .w(px(mw * w))
                                .h(px(mh * h))
                                .border_2()
                                .border_color(rgb(0x24834b))
                                .bg(rgba(0x84cc1610)),
                        ),
                );
            }
            _ => {
                canvas = canvas
                    .text_xs()
                    .text_color(rgb(0x71717a))
                    .child(text("准备采集…"))
            }
        }
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text("滚动截图")),
            )
            .children(self.view.as_ref().map(|v| {
                div()
                    .text_color(rgb(0x71717a))
                    .child(format!("{} × {}", v.width, v.height))
            }));
        let frames = self
            .view
            .as_ref()
            .map(|v| text("已拼接 {0} 帧").replace("{0}", &v.frames.to_string()));
        let hint = self
            .view
            .as_ref()
            .map(|v| snap_core::i18n::for_language(&self.language, &v.message).to_owned())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| text("慢慢滚动，停一下再继续。"));
        let recovering = self.view.as_ref().is_some_and(|v| v.status == "recovering");
        let failed = self.view.as_ref().is_some_and(|v| v.status == "failed");
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(6.0))
            .rounded(px(6.0))
            .bg(rgb(0xfafafa))
            .border_1()
            .border_color(rgb(0xd4d4d8))
            .text_color(rgb(0x27272a))
            .child(header)
            .child(canvas)
            .children(frames.map(|f| div().text_xs().text_color(rgb(0x57606a)).child(f)))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if failed {
                        0xb91c1c
                    } else if recovering {
                        0x92400e
                    } else {
                        0x71717a
                    }))
                    .child(hint),
            )
    }
}

/// 控制条：状态、完成/保存已确认部分/重试、取消。
struct Controls {
    id: u64,
    language: Language,
    view: Option<ScrollView>,
    /// 已点击：Some(true) 为取消中，Some(false) 为保存中。
    busy: Option<bool>,
    hidden: bool,
    /// 会话已关闭但完成通知尚未到达的等待次数。
    waiting: u32,
    notice: Option<String>,
}

impl Controls {
    fn t(&self, key: &'static str) -> String {
        t(&self.language, key)
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let Some(scroller) = cx.try_global::<Scroller>() else {
            return;
        };
        let cancelled = scroller.state.cancel.load(Ordering::SeqCst);
        let (view, hide, show, closed, result, error) = {
            let Ok(mut shared) = scroller.shared.lock() else {
                return;
            };
            (
                shared.view.clone(),
                std::mem::take(&mut shared.hidden),
                std::mem::take(&mut shared.show),
                shared.closed,
                shared.result.take(),
                shared.error.take(),
            )
        };
        if let Some(next) = view.filter(|v| v.id == self.id) {
            // 新的失败版本表示后台已结束本轮输出：恢复重试/取消入口。
            if next.status == "failed"
                && self
                    .view
                    .as_ref()
                    .is_none_or(|v| v.revision < next.revision)
            {
                self.busy = None;
            }
            self.view = Some(next);
        }
        if let Some(error) = error {
            self.notice = Some(error);
        }
        if show {
            self.hidden = false;
            set_visible(cx, true);
        } else if hide && !self.hidden {
            self.hidden = true;
            set_visible(cx, false);
        }
        if closed == Some(self.id) {
            match result {
                // 完成即全部收起：结果已复制并保存在 scroll-results（通用页可打开），不再留结果条。
                // 结果条被排除在采集之外，留在屏幕上时紧接着截图会在它的位置出现一块黑色。
                Some((path, copied)) => {
                    log::info!(
                        "event=gpui.scroll_finished id={} saved={} copied={copied}",
                        self.id,
                        path.is_some()
                    );
                    close_windows(cx, true);
                }
                // 取消，或完成通知迟迟不来（不应发生）：直接收起。
                None if cancelled || self.waiting >= 20 => close_windows(cx, true),
                None => self.waiting += 1,
            }
        }
        cx.notify();
    }

    fn act(&mut self, cancel: bool, cx: &mut Context<Self>) {
        let Some(view) = self.view.as_ref() else {
            return;
        };
        if self.busy.is_some() && (!cancel || self.busy == Some(true)) {
            return;
        }
        let Some(scroller) = cx.try_global::<Scroller>() else {
            return;
        };
        self.busy = Some(cancel);
        self.notice = None;
        let result = if cancel {
            match scroller.host.clone() {
                Some(host) if core::cancel(host.as_ref(), view.id, "window") => Ok(()),
                _ => Err("滚动截图已结束。".to_owned()),
            }
        } else {
            let recovering = view.collecting && view.status == "recovering";
            core::finish(&scroller.state, view.id, Some(recovering))
        };
        if let Err(error) = result {
            self.busy = None;
            self.notice = Some(error);
        }
        cx.notify();
    }

    fn status(&self) -> String {
        let key = match (self.busy, self.view.as_ref()) {
            (Some(true), _) => "正在取消…",
            (Some(false), _) => "正在保存…",
            (None, None) => "准备中…",
            (None, Some(v)) => match v.status {
                "failed" => "输出失败",
                "finishing" => "正在保存…",
                "sampling" => "请停一下",
                "no_progress" => "暂无新增",
                "waiting" => "请稍候",
                "recovering" => "请往回滚",
                _ if !v.collecting => "采集结束",
                _ => "自动拼接",
            },
        };
        self.t(key)
    }

    fn button(
        &self,
        id: &'static str,
        label: String,
        primary: bool,
        enabled: bool,
    ) -> Stateful<Div> {
        let button = div()
            .id(id)
            .h(px(30.0))
            .px(px(10.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0))
            .rounded(px(5.0))
            .text_xs()
            .text_color(rgb(if primary { 0xffffff } else { 0x27272a }))
            .bg(rgb(if primary { ACCENT } else { 0xf4f4f5 }))
            .children(primary.then(|| crate::annotate::icon(crate::annotate::icons::SAVE, 0xffffff).size(px(15.0))))
            .child(label);
        if enabled {
            button.cursor_pointer().hover(|s| s.opacity(0.85))
        } else {
            button.opacity(0.5)
        }
    }
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(7.0))
            .px(px(8.0))
            .py(px(5.0))
            .bg(rgb(0xfafafa))
            .border_1()
            .border_color(rgb(0xd4d4d8))
            .text_color(rgb(0x27272a))
            // Esc 取消且不保存。
            .on_key_down(cx.listener(|v, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    v.act(true, cx);
                }
            }));
        let view = self.view.as_ref();
        let recovering = view.is_some_and(|v| v.collecting && v.status == "recovering");
        let failed = view.is_some_and(|v| v.status == "failed");
        let stopped = self.busy.is_some() || !view.is_some_and(|v| v.collecting);
        let detail = self.notice.clone().or_else(|| {
            view.map(|v| {
                let message = snap_core::i18n::for_language(&self.language, &v.message);
                if message.is_empty() {
                    format!(
                        "{} · {} px",
                        self.t("{0} 帧").replace("{0}", &v.frames.to_string()),
                        v.height
                    )
                } else {
                    message.to_owned()
                }
            })
        });
        let can_finish = self.busy.is_none() && view.is_some_and(|v| v.status != "finishing");
        let can_cancel = view.is_some() && self.busy != Some(true);
        let finish_label = self.t(if failed {
            "重试"
        } else if recovering {
            "保存已确认部分"
        } else {
            "完成"
        });
        base.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .text_xs()
                        .whitespace_nowrap()
                        .child(div().size(px(6.0)).rounded_full().bg(rgb(if stopped {
                            0xa1a1aa
                        } else {
                            ACCENT
                        })))
                        .child(self.status()),
                )
                .children(detail.map(|d| {
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(px(10.0))
                        .text_color(rgb(if self.notice.is_some() {
                            0xb91c1c
                        } else {
                            0x71717a
                        }))
                        .child(d)
                })),
        )
        // 禁用状态下 act 自身也会拒绝（忙碌、已在保存），点击回调始终绑定。
        .child(
            self.button("scroll-finish", finish_label, true, can_finish)
                .px(px(9.0))
                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.act(false, cx))),
        )
        .child({
            // 取消：只有图标，恢复或失败时带文字；悬停为浅红。
            let labelled = recovering || failed;
            let mut cancel = div()
                .id("scroll-cancel")
                .h(px(30.0))
                .px(px(5.0))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.0))
                .rounded(px(5.0))
                .text_xs()
                .text_color(rgb(0x27272a))
                .child(crate::annotate::icon(crate::annotate::icons::CLOSE, 0x27272a).size(px(17.0)))
                .children(labelled.then(|| self.t("取消")))
                .tooltip({
                    let tip = self.t("取消（Esc，不保存）");
                    move |_, cx| cx.new(|_| crate::annotate::Tooltip(tip.clone().into())).into()
                });
            cancel = if can_cancel {
                cancel.cursor_pointer().hover(|s| s.bg(rgb(0xfee2e2)).text_color(rgb(0xb91c1c)))
            } else {
                cancel.opacity(0.5)
            };
            cancel.on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.act(true, cx)))
        })
    }
}

/// GPUI 外壳为采集线程提供的能力：只写共享状态，窗口操作由控制条轮询执行。
struct GpuiHost {
    state: Arc<ScrollState>,
    operation: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
}

impl GpuiHost {
    fn with(&self, f: impl FnOnce(&mut Shared)) {
        if let Ok(mut shared) = self.shared.lock() {
            f(&mut shared);
        }
    }
}

impl Host for GpuiHost {
    fn state(&self) -> &ScrollState {
        &self.state
    }
    fn operation(&self) -> &AtomicBool {
        &self.operation
    }
    fn screen(&self, x: i32, y: i32) -> Result<Screen, String> {
        core::screen_at(x, y)
    }
    fn hide_for_sample(&self, _id: u64, _windows: &[String]) -> Result<Box<dyn Any>, String> {
        // 辅助窗口已排除在采集之外（Windows），无需临时隐藏。
        Ok(Box::new(()))
    }
    fn updated(&self, view: &ScrollView) {
        let view = view.clone();
        self.with(|s| s.view = Some(view));
    }
    fn hide_windows(&self) {
        self.with(|s| s.hidden = true);
    }
    fn show_output(&self) -> Result<(), String> {
        self.with(|s| s.show = true);
        Ok(())
    }
    fn report_error(&self, message: String) {
        log::error!("event=gpui.scroll_error message={message}");
        self.with(|s| s.error = Some(message));
    }
    fn close_session(&self, id: u64) -> Result<(), String> {
        {
            let mut session = self.state.session.lock().map_err(|e| e.to_string())?;
            if session.as_ref().is_some_and(|s| s.id == id) {
                *session = None;
            }
        }
        self.state.collecting.store(false, Ordering::SeqCst);
        self.with(|s| s.closed = Some(id));
        Ok(())
    }
    fn results_dir(&self) -> Result<PathBuf, String> {
        Ok(snap_core::paths::local_data_dir()
            .ok_or("无法定位数据目录。")?
            .join("scroll-results"))
    }
    fn publish(&self, root: &Path, png: &[u8]) -> Result<PathBuf, String> {
        let path = core::write_png(root, png)?;
        let saved = path.clone();
        self.with(|s| s.saved = Some(saved));
        Ok(path)
    }
    fn copy(&self, image: &RgbaImage, png: &[u8]) -> Result<(), String> {
        let result = copy_image(image, png);
        let copied = result.is_ok();
        self.with(|s| s.copied = copied);
        result
    }
    fn finished(&self, _result: &ScrollView, _message: String) {
        self.with(|s| s.result = Some((s.saved.take(), s.copied)));
    }
}

/// 采集线程直接写剪贴板（同截图复制：PNG + 保留 alpha 的 DIBV5）。
#[cfg(windows)]
fn copy_image(image: &RgbaImage, png: &[u8]) -> Result<(), String> {
    snap_core::clipboard::windows_image::copy_png(0, image, png)
}

#[cfg(not(windows))]
fn copy_image(_image: &RgbaImage, _png: &[u8]) -> Result<(), String> {
    Err("当前平台的滚动截图暂不支持直接复制，PNG 已保存。".into())
}

#[cfg(test)]
mod tests {
    use super::{bar_bounds, marker, preview_bounds, BAR_HEIGHT, GAP, PREVIEW_WIDTH};
    use gpui_kit::{point, px, size, Bounds};
    use snap_core::scroll::{Bounds as Canvas, ScrollView};
    use std::sync::Arc;

    #[test]
    fn preview_goes_right_then_left_then_screen_edge() {
        let screen = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let region = |x: f32, w: f32| Bounds::new(point(px(x), px(100.0)), size(px(w), px(400.0)));
        assert_eq!(
            preview_bounds(screen, region(100.0, 600.0)).origin.x,
            px(708.0)
        );
        assert_eq!(
            preview_bounds(screen, region(1500.0, 400.0)).origin.x,
            px(1500.0 - GAP - PREVIEW_WIDTH)
        );
        assert_eq!(
            preview_bounds(screen, region(0.0, 1920.0)).origin.x,
            px(1920.0 - PREVIEW_WIDTH)
        );
        let bar = bar_bounds(screen, region(100.0, 600.0));
        assert_eq!((bar.origin.y, bar.size.height), (px(508.0), px(BAR_HEIGHT)));
    }

    #[test]
    fn marker_is_clipped_to_the_canvas() {
        let view = ScrollView {
            id: 1,
            revision: 1,
            collecting: true,
            archive_path: None,
            width: 100,
            height: 400,
            frames: 3,
            preview: Arc::from(Vec::new()),
            frame_height: 100,
            canvas_bounds: Canvas {
                x: 0,
                y: -100,
                width: 100,
                height: 400,
            },
            viewport_rect: Canvas {
                x: 0,
                y: 200,
                width: 100,
                height: 200,
            },
            coverage_percent: 100.0,
            has_holes: false,
            direction: "vertical",
            status: "collecting",
            message: String::new(),
        };
        // 视口下半部分超出画布，裁到底边。
        assert_eq!(marker(&view), (0.0, 0.75, 1.0, 0.25));
    }
}
