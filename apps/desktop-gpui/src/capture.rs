//! 截图浮层：冻结桌面 → 框选（拖动新建、移动、八向缩放、窗口/控件吸附）→ 复制或保存。
//!
//! 流程与所有权：
//! 1. `start` 在后台线程调用 `snap_core::capture::capture` 拍下整个虚拟桌面（Windows 为所有显示器拼接，
//!    物理像素），窗口身份等非 Send 数据留在后台线程丢弃，只把图片、显示器与吸附候选带回主线程。
//! 2. 每个显示器开一个置顶 PopUp 浮层窗口，背景是该屏的冻结画面（`RenderImage`，BGRA，直接交给 GPU，
//!    不经文件或自定义协议）。所有浮层共享一个 [`Session`] 实体，选区存于虚拟桌面物理像素坐标，
//!    任一浮层修改后其他浮层通过 `observe` 重绘。
//! 3. 复制/保存/贴图时在后台线程用 `snap_core::capture::compose` 裁剪合成，再写剪贴板或文件；
//!    结束或取消时关闭全部浮层并丢弃会话，冻结画面随之释放。
//!
//! 坐标：浮层事件位置是窗口内逻辑像素；乘以该屏 `Display::scale` 再加上该屏在截图中的偏移，
//! 即得到截图物理像素。限制：新建选区只在按下时所在的屏幕内（跨屏选区留待后续）；
//! 取色、倒计时界面尚未接入。贴图窗口见 `pins.rs`。
//!
//! 标注（`annotate.rs`）：选区下方工具栏选择工具后，选区内的按下交给编辑器；标注坐标与截图同为
//! 物理像素，导出时随选区一起交给 `compose`。已有标注时选区不能再移动或缩放（避免标注被裁掉）。
use crate::annotate::{self, Action, Editor, Job, Layers, Tool, ToolbarProps};
use crate::selection::{self, Handle};
use gpui_kit::{
    component::input::{Textarea, TextareaState},
    *,
};
use image::RgbaImage;
use snap_core::{
    annotations::{Annotation, Point as ImagePoint, edit::Bounds as ImageBounds},
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
    /// 设置“框选完成后的动作”：none / copy / save / pin。
    after: String,
    language: Language,
    windows: Vec<AnyWindowHandle>,
    /// 复制或保存进行中，忽略重复触发。
    busy: bool,
    editor: Editor,
    layers: Layers,
    /// 选择了标注工具：选区内的按下交给编辑器；否则为调整选区模式。
    annotating: bool,
    /// 文字输入框所在的浮层与输入状态。
    text_input: Option<(usize, Entity<TextareaState>)>,
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

    /// 编辑范围跟随选区，然后刷新预览图层。每次修改选区或编辑器后调用。
    fn sync(&mut self) {
        let crop = self.selection.unwrap_or(self.area());
        self.editor.crop = ImageBounds {
            x: f64::from(crop.x),
            y: f64::from(crop.y),
            width: f64::from(crop.width),
            height: f64::from(crop.height),
        };
        self.layers.update(&self.editor);
    }

    /// 已有标注或正在编辑时锁定选区。
    fn selection_locked(&self) -> bool {
        !self.editor.shapes().is_empty() || self.editor.interacting()
    }
}

/// 在后台线程栅格化文字/序号/滤镜，完成后写回编辑器。
fn run_job(cx: &mut App, session: Entity<Session>, job: Job) {
    let image = session.read(cx).image.clone();
    cx.spawn(async move |cx| {
        let result = cx
            .background_executor()
            .spawn({
                let job = job.clone();
                async move { job.run(&image, annotate::user_font()) }
            })
            .await;
        if let Err(error) = &result {
            log::warn!("event=gpui.annotation_raster_failed message={error}");
        }
        cx.update(|cx| {
            session.update(cx, |s, cx| {
                s.editor.complete(job, result);
                s.sync();
                cx.notify();
            });
            flush(&session, None, cx);
        });
    })
    .detach();
}

/// 释放被替换的预览图层纹理。在某个窗口的事件处理中调用时必须传入该窗口（它此时不在应用的窗口表里）。
fn flush(session: &Entity<Session>, window: Option<&mut Window>, cx: &mut App) {
    let retired = session.update(cx, |s, _| s.layers.take_retired());
    match window {
        Some(window) => {
            for image in retired {
                cx.drop_image(image, Some(&mut *window));
            }
        }
        None => {
            for image in retired {
                cx.drop_image(image, None);
            }
        }
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
                        editor: Editor::new(ImageBounds { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }, Vec::new()),
                        layers: Layers::default(),
                        annotating: false,
                        text_input: None,
                    });
                    session.update(cx, |s, _| s.sync());
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
        let matched = match_display(&platform_displays, logical);
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

/// 按逻辑边界（物理像素 ÷ 该屏缩放）找到对应的 GPUI 显示器。
fn match_display(
    displays: &[std::rc::Rc<dyn PlatformDisplay>],
    logical: Bounds<Pixels>,
) -> Option<&std::rc::Rc<dyn PlatformDisplay>> {
    let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() <= 2.0;
    displays.iter().find(|candidate| {
        let bounds = candidate.bounds();
        near(bounds.origin.x, logical.origin.x)
            && near(bounds.origin.y, logical.origin.y)
            && near(bounds.size.width, logical.size.width)
            && near(bounds.size.height, logical.size.height)
    })
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

/// 取出待导出的选区、源图与标注并标记忙碌；没有选区、已在导出、文字输入或栅格化未完成时返回 None。
fn begin_export(cx: &mut App) -> Option<(Entity<Session>, Arc<RgbaImage>, Rect, Vec<Annotation>)> {
    let session = current(cx)?;
    let (image, rect, shapes) = {
        let s = session.read(cx);
        if s.busy || s.editor.pending || s.editor.text.is_some() {
            return None;
        }
        (s.image.clone(), s.selection?, s.editor.shapes().to_vec())
    };
    session.update(cx, |s, _| {
        // 未完成的笔画不导出。
        s.editor.abort();
        s.sync();
        s.busy = true;
    });
    Some((session, image, rect, shapes))
}

pub(crate) fn png(image: &RgbaImage) -> Result<Vec<u8>, String> {
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
    let Some((session, image, rect, shapes)) = begin_export(cx) else {
        return;
    };
    cx.spawn(async move |cx| {
        let encoded = cx
            .background_executor()
            .spawn(async move {
                let image = snap_core::capture::compose(&image, rect, &shapes)?;
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
pub(crate) fn write_clipboard(_cx: &App, owner: isize, image: &RgbaImage, png: Vec<u8>) -> Result<(), String> {
    // 与 Tauri 版一致：同时写 PNG 与保留 alpha 的 DIBV5，兼容只认位图的老程序。
    snap_core::clipboard::windows_image::copy_png(owner, image, &png)
}

#[cfg(not(windows))]
pub(crate) fn write_clipboard(cx: &App, _owner: isize, _image: &RgbaImage, png: Vec<u8>) -> Result<(), String> {
    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, png)));
    Ok(())
}

/// 把选区贴到屏幕上：图片首像素对准选区原位置，然后结束截图。
/// 截图中按全局贴图键、工具栏“贴图”或“截图后动作 = 贴图”都走这里。
pub fn pin(cx: &mut App) {
    let Some((session, image, rect, shapes)) = begin_export(cx) else {
        return;
    };
    // 贴图保留可编辑的标注：底图不合成，标注坐标换算到贴图图像（减去选区原点）。
    let (ox, oy) = (f64::from(rect.x), f64::from(rect.y));
    let shapes: Vec<Annotation> = shapes
        .iter()
        .map(|s| {
            snap_core::annotations::edit::map_shape(s, |p| ImagePoint {
                x: p.x - ox,
                y: p.y - oy,
            })
        })
        .collect();
    let (origin, scale, logical) = {
        let s = session.read(cx);
        let global = (s.origin.0 + rect.x as i32, s.origin.1 + rect.y as i32);
        // 选区所在屏：包含选区左上角的那块（新建选区不跨屏）。
        let display = s
            .displays
            .iter()
            .find(|d| {
                global.0 >= d.x
                    && global.1 >= d.y
                    && global.0 < d.x + d.width as i32
                    && global.1 < d.y + d.height as i32
            })
            .or(s.displays.first())
            .cloned();
        let scale = display.as_ref().map(|d| d.scale).unwrap_or(1.0);
        let logical = display.map(|d| {
            let k = d.scale as f32;
            Bounds::new(
                point(px(d.x as f32 / k), px(d.y as f32 / k)),
                size(px(d.width as f32 / k), px(d.height as f32 / k)),
            )
        });
        (global, scale, logical)
    };
    let displays = cx.displays();
    let display = logical.and_then(|logical| match_display(&displays, logical).map(|d| d.id()));
    cx.spawn(async move |cx| {
        let composed = cx
            .background_executor()
            .spawn(async move { snap_core::capture::compose(&image, rect, &[]) })
            .await;
        cx.update(|cx| {
            let result = composed.and_then(|image| {
                crate::pins::create(cx, image, shapes, crate::pins::Placement::Source { origin, scale, display })
            });
            match result {
                Ok(()) => {
                    log::info!("event=gpui.capture_pinned width={} height={}", rect.width, rect.height);
                    finish(cx);
                }
                Err(error) => {
                    log::error!("event=gpui.capture_pin_failed message={error}");
                    session.update(cx, |s, _| s.busy = false);
                }
            }
        });
    })
    .detach();
}

/// 保存选区。置顶浮层会挡住系统保存对话框，因此先关闭浮层但保留会话；取消或失败时重新打开浮层，
/// 选区保持不变。文件名、格式、目录记忆与 Tauri 版共用 `snap_core::export`。
fn save(cx: &mut App) {
    let Some((session, image, rect, shapes)) = begin_export(cx) else {
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
            .spawn(async move { snap_core::capture::compose(&image, rect, &shapes) })
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
        // 输入框属于已关闭的浮层窗口，随之丢弃；旧窗口的纹理已随窗口释放，图层需重建。
        s.text_input = None;
        s.layers.clear();
        let _ = s.layers.take_retired();
        s.sync();
    });
    open_overlays(cx, session);
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
        let p = self.to_point(position, cx);
        (p.x.round() as i64, p.y.round() as i64)
    }

    /// 窗口内逻辑坐标 → 截图物理像素（浮点，供标注使用）。
    fn to_point(&self, position: Point<Pixels>, cx: &App) -> ImagePoint {
        let s = self.session.read(cx);
        let region = s.region(self.index);
        let scale = s.displays[self.index].scale;
        ImagePoint {
            x: f64::from(region.x) + f64::from(f32::from(position.x)) * scale,
            y: f64::from(region.y) + f64::from(f32::from(position.y)) * scale,
        }
    }

    /// 编辑器状态变化后：释放旧纹理，按需创建或移除文字输入框。
    fn after_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        flush(&self.session, Some(window), cx);
        let (wanted, has, initial) = {
            let s = self.session.read(cx);
            (
                s.editor.text.as_ref().map(|t| t.shape.text.clone().unwrap_or_default()),
                s.text_input.is_some(),
                (),
            )
        };
        let _ = initial;
        match (wanted, has) {
            (Some(value), false) => {
                let state = cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx);
                    state.set_auto_grow(1, 20, cx);
                    state.set_value(value, window, cx);
                    state
                });
                state.update(cx, |state, cx| state.focus(window, cx));
                let index = self.index;
                self.session.update(cx, |s, _| s.text_input = Some((index, state)));
            }
            (None, true) => {
                self.session.update(cx, |s, _| s.text_input = None);
                window.focus(&self.focus, cx);
            }
            _ => {}
        }
    }

    fn run(&self, job: Option<Job>, cx: &mut App) {
        if let Some(job) = job {
            run_job(cx, self.session.clone(), job);
        }
    }

    /// 完成文字输入：读取输入框内容交给编辑器栅格化。
    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .session
            .read(cx)
            .text_input
            .as_ref()
            .map(|(_, state)| state.read(cx).value().to_string())
            .unwrap_or_default();
        let job = self.session.update(cx, |s, cx| {
            let job = s.editor.finish_text(&value);
            s.sync();
            cx.notify();
            job
        });
        self.run(job, cx);
        self.after_edit(window, cx);
    }

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.read(cx).editor.text.is_some() {
            // 点击输入框外完成文字。
            self.commit_text(window, cx);
            return;
        }
        let p = self.to_image(event.position, cx);
        let point = self.to_point(event.position, cx);
        let tolerance = (HANDLE_HIT * self.scale(cx)).round() as i64;
        let index = self.index;
        let alt = event.modifiers.alt;
        let copy_now = self.session.update(cx, |s, cx| {
            let inside = s.selection.is_some_and(|r| selection::contains(r, p));
            let t = tolerance as f64;
            // 标注：选中图形的手柄 → 缩放；标注模式下选区内 → 编辑器（绘制或选中移动）。
            if s.annotating && inside {
                if event.click_count >= 2 {
                    if let Some(i) = s.editor.hit(point, t).filter(|&i| {
                        s.editor.shapes()[i].kind == snap_core::annotations::Kind::Text
                    }) {
                        s.editor.edit_text(i);
                        s.sync();
                        cx.notify();
                        return false;
                    }
                }
                let handle = s.editor.handle_at(point, t);
                s.editor.begin(point, t, handle, alt);
                s.sync();
                cx.notify();
                return false;
            }
            if let Some(handle) = s.editor.handle_at(point, t) {
                s.editor.begin(point, t, Some(handle), false);
                s.sync();
                cx.notify();
                return false;
            }
            if event.click_count >= 2 && inside {
                return true;
            }
            if s.selection_locked() {
                // 已有标注：选区固定，单击只取消选中。
                s.editor.select(None);
                cx.notify();
                return false;
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
        self.after_edit(window, cx);
        if copy_now {
            copy(cx, crate::native::hwnd(window));
        }
    }

    fn motion(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.to_image(event.position, cx);
        let point = self.to_point(event.position, cx);
        let edited = self.session.update(cx, |s, cx| {
            let pressed = event.pressed_button == Some(MouseButton::Left);
            if pressed && s.drag.is_none() && s.editor.draft.is_some() {
                s.editor.drag(point);
                s.layers.update(&s.editor);
                cx.notify();
                return true;
            }
            match (pressed, s.drag) {
                (true, Some(Drag::Create { anchor, moved, area })) => {
                    let far = (p.0 - anchor.0).abs().max((p.1 - anchor.1).abs()) >= CLICK_SLOP;
                    if !moved && !far {
                        return false;
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
                        return false;
                    }
                    s.hover = hover;
                }
                _ => return false,
            }
            s.sync();
            cx.notify();
            false
        });
        if edited {
            flush(&self.session, Some(window), cx);
        }
    }

    fn release(&mut self, _event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (after, job) = self.session.update(cx, |s, cx| {
            if s.drag.is_none() && s.editor.draft.is_some() {
                let job = s.editor.finish(false);
                s.sync();
                cx.notify();
                return (None, job);
            }
            let drag = s.drag.take();
            // 单击（没有拖动）选中鼠标下的窗口或控件。
            if let Some(Drag::Create { moved: false, .. }) = drag {
                s.selection = s.hover;
            }
            s.sync();
            cx.notify();
            let after = matches!(drag, Some(Drag::Create { .. }))
                .then(|| s.selection.map(|_| s.after.clone()))
                .flatten();
            (after, None)
        });
        self.run(job, cx);
        self.after_edit(window, cx);
        match after.as_deref() {
            Some("copy") => copy(cx, crate::native::hwnd(window)),
            Some("save") => save(cx),
            Some("pin") => pin(cx),
            _ => {}
        }
    }

    /// 右键：放弃未完成的笔画 → 退出标注模式 → 清除选区重新框选 → 退出截图。
    fn right_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let cleared = self.session.update(cx, |s, cx| {
            cx.notify();
            if s.editor.text.is_some() {
                s.editor.cancel_text();
            } else if s.editor.interacting() {
                s.editor.abort();
            } else if s.annotating {
                s.annotating = false;
                s.editor.set_tool(Tool::Edit);
            } else {
                let had = s.selection.is_some() || s.drag.is_some();
                s.selection = None;
                s.drag = None;
                s.editor.reset(Vec::new());
                s.sync();
                return had;
            }
            s.sync();
            true
        });
        self.after_edit(window, cx);
        if !cleared {
            cancel(cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;
        let key = keystroke.key.as_str();
        if self.session.read(cx).editor.text.is_some() {
            // 输入中只处理取消与完成，其余按键交给输入框。
            match key {
                "escape" => {
                    self.session.update(cx, |s, cx| {
                        s.editor.cancel_text();
                        s.sync();
                        cx.notify();
                    });
                    self.after_edit(window, cx);
                    cx.stop_propagation();
                }
                "enter" if command => {
                    self.commit_text(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        let mut job = None;
        let mut handled = true;
        self.session.update(cx, |s, cx| {
            let e = &mut s.editor;
            match key {
                "z" if command && keystroke.modifiers.shift => e.redo(),
                "z" if command => e.undo(),
                "y" if command => e.redo(),
                "delete" | "backspace" => e.remove(),
                "escape" if e.interacting() => e.abort(),
                "enter" if e.tool == Tool::Polyline && e.draft.is_some() => job = e.finish(true),
                "left" | "right" | "up" | "down" if e.selected.is_some() => {
                    let step = if keystroke.modifiers.shift { 10.0 } else { 1.0 };
                    let (dx, dy) = match key {
                        "left" => (-step, 0.0),
                        "right" => (step, 0.0),
                        "up" => (0.0, -step),
                        _ => (0.0, step),
                    };
                    job = e.nudge(dx, dy).1;
                }
                _ => handled = false,
            }
            if handled {
                s.sync();
                cx.notify();
            }
        });
        self.run(job, cx);
        self.after_edit(window, cx);
        if handled {
            return;
        }
        match key {
            "escape" => cancel(cx),
            "enter" => copy(cx, crate::native::hwnd(window)),
            "c" if command => copy(cx, crate::native::hwnd(window)),
            "s" if command => save(cx),
            // 方向键微调选区 1 像素，Shift 为 10 像素（已有标注时选区固定）。
            "left" | "right" | "up" | "down" => {
                let step = if keystroke.modifiers.shift { 10 } else { 1 };
                let (dx, dy) = match key {
                    "left" => (-step, 0),
                    "right" => (step, 0),
                    "up" => (0, -step),
                    _ => (0, step),
                };
                self.session.update(cx, |s, cx| {
                    if s.selection_locked() {
                        return;
                    }
                    if let Some(rect) = s.selection {
                        s.selection = Some(selection::moved(rect, dx, dy, s.area()));
                        s.sync();
                        cx.notify();
                    }
                });
            }
            _ => {}
        }
    }

    /// 工具栏与样式条的操作。
    fn action(this: &mut Self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let job = this.session.update(cx, |s, cx| {
            let e = &mut s.editor;
            let job = match action {
                Action::Tool(tool) => {
                    s.annotating = true;
                    e.set_tool(tool);
                    None
                }
                Action::Undo => {
                    e.undo();
                    None
                }
                Action::Redo => {
                    e.redo();
                    None
                }
                Action::Delete => {
                    e.remove();
                    None
                }
                Action::Clear => {
                    e.clear();
                    None
                }
                Action::Style(patch) => e.style(patch),
            };
            s.sync();
            cx.notify();
            job
        });
        this.run(job, cx);
        this.after_edit(window, cx);
    }

    /// 回到调整选区模式（不再把选区内的按下交给编辑器）。
    fn select_mode(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            if s.editor.interacting() || s.editor.pending {
                return;
            }
            s.annotating = false;
            s.editor.set_tool(Tool::Edit);
            s.sync();
            cx.notify();
        });
        self.after_edit(window, cx);
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

/// 导出动作按钮（取消、贴图、保存、复制），在工具行末尾。
fn export_button(
    id: &'static str,
    icon: &'static [u8],
    label: String,
    primary: bool,
    action: fn(&mut App, isize),
) -> AnyElement {
    let mut button = annotate::icon_button(id, icon, label, false, false);
    if primary {
        button = button.bg(rgb(0x2563eb));
    }
    button
        .on_click(move |_, window, cx| action(cx, crate::native::hwnd(window)))
        .into_any_element()
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus, cx);
            self.focused = true;
        }
        let viewport = window.viewport_size();
        let view = cx.entity();
        let session = self.session.clone();
        let s = session.read(cx);
        let region = s.region(self.index);
        let scale = s.displays[self.index].scale as f32;
        let (selection, hover, dragging) = (s.selection, s.hover, s.drag.is_some());
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

        let Some(rect) = selection.filter(|_| visible.is_some()) else {
            return root;
        };
        let b = local(rect, region, scale);
        // 标注预览：底层与草稿图层都在截图物理像素坐标中，按本屏缩放换算后叠在冻结画面上。
        for layer in [s.layers.base(), s.layers.draft()].into_iter().flatten() {
            let lb = local(
                Rect { x: layer.x.max(0) as u32, y: layer.y.max(0) as u32, width: layer.width, height: layer.height },
                region,
                scale,
            );
            root = root.child(
                img(layer.image.clone())
                    .absolute()
                    .left(lb.origin.x)
                    .top(lb.origin.y)
                    .w(lb.size.width)
                    .h(lb.size.height)
                    .object_fit(ObjectFit::Fill),
            );
        }
        let map = |p: ImagePoint| {
            point(
                px((p.x as f32 - region.x as f32) / scale),
                px((p.y as f32 - region.y as f32) / scale),
            )
        };
        root = root.children(annotate::selection_overlay(&s.editor, &map));
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
        if !s.selection_locked() {
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
        }
        // 文字输入框：放在文字起点，宽度到选区右边缘，与最终排版的换行宽度一致。
        if let (Some(text), Some((owner, state))) = (s.editor.text.as_ref(), s.text_input.as_ref()) {
            if *owner == self.index {
                let at = map(text.shape.start);
                let crop = s.editor.crop;
                let width = ((crop.x + crop.width - text.shape.start.x).min(2048.0) as f32 / scale).max(40.0);
                let font = text.shape.font_size.unwrap_or(24.0) as f32 / scale;
                let color = u32::from_str_radix(text.shape.color.trim_start_matches('#'), 16).unwrap_or(0xef4444);
                root = root.child(
                    div()
                        .absolute()
                        .left(at.x)
                        .top(at.y)
                        .w(px(width))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(ACCENT))
                        .bg(rgba(0xffffff18))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Textarea::new(state)
                                .appearance(false)
                                .text_size(px(font))
                                .text_color(rgb(color)),
                        ),
                );
            }
        }
        // 工具栏只画在选区右下角所在的屏幕上；下方放不下时放到选区上方，再不行贴着底边放在内侧。
        let corner = (i64::from(rect.x + rect.width) - 1, i64::from(rect.y + rect.height) - 1);
        if dragging || !selection::contains(region, corner) {
            return root;
        }
        let text = |key: &'static str| s.text(key);
        let props = ToolbarProps { editor: &s.editor, active: s.annotating, disabled: s.busy, text: &text };
        let host = annotate::Host { view: &view, window, dispatch: Self::action };
        let leading = vec![
            annotate::icon_button("capture-select", annotate::icons::MOVE, s.text("调整选区"), !s.annotating, false)
                .on_click(window.listener_for(&view, Self::select_mode))
                .into_any_element(),
            annotate::divider().into_any_element(),
        ];
        let trailing = vec![
            export_button("capture-cancel", annotate::icons::CLOSE, s.text("取消"), false, |cx, _| cancel(cx)),
            export_button("capture-pin", annotate::icons::PIN, s.text("贴图"), false, |cx, _| pin(cx)),
            export_button("capture-save", annotate::icons::SAVE, s.text("保存"), false, |cx, _| save(cx)),
            export_button("capture-copy", annotate::icons::COPY, s.text("复制"), true, copy),
        ];
        let tools = annotate::tool_row(&props, leading, trailing, &host);
        let options = annotate::options_row(&props, &host);
        let error = s.editor.error.clone();
        let height = annotate::ROW_HEIGHT
            + options.as_ref().map_or(0.0, |_| annotate::OPTIONS_HEIGHT + 6.0)
            + error.as_ref().map_or(0.0, |_| 30.0);
        let right = b.origin.x + b.size.width;
        let bottom = b.origin.y + b.size.height;
        let top = if bottom + px(height + 8.0) <= viewport.height {
            bottom + px(6.0)
        } else if b.origin.y - px(height + 8.0) >= px(0.0) {
            b.origin.y - px(height + 6.0)
        } else {
            (bottom - px(height + 6.0)).max(px(4.0))
        };
        // 右对齐到选区右边；选区太靠左时改为从屏幕左边起排，避免工具栏被挤出屏幕。
        const MIN_WIDTH: f32 = 900.0;
        let mut toolbar = div()
            .id("capture-toolbar")
            .absolute()
            .top(top)
            .flex()
            .flex_col()
            .items_end()
            .gap(px(6.0))
            .cursor_default()
            // 工具栏上的按下不能冒泡到浮层，否则会被当作新建选区或绘制。
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(tools)
            .children(options)
            .children(error.map(annotate::error_row));
        toolbar = if right >= px(MIN_WIDTH) {
            toolbar.right((viewport.width - right).max(px(4.0)))
        } else {
            toolbar.left(px(4.0)).items_start()
        };
        root.child(toolbar)
    }
}
