//! 截图浮层：冻结桌面 → 框选（拖动新建、移动、八向缩放、窗口/控件吸附）→ 复制或保存。
//!
//! 流程与所有权：
//! 1. `start` 在后台线程调用 `snap_core::capture::capture` 拍下整个虚拟桌面（Windows 为所有显示器拼接，
//!    物理像素），窗口身份等非 Send 数据留在后台线程丢弃，只把图片、显示器与吸附候选带回主线程。
//! 2. 每个显示器开一个置顶 PopUp 浮层窗口，背景是该屏的冻结画面（`RenderImage`，BGRA，直接交给 GPU，
//!    不经文件或自定义协议）。所有浮层共享一个 [`Session`] 实体，选区存于虚拟桌面物理像素坐标，
//!    任一浮层修改后其他浮层通过 `observe` 重绘。
//! 3. 复制/保存时在后台线程用 `snap_core::capture::compose` 裁剪合成，再写剪贴板或文件；
//!    结束或取消时关闭全部浮层并丢弃会话，冻结画面随之释放。
//!
//! 坐标：浮层事件位置是窗口内逻辑像素；乘以该屏 `Display::scale` 再加上该屏在截图中的偏移，
//! 即得到截图物理像素。限制：新建选区只在按下时所在的屏幕内（跨屏选区留待后续）；标注、贴图、
//! 取色、倒计时界面尚未接入。
use crate::selection::{self, Handle};
use gpui_kit::*;
use image::RgbaImage;
use snap_core::{
    capture::{Display, Rect},
    settings::{ExportSettings, Language, Settings},
};
use std::{sync::Arc, time::{Duration, Instant}};

const ACCENT: u32 = 0x2f80ff;
/// 选区外的遮罩：半透明黑，保留冻结画面可辨认。
const MASK: u32 = 0x0000_0088;
/// 手柄命中半径与绘制边长（逻辑像素）。
const HANDLE_HIT: f32 = 6.0;
const HANDLE_SIZE: f32 = 7.0;
/// 新建选区前允许的抖动（物理像素）：小于它视为单击，用于选中吸附候选。
const CLICK_SLOP: i64 = 4;

/// 拖动中的操作；起点都记录在按下时，移动时总是从起点重新计算，避免累计误差。
#[derive(Clone, Copy)]
enum Drag {
    Create { anchor: selection::Point, moved: bool, area: Rect },
    Move { anchor: selection::Point, start: Rect },
    Resize { anchor: selection::Point, start: Rect, handle: Handle },
}

/// 一次截图会话；由所有浮层共享。
pub struct Session {
    /// 整个虚拟桌面的冻结画面（RGBA，物理像素）；合成时只读克隆 Arc。
    image: Arc<RgbaImage>,
    /// 截图左上角的全局物理坐标；`Display::x - origin.0` 即该屏在截图中的偏移。
    origin: (i32, i32),
    displays: Vec<Display>,
    /// 可吸附的窗口与控件（截图物理像素），顺序为系统前后层次。
    targets: Vec<Rect>,
    selection: Option<Rect>,
    hover: Option<Rect>,
    drag: Option<Drag>,
    export: ExportSettings,
    /// 设置“框选完成后的动作”：none / copy / save / pin（贴图未接入，按 none 处理）。
    after: String,
    language: Language,
    windows: Vec<AnyWindowHandle>,
    /// 复制或保存进行中，忽略重复触发。
    busy: bool,
}

impl Session {
    fn area(&self) -> Rect {
        Rect { x: 0, y: 0, width: self.image.width(), height: self.image.height() }
    }

    /// 某屏在截图中的区域（物理像素）。
    fn region(&self, index: usize) -> Rect {
        let d = &self.displays[index];
        Rect {
            x: (d.x - self.origin.0).max(0) as u32,
            y: (d.y - self.origin.1).max(0) as u32,
            width: d.width,
            height: d.height,
        }
    }

    fn text(&self, message: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, message).to_owned()
    }
}

#[derive(Default)]
struct CaptureState {
    session: Option<Entity<Session>>,
    /// 后台正在拍屏：此时再次触发截图直接忽略。
    starting: bool,
}
impl Global for CaptureState {}

/// 在后台线程得到的结果；只含 Send 数据。
struct Grabbed {
    image: RgbaImage,
    origin: (i32, i32),
    displays: Vec<Display>,
    targets: Vec<Rect>,
}

fn grab(include_cursor: bool, snap: bool) -> Result<Grabbed, String> {
    let desktop = snap_core::capture::capture(false, include_cursor, snap)?;
    let targets = desktop
        .targets
        .iter()
        .filter(|target| !target.recording_only)
        .map(|target| target.rect)
        .collect();
    Ok(Grabbed {
        image: desktop.image,
        origin: (desktop.x, desktop.y),
        displays: desktop.displays,
        targets,
    })
}

/// 是否已有截图会话（准备中或浮层已显示）。
pub fn active(cx: &App) -> bool {
    cx.try_global::<CaptureState>()
        .is_some_and(|state| state.starting || state.session.is_some())
}

/// 开始截图。`delay` 用于等托盘菜单等短暂界面消失；设置中的截图延时也在此等待（倒计时界面未接入）。
pub fn start(cx: &mut App, settings: &Settings, delay: Duration) {
    if active(cx) {
        log::info!("event=gpui.capture_ignored reason=active");
        return;
    }
    cx.default_global::<CaptureState>().starting = true;
    let include_cursor = settings.screenshot_cursor;
    let snap = settings.screenshot_snap;
    let export = settings.export.clone();
    let after = settings.screenshot_after.clone();
    let language = settings.appearance.language.clone();
    let delay = delay + Duration::from_secs(u64::from(settings.delay_seconds));
    cx.spawn(async move |cx| {
        if !delay.is_zero() {
            cx.background_executor().timer(delay).await;
        }
        let started = Instant::now();
        let result = cx
            .background_executor()
            .spawn(async move { grab(include_cursor, snap) })
            .await;
        let capture_ms = started.elapsed().as_millis();
        cx.update(|cx| {
            cx.global_mut::<CaptureState>().starting = false;
            match result {
                Ok(grabbed) => {
                    log::info!(
                        "event=gpui.capture_ready width={} height={} displays={} targets={} capture_ms={capture_ms}",
                        grabbed.image.width(),
                        grabbed.image.height(),
                        grabbed.displays.len(),
                        grabbed.targets.len()
                    );
                    let session = cx.new(|_| Session {
                        image: Arc::new(grabbed.image),
                        origin: grabbed.origin,
                        displays: grabbed.displays,
                        targets: grabbed.targets,
                        selection: None,
                        hover: None,
                        drag: None,
                        export,
                        after,
                        language,
                        windows: Vec::new(),
                        busy: false,
                    });
                    cx.global_mut::<CaptureState>().session = Some(session.clone());
                    open_overlays(cx, &session);
                    log::info!(
                        "event=gpui.overlay_shown total_ms={}",
                        started.elapsed().as_millis()
                    );
                }
                Err(error) => log::error!("event=gpui.capture_failed message={error}"),
            }
        });
    })
    .detach();
}

/// 截图里某屏的画面转为 GPUI 需要的 BGRA 帧。
fn display_frame(image: &RgbaImage, region: Rect) -> Arc<RenderImage> {
    let mut frame =
        image::imageops::crop_imm(image, region.x, region.y, region.width, region.height).to_image();
    for pixel in frame.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(frame)]))
}

/// 每屏一个浮层。GPUI 的显示器边界是“物理像素 ÷ 该屏缩放”的逻辑坐标，据此匹配截图里的显示器；
/// 匹配不到时按同样规则自行换算，不指定显示器。
fn open_overlays(cx: &mut App, session: &Entity<Session>) {
    let (displays, regions, image) = {
        let s = session.read(cx);
        let regions: Vec<_> = (0..s.displays.len()).map(|i| s.region(i)).collect();
        (s.displays.clone(), regions, s.image.clone())
    };
    let platform_displays = cx.displays();
    let mut handles: Vec<AnyWindowHandle> = Vec::new();
    for (index, display) in displays.iter().enumerate() {
        let scale = display.scale as f32;
        let logical = Bounds::new(
            point(px(display.x as f32 / scale), px(display.y as f32 / scale)),
            size(px(display.width as f32 / scale), px(display.height as f32 / scale)),
        );
        let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() <= 2.0;
        let matched = platform_displays.iter().find(|candidate| {
            let bounds = candidate.bounds();
            near(bounds.origin.x, logical.origin.x)
                && near(bounds.origin.y, logical.origin.y)
                && near(bounds.size.width, logical.size.width)
                && near(bounds.size.height, logical.size.height)
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(
                matched.map(|d| d.bounds()).unwrap_or(logical),
            )),
            display_id: matched.map(|d| d.id()),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // 与原型一致：透明背景的 PopUp 窗口走 DirectComposition 合成，冻结画面铺满后与不透明无异。
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        let frame = display_frame(&image, regions[index]);
        let shared = session.clone();
        match cx.open_window(options, |_, cx| {
            cx.new(|cx| Overlay::new(shared, index, frame, cx))
        }) {
            Ok(handle) => handles.push(handle.into()),
            Err(error) => log::error!("event=gpui.overlay_failed display={index} message={error}"),
        }
    }
    // focus: true 只在本进程已是前台时生效；从命令行转交、或保存对话框关闭后系统把焦点还给了
    // 原来的应用，浮层就收不到 Esc / 回车 / Ctrl+S。主动激活一个浮层（所有浮层共享会话，激活哪个都行）。
    if let Some(first) = handles.first() {
        let _ = first.update(cx, |_, window, _| window.activate_window());
    }
    session.update(cx, |s, _| s.windows = handles);
}

/// 关闭全部浮层。延后到当前事件处理结束再执行：正在分发事件的窗口此时不能被 update。
fn close_overlays(cx: &mut App, session: &Entity<Session>) {
    let handles = session.update(cx, |s, _| std::mem::take(&mut s.windows));
    cx.defer(move |cx| {
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    });
}

/// 结束会话：关闭浮层并释放冻结画面。
fn finish(cx: &mut App) {
    let Some(session) = cx.global_mut::<CaptureState>().session.take() else {
        return;
    };
    close_overlays(cx, &session);
}

pub fn cancel(cx: &mut App) {
    if cx.try_global::<CaptureState>().is_some_and(|s| s.session.is_some()) {
        log::info!("event=gpui.capture_cancelled");
        finish(cx);
    }
}

fn current(cx: &App) -> Option<Entity<Session>> {
    cx.try_global::<CaptureState>()?.session.clone()
}

/// 取出待导出的选区与源图并标记忙碌；没有选区或已在导出时返回 None。
fn begin_export(cx: &mut App) -> Option<(Entity<Session>, Arc<RgbaImage>, Rect)> {
    let session = current(cx)?;
    let (image, rect) = {
        let s = session.read(cx);
        if s.busy {
            return None;
        }
        (s.image.clone(), s.selection?)
    };
    session.update(cx, |s, _| s.busy = true);
    Some((session, image, rect))
}

fn png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    use image::{codecs::png, ImageEncoder};
    let mut bytes = Vec::new();
    // 剪贴板用快速压缩：体积略大但编码快，粘贴目标不在乎压缩率。
    png::PngEncoder::new_with_quality(&mut bytes, png::CompressionType::Fast, png::FilterType::Adaptive)
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("图片编码失败：{e}"))?;
    Ok(bytes)
}

/// 复制选区到剪贴板。`owner` 是 Windows 剪贴板所有者窗口（当前浮层），复制完成后才关闭浮层。
fn copy(cx: &mut App, owner: isize) {
    let Some((session, image, rect)) = begin_export(cx) else {
        return;
    };
    cx.spawn(async move |cx| {
        let encoded = cx
            .background_executor()
            .spawn(async move {
                let image = snap_core::capture::compose(&image, rect, &[])?;
                let png = png(&image)?;
                Ok::<_, String>((image, png))
            })
            .await;
        cx.update(|cx| {
            let result = encoded.and_then(|(image, png)| {
                write_clipboard(cx, owner, &image, png).map(|_| image.dimensions())
            });
            match result {
                Ok((width, height)) => {
                    log::info!("event=gpui.capture_copied width={width} height={height}");
                    finish(cx);
                }
                Err(error) => {
                    log::error!("event=gpui.capture_copy_failed message={error}");
                    session.update(cx, |s, _| s.busy = false);
                }
            }
        });
    })
    .detach();
}

#[cfg(windows)]
fn write_clipboard(_cx: &App, owner: isize, image: &RgbaImage, png: Vec<u8>) -> Result<(), String> {
    // 与 Tauri 版一致：同时写 PNG 与保留 alpha 的 DIBV5，兼容只认位图的老程序。
    snap_core::clipboard::windows_image::copy_png(owner, image, &png)
}

#[cfg(not(windows))]
fn write_clipboard(cx: &App, _owner: isize, _image: &RgbaImage, png: Vec<u8>) -> Result<(), String> {
    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, png)));
    Ok(())
}

/// 保存选区。置顶浮层会挡住系统保存对话框，因此先关闭浮层但保留会话；取消或失败时重新打开浮层，
/// 选区保持不变。文件名、格式、目录记忆与 Tauri 版共用 `snap_core::export`。
fn save(cx: &mut App) {
    let Some((session, image, rect)) = begin_export(cx) else {
        return;
    };
    let (export, title, label) = {
        let s = session.read(cx);
        let (label, _) = s.export.format.filter();
        (s.export.clone(), s.text("保存图片"), s.text(label))
    };
    close_overlays(cx, &session);
    cx.spawn(async move |cx| {
        let composed = cx
            .background_executor()
            .spawn(async move { snap_core::capture::compose(&image, rect, &[]) })
            .await;
        let image = match composed {
            Ok(image) => image,
            Err(error) => {
                log::error!("event=gpui.capture_save_failed message={error}");
                cx.update(|cx| reopen(cx, &session));
                return;
            }
        };
        let name = match snap_core::export::filename(
            &export,
            &snap_core::export::timestamp(),
            image.width(),
            image.height(),
        ) {
            Ok(name) => name,
            Err(error) => {
                log::error!("event=gpui.capture_save_failed message={error}");
                cx.update(|cx| reopen(cx, &session));
                return;
            }
        };
        let record = snap_core::export::directory_record();
        let (_, extensions) = export.format.filter();
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title(title)
            .add_filter(label, extensions)
            .set_file_name(name);
        if let Some(directory) = record
            .as_deref()
            .and_then(|path| snap_core::export::preferred_directory(path, export.remember_directory))
        {
            dialog = dialog.set_directory(directory);
        }
        let Some(file) = dialog.save_file().await else {
            cx.update(|cx| reopen(cx, &session));
            return;
        };
        let path = file.path().to_path_buf();
        let (width, height) = image.dimensions();
        let written = cx
            .background_executor()
            .spawn({
                let export = export.clone();
                let path = path.clone();
                async move { snap_core::export::write_file(&path, &image, &export) }
            })
            .await;
        cx.update(|cx| match written {
            Ok(()) => {
                if export.remember_directory {
                    if let (Some(record), Some(parent)) = (record, path.parent()) {
                        // 图片已保存；目录记忆失败只记录，不把保存报告为失败。
                        if snap_core::export::remember_directory(&record, parent).is_err() {
                            log::warn!("event=gpui.directory_memory_failed");
                        }
                    }
                }
                log::info!("event=gpui.capture_saved width={width} height={height}");
                finish(cx);
            }
            Err(error) => {
                log::error!("event=gpui.capture_save_failed message={error}");
                reopen(cx, &session);
            }
        });
    })
    .detach();
}

/// 保存取消或失败后重新显示浮层，沿用原选区。
fn reopen(cx: &mut App, session: &Entity<Session>) {
    let still_current = current(cx).is_some_and(|active| active == *session);
    if !still_current {
        return;
    }
    session.update(cx, |s, _| {
        s.busy = false;
        s.drag = None;
    });
    open_overlays(cx, session);
}

/// Windows 剪贴板所有者：当前浮层的 HWND。其他平台不需要。
fn owner(window: &Window) -> isize {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window) {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                return handle.hwnd.get();
            }
        }
    }
    let _ = window;
    0
}

/// 单个显示器上的浮层视图。
struct Overlay {
    session: Entity<Session>,
    index: usize,
    frame: Arc<RenderImage>,
    focus: FocusHandle,
    focused: bool,
    _observe: Subscription,
}

impl Overlay {
    fn new(session: Entity<Session>, index: usize, frame: Arc<RenderImage>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        Self { session, index, frame, focus: cx.focus_handle(), focused: false, _observe: observe }
    }

    fn scale(&self, cx: &App) -> f32 {
        self.session.read(cx).displays[self.index].scale as f32
    }

    /// 窗口内逻辑坐标 → 截图物理像素。
    fn to_image(&self, position: Point<Pixels>, cx: &App) -> selection::Point {
        let s = self.session.read(cx);
        let region = s.region(self.index);
        let scale = s.displays[self.index].scale;
        (
            i64::from(region.x) + (f64::from(f32::from(position.x)) * scale).round() as i64,
            i64::from(region.y) + (f64::from(f32::from(position.y)) * scale).round() as i64,
        )
    }

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.to_image(event.position, cx);
        let tolerance = (HANDLE_HIT * self.scale(cx)).round() as i64;
        let index = self.index;
        let copy_now = self.session.update(cx, |s, cx| {
            if event.click_count >= 2 && s.selection.is_some_and(|r| selection::contains(r, p)) {
                return true;
            }
            let area = s.region(index);
            s.drag = Some(match s.selection {
                Some(rect) => match selection::handle_at(rect, p, tolerance) {
                    Some(handle) => Drag::Resize { anchor: p, start: rect, handle },
                    None if selection::contains(rect, p) => Drag::Move { anchor: p, start: rect },
                    None => Drag::Create { anchor: p, moved: false, area },
                },
                None => Drag::Create { anchor: p, moved: false, area },
            });
            cx.notify();
            false
        });
        if copy_now {
            copy(cx, owner(window));
        }
    }

    fn motion(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let p = self.to_image(event.position, cx);
        self.session.update(cx, |s, cx| {
            let pressed = event.pressed_button == Some(MouseButton::Left);
            match (pressed, s.drag) {
                (true, Some(Drag::Create { anchor, moved, area })) => {
                    let far = (p.0 - anchor.0).abs().max((p.1 - anchor.1).abs()) >= CLICK_SLOP;
                    if !moved && !far {
                        return;
                    }
                    s.drag = Some(Drag::Create { anchor, moved: true, area });
                    s.selection = selection::from_points(anchor, p, area);
                    s.hover = None;
                }
                (true, Some(Drag::Move { anchor, start })) => {
                    s.selection = Some(selection::moved(start, p.0 - anchor.0, p.1 - anchor.1, s.area()));
                }
                (true, Some(Drag::Resize { anchor, start, handle })) => {
                    if let Some(rect) = selection::resized(start, handle, p.0 - anchor.0, p.1 - anchor.1, s.area()) {
                        s.selection = Some(rect);
                    }
                }
                _ if s.selection.is_none() => {
                    let hover = selection::snap_target(&s.targets, p);
                    if hover == s.hover {
                        return;
                    }
                    s.hover = hover;
                }
                _ => return,
            }
            cx.notify();
        });
    }

    fn release(&mut self, _event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let after = self.session.update(cx, |s, cx| {
            let drag = s.drag.take();
            // 单击（没有拖动）选中鼠标下的窗口或控件。
            if let Some(Drag::Create { moved: false, .. }) = drag {
                s.selection = s.hover;
            }
            cx.notify();
            matches!(drag, Some(Drag::Create { .. }))
                .then(|| s.selection.map(|_| s.after.clone()))
                .flatten()
        });
        match after.as_deref() {
            Some("copy") => copy(cx, owner(window)),
            Some("save") => save(cx),
            _ => {}
        }
    }

    /// 右键：有选区时清除选区重新框选，否则退出截图。
    fn right_click(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let cleared = self.session.update(cx, |s, cx| {
            let had = s.selection.is_some() || s.drag.is_some();
            s.selection = None;
            s.drag = None;
            cx.notify();
            had
        });
        if !cleared {
            cancel(cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;
        match keystroke.key.as_str() {
            "escape" => cancel(cx),
            "enter" => copy(cx, owner(window)),
            "c" if command => copy(cx, owner(window)),
            "s" if command => save(cx),
            // 方向键微调选区 1 像素，Shift 为 10 像素。
            key @ ("left" | "right" | "up" | "down") => {
                let step = if keystroke.modifiers.shift { 10 } else { 1 };
                let (dx, dy) = match key {
                    "left" => (-step, 0),
                    "right" => (step, 0),
                    "up" => (0, -step),
                    _ => (0, step),
                };
                self.session.update(cx, |s, cx| {
                    if let Some(rect) = s.selection {
                        s.selection = Some(selection::moved(rect, dx, dy, s.area()));
                        cx.notify();
                    }
                });
            }
            _ => {}
        }
    }

    fn button(&self, id: &'static str, label: String, cx: &mut Context<Self>, action: fn(&mut App, isize)) -> Stateful<Div> {
        div()
            .id(id)
            .px_3()
            .py_1()
            .rounded_sm()
            .cursor_pointer()
            .text_sm()
            .text_color(rgb(0xf4f4f5))
            .hover(|style| style.bg(rgb(0x3f3f46)))
            .child(label)
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| action(cx, owner(window))))
    }
}

/// 截图物理像素矩形 → 本屏窗口内逻辑像素边界。
fn local(rect: Rect, region: Rect, scale: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(
            px((rect.x as f32 - region.x as f32) / scale),
            px((rect.y as f32 - region.y as f32) / scale),
        ),
        size(px(rect.width as f32 / scale), px(rect.height as f32 / scale)),
    )
}

fn shade(bounds: Bounds<Pixels>) -> Div {
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .bg(rgba(MASK))
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus, cx);
            self.focused = true;
        }
        let viewport = window.viewport_size();
        let (region, scale, selection, hover, dragging, labels) = {
            let s = self.session.read(cx);
            (
                s.region(self.index),
                s.displays[self.index].scale as f32,
                s.selection,
                s.hover,
                s.drag.is_some(),
                [s.text("复制"), s.text("保存"), s.text("取消")],
            )
        };
        let mut root = div()
            .id("capture-overlay")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .cursor_crosshair()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_move(cx.listener(Self::motion))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::release))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_click))
            .child(img(self.frame.clone()).absolute().size_full().object_fit(ObjectFit::Fill));

        let full = Bounds::new(point(px(0.0), px(0.0)), viewport);
        let visible = selection.and_then(|rect| selection::intersect(rect, region));
        match visible.map(|rect| local(rect, region, scale)) {
            None => root = root.child(shade(full)),
            Some(b) => {
                let (left, top) = (b.origin.x, b.origin.y);
                let (right, bottom) = (b.origin.x + b.size.width, b.origin.y + b.size.height);
                root = root
                    .child(shade(Bounds::new(point(px(0.0), px(0.0)), size(viewport.width, top))))
                    .child(shade(Bounds::new(point(px(0.0), bottom), size(viewport.width, viewport.height - bottom))))
                    .child(shade(Bounds::new(point(px(0.0), top), size(left, b.size.height))))
                    .child(shade(Bounds::new(point(right, top), size(viewport.width - right, b.size.height))));
            }
        }

        if selection.is_none() {
            if let Some(rect) = hover.and_then(|rect| selection::intersect(rect, region)) {
                let b = local(rect, region, scale);
                root = root.child(
                    div()
                        .absolute()
                        .left(b.origin.x)
                        .top(b.origin.y)
                        .w(b.size.width)
                        .h(b.size.height)
                        .border_2()
                        .border_color(rgb(ACCENT)),
                );
            }
        }

        if let (Some(rect), Some(_)) = (selection, visible) {
            let b = local(rect, region, scale);
            root = root.child(
                div()
                    .absolute()
                    .left(b.origin.x)
                    .top(b.origin.y)
                    .w(b.size.width)
                    .h(b.size.height)
                    .border_1()
                    .border_color(rgb(ACCENT)),
            );
            // 尺寸标签显示物理像素，与导出图片一致；顶部放不下时放进选区内。
            let label_top = if b.origin.y > px(24.0) { b.origin.y - px(24.0) } else { b.origin.y + px(4.0) };
            root = root.child(
                div()
                    .absolute()
                    .left(b.origin.x)
                    .top(label_top)
                    .px_1()
                    .rounded_sm()
                    .bg(rgba(0x000000b0))
                    .text_xs()
                    .text_color(rgb(0xffffff))
                    .child(format!("{} × {}", rect.width, rect.height)),
            );
            for handle in Handle::ALL {
                let (x, y) = selection::handle_point(rect, handle);
                let center = point(
                    px((x as f32 - region.x as f32) / scale),
                    px((y as f32 - region.y as f32) / scale),
                );
                root = root.child(
                    div()
                        .absolute()
                        .left(center.x - px(HANDLE_SIZE / 2.0))
                        .top(center.y - px(HANDLE_SIZE / 2.0))
                        .size(px(HANDLE_SIZE))
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(ACCENT)),
                );
            }
            // 工具栏只画在选区右下角所在的屏幕上；下方放不下时贴着选区底边放在内侧。
            let corner = (
                i64::from(rect.x + rect.width) - 1,
                i64::from(rect.y + rect.height) - 1,
            );
            if !dragging && selection::contains(region, corner) {
                const WIDTH: f32 = 200.0;
                const HEIGHT: f32 = 34.0;
                let right = b.origin.x + b.size.width;
                let bottom = b.origin.y + b.size.height;
                let left = (right - px(WIDTH)).max(px(4.0)).min(viewport.width - px(WIDTH + 4.0));
                let top = if bottom + px(HEIGHT + 8.0) <= viewport.height {
                    bottom + px(6.0)
                } else {
                    (bottom - px(HEIGHT + 6.0)).max(px(4.0))
                };
                let [copy_label, save_label, cancel_label] = labels;
                root = root.child(
                    div()
                        .id("capture-toolbar")
                        .absolute()
                        .left(left)
                        .top(top)
                        .h(px(HEIGHT))
                        .px_1()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .rounded_md()
                        .bg(rgb(0x18181b))
                        .shadow_lg()
                        .cursor_default()
                        // 工具栏上的按下不能冒泡到浮层，否则会被当作新建选区。
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .child(self.button("capture-cancel", cancel_label, cx, |cx, _| cancel(cx)))
                        .child(self.button("capture-save", save_label, cx, |cx, _| save(cx)))
                        .child(self.button("capture-copy", copy_label, cx, copy)),
                );
            }
        }
        root
    }
}
