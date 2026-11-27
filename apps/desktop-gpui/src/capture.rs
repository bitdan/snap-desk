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
//! 即得到截图物理像素。选区可跨屏（坐标是整张虚拟桌面截图），各浮层只画与本屏相交的部分；
//! 取色放大镜见 `magnifier.rs`。贴图窗口见 `pins.rs`。
//!
//! 标注（`annotate.rs`）：选区下方工具栏选择工具后，选区内的按下交给编辑器；标注坐标与截图同为
//! 物理像素，导出时随选区一起交给 `compose`。已有标注时选区不能再移动或缩放（避免标注被裁掉）。
use crate::annotate::{self, Action, Editor, Job, Layers, Tool, ToolbarProps};
use crate::magnifier::{self, Picker};
use crate::recognition::Mode as RecognitionMode;
use crate::selection::{self, Handle, Snap, SnapKind};
use gpui_kit::{
    component::{
        Sizable,
        input::{Input, InputState, Textarea, TextareaState},
    },
    *,
};
use image::RgbaImage;
use snap_core::{
    annotations::{Annotation, Point as ImagePoint, edit::Bounds as ImageBounds},
    capture::{Display, Rect, recent::{self, RecentSelection}, sizing::{self, Size as Dimensions, Unit}},
    settings::{ExportSettings, Language, Settings},
};
use std::{sync::Arc, time::{Duration, Instant}};

#[path = "capture_history.rs"]
mod history;

/// 选区边框与吸附框（#60a5fa）。
const ACCENT: u32 = 0x60a5fa;
/// 手柄边框。
const HANDLE_BORDER: u32 = 0x3b82f6;
/// 选区外的遮罩：半透明黑，保留冻结画面可辨认。
const MASK: u32 = 0x0000_0066;
/// 手柄命中半径与绘制边长（逻辑像素）。
const HANDLE_HIT: f32 = 6.0;
const HANDLE_SIZE: f32 = 8.0;
/// 新建选区前允许的抖动（物理像素）：小于它视为单击，用于选中吸附候选。
const CLICK_SLOP: i64 = 4;
/// 位置与尺寸条（逻辑像素）：最小宽度；固定三行（输入、比例、预设）的高度：
/// 边框 1 + 内边距 6 + 输入行 28 + 间距 4 + 比例行 24 + 间距 4 + 预设行 24 + 内边距 6 + 边框 1。
const SIZE_PANEL_WIDTH: f32 = 560.0;
const SIZE_PANEL_HEIGHT: f32 = 98.0;
/// 一个预设按钮（如“1920×1080”加内边距与间距）的估算宽度，预设多时据此加宽尺寸条、不换行。
const SIZE_PRESET_WIDTH: f32 = 80.0;

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
    targets: Vec<Snap>,
    selection: Option<Rect>,
    /// 当前高亮的吸附区域，即 `levels[level]` 的矩形。
    hover: Option<Rect>,
    /// 鼠标下可切换的吸附层级（从内到外，最外层可能是整个屏幕）与当前所选层级。
    levels: Vec<Snap>,
    level: usize,
    /// 触控板滚动的累计量，满一格才切换一次层级。
    wheel: f32,
    /// 用户用滚轮 / Tab / Alt 选过层级时，当时鼠标下最内层目标的区域；鼠标仍在同一个目标上时
    /// 保持所选层级（也不被新探测结果改回），移到别的控件上就回到最内层。
    level_manual: Option<Rect>,
    /// 悬停探测鼠标处的深层控件（一次性枚举有时间上限，深层网页常来不及覆盖）；Wayland 等不支持时为空。
    probe: Option<Arc<snap_core::capture::HoverProbe>>,
    /// 最近一次悬停探测的序号；鼠标继续移动后旧结果只合并候选，不再改动高亮。
    probe_seq: u64,
    /// 最近一次按画面识别的窗口及其区域划分；同一窗口内移动鼠标直接复用。
    regions: Option<(usize, Arc<snap_core::capture::regions::Regions>)>,
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
    /// 文字输入框：所在浮层、所属文字草稿编号（TextDraft::id）与输入状态。
    text_input: Option<(usize, u64, Entity<TextareaState>)>,
    picker: Picker,
    /// 最近一次成功导出的选区（R 恢复；设置“沿用上次选区”时打开即预选）与当前显示器布局键。
    recent: Option<RecentSelection>,
    layout_key: String,
    /// 由“截图识字”快捷键发起：框选完成后直接识别，不执行“截图后动作”。
    purpose: Option<RecognitionMode>,
    /// 录屏框选：工具栏换成录制选项与“开始录屏”，框选完成不执行“截图后动作”。
    recording: bool,
    /// 录制偏好（格式、帧率、倒计时、光标、键鼠、声音），修改即保存。
    record: snap_core::recording::preferences::Preferences,
    /// 截图时的顶层窗口（录屏“整个窗口”来源）；选区与其边界一致时可选。
    window_targets: Vec<crate::recording::WindowTarget>,
    /// 本次录屏选择录制整个窗口（不保存为偏好）。
    record_window: bool,
    /// 已识别的顶层窗口及原生句柄（“原生窗口截图”只接受这些窗口）与截图是否含光标。
    natives: Vec<(Rect, snap_core::capture::NativeWindow)>,
    include_cursor: bool,
    /// 工具栏被拖动后的偏移（逻辑像素；双击工具栏恢复位置），与拖动起点。
    toolbar_offset: (f32, f32),
    toolbar_drag: Option<(Point<Pixels>, (f32, f32))>,
    /// 位置与尺寸输入条（单击选区尺寸标签打开）。
    size_editor: Option<SizeEditor>,
    /// 比例锁定、尺寸条单位与尺寸预设（`capture-size.json`），修改即保存。
    sizing: sizing::Sizing,
}

/// 位置与尺寸输入条：所在浮层与 X、Y、宽、高四个输入框（数值为尺寸条当前单位）。
/// 输入框属于该浮层窗口，浮层关闭或选区被别的操作替换时随之丢弃。
struct SizeEditor {
    owner: usize,
    fields: [Entity<InputState>; 4],
}

impl Session {
    /// 选区所在屏的缩放：按左上角所在屏，找不到时为 1。
    fn scale_at(&self, rect: Rect) -> f64 {
        let corner = (i64::from(rect.x), i64::from(rect.y));
        (0..self.displays.len())
            .find(|&i| selection::contains(self.region(i), corner))
            .map_or(1.0, |i| self.displays[i].scale)
    }

    /// 尺寸条单位的换算系数：px 为 1，dp 为选区所在屏的缩放。
    fn unit_scale(&self, rect: Rect) -> f64 {
        match self.sizing.unit {
            Unit::Px => 1.0,
            Unit::Dp => self.scale_at(rect),
        }
    }

    /// 选区在尺寸条单位下的 X、Y、宽、高。
    fn size_values(&self, rect: Rect) -> [i64; 4] {
        let k = self.unit_scale(rect);
        [rect.x, rect.y, rect.width, rect.height].map(|v| (f64::from(v) / k).round() as i64)
    }

    fn ratio(&self) -> Option<selection::Ratio> {
        self.sizing.ratio.map(|r| (r.width, r.height))
    }

    /// 保存尺寸偏好；失败只影响下次截图，记录日志即可。
    fn save_sizing(&self) {
        if let Some(path) = sizing::default_path()
            && let Err(error) = sizing::save(&path, &self.sizing)
        {
            log::warn!("event=gpui.capture_sizing_save_failed message={error}");
        }
    }

    /// 按尺寸条输入改选区。数值为当前单位，空或无效的项沿用原值（未改动的项直接用原物理像素，
    /// 避免 dp 来回换算差 1 像素）；比例锁定时由改动的那一边推出另一边。返回选区是否变化。
    fn apply_size(&mut self, typed: [Option<i64>; 4]) -> bool {
        let Some(rect) = self.selection else {
            return false;
        };
        let k = self.unit_scale(rect);
        let current = self.size_values(rect);
        let original = [rect.x, rect.y, rect.width, rect.height].map(i64::from);
        let changed = |i: usize| typed[i].is_some_and(|v| v != current[i]);
        let value = |i: usize| match typed[i] {
            Some(v) if changed(i) => (v as f64 * k).round() as i64,
            _ => original[i],
        };
        let (x, y, mut w, mut h) = (value(0), value(1), value(2), value(3));
        let ratio = self.ratio();
        if let Some((rw, rh)) = ratio {
            let (rw, rh) = (i64::from(rw), i64::from(rh));
            if changed(3) && !changed(2) {
                w = (h * rw / rh).max(1);
            } else {
                h = (w * rh / rw).max(1);
            }
        }
        let Some(next) = selection::placed(x, y, w, h, ratio.is_some(), self.area()) else {
            return false;
        };
        self.selection = Some(next);
        next != rect
    }

    /// 选区被替换（全屏、重新框选、最近选区、回放等）时关闭尺寸条，避免输入框里留着旧数值。
    fn drop_size_editor(&mut self) {
        self.size_editor = None;
    }
}

impl Session {
    /// 选区正好是某个顶层窗口时返回该窗口。
    fn selected_window(&self) -> Option<crate::recording::WindowTarget> {
        let selection = self.selection?;
        self.window_targets
            .iter()
            .find(|w| snap_core::capture::same_bounds(w.rect, selection))
            .copied()
    }
}

impl Session {
    /// 按鼠标位置重算吸附层级。用户手动选过层级且鼠标仍在该区域内时保持（例如切到整个窗口后
    /// 在窗口内移动），否则回到最内层；按住 Alt 时直接选整个窗口。返回高亮是否变化。
    fn hover_at(&mut self, p: selection::Point, alt: bool) -> bool {
        let mut levels = selection::snap_levels(&self.targets, p);
        if !self.targets.is_empty() {
            let screen = (0..self.displays.len()).map(|i| self.region(i)).find(|r| selection::contains(*r, p));
            if let Some(rect) = screen.filter(|r| levels.last().is_none_or(|last| last.rect != *r)) {
                levels.push(Snap { rect, kind: SnapKind::Screen, window: usize::MAX, name: None });
            }
        }
        let inner = levels.first().map(|l| l.rect);
        let kept = self
            .hover
            .filter(|_| self.level_manual.is_some() && self.level_manual == inner)
            .and_then(|rect| levels.iter().position(|l| l.rect == rect));
        let level = if alt { window_level(&levels).or(kept) } else { kept };
        self.level_manual = level.and(inner);
        self.set_levels(levels, level.unwrap_or(0))
    }

    /// Tab / 滚轮切换层级：正数向外（更大），负数向内；`wrap` 时到头后从另一端继续。
    fn step_level(&mut self, delta: i32, wrap: bool) -> bool {
        let count = self.levels.len() as i32;
        if self.hover.is_none() || count < 2 {
            return false;
        }
        let next = self.level as i32 + delta;
        let next = if wrap { next.rem_euclid(count) } else { next.clamp(0, count - 1) };
        let levels = std::mem::take(&mut self.levels);
        self.level_manual = levels.first().map(|l| l.rect);
        self.set_levels(levels, next as usize)
    }

    /// 追加吸附候选，跳过与已有候选重合的，总数不超过 4096；返回是否有新增。
    fn merge_targets(&mut self, found: Vec<Snap>) -> bool {
        let before = self.targets.len();
        for target in found {
            if self.targets.len() >= 4096 {
                break;
            }
            if !self.targets.iter().any(|existing| snap_core::capture::same_bounds(existing.rect, target.rect)) {
                self.targets.push(target);
            }
        }
        self.targets.len() > before
    }

    fn set_levels(&mut self, levels: Vec<Snap>, level: usize) -> bool {
        let hover = levels.get(level).map(|l| l.rect);
        let changed = hover != self.hover || level != self.level || levels.len() != self.levels.len();
        self.levels = levels;
        self.level = level;
        self.hover = hover;
        changed
    }

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

    /// 取色放大镜只在框出选区之前（含拖出新选区的过程中）显示，选区确定后隐藏；
    /// 标注、导出时也不显示。取色快捷键同样只在此时有效。
    fn picking(&self) -> bool {
        let choosing = self.selection.is_none() || matches!(self.drag, Some(Drag::Create { .. }));
        choosing && !self.busy && !self.annotating && self.editor.shapes().is_empty() && !self.editor.interacting()
    }

    /// 复制光标处色值；保留截图会话，可继续取色或框选。
    fn copy_color(&mut self, cx: &mut App) {
        let Some((_, x, y)) = self.picker.at else {
            return;
        };
        if let Some(text) = magnifier::color_text(&self.image, x, y, self.picker.hex) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.picker.copied = true;
            log::info!("event=gpui.capture_color_copied");
        }
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
    /// 倒计时或后台拍屏中：此时再次触发截图视为取消（倒计时）或忽略（拍屏）。
    starting: bool,
    /// 延时截图倒计时中的取消句柄。
    countdown: Option<crate::countdown::Countdown>,
    /// 浮层被系统或用户从外部关掉（Alt+F4、显示器断开等）时结束会话；首次打开浮层时注册，常驻。
    closed_watch: Option<Subscription>,
}
impl Global for CaptureState {}

/// 在后台线程得到的结果；只含 Send 数据。
struct Grabbed {
    image: RgbaImage,
    origin: (i32, i32),
    displays: Vec<Display>,
    targets: Vec<Snap>,
    windows: Vec<crate::recording::WindowTarget>,
    natives: Vec<(Rect, snap_core::capture::NativeWindow)>,
    include_cursor: bool,
    /// Windows：UI Automation 控件候选在浮层显示后再后台补充（控件枚举占拍屏耗时大半）。
    deferred: Option<Deferred>,
    probe: Option<snap_core::capture::HoverProbe>,
}

#[cfg(windows)]
type Deferred = snap_core::capture::windows::AccessibilityRequest;
#[cfg(not(windows))]
type Deferred = std::convert::Infallible;

fn snap_targets(targets: &[snap_core::capture::Target]) -> Vec<Snap> {
    targets
        .iter()
        .filter(|target| !target.recording_only)
        .map(|target| {
            let window = target.kind == "window";
            Snap {
                rect: target.rect,
                kind: if window { SnapKind::Window } else { SnapKind::Element },
                window: target.window_index,
                name: window.then(|| window_name(&target.title, &target.app_name)).flatten(),
            }
        })
        .collect()
}

/// 层级中“整个窗口”的位置。
fn window_level(levels: &[Snap]) -> Option<usize> {
    levels.iter().position(|l| l.kind == SnapKind::Window)
}

/// 层级标签上的窗口名：优先标题，过长时截断；没有标题时用应用名。
fn window_name(title: &str, app: &str) -> Option<Arc<str>> {
    let source = if title.trim().is_empty() { app.trim() } else { title.trim() };
    if source.is_empty() {
        return None;
    }
    let mut name: String = source.chars().take(40).collect();
    if source.chars().nth(40).is_some() {
        name.push('…');
    }
    Some(name.into())
}

/// 可吸附的顶层窗口及其原生身份（录屏“整个窗口”来源）；读取失败的窗口跳过。
fn window_targets(desktop: &snap_core::capture::Desktop) -> Vec<crate::recording::WindowTarget> {
    desktop
        .targets
        .iter()
        .filter(|t| t.kind == "window" && !t.recording_only)
        .filter_map(|t| {
            let native = desktop.windows.get(t.window_index)?;
            Some(crate::recording::WindowTarget { rect: t.rect, id: native.id().ok()?, pid: native.pid().ok()? })
        })
        .collect()
}

fn grab(include_cursor: bool, snap: bool) -> Result<Grabbed, String> {
    #[cfg(windows)]
    let (desktop, deferred) = snap_core::capture::capture_deferred(false, include_cursor, snap)?;
    #[cfg(not(windows))]
    let (desktop, deferred) = (snap_core::capture::capture(false, include_cursor, snap)?, None);
    let probe = snap_core::capture::hover_probe(&desktop);
    let natives = desktop
        .targets
        .iter()
        .filter(|t| t.kind == "window" && !t.recording_only)
        .filter_map(|t| Some((t.rect, desktop.windows.get(t.window_index)?.clone())))
        .collect();
    Ok(Grabbed {
        natives,
        include_cursor: desktop.include_cursor,
        windows: window_targets(&desktop),
        targets: snap_targets(&desktop.targets),
        image: desktop.image,
        origin: (desktop.x, desktop.y),
        displays: desktop.displays,
        deferred,
        probe,
    })
}

/// 鼠标停稳后再探测，移动中不发起跨进程调用。
const PROBE_DELAY: Duration = Duration::from_millis(60);

/// 悬停探测：鼠标停稳后在后台补全鼠标处的层级，鼠标仍停在原处时立即刷新高亮。
/// 先用系统辅助功能查出由外到内的控件链（Windows、macOS、Linux X11）；鼠标处仍没有控件时按画面识别界面区域，
/// 照顾 Java / Compose、Qt、游戏等不提供控件信息的应用。回放历史截图时不探测。
fn schedule_probe(session: &Entity<Session>, p: selection::Point, cx: &mut App) {
    if history::position(cx).is_some() || p.0 < 0 || p.1 < 0 {
        return;
    }
    let job = session.update(cx, |s, _| {
        let window = s.levels.iter().find(|l| l.kind == SnapKind::Window)?.clone();
        s.probe_seq += 1;
        Some((s.probe.clone(), window, s.probe_seq))
    });
    let Some((request, window, seq)) = job else {
        return;
    };
    let session = session.clone();
    let point = (p.0 as u32, p.1 as u32);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(PROBE_DELAY).await;
        if cx.update(|cx| session.read(cx).probe_seq != seq) {
            return;
        }
        let probe_started = Instant::now();
        let found = match request {
            Some(request) => {
                let index = window.window;
                cx.background_executor().spawn(async move { request.probe(index, point) }).await
            }
            None => Vec::new(),
        };
        log::info!(
            "event=gpui.capture_probe found={} elapsed_ms={}",
            found.len(),
            probe_started.elapsed().as_millis()
        );
        let visual = cx.update(|cx| {
            session.update(cx, |s, cx| {
                let added = s.merge_targets(snap_targets(&found));
                let fresh = s.probe_seq == seq && s.selection.is_none() && s.drag.is_none();
                if added && fresh && s.hover_at(p, false) {
                    cx.notify();
                }
                // 鼠标处没有真正的控件时按画面识别；已有的区域可能只是别处算出的外层，
                // 每个位置都要算出自己的内层链（同一窗口复用划分，单次查询只需微秒）。
                let bare = !selection::snap_levels(&s.targets, p).iter().any(|l| l.kind == SnapKind::Element);
                (fresh && bare).then(|| {
                    let cached = s.regions.as_ref().filter(|(index, _)| *index == window.window).map(|(_, r)| r.clone());
                    // 挡在这个窗口上面的窗口（前后层次靠前），它们的画面不属于这个窗口。
                    let covered: Vec<Rect> = s
                        .targets
                        .iter()
                        .filter(|t| t.kind == SnapKind::Window && t.window < window.window)
                        .filter_map(|t| selection::intersect(t.rect, window.rect))
                        .collect();
                    (s.image.clone(), cached, covered)
                })
            })
        });
        let Some((image, cached, covered)) = visual else {
            return;
        };
        let rect = window.rect;
        let (regions, levels) = cx
            .background_executor()
            .spawn(async move {
                let started = Instant::now();
                let regions = cached
                    .unwrap_or_else(|| Arc::new(snap_core::capture::regions::Regions::build(&image, rect, &covered)));
                let levels = regions.levels(point);
                log::info!(
                    "event=gpui.capture_regions levels={} covered={} elapsed_ms={}",
                    levels.len(),
                    covered.len(),
                    started.elapsed().as_millis()
                );
                (regions, levels)
            })
            .await;
        cx.update(|cx| {
            session.update(cx, |s, cx| {
                s.regions = Some((window.window, regions));
                let snaps = levels
                    .into_iter()
                    .map(|rect| Snap { rect, kind: SnapKind::Region, window: window.window, name: None });
                let added = s.merge_targets(snaps.collect());
                let fresh = s.probe_seq == seq && s.selection.is_none() && s.drag.is_none();
                if added && fresh && s.hover_at(p, false) {
                    cx.notify();
                }
            });
        });
    })
    .detach();
}

/// 浮层显示后在后台补充控件候选，追加到吸附列表（窗口候选之后，与一次性采集的顺序一致）。
#[cfg(windows)]
async fn complete_targets(cx: &mut AsyncApp, session: Entity<Session>, request: Deferred) {
    let started = Instant::now();
    let found = cx.background_executor().spawn(async move { request.collect() }).await;
    cx.update(|cx| {
        session.update(cx, |s, cx| {
            let before = s.targets.len();
            s.merge_targets(snap_targets(&found));
            log::info!(
                "event=gpui.capture_targets_completed added={} elements_ms={}",
                s.targets.len() - before,
                started.elapsed().as_millis()
            );
            cx.notify();
        });
    });
}

#[cfg(not(windows))]
async fn complete_targets(_: &mut AsyncApp, _: Entity<Session>, request: Deferred) {
    match request {}
}

/// 是否已有截图会话（准备中或浮层已显示）。
pub fn active(cx: &App) -> bool {
    cx.try_global::<CaptureState>()
        .is_some_and(|state| state.starting || state.session.is_some())
}

/// 开始截图。`delay` 用于等托盘菜单等短暂界面消失；设置中的截图延时显示倒计时，期间可取消。
pub fn start(cx: &mut App, settings: &Settings, delay: Duration, purpose: Option<RecognitionMode>) {
    begin(cx, settings, delay, purpose, false);
}

/// 录屏框选：与截图共用浮层，确认后交给 `recording`（不使用截图延时，也不预选上次选区）。
pub fn start_recording(cx: &mut App, settings: &Settings) {
    begin(cx, settings, Duration::ZERO, None, true);
}

fn begin(cx: &mut App, settings: &Settings, delay: Duration, purpose: Option<RecognitionMode>, recording: bool) {
    if active(cx) {
        log::info!("event=gpui.capture_ignored reason=active");
        return;
    }
    // 录屏中允许截图（拍屏前隐藏录屏窗口，见下）；录屏框选本身不能叠加在进行中的录屏上。
    if recording && crate::recording::active(cx) {
        log::info!("event=gpui.capture_ignored reason=recording");
        return;
    }
    // 滚动截图占用屏幕期间不截图（截图键在滚动截图中表示完成，由 shell 先行处理）。
    if crate::scroll::active(cx) {
        log::info!("event=gpui.capture_ignored reason=scroll");
        return;
    }
    // macOS 未授权“屏幕录制”时系统只返回壁纸和本应用窗口，浮层会是一片黑；先拒绝并引导授权。
    // 首次调用会弹出系统授权框；已拒绝或授权记录失效（ad-hoc 签名的应用更新后常见）时打开设置的系统权限卡片。
    #[cfg(target_os = "macos")]
    if !snap_core::macos::screen_capture_granted() {
        log::warn!("event=gpui.capture_refused reason=screen_capture_permission");
        if !snap_core::macos::request_screen_capture() {
            crate::settings_window::open_permissions(cx);
        }
        return;
    }
    cx.default_global::<CaptureState>().starting = true;
    let include_cursor = settings.screenshot_cursor;
    let snap = settings.screenshot_snap;
    let export = settings.export.clone();
    let after = settings.screenshot_after.clone();
    let reuse_last = settings.screenshot_reuse_last;
    let language = settings.appearance.language.clone();
    let seconds = settings.delay_seconds;
    let hint = snap_core::i18n::for_language(&language, "再次按截图快捷键取消").to_owned();
    cx.spawn(async move |cx| {
        if !delay.is_zero() {
            cx.background_executor().timer(delay).await;
        }
        if seconds > 0 {
            let elapsed = crate::countdown::run(cx, seconds, hint, |cx, countdown| {
                cx.default_global::<CaptureState>().countdown = Some(countdown);
            })
            .await;
            let proceed = cx.update(|cx| {
                let state = cx.default_global::<CaptureState>();
                state.countdown = None;
                if !elapsed {
                    state.starting = false;
                }
                elapsed
            });
            if !proceed {
                return;
            }
        }
        // 展开中的暂存架、录屏中的红框与控制条、“已保存”提示先临时收起，等桌面合成去掉它们再拍屏；会话结束后恢复。
        let hidden = cx.update(|cx| {
            let shelf = crate::shelf::suspend(cx);
            let recording = crate::recording::set_overlays_visible(cx, false);
            let toast = crate::toast::dismiss(cx);
            shelf || recording || toast
        });
        if hidden {
            cx.background_executor().timer(Duration::from_millis(120)).await;
        }
        let started = Instant::now();
        let result = cx
            .background_executor()
            .spawn(async move { grab(include_cursor, snap) })
            .await;
        let capture_ms = started.elapsed().as_millis();
        let deferred = cx.update(|cx| {
            cx.global_mut::<CaptureState>().starting = false;
            match result {
                Ok(mut grabbed) => {
                    let deferred = grabbed.deferred.take();
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
                        levels: Vec::new(),
                        level: 0,
                        wheel: 0.0,
                        level_manual: None,
                        probe: grabbed.probe.take().map(Arc::new),
                        probe_seq: 0,
                        regions: None,
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
                        picker: Picker::default(),
                        recent: None,
                        layout_key: String::new(),
                        purpose,
                        recording,
                        record: crate::recording::preferences(),
                        window_targets: grabbed.windows,
                        record_window: false,
                        natives: grabbed.natives,
                        include_cursor: grabbed.include_cursor,
                        toolbar_offset: (0.0, 0.0),
                        toolbar_drag: None,
                        size_editor: None,
                        sizing: sizing::default_path().map(|path| sizing::load(&path)).unwrap_or_default(),
                    });
                    session.update(cx, |s, _| {
                        s.layout_key = recent::layout_key(&s.displays);
                        s.recent = recent::default_path().and_then(|path| recent::load(&path));
                        if reuse_last && purpose.is_none() && !recording {
                            s.selection = recent::restore(s.recent.as_ref(), s.image.width(), s.image.height(), &s.layout_key);
                        }
                    });
                    session.update(cx, |s, _| s.sync());
                    history::reset(cx);
                    cx.global_mut::<CaptureState>().session = Some(session.clone());
                    open_overlays(cx, &session);
                    log::info!(
                        "event=gpui.overlay_shown total_ms={}",
                        started.elapsed().as_millis()
                    );
                    deferred.map(|request| (session, request))
                }
                Err(error) => {
                    log::error!("event=gpui.capture_failed message={error}");
                    crate::shelf::resume(cx);
                    crate::recording::set_overlays_visible(cx, true);
                    None
                }
            }
        });
        if let Some((session, request)) = deferred {
            complete_targets(cx, session, request).await;
        }
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
        // 录屏中截图：浮层不进入录像，录下来的仍是实际桌面。
        let exclude = crate::recording::active(cx);
        match cx.open_window(options, |window, cx| {
            // 复制、保存等完成后浮层立即消失，不淡出。
            crate::native::disable_transitions(window);
            if exclude {
                if let Err(error) = crate::native::exclude_from_capture(window) {
                    log::warn!("event=gpui.overlay_exclude_failed message={error}");
                }
            }
            cx.new(|cx| Overlay::new(shared, index, frame, cx))
        }) {
            Ok(handle) => handles.push(handle.into()),
            Err(error) => log::error!("event=gpui.overlay_failed display={index} message={error}"),
        }
    }
    // 一个浮层都没打开时会话无处可操作，却仍算“截图中”：快捷键只会取消一个看不见的会话，
    // 导入配置、安装更新等也会被拒绝。直接结束会话（issue #71）。
    if handles.is_empty() {
        log::error!("event=gpui.capture_aborted reason=no_overlay");
        finish(cx);
        return;
    }
    // focus: true 只在本进程已是前台时生效；从命令行转交、或保存对话框关闭后系统把焦点还给了
    // 原来的应用，浮层就收不到 Esc / 回车 / Ctrl+S。主动激活一个浮层（所有浮层共享会话，激活哪个都行）。
    if let Some(first) = handles.first() {
        let _ = first.update(cx, |_, window, _| window.activate_window());
    }
    session.update(cx, |s, _| s.windows = handles);
    watch_closed(cx);
}

/// 浮层窗口被外部关掉时结束会话。自己关闭浮层（`close_overlays`）会先把句柄从 `windows` 取走，
/// 因此这里仍能在 `windows` 中找到的窗口，一定不是本模块关闭的；剩下的浮层随会话一起关闭。
fn watch_closed(cx: &mut App) {
    if cx.default_global::<CaptureState>().closed_watch.is_some() {
        return;
    }
    let subscription = cx.on_window_closed(|cx, closed| {
        let Some(session) = current(cx) else {
            return;
        };
        if session.read(cx).windows.iter().any(|handle| handle.window_id() == closed) {
            log::warn!("event=gpui.capture_aborted reason=overlay_closed");
            // 回调在窗口移除流程中执行，结束会话延后到当前更新完成。
            cx.defer(finish);
        }
    });
    cx.global_mut::<CaptureState>().closed_watch = Some(subscription);
}

/// 截图显示器对应的 GPUI 显示器（录屏红框与控制条定位）。
pub fn matched_display(cx: &App, display: &Display) -> Option<std::rc::Rc<dyn PlatformDisplay>> {
    let scale = display.scale as f32;
    let logical = Bounds::new(
        point(px(display.x as f32 / scale), px(display.y as f32 / scale)),
        size(px(display.width as f32 / scale), px(display.height as f32 / scale)),
    );
    match_display(&cx.displays(), logical).cloned()
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

/// 记住成功导出的选区（复制、保存、贴图）；写入失败只记日志。
fn remember(cx: &mut App) {
    let Some(session) = current(cx) else {
        return;
    };
    history::remember(cx, &session);
    let s = session.read(cx);
    let (Some(rect), Some(path)) = (s.selection, recent::default_path()) else {
        return;
    };
    if let Err(error) = recent::save(&path, rect, s.image.width(), s.image.height(), s.layout_key.clone()) {
        log::warn!("event=gpui.recent_selection_save_failed message={error}");
    }
}

/// 结束会话：关闭浮层并释放冻结画面。
fn finish(cx: &mut App) {
    let Some(session) = cx.global_mut::<CaptureState>().session.take() else {
        return;
    };
    close_overlays(cx, &session);
    history::reset(cx);
    crate::shelf::resume(cx);
    // 录屏中截图结束：重新显示录屏红框与控制条。
    crate::recording::set_overlays_visible(cx, true);
}

/// 选择光标所在的整个屏幕（Ctrl/⌘+A 或工具栏）；已有标注时选区固定。
fn select_screen(cx: &mut App, _owner: isize) {
    if let Some(session) = current(cx) {
        session.update(cx, |s, cx| {
            if s.selection_locked() || s.busy {
                return;
            }
            let index = s.picker.at.map_or(0, |(owner, _, _)| owner).min(s.displays.len().saturating_sub(1));
            s.selection = Some(s.region(index));
            s.drop_size_editor();
            s.hover = None;
            s.drag = None;
            s.sync();
            cx.notify();
        });
    }
}

/// 选区正好是已识别窗口时的原生句柄（选中的吸附目标是窗口时才显示“原生窗口截图”）。
fn selected_native(s: &Session) -> Option<(Rect, snap_core::capture::NativeWindow)> {
    let selection = s.selection?;
    s.natives.iter().find(|(rect, _)| snap_core::capture::same_bounds(*rect, selection)).cloned()
}

/// 原生窗口截图：隐藏浮层后用 PrintWindow 等重抓该窗口（取到被遮挡部分），贴回冻结画面；
/// 窗口位置或尺寸变化时保留原图。成功与失败都恢复浮层。
fn native_window(cx: &mut App, _owner: isize) {
    let Some(session) = current(cx) else {
        return;
    };
    let prepared = session.update(cx, |s, cx| {
        if s.busy || s.selection_locked() {
            return None;
        }
        let (rect, native) = selected_native(s)?;
        s.busy = true;
        s.editor.error = None;
        cx.notify();
        Some((rect, native, s.include_cursor, s.origin, s.windows.clone()))
    });
    let Some((old, native, include_cursor, origin, handles)) = prepared else {
        return;
    };
    let visible = move |cx: &mut App, visible: bool| {
        for handle in &handles {
            let _ = handle.update(cx, |_, window, _| crate::native::set_visible_async(window, visible));
        }
    };
    visible(cx, false);
    let shown = visible.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        // 等浮层真正从屏幕上消失再抓取。
        cx.background_executor().timer(Duration::from_millis(180)).await;
        let grabbed = cx
            .background_executor()
            .spawn(async move { snap_core::capture::native_window(&native, include_cursor) })
            .await;
        cx.update(|cx| {
            let result = session.update(cx, |s, cx| {
                s.busy = false;
                let (image, x, y) = grabbed?;
                let size = (s.image.width(), s.image.height());
                let rect = snap_core::capture::clipped(x, y, image.width(), image.height(), origin, size)
                    .ok_or("窗口已移出截图范围。")?;
                if !snap_core::capture::same_bounds(rect, old) {
                    return Err("窗口位置或尺寸已变化，请重新截图。".to_string());
                }
                image::imageops::replace(
                    Arc::make_mut(&mut s.image),
                    &image,
                    i64::from(x) - i64::from(origin.0),
                    i64::from(y) - i64::from(origin.1),
                );
                cx.notify();
                Ok((0..s.displays.len()).map(|i| (s.windows.get(i).copied(), s.region(i))).collect::<Vec<_>>())
            });
            match result {
                Ok(regions) => {
                    let image = session.read(cx).image.clone();
                    for (handle, region) in regions {
                        let Some(overlay) = handle.and_then(|h| h.downcast::<Overlay>()) else {
                            continue;
                        };
                        let frame = display_frame(&image, region);
                        let _ = overlay.update(cx, |view, window, cx| {
                            let old = std::mem::replace(&mut view.frame, frame);
                            cx.drop_image(old, Some(window));
                            cx.notify();
                        });
                    }
                    log::info!("event=gpui.capture_native_replaced");
                }
                Err(error) => {
                    log::warn!("event=gpui.capture_native_failed message={error}");
                    session.update(cx, |s, cx| {
                        s.editor.error = Some(error);
                        cx.notify();
                    });
                }
            }
            shown(cx, true);
            if let Some(first) = session.read(cx).windows.first().copied() {
                let _ = first.update(cx, |_, window, _| window.activate_window());
            }
        });
    })
    .detach();
}

/// 清空标注并重新框选。
fn reselect(cx: &mut App, _owner: isize) {
    let Some(session) = current(cx) else {
        return;
    };
    session.update(cx, |s, cx| {
        if s.busy {
            return;
        }
        s.editor.reset(Vec::new());
        s.annotating = false;
        s.text_input = None;
        s.selection = None;
        s.drop_size_editor();
        s.drag = None;
        s.layers.clear();
        s.sync();
        cx.notify();
    });
    flush(&session, None, cx);
}

/// 截图中按录屏快捷键或点“录屏”：工具栏切换为录制选项。
pub fn enter_recording(cx: &mut App) {
    if let Some(session) = current(cx) {
        session.update(cx, |s, cx| {
            s.recording = true;
            s.purpose = None;
            cx.notify();
        });
    }
}

/// 修改录制偏好并保存。
fn set_record(session: &Entity<Session>, cx: &mut App, change: impl FnOnce(&mut snap_core::recording::preferences::Preferences)) {
    session.update(cx, |s, cx| {
        change(&mut s.record);
        crate::recording::save_preferences(&s.record);
        cx.notify();
    });
}

/// 开始录屏：后台准备编码器与采集，成功后收起浮层交给录屏控制条（暂存架保持收起，录屏结束再恢复）；
/// 失败时浮层保留并在工具栏下显示原因。
fn record(cx: &mut App, _owner: isize) {
    // 录屏中打开的截图会话不能再开始第二段录屏。
    if crate::recording::active(cx) {
        if let Some(session) = current(cx) {
            session.update(cx, |s, cx| {
                s.editor.error = Some("正在录屏，请先结束当前录屏。".into());
                cx.notify();
            });
        }
        return;
    }
    let Some((session, _, rect, shapes)) = begin_export(cx) else {
        return;
    };
    let request = {
        let s = session.read(cx);
        crate::recording::Request {
            rect,
            origin: s.origin,
            displays: s.displays.clone(),
            window: s.record_window.then(|| s.selected_window()).flatten(),
            shapes,
            preferences: s.record,
            language: s.language.clone(),
        }
    };
    cx.spawn(async move |cx| {
        let prepared = crate::recording::prepare(cx, request).await;
        cx.update(|cx| match prepared {
            Ok(prepared) => {
                if let Some(session) = cx.global_mut::<CaptureState>().session.take() {
                    close_overlays(cx, &session);
                }
                crate::recording::launch(cx, prepared);
            }
            Err(error) => session.update(cx, |s, cx| {
                s.busy = false;
                s.editor.error = Some(error);
                cx.notify();
            }),
        });
    })
    .detach();
}

/// 滚动截图：以选区为首帧开始采集，成功后收起浮层（暂存架保持收起，结束后恢复）；
/// 已有标注时不可用（滚动截图只输出拼接结果）。失败时浮层保留并显示原因。
fn scroll(cx: &mut App, _owner: isize) {
    let Some(session) = current(cx) else {
        return;
    };
    let request = {
        let s = session.read(cx);
        if s.busy || s.editor.pending || s.editor.text.is_some() {
            return;
        }
        if !s.editor.shapes().is_empty() {
            session.update(cx, |s, cx| {
                s.editor.error = Some("滚动截图不包含标注，请先清除标注。".into());
                cx.notify();
            });
            return;
        }
        let Some(rect) = s.selection else {
            return;
        };
        crate::scroll::Request {
            image: s.image.clone(),
            rect,
            origin: s.origin,
            displays: s.displays.clone(),
            language: s.language.clone(),
        }
    };
    match crate::scroll::start(cx, request) {
        Ok(()) => {
            if let Some(session) = cx.global_mut::<CaptureState>().session.take() {
                close_overlays(cx, &session);
            }
        }
        Err(error) => session.update(cx, |s, cx| {
            s.editor.error = Some(error);
            cx.notify();
        }),
    }
}

pub fn cancel(cx: &mut App) {
    // 倒计时中：只设置取消标志，由倒计时任务关闭提示窗并结束流程。
    if let Some(countdown) = cx.try_global::<CaptureState>().and_then(|s| s.countdown.clone()) {
        countdown.cancel();
        return;
    }
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
            let result = encoded.and_then(|(image, png)| write_clipboard(cx, owner, &image, png).map(|_| image));
            match result {
                Ok(image) => {
                    let (width, height) = image.dimensions();
                    log::info!("event=gpui.capture_copied width={width} height={height}");
                    auto_save(cx, Arc::new(image));
                    remember(cx);
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
    // 同时写 PNG 与保留 alpha 的 DIBV5，兼容只认位图的老程序。
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
    // 自动保存的那一份带上标注（贴图本身的标注仍可编辑，底图不合成）。
    let annotated = crate::shell::settings(cx).export.auto_save.then(|| (image.clone(), shapes.clone()));
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
        // 选区所在屏：包含选区左上角的那块（跨屏选区按左上角所在屏贴图）。
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
                crate::pins::create(cx, image, shapes, crate::pins::Placement::Source { origin, scale, display }, None)
            });
            match result {
                Ok(()) => {
                    log::info!("event=gpui.capture_pinned width={} height={}", rect.width, rect.height);
                    if let Some((source, shapes)) = annotated {
                        auto_save_composed(cx, source, rect, shapes);
                    }
                    remember(cx);
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

/// 识别选区：文字识别与翻译用原始像素（标注可能遮挡文字），二维码 / 条形码包含标注。
/// 识别在结果窗口中进行；浮层随即关闭，结果窗口独立存活。
pub fn recognize(cx: &mut App, mode: RecognitionMode) {
    let Some((session, image, rect, shapes)) = begin_export(cx) else {
        return;
    };
    let shapes = if mode == RecognitionMode::Qr { shapes } else { Vec::new() };
    cx.spawn(async move |cx| {
        let composed = cx
            .background_executor()
            .spawn(async move { snap_core::capture::compose(&image, rect, &shapes) })
            .await;
        cx.update(|cx| match composed {
            Ok(image) => {
                log::info!("event=gpui.capture_recognize mode={mode:?} width={} height={}", rect.width, rect.height);
                finish(cx);
                crate::recognition::open(cx, image, mode);
            }
            Err(error) => {
                log::error!("event=gpui.capture_recognize_failed message={error}");
                session.update(cx, |s, _| s.busy = false);
            }
        });
    })
    .detach();
}

/// 保存选区。置顶浮层会挡住系统保存对话框，因此先关闭浮层但保留会话；取消或失败时重新打开浮层，
/// 选区保持不变。文件名、格式、目录记忆见 `snap_core::export`。
fn save(cx: &mut App) {
    let Some((session, image, rect, shapes)) = begin_export(cx) else {
        return;
    };
    let (export, title, language) = {
        let s = session.read(cx);
        (s.export.clone(), s.text("保存图片"), s.language.clone())
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
        let directory = record
            .as_deref()
            .and_then(|path| snap_core::export::preferred_directory(path, export.remember_directory));
        let dialog = SaveDialog::new(title, name, directory, export.format, &language, None);
        // 浮层已关闭；置顶贴图同样会盖住对话框，期间临时隐藏。
        let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
        let picked = dialog.pick().await;
        cx.update(|cx| crate::pins::show_pins(cx, hidden));
        let Some(path) = picked else {
            cx.update(|cx| reopen(cx, &session));
            return;
        };
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
                remember(cx);
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

/// 快速保存：不弹对话框，按默认格式与文件名规则存进截图保存目录（重名加 -2、-3），然后结束截图并提示已保存，
/// 单击提示在文件夹中定位文件。失败时保留浮层与选区，在工具栏下方显示原因。
pub(crate) fn quick_save(cx: &mut App) {
    let Some((session, image, rect, shapes)) = begin_export(cx) else {
        return;
    };
    let export = session.read(cx).export.clone();
    cx.spawn(async move |cx| {
        let saved = cx
            .background_executor()
            .spawn(async move {
                let image = snap_core::capture::compose(&image, rect, &shapes)?;
                let directory = snap_core::export::save_directory(export.remember_directory)
                    .ok_or("无法确定截图保存目录，请在设置中选择。")?;
                snap_core::export::quick_save(&image, &export, &directory)
            })
            .await;
        cx.update(|cx| match saved {
            Ok(path) => {
                log::info!("event=gpui.capture_quick_saved width={} height={}", rect.width, rect.height);
                remember(cx);
                finish(cx);
                crate::toast::saved(cx, path);
            }
            Err(error) => {
                log::error!("event=gpui.capture_quick_save_failed message={error}");
                session.update(cx, |s, cx| {
                    s.busy = false;
                    s.editor.error = Some(snap_core::i18n::for_language(&s.language, &error).to_owned());
                    cx.notify();
                });
            }
        });
    })
    .detach();
}

/// 设置开启“自动保存”时，在后台把成图另存一份到截图保存目录，并按保留天数清理到期的自动保存文件。
/// 复制、贴图、滚动截图完成后调用；失败只记录日志，不影响已完成的操作。
pub(crate) fn auto_save(cx: &App, image: Arc<RgbaImage>) {
    let export = crate::shell::settings(cx).export;
    if !export.auto_save {
        return;
    }
    cx.background_executor()
        .spawn(async move {
            let Some(directory) = snap_core::export::save_directory(export.remember_directory) else {
                log::warn!("event=gpui.auto_save_failed message=no_directory");
                return;
            };
            match snap_core::auto_save::save(&image, &export, &directory) {
                Ok(_) => log::info!("event=gpui.auto_saved width={} height={}", image.width(), image.height()),
                Err(error) => log::warn!("event=gpui.auto_save_failed message={error}"),
            }
            clean_expired(export.auto_clean_days);
        })
        .detach();
}

/// 贴图时的自动保存：成图要先把标注合成进去，合成也放在后台线程。
fn auto_save_composed(cx: &App, source: Arc<RgbaImage>, rect: Rect, shapes: Vec<Annotation>) {
    let export = crate::shell::settings(cx).export;
    cx.background_executor()
        .spawn(async move {
            let composed = snap_core::capture::compose(&source, rect, &shapes);
            let directory = snap_core::export::save_directory(export.remember_directory);
            match (composed, directory) {
                (Ok(image), Some(directory)) => match snap_core::auto_save::save(&image, &export, &directory) {
                    Ok(_) => log::info!("event=gpui.auto_saved width={} height={}", image.width(), image.height()),
                    Err(error) => log::warn!("event=gpui.auto_save_failed message={error}"),
                },
                (Err(error), _) => log::warn!("event=gpui.auto_save_failed message={error}"),
                (_, None) => log::warn!("event=gpui.auto_save_failed message=no_directory"),
            }
            clean_expired(export.auto_clean_days);
        })
        .detach();
}

/// 按保留天数删除到期的自动保存文件（启动时与每次自动保存后调用，在后台线程执行）。
pub(crate) fn clean_expired(days: u16) {
    match snap_core::auto_save::clean(days) {
        Ok(0) => {}
        Ok(count) => log::info!("event=gpui.auto_save_cleaned count={count}"),
        Err(error) => log::warn!("event=gpui.auto_save_clean_failed message={error}"),
    }
}

/// 图片保存对话框（截图、贴图、滚动截图结果共用）。类型列表为设置中的默认格式在前、默认选中，其余格式随后；
/// 实际编码按返回路径的扩展名决定（`snap_core::export::write_file`）。
/// Windows 用 `snap_core::save_dialog`，默认文件名不带扩展名，按所选类型补上，选 PDF 就得到 .pdf，对话框在独立线程中显示；
/// 其他平台用 rfd，默认文件名带默认格式的扩展名，可直接改写。
pub(crate) struct SaveDialog {
    #[cfg(windows)]
    request: snap_core::save_dialog::SaveRequest,
    #[cfg(not(windows))]
    dialog: rfd::AsyncFileDialog,
}

impl SaveDialog {
    /// `owner` 为对话框所属窗口（贴图、结果窗口）；截图浮层已关闭时传 None。
    pub(crate) fn new(
        title: String,
        file_name: String,
        directory: Option<std::path::PathBuf>,
        default: snap_core::settings::ExportFormat,
        language: &Language,
        owner: Option<&Window>,
    ) -> Self {
        let types = default.dialog_order().map(|format| {
            let (label, extensions) = format.filter();
            (snap_core::i18n::for_language(language, label).to_owned(), extensions)
        });
        #[cfg(windows)]
        {
            let types = types
                .map(|(name, extensions)| snap_core::save_dialog::FileType {
                    name,
                    extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
                })
                .collect();
            let owner = owner.map(crate::native::hwnd);
            // 文件名不带扩展名：扩展名由“保存类型”决定，关闭后按所选类型补上（见 `snap_core::save_dialog`），
            // 免得选了 PDF 时文件名里仍显示 .png。
            let file_name = file_name.rsplit_once('.').map_or(file_name.as_str(), |(stem, _)| stem).to_owned();
            let replace_prompt = snap_core::i18n::for_language(language, "“{0}”已存在，要替换吗？").to_owned();
            Self { request: snap_core::save_dialog::SaveRequest { title, file_name, directory, types, owner, replace_prompt } }
        }
        #[cfg(not(windows))]
        {
            let mut dialog = rfd::AsyncFileDialog::new().set_title(title).set_file_name(file_name);
            for (name, extensions) in types {
                dialog = dialog.add_filter(name, extensions);
            }
            if let Some(directory) = directory {
                dialog = dialog.set_directory(directory);
            }
            if let Some(window) = owner {
                dialog = dialog.set_parent(window);
            }
            Self { dialog }
        }
    }

    /// 显示对话框并返回选定的路径；取消或对话框出错时为 None（出错记录日志）。
    pub(crate) async fn pick(self) -> Option<std::path::PathBuf> {
        #[cfg(windows)]
        {
            // 模态对话框会阻塞所在线程直到关闭，不能占用界面线程或后台执行器。
            let (sender, receiver) = futures::channel::oneshot::channel();
            let request = self.request;
            std::thread::spawn(move || {
                let _ = sender.send(snap_core::save_dialog::show(&request));
            });
            match receiver.await {
                Ok(Ok(path)) => path,
                Ok(Err(error)) => {
                    log::error!("event=gpui.save_dialog_failed message={error}");
                    None
                }
                Err(_) => None,
            }
        }
        #[cfg(not(windows))]
        {
            self.dialog.save_file().await.map(|file| file.path().to_path_buf())
        }
    }
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
        s.drop_size_editor();
        s.layers.clear();
        let _ = s.layers.take_retired();
        s.sync();
    });
    open_overlays(cx, session);
}

/// 截图历史模式条：上一张、位置、下一张。
fn mode_bar(s: &Session, position: Option<(usize, usize)>, has_history: bool) -> Stateful<Div> {
    let enabled = !s.busy && !s.editor.pending && !s.editor.interacting();
    let button = |id: &'static str, label: String, on: bool| {
        let button = div()
            .id(id)
            .h(px(30.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .rounded(px(6.0))
            .child(label);
        if on {
            button.cursor_pointer().hover(|b| b.bg(rgb(annotate::hover())))
        } else {
            button.opacity(0.4)
        }
    };
    let mut bar = div()
        .id("capture-modebar")
        .absolute()
        .left(px(8.0))
        .top(px(8.0))
        .min_h(px(40.0))
        .px(px(6.0))
        .py(px(4.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(rgb(annotate::border()))
        .bg(rgb(annotate::surface()))
        .shadow_lg()
        .text_xs()
        .text_color(rgb(annotate::text()))
        .cursor_default()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
    if !s.recording {
        let previous = enabled && has_history;
        let mut back = button("capture-replay-previous", s.text("上一张 ,"), previous);
        if previous {
            back = back.on_click(|_, _, cx| history::replay(cx, history::Direction::Previous));
        }
        bar = bar.child(back);
        if let Some((index, total)) = position {
            bar = bar.child(
                div()
                    .px(px(6.0))
                    .text_color(rgb(annotate::muted()))
                    .child(s.text("截图历史 {0}/{1}").replace("{0}", &index.to_string()).replace("{1}", &total.to_string())),
            );
        }
        let next = enabled && position.is_some();
        let mut forward = button("capture-replay-next", s.text("下一张 ."), next);
        if next {
            forward = forward.on_click(|_, _, cx| history::replay(cx, history::Direction::Next));
        }
        bar = bar.child(forward);
    }
    if !s.recording {
        let live = enabled && position.is_none();
        let mut refresh = button("capture-refresh", s.text("刷新 F5"), live);
        if live {
            refresh = refresh.on_click(|_, _, cx| history::refresh(cx));
        }
        bar = bar.child(refresh);
    }
    let recent = enabled
        && !s.selection_locked()
        && recent::restore(s.recent.as_ref(), s.image.width(), s.image.height(), &s.layout_key).is_some();
    let mut restore = button("capture-recent", s.text("最近选区 R"), recent)
        .child(annotate::icon(annotate::icons::FOCUS, annotate::text()).size(px(15.0)));
    if recent {
        restore = restore.on_click(|_, _, cx| {
            if let Some(session) = current(cx) {
                session.update(cx, |s, cx| {
                    if let Some(rect) = recent::restore(s.recent.as_ref(), s.image.width(), s.image.height(), &s.layout_key) {
                        s.selection = Some(rect);
                        s.drop_size_editor();
                        s.drag = None;
                        s.hover = None;
                        s.sync();
                        cx.notify();
                    }
                });
            }
        });
    }
    bar.child(restore)
}

/// 未框选时的提示：显示器顶部居中的深色圆角框。
fn capture_tip(s: &Session) -> Div {
    let (main, keys) = if s.recording {
        ("拖动选择录屏范围", "Esc 退出 · Ctrl / ⌘ + A 全屏 · R 最近选区")
    } else if !s.targets.is_empty() {
        ("点击吸附 · 滚轮 / Tab 切换层级 · Alt 整个窗口 · 拖动框选", "C 取色 · Esc 取消 · Ctrl / ⌘ + A 全屏 · R 最近选区")
    } else {
        ("拖动框选屏幕", "C 取色 · Esc 取消 · Ctrl / ⌘ + A 全屏 · R 最近选区")
    };
    div()
        .absolute()
        .top(px(64.0))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .px(px(24.0))
                .py(px(14.0))
                .rounded(px(10.0))
                .bg(rgba(0x242330ed))
                .text_size(px(14.0))
                .text_color(rgb(0xffffff))
                .flex()
                .flex_wrap()
                .items_baseline()
                .justify_center()
                .gap_x(px(20.0))
                .child(s.text(main))
                .child(div().text_xs().text_color(rgb(0xb7b4c8)).child(s.text(keys)))
                // 锁定了比例时提示：拖动会保持该比例（在选区尺寸标签打开的尺寸条里解除）。
                .children(s.ratio().map(|(w, h)| {
                    div().text_xs().text_color(rgb(0x93c5fd)).child(s.text("比例 {0}").replace("{0}", &format!("{w}:{h}")))
                })),
        )
}

/// 高亮区域左上角的层级标签：种类、窗口名、物理尺寸与“当前层 / 总层数”，
/// 让用户知道吸附到了什么、还能不能用滚轮或 Tab 换一层。
fn level_label(s: &Session, b: Bounds<Pixels>, viewport: Size<Pixels>) -> Div {
    const HEIGHT: f32 = 22.0;
    let Some(snap) = s.levels.get(s.level) else {
        return div();
    };
    let kind = match snap.kind {
        SnapKind::Element => s.text("控件"),
        SnapKind::Region => s.text("区域"),
        SnapKind::Window => s.text("窗口"),
        SnapKind::Screen => s.text("整个屏幕"),
    };
    let mut parts = vec![kind];
    if let Some(name) = &snap.name {
        parts.push(name.to_string());
    }
    parts.push(format!("{} × {}", snap.rect.width, snap.rect.height));
    if s.levels.len() > 1 {
        parts.push(format!("{} / {}", s.level + 1, s.levels.len()));
    }
    // 上方放得下就贴在边框外，否则放进区域内左上角；水平方向不超出屏幕右侧。
    let top = if f32::from(b.origin.y) >= HEIGHT + 2.0 { b.origin.y - px(HEIGHT + 2.0) } else { b.origin.y + px(4.0) };
    let left = b.origin.x.max(px(0.0)).min((viewport.width - px(320.0)).max(px(0.0)));
    div()
        .absolute()
        .left(left)
        .top(top)
        .h(px(HEIGHT))
        .max_w(px(480.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .bg(rgba(0x242330ed))
        .text_size(px(12.0))
        .text_color(rgb(0xffffff))
        .whitespace_nowrap()
        .overflow_hidden()
        .child(parts.join(" · "))
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
        let (wanted, has) = {
            let s = self.session.read(cx);
            (
                s.editor.text.as_ref().map(|t| (t.id, t.shape.text.clone().unwrap_or_default())),
                s.text_input.as_ref().map(|(_, id, _)| *id),
            )
        };
        // 文字提交后要等栅格化完成才结束草稿，期间输入框不会被移除；
        // 因此按草稿编号判断，下一段文字总是换一个新的空输入框，而不是沿用上一段的内容。
        match (wanted, has) {
            (Some((id, value)), has) if has != Some(id) => {
                let state = cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx);
                    state.set_auto_grow(1, 20, cx);
                    state.set_value(value, window, cx);
                    state
                });
                state.update(cx, |state, cx| state.focus(window, cx));
                // 输入框在按下鼠标时创建；阻止 GPUI 随后把焦点交还给浮层，否则还要再点一次输入框才能打字。
                window.prevent_default();
                let index = self.index;
                self.session.update(cx, |s, _| s.text_input = Some((index, id, state)));
            }
            (None, Some(_)) => {
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
            .map(|(_, _, state)| state.read(cx).value().to_string())
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
        if self.session.read(cx).size_editor.is_some() {
            // 点击尺寸条以外：应用输入并关闭，这次按下不再新建或移动选区。
            self.apply_size_editor(true, window, cx);
            return;
        }
        let p = self.to_image(event.position, cx);
        let point = self.to_point(event.position, cx);
        let tolerance = (HANDLE_HIT * self.scale(cx)).round() as i64;
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
            // 新建选区可跨屏：限制在整张虚拟桌面截图内，拖到其他屏时由该屏浮层继续显示。
            let area = s.area();
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
        // 拖动工具栏把手：只移动工具栏，不影响选区与标注。
        let dragging_toolbar = self.session.update(cx, |s, cx| {
            let Some((start, origin)) = s.toolbar_drag else {
                return false;
            };
            if event.pressed_button != Some(MouseButton::Left) {
                s.toolbar_drag = None;
                return false;
            }
            let delta = event.position - start;
            s.toolbar_offset = (origin.0 + f32::from(delta.x), origin.1 + f32::from(delta.y));
            cx.notify();
            true
        });
        if dragging_toolbar {
            return;
        }
        let p = self.to_image(event.position, cx);
        let point = self.to_point(event.position, cx);
        let index = self.index;
        self.session.update(cx, |s, cx| {
            let pixel = (
                (point.x.floor().max(0.0) as u32).min(s.image.width().saturating_sub(1)),
                (point.y.floor().max(0.0) as u32).min(s.image.height().saturating_sub(1)),
            );
            let at = Some((index, pixel.0, pixel.1));
            if s.picker.at != at {
                // 换屏时模式条跟随光标所在屏重画。
                let moved_screen = s.picker.at.map(|(owner, _, _)| owner) != Some(index);
                s.picker.at = at;
                s.picker.copied = false;
                if s.picking() || moved_screen {
                    cx.notify();
                }
            }
        });
        let mut probe_at = None;
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
                    // 锁定比例时按比例拖出；未锁定时按住 Shift 拖出正方形。
                    s.selection = match s.ratio().or(event.modifiers.shift.then_some((1, 1))) {
                        Some(ratio) => selection::from_points_ratio(anchor, p, ratio, area),
                        None => selection::from_points(anchor, p, area),
                    };
                    s.hover = None;
                }
                (true, Some(Drag::Move { anchor, start })) => {
                    s.selection = Some(selection::moved(start, p.0 - anchor.0, p.1 - anchor.1, s.area()));
                }
                (true, Some(Drag::Resize { anchor, start, handle })) => {
                    // 锁定比例时保持该比例；未锁定时按住 Shift 保持开始拖动时的宽高比。
                    let (dx, dy) = (p.0 - anchor.0, p.1 - anchor.1);
                    let resized = match s.ratio().or(event.modifiers.shift.then_some((start.width, start.height))) {
                        Some(ratio) => selection::resized_ratio(start, handle, dx, dy, ratio, s.area()),
                        None => selection::resized(start, handle, dx, dy, s.area()),
                    };
                    if let Some(rect) = resized {
                        s.selection = Some(rect);
                    }
                }
                _ if s.selection.is_none() => {
                    probe_at = Some(p);
                    if !s.hover_at(p, event.modifiers.alt) {
                        return false;
                    }
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
        if let Some(p) = probe_at {
            schedule_probe(&self.session, p, cx);
        }
    }

    fn release(&mut self, _event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.update(cx, |s, _| s.toolbar_drag.take().is_some()) {
            return;
        }
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
            // 回放的截图不执行“截图后动作”。
            let after = (matches!(drag, Some(Drag::Create { .. })) && !s.recording && history::position(cx).is_none())
                .then(|| {
                    s.selection.map(|_| match s.purpose {
                        Some(RecognitionMode::Ocr) => "ocr".to_owned(),
                        Some(RecognitionMode::Translate) => "translate".to_owned(),
                        Some(RecognitionMode::Qr) => "qr".to_owned(),
                        None => s.after.clone(),
                    })
                })
                .flatten();
            (after, None)
        });
        self.run(job, cx);
        self.after_edit(window, cx);
        match after.as_deref() {
            Some("copy") => copy(cx, crate::native::hwnd(window)),
            Some("save") => save(cx),
            Some("pin") => pin(cx),
            Some("ocr") => recognize(cx, RecognitionMode::Ocr),
            Some("translate") => recognize(cx, RecognitionMode::Translate),
            Some("qr") => recognize(cx, RecognitionMode::Qr),
            _ => {}
        }
    }

    /// 右键：放弃未完成的笔画 → 退出标注模式 → 清除选区重新框选 → 退出截图。
    fn right_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.read(cx).size_editor.is_some() {
            self.close_size_editor(window, cx);
            return;
        }
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
        if self.session.read(cx).size_editor.is_some() {
            // 尺寸条打开时只处理应用、关闭与切换输入框；数字等其余按键交给输入框，不触发截图快捷键。
            match key {
                "enter" => self.apply_size_editor(true, window, cx),
                "escape" => self.close_size_editor(window, cx),
                "tab" => self.focus_size_field(if keystroke.modifiers.shift { -1 } else { 1 }, window, cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        // 取色快捷键：C 复制色值、P 切换全局/相对坐标（Shift 切换格式见 modifiers）。
        if !command && !keystroke.modifiers.alt && matches!(key, "c" | "p") && self.session.read(cx).picking() {
            self.session.update(cx, |s, cx| {
                if key == "c" {
                    s.copy_color(cx);
                } else {
                    s.picker.relative = !s.picker.relative;
                }
                cx.notify();
            });
            return;
        }
        // F5：重新拍屏刷新画面，保留选区与标注。
        if key == "f5" && !command && !keystroke.modifiers.alt {
            history::refresh(cx);
            return;
        }
        // , / .：截图历史上一张、下一张（越过最新一张回到现场）。
        if matches!(key, "," | ".") && !command && !keystroke.modifiers.alt {
            let direction = if key == "," { history::Direction::Previous } else { history::Direction::Next };
            history::replay(cx, direction);
            return;
        }
        // Tab / Shift+Tab：在鼠标下的吸附层级间向外 / 向内循环。
        if key == "tab" && !command && !keystroke.modifiers.alt {
            let delta = if keystroke.modifiers.shift { -1 } else { 1 };
            self.session.update(cx, |s, cx| {
                if s.selection.is_none() && s.drag.is_none() && s.step_level(delta, true) {
                    cx.notify();
                }
            });
            cx.stop_propagation();
            return;
        }
        if key == "a" && command && !keystroke.modifiers.alt {
            select_screen(cx, 0);
            return;
        }
        // R：恢复最近选区（已有标注或正在编辑时选区固定）。
        if key == "r" && !command && !keystroke.modifiers.alt {
            self.session.update(cx, |s, cx| {
                if s.selection_locked() || s.busy {
                    return;
                }
                match recent::restore(s.recent.as_ref(), s.image.width(), s.image.height(), &s.layout_key) {
                    Some(rect) => {
                        s.selection = Some(rect);
                        s.drag = None;
                        s.hover = None;
                        s.sync();
                        cx.notify();
                    }
                    None => log::info!("event=gpui.recent_selection_unavailable"),
                }
            });
            self.after_edit(window, cx);
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
        let recording = self.session.read(cx).recording;
        match key {
            "escape" => cancel(cx),
            "enter" if recording => record(cx, 0),
            "enter" => copy(cx, crate::native::hwnd(window)),
            "c" if command => copy(cx, crate::native::hwnd(window)),
            "s" if command && keystroke.modifiers.shift => quick_save(cx),
            "s" if command => save(cx),
            // 与工具栏“贴图 Ctrl/⌘+P”提示一致；录屏选区没有贴图按钮。
            "p" if command && !recording => pin(cx),
            // 方向键微调选区（已有标注时选区固定）：单按移动 1 像素，Shift 移动 10 像素；
            // Ctrl/⌘ 把方向键所指的边向外扩 1 像素，Ctrl/⌘+Shift 向内收 1 像素；再加 Alt 时对边同步（对称缩放）。
            "left" | "right" | "up" | "down" => {
                let side = match key {
                    "left" => selection::Side::Left,
                    "right" => selection::Side::Right,
                    "up" => selection::Side::Up,
                    _ => selection::Side::Down,
                };
                let shift = keystroke.modifiers.shift;
                let (nudge, step) = match (command, shift) {
                    (true, false) => (selection::Nudge::Grow, 1),
                    (true, true) => (selection::Nudge::Shrink, 1),
                    (false, true) => (selection::Nudge::Move, 10),
                    (false, false) => (selection::Nudge::Move, 1),
                };
                let symmetric = command && keystroke.modifiers.alt;
                self.session.update(cx, |s, cx| {
                    if s.selection_locked() || s.busy {
                        return;
                    }
                    if let Some(rect) = s.selection {
                        let area = s.area();
                        s.selection = Some(selection::nudged(rect, side, nudge, step, symmetric, area));
                        s.sync();
                        cx.notify();
                    }
                });
            }
            _ => {}
        }
    }

    /// 单独按下 Shift 切换色值格式；按下 Alt 时吸附切到整个窗口。
    fn modifiers(&mut self, event: &ModifiersChangedEvent, _: &mut Window, cx: &mut Context<Self>) {
        let shift = event.modifiers.shift;
        let only_shift = shift && !event.modifiers.control && !event.modifiers.alt && !event.modifiers.platform;
        self.session.update(cx, |s, cx| {
            if only_shift && s.picking() && s.drag.is_none() {
                s.picker.hex = !s.picker.hex;
                s.picker.copied = false;
                cx.notify();
            }
            if event.modifiers.alt
                && s.selection.is_none()
                && s.drag.is_none()
                && let Some(level) = window_level(&s.levels).filter(|&level| level != s.level)
            {
                let levels = std::mem::take(&mut s.levels);
                s.level_manual = levels.first().map(|l| l.rect);
                s.set_levels(levels, level);
                cx.notify();
            }
        });
    }

    /// 未框选时滚轮切换吸附层级：向上滚向外（更大），向下滚向内。
    /// 鼠标滚轮（按行滚动）每格一个事件，每个事件切一级，不管系统设置一格滚几行；
    /// 触控板（按像素滚动）累计满一格的距离切一级。
    fn wheel(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        const NOTCH: f32 = 40.0;
        self.session.update(cx, |s, cx| {
            if s.selection.is_some() || s.drag.is_some() || s.hover.is_none() {
                s.wheel = 0.0;
                return;
            }
            let step = match event.delta {
                ScrollDelta::Lines(lines) => {
                    s.wheel = 0.0;
                    if lines.y == 0.0 { 0 } else { lines.y.signum() as i32 }
                }
                ScrollDelta::Pixels(pixels) => {
                    s.wheel += f32::from(pixels.y);
                    let steps = (s.wheel / NOTCH).trunc();
                    s.wheel -= steps * NOTCH;
                    steps.clamp(-8.0, 8.0) as i32
                }
            };
            if step != 0 && s.step_level(step, false) {
                cx.notify();
            }
        });
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
                Action::Ui(ui) => {
                    e.ui(ui);
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

    /// 打开位置与尺寸条：四个只收数字的输入框，焦点放在宽度上并全选，直接输入即可替换。
    fn open_size_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = {
            let s = self.session.read(cx);
            match s.selection {
                Some(rect) if !s.busy && !s.selection_locked() && s.editor.text.is_none() => s.size_values(rect),
                _ => return,
            }
        };
        let fields = values.map(|value| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .validate(|text, _| text.len() <= 6 && text.chars().all(|c| c.is_ascii_digit()))
            })
        });
        fields[2].update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        let owner = self.index;
        self.session.update(cx, |s, cx| {
            s.size_editor = Some(SizeEditor { owner, fields });
            cx.notify();
        });
    }

    /// 应用输入框里的数值；`close` 时随后关闭尺寸条，否则把输入框同步为应用后的数值。
    fn apply_size_editor(&mut self, close: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fields) = self.session.read(cx).size_editor.as_ref().map(|e| e.fields.clone()) else {
            return;
        };
        let typed = fields.map(|field| field.read(cx).value().trim().parse::<i64>().ok());
        self.session.update(cx, |s, cx| {
            if s.apply_size(typed) {
                s.sync();
            }
            if close {
                s.size_editor = None;
            }
            cx.notify();
        });
        if close {
            window.focus(&self.focus, cx);
        } else {
            self.sync_size_fields(window, cx);
        }
    }

    fn close_size_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let closed = self.session.update(cx, |s, cx| {
            cx.notify();
            s.size_editor.take().is_some()
        });
        if closed {
            window.focus(&self.focus, cx);
        }
    }

    /// 比例、预设或单位改动后，把输入框改成当前选区的数值。
    fn sync_size_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let synced = {
            let s = self.session.read(cx);
            match (s.size_editor.as_ref(), s.selection) {
                (Some(editor), Some(rect)) => Some((editor.fields.clone(), s.size_values(rect))),
                _ => None,
            }
        };
        let Some((fields, values)) = synced else {
            return;
        };
        for (field, value) in fields.iter().zip(values) {
            field.update(cx, |state, cx| state.set_value(value.to_string(), window, cx));
        }
    }

    /// Tab / Shift+Tab 在 X、Y、宽、高之间循环，进入的输入框全选。
    fn focus_size_field(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fields) = self.session.read(cx).size_editor.as_ref().map(|e| e.fields.clone()) else {
            return;
        };
        let current = fields.iter().position(|field| field.read(cx).focus_handle(cx).is_focused(window));
        let next = current.map_or(0, |i| (i as i32 + delta).rem_euclid(fields.len() as i32) as usize);
        fields[next].update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
    }

    /// 锁定或解除比例。锁定时立即把选区改成该比例（保持左上角与宽度），之后拖动也保持它。
    fn set_ratio(&mut self, ratio: Option<Dimensions>, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            s.sizing.set_ratio(ratio);
            s.save_sizing();
            if let (Some(ratio), Some(rect)) = (s.ratio(), s.selection.filter(|_| !s.selection_locked())) {
                s.selection = Some(selection::with_ratio(rect, ratio, s.area()));
                s.sync();
            }
            cx.notify();
        });
        self.sync_size_fields(window, cx);
    }

    /// 套用尺寸预设：保持左上角，放不下时向内推、仍放不下时等比缩小。锁定了比例时改锁为预设的比例。
    fn apply_preset(&mut self, size: Dimensions, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            let Some(rect) = s.selection.filter(|_| !s.selection_locked() && !s.busy) else {
                return;
            };
            if s.sizing.ratio.is_some() {
                s.sizing.set_ratio(Some(size));
                s.save_sizing();
            }
            let (width, height) = (i64::from(size.width), i64::from(size.height));
            if let Some(next) = selection::placed(i64::from(rect.x), i64::from(rect.y), width, height, true, s.area()) {
                s.selection = Some(next);
                s.sync();
            }
            cx.notify();
        });
        self.sync_size_fields(window, cx);
    }

    /// 把当前选区尺寸存为预设（物理像素）。
    fn save_preset(&mut self, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            let Some(rect) = s.selection else {
                return;
            };
            if s.sizing.add_preset(Dimensions::new(rect.width, rect.height)) {
                s.save_sizing();
                cx.notify();
            }
        });
    }

    fn remove_preset(&mut self, size: Dimensions, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            if s.sizing.remove_preset(size) {
                s.save_sizing();
                cx.notify();
            }
        });
    }

    /// 在 px（导出图片像素）与 dp（按显示器缩放换算）之间切换尺寸条的单位。
    fn toggle_unit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            s.sizing.unit = match s.sizing.unit {
                Unit::Px => Unit::Dp,
                Unit::Dp => Unit::Px,
            };
            s.save_sizing();
            cx.notify();
        });
        self.sync_size_fields(window, cx);
    }
}

/// 选区尺寸标签的文字：当前单位下的宽高与左上角坐标，锁定比例时附上比例。
fn size_label(s: &Session, rect: Rect) -> String {
    let [x, y, width, height] = s.size_values(rect);
    let unit = match s.sizing.unit {
        Unit::Px => "px",
        Unit::Dp => "dp",
    };
    let mut label = format!("{width} × {height} {unit} · ({x}, {y})");
    if let Some((w, h)) = s.ratio() {
        label.push_str(&format!(" · {w}:{h}"));
    }
    label
}

/// 尺寸条上的文字小按钮（比例、预设、单位）。
fn size_chip(id: impl Into<ElementId>, label: String, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.0))
        .px(px(7.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(5.0))
        .text_xs()
        .cursor_pointer()
        .text_color(rgb(if on { annotate::active_fg() } else { annotate::text() }))
        .bg(rgb(if on { annotate::active_bg() } else { annotate::surface() }))
        .hover(|b| b.bg(rgb(annotate::hover())))
        .child(label)
}

/// 位置与尺寸条的大小：高度固定三行；预设一行不换行，预设多时按估算宽度加宽（行首标签与“+”另计）。
fn size_panel_size(presets: usize) -> Size<Pixels> {
    let presets_row = 14.0 + 52.0 + (presets + 1) as f32 * SIZE_PRESET_WIDTH;
    size(px(SIZE_PANEL_WIDTH.max(presets_row)), px(SIZE_PANEL_HEIGHT))
}

/// 位置与尺寸条的左上角：与选区左边对齐、不超出屏幕；依次尝试选区上方、选区下方、工具栏下方、工具栏上方，
/// 取第一个放得下且不压住选区和工具栏的位置；都不行时放进选区内左上角。
fn size_panel_origin(
    selection: Bounds<Pixels>,
    viewport: Size<Pixels>,
    panel: Size<Pixels>,
    toolbar: Option<Bounds<Pixels>>,
) -> Point<Pixels> {
    let gap = px(6.0);
    let left = selection.origin.x.min(viewport.width - panel.width - px(4.0)).max(px(4.0));
    let mut tops = vec![selection.origin.y - gap - panel.height, selection.origin.y + selection.size.height + gap];
    if let Some(toolbar) = toolbar {
        tops.push(toolbar.origin.y + toolbar.size.height + gap);
        tops.push(toolbar.origin.y - gap - panel.height);
    }
    let free = |top: Pixels| {
        let bounds = Bounds::new(point(left, top), panel);
        top >= px(4.0)
            && top + panel.height <= viewport.height - px(4.0)
            && !bounds.intersects(&selection)
            && toolbar.is_none_or(|toolbar| !bounds.intersects(&toolbar))
    };
    let top = tops.into_iter().find(|&top| free(top)).unwrap_or((selection.origin.y + gap).max(px(4.0)));
    point(left, top)
}

/// 位置与尺寸条：第一行输入 X、Y、宽、高与单位，第二行锁定比例，第三行套用 / 保存尺寸预设。
fn size_panel(s: &Session, editor: &SizeEditor, panel: Size<Pixels>, view: &Entity<Overlay>, window: &Window) -> Stateful<Div> {
    let field = |label: String, state: &Entity<InputState>| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .child(div().text_color(rgb(annotate::muted())).child(label))
            .child(
                div()
                    .w(px(62.0))
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(rgb(annotate::border()))
                    .bg(rgb(annotate::surface()))
                    .text_size(px(12.0))
                    .child(Input::new(state).appearance(false).small()),
            )
    };
    let unit = match s.sizing.unit {
        Unit::Px => "px",
        Unit::Dp => "dp",
    };
    let inputs = div()
        .h(px(28.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(field("X".into(), &editor.fields[0]))
        .child(field("Y".into(), &editor.fields[1]))
        .child(annotate::divider())
        .child(field(s.text("宽"), &editor.fields[2]))
        .child(field(s.text("高"), &editor.fields[3]))
        .child(
            size_chip("capture-size-unit", unit.into(), false)
                .tooltip({
                    let tip = s.text("切换单位：px 为导出图片的像素，dp 按显示器缩放换算");
                    move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into()
                })
                .on_click(window.listener_for(view, |this: &mut Overlay, _: &ClickEvent, window, cx| this.toggle_unit(window, cx))),
        )
        .child(div().flex_1())
        .child(
            annotate::icon_button("capture-size-apply", annotate::icons::CHECK, s.text("应用 Enter"), false, false)
                .on_click(window.listener_for(view, |this: &mut Overlay, _: &ClickEvent, window, cx| {
                    this.apply_size_editor(true, window, cx)
                })),
        )
        .child(
            annotate::icon_button("capture-size-close", annotate::icons::CLOSE, s.text("关闭 Esc"), false, false)
                .on_click(window.listener_for(view, |this: &mut Overlay, _: &ClickEvent, window, cx| {
                    this.close_size_editor(window, cx)
                })),
        );
    let mut ratios = div()
        .h(px(24.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .child(div().mr(px(4.0)).text_color(rgb(annotate::muted())).child(s.text("比例")))
        .child(
            size_chip("capture-ratio-free", s.text("自由"), s.sizing.ratio.is_none())
                .on_click(window.listener_for(view, |this: &mut Overlay, _: &ClickEvent, window, cx| this.set_ratio(None, window, cx))),
        );
    for (index, ratio) in sizing::RATIOS.into_iter().enumerate() {
        ratios = ratios.child(
            size_chip(("capture-ratio", index), format!("{}:{}", ratio.width, ratio.height), s.sizing.ratio == Some(ratio))
                .on_click(window.listener_for(view, move |this: &mut Overlay, _: &ClickEvent, window, cx| {
                    this.set_ratio(Some(ratio), window, cx)
                })),
        );
    }
    let current = s.selection.map(|rect| Dimensions::new(rect.width, rect.height));
    let mut presets = div()
        .h(px(24.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .child(div().mr(px(4.0)).text_color(rgb(annotate::muted())).child(s.text("预设")));
    let tip = s.text("单击套用，右键删除");
    for (index, preset) in s.sizing.presets.iter().copied().enumerate() {
        let tip = tip.clone();
        presets = presets.child(
            size_chip(("capture-preset", index), format!("{}×{}", preset.width, preset.height), current == Some(preset))
                .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into())
                .on_click(window.listener_for(view, move |this: &mut Overlay, _: &ClickEvent, window, cx| {
                    this.apply_preset(preset, window, cx)
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    window.listener_for(view, move |this: &mut Overlay, _: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.remove_preset(preset, cx)
                    }),
                ),
        );
    }
    let can_save = current.is_some_and(|size| !s.sizing.presets.contains(&size)) && s.sizing.presets.len() < sizing::MAX_PRESETS;
    if can_save {
        presets = presets.child(
            size_chip("capture-preset-add", "+".into(), false)
                .tooltip({
                    let tip = s.text("把当前尺寸存为预设");
                    move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into()
                })
                .on_click(window.listener_for(view, |this: &mut Overlay, _: &ClickEvent, _, cx| this.save_preset(cx))),
        );
    }
    div()
        .id("capture-size-panel")
        .absolute()
        .w(panel.width)
        .h(panel.height)
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(annotate::border()))
        .bg(rgb(annotate::surface()))
        .shadow_lg()
        .text_xs()
        .text_color(rgb(annotate::text()))
        .whitespace_nowrap()
        .cursor_default()
        // 尺寸条上的按下不能冒泡到浮层，否则会被当作新建选区或“点击框外应用”。
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(inputs)
        .child(ratios)
        .child(presets)
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
    let button = if primary {
        // 主操作（复制）：蓝底白色图标。
        div()
            .id(id)
            .size(px(30.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .bg(rgb(0x2563eb))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x1d4ed8)))
            .child(annotate::icon(icon, 0xffffff))
            .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(label.clone().into())).into())
    } else {
        annotate::icon_button(id, icon, label, false, false)
    };
    button
        .on_click(move |_, window, cx| action(cx, crate::native::hwnd(window)))
        .into_any_element()
}

/// 录屏框选时工具栏上方的录制选项：格式、帧率、倒计时、光标、键鼠特效、声音。
fn record_options(
    session: &Entity<Session>,
    p: &snap_core::recording::preferences::Preferences,
    window: Option<bool>,
    text: &dyn Fn(&'static str) -> String,
) -> AnyElement {
    use snap_core::recording::config::OutputFormat;
    let chip = |id: &'static str, label: String, on: bool, change: Box<dyn Fn(&mut snap_core::recording::preferences::Preferences)>| {
        let session = session.clone();
        div()
            .id(id)
            .h(px(24.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(5.0))
            .text_xs()
            .cursor_pointer()
            .text_color(rgb(if on { annotate::active_fg() } else { annotate::muted() }))
            .bg(rgb(if on { annotate::active_bg() } else { annotate::surface() }))
            .hover(|s| s.text_color(rgb(annotate::text())))
            .child(label)
            .on_click(move |_, _, cx| set_record(&session, cx, |p| change(p)))
            .into_any_element()
    };
    let gif = p.format == OutputFormat::Gif;
    let fps = if gif { [10, 20] } else { [30, 60] };
    let current_fps = p.fps();
    let mut row = div()
        .h(px(annotate::OPTIONS_HEIGHT))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(8.0))
        .bg(rgb(annotate::surface()))
        .border_1()
        .border_color(rgb(annotate::border()))
        .children(window.map(|on| {
            // 选区正好是吸附到的窗口：可改为录制整个窗口（跟随窗口移动）。
            let session = session.clone();
            div()
                .id("rec-window")
                .h(px(24.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(5.0))
                .text_xs()
                .cursor_pointer()
                .text_color(rgb(if on { annotate::active_fg() } else { annotate::muted() }))
                .bg(rgb(if on { annotate::active_bg() } else { annotate::surface() }))
                .hover(|s| s.text_color(rgb(annotate::text())))
                .child(text("整个窗口"))
                .on_click(move |_, _, cx| {
                    session.update(cx, |s, cx| {
                        s.record_window = !s.record_window;
                        cx.notify();
                    })
                })
        }))
        .children(window.map(|_| annotate::divider()))
        .child(chip("rec-mp4", "MP4".into(), !gif, Box::new(|p| p.format = OutputFormat::Mp4)))
        .child(chip("rec-gif", "GIF".into(), gif, Box::new(|p| p.format = OutputFormat::Gif)))
        .child(annotate::divider());
    for (index, value) in fps.into_iter().enumerate() {
        let id = ["rec-fps-low", "rec-fps-high"][index];
        row = row.child(chip(id, format!("{value} FPS"), current_fps == value, Box::new(move |p| {
            if p.format == OutputFormat::Gif { p.gif_fps = value } else { p.mp4_fps = value }
        })));
    }
    row = row.child(annotate::divider());
    for (id, seconds) in [("rec-countdown-0", 0u8), ("rec-countdown-3", 3), ("rec-countdown-5", 5)] {
        let label = if seconds == 0 { text("立即开始") } else { format!("{seconds}s") };
        row = row.child(chip(id, label, p.countdown == seconds, Box::new(move |p| p.countdown = seconds)));
    }
    let cursor = p.capture_cursor;
    let effects = p.effects.keyboard || p.effects.clicks;
    row = row
        .child(annotate::divider())
        .child(chip("rec-cursor", text("鼠标指针"), cursor, Box::new(move |p| p.capture_cursor = !cursor)))
        .child(chip("rec-effects", text("按键与点击"), effects, Box::new(move |p| {
            p.effects.keyboard = !effects;
            p.effects.clicks = !effects;
        })));
    if p.audio_supported() {
        let (system, mic) = (p.system_audio, p.microphone);
        row = row
            .child(annotate::divider())
            .child(chip("rec-system-audio", text("系统声音"), system, Box::new(move |p| p.system_audio = !system)))
            .child(chip("rec-microphone", text("麦克风"), mic, Box::new(move |p| p.microphone = !mic)));
    }
    row.into_any_element()
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
            .on_modifiers_changed(cx.listener(Self::modifiers))
            .on_scroll(cx.listener(Self::wheel))
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
                if !dragging {
                    root = root.child(level_label(s, b, viewport));
                }
            }
        }

        // 左上角模式条：截图历史上一张 / 下一张（录屏框选时不显示），只画在光标所在屏。
        let active = s.picker.at.map_or(0, |(owner, _, _)| owner);
        if active == self.index && !dragging {
            if selection.is_none() {
                root = root.child(capture_tip(s));
            }
            root = root.child(mode_bar(s, history::position(cx), history::has_entries(cx)));
        }
                // 取色放大镜画在最上层（工具栏之后追加），光标在本屏时才显示。
        let picker = s.picker;
        let loupe = picker.at.filter(|(owner, _, _)| *owner == self.index && s.picking()).map(|(_, x, y)| {
            let pointer = (
                (x as f32 + 0.5 - region.x as f32) / scale,
                (y as f32 + 0.5 - region.y as f32) / scale,
            );
            let origin = selection.map_or((0, 0), |r| (r.x, r.y));
            let text = |key: &'static str| s.text(key);
            magnifier::element(
                &s.image,
                &picker,
                (x, y),
                pointer,
                (f32::from(viewport.width), f32::from(viewport.height)),
                origin,
                &text,
            )
        });
        let Some(rect) = selection.filter(|_| visible.is_some()) else {
            return root.children(loupe);
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
        // 位置与尺寸条替代尺寸标签；这里只构建，等工具栏定好位置后再放（避开选区与工具栏），
        // 并且最后加入，画在选区手柄之上。
        let panel_size = size_panel_size(s.sizing.presets.len());
        let mut panel = s
            .size_editor
            .as_ref()
            .filter(|editor| editor.owner == self.index)
            .map(|editor| size_panel(s, editor, panel_size, &view, window));
        if panel.is_none() {
            // 尺寸标签：px 与导出图片一致，dp 按显示器缩放换算；单击打开位置与尺寸条。顶部放不下时放进选区内。
            let label_top = if b.origin.y >= px(30.0) { b.origin.y - px(28.0) } else { b.origin.y + px(6.0) };
            let mut label = div()
                .id("capture-size-label")
                .absolute()
                .left(b.origin.x)
                .top(label_top)
                .px(px(7.0))
                .py(px(3.0))
                .bg(rgba(0x242330dc))
                .text_size(px(11.0))
                .text_color(rgb(0xffffff))
                .whitespace_nowrap()
                .child(size_label(s, rect));
            if !s.selection_locked() && !s.busy {
                let tip = s.text("输入位置和尺寸");
                label = label
                    .cursor_pointer()
                    .hover(|label| label.bg(rgba(0x2563ebe6)))
                    .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into())
                    // 标签可能在选区内：按下不能冒泡成移动选区。
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(window.listener_for(&view, |this: &mut Self, _: &ClickEvent, window, cx| {
                        this.open_size_editor(window, cx)
                    }));
            }
            root = root.child(label);
        }
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
                        .border_color(rgb(HANDLE_BORDER)),
                );
            }
        }
        // 文字输入框：放在文字起点，宽度到选区右边缘，与最终排版的换行宽度一致。
        if let (Some(text), Some((owner, id, state))) = (s.editor.text.as_ref(), s.text_input.as_ref()) {
            if *owner == self.index && *id == text.id {
                let at = map(text.shape.start);
                let crop = s.editor.crop;
                let width = ((crop.x + crop.width - text.shape.start.x).min(2048.0) as f32 / scale).max(40.0);
                let font = text.shape.font_size.unwrap_or(24.0) as f32 / scale;
                let (color, background) = annotate::text_input_colors(&text.shape);
                root = root.child(
                    div()
                        .absolute()
                        .left(at.x)
                        .top(at.y)
                        .w(px(width))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(ACCENT))
                        .bg(rgba(background))
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
            if let Some(panel) = panel.take() {
                let origin = size_panel_origin(b, viewport, panel_size, None);
                root = root.child(panel.left(origin.x).top(origin.y));
            }
            return root.children(loupe);
        }
        let text = |key: &'static str| s.text(key);
        let grouped = !annotate::expanded();
        // 分组下拉默认向下弹出；选区下方放不下下拉时改为向上。
        let popover_up = b.origin.y + b.size.height + px(6.0 + annotate::ROW_HEIGHT + 150.0) > viewport.height;
        let props = ToolbarProps {
            editor: &s.editor,
            active: s.annotating,
            disabled: s.busy,
            compact: false,
            grouped,
            collapsible: true,
            popover_up,
            text: &text,
        };
        let host = annotate::Host { view: &view, window, dispatch: Self::action };
        let grip = annotate::icon_button("capture-grip", annotate::icons::GRIP, s.text("拖动工具栏；双击恢复位置"), false, false)
            .w(px(20.0))
            .cursor_grab()
            .on_mouse_down(MouseButton::Left, window.listener_for(&view, |this: &mut Self, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.session.update(cx, |s, _| {
                    if event.click_count >= 2 {
                        s.toolbar_offset = (0.0, 0.0);
                        s.toolbar_drag = None;
                    } else {
                        s.toolbar_drag = Some((event.position, s.toolbar_offset));
                    }
                });
                cx.notify();
            }));
        let leading = vec![
            grip.into_any_element(),
            annotate::icon_button("capture-select", annotate::icons::MOVE, s.text("调整选区"), !s.annotating, false)
                .on_click(window.listener_for(&view, Self::select_mode))
                .into_any_element(),
            annotate::divider().into_any_element(),
        ];
        let trailing = if s.recording {
            vec![
                export_button("capture-cancel", annotate::icons::CLOSE, s.text("取消"), false, |cx, _| cancel(cx)),
                export_button("capture-record-start", annotate::icons::VIDEO, s.text("开始录屏"), true, record),
            ]
        } else {
            // 顺序：重新框选、全屏、滚动截图、录屏 | 贴图、识别文字、截图翻译、二维码 / 条形码、保存、复制（主色）、取消。
            // 回放中的画面不是现场，不能开始滚动截图或录屏。
            let replaying = history::position(cx).is_some();
            let mut actions = vec![
                export_button("capture-reselect", annotate::icons::RESELECT, s.text("清空标注并重新框选"), false, reselect),
                export_button("capture-fullscreen", annotate::icons::MONITOR, s.text("选择整个屏幕 Ctrl/⌘+A"), false, select_screen),
            ];
            if !replaying && !s.selection_locked() && selected_native(s).is_some() {
                actions.push(export_button("capture-native", annotate::icons::APP_WINDOW, s.text("原生窗口截图"), false, native_window));
            }
            if !replaying {
                actions.push(export_button("capture-scroll", annotate::icons::SCROLL, s.text("滚动截图：框选正文后慢慢滚动"), false, scroll));
                actions.push(export_button("capture-record", annotate::icons::VIDEO, s.text("切换到录屏"), false, |cx, _| enter_recording(cx)));
            }
            actions.extend([
                export_button("capture-pin", annotate::icons::PIN, s.text("贴图 Ctrl/⌘+P"), false, |cx, _| pin(cx)),
                export_button("capture-ocr", annotate::icons::SCAN_TEXT, s.text("识别文字"), false, |cx, _| recognize(cx, RecognitionMode::Ocr)),
                export_button("capture-translate", annotate::icons::LANGUAGES, s.text("截图翻译"), false, |cx, _| recognize(cx, RecognitionMode::Translate)),
                export_button("capture-qr", annotate::icons::QR_CODE, s.text("识别二维码 / 条形码"), false, |cx, _| recognize(cx, RecognitionMode::Qr)),
                // 按住 Shift 单击为快速保存（与 Snipaste、PixPin 一致）。
                annotate::icon_button("capture-save", annotate::icons::SAVE, s.text("保存图片 Ctrl/⌘+S · 快速保存 Shift+单击 或 Ctrl/⌘+Shift+S"), false, false)
                    .on_click(|event: &ClickEvent, _, cx| if event.modifiers().shift { quick_save(cx) } else { save(cx) })
                    .into_any_element(),
                export_button("capture-copy", annotate::icons::COPY, s.text("复制图片 Enter"), true, copy),
                export_button("capture-cancel", annotate::icons::CLOSE, s.text("取消截图"), false, |cx, _| cancel(cx)),
            ]);
            actions
        };
        let tools = annotate::tool_row(&props, leading, trailing, &host);
        let options = annotate::options_row(&props, &host);
        let error = s.editor.error.clone();
        let window_choice = s.selected_window().map(|_| s.record_window);
        let record_row = s.recording.then(|| record_options(&self.session, &s.record, window_choice, &text));
        let height = annotate::ROW_HEIGHT
            + record_row.as_ref().map_or(0.0, |_| annotate::OPTIONS_HEIGHT + 6.0)
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
        let min_width = if grouped { 760.0 } else { 1040.0 };
        let mut toolbar = div()
            .id("capture-toolbar")
            .absolute()
            .top(top + px(s.toolbar_offset.1))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(6.0))
            .cursor_default()
            // 工具栏上的按下不能冒泡到浮层，否则会被当作新建选区或绘制。
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(record_row)
            .child(tools)
            .children(options)
            .children(error.map(annotate::error_row));
        let dx = px(s.toolbar_offset.0);
        // 工具栏的大致范围（宽度按最小排布宽度估算），供尺寸条避让。
        let toolbar_left = if right >= px(min_width) {
            toolbar = toolbar.right((viewport.width - right).max(px(4.0)) - dx);
            right.min(viewport.width - px(4.0)) + dx - px(min_width)
        } else {
            toolbar = toolbar.left(px(4.0) + dx).items_start();
            px(4.0) + dx
        };
        root = root.child(toolbar);
        if let Some(panel) = panel {
            let area = Bounds::new(point(toolbar_left, top + px(s.toolbar_offset.1)), size(px(min_width), px(height)));
            let origin = size_panel_origin(b, viewport, panel_size, Some(area));
            root = root.child(panel.left(origin.x).top(origin.y));
        }
        root.children(loupe)
    }
}

#[cfg(test)]
mod tests {
    // 不用 `super::*`：其中的 `gpui_kit::*` 带有 GPUI 自己的 `test` 宏，会遮住标准的 `#[test]`。
    use super::{size_panel_origin, size_panel_size, SIZE_PANEL_HEIGHT};
    use gpui_kit::{point, px, size, Bounds};

    #[test]
    fn size_panel_goes_above_then_below_avoiding_the_toolbar_then_inside() {
        let panel = size_panel_size(5);
        assert_eq!(panel.height, px(SIZE_PANEL_HEIGHT));
        let screen = size(px(1920.0), px(1080.0));
        // 上方放得下：贴在选区上方。
        let low = Bounds::new(point(px(200.0), px(300.0)), size(px(800.0), px(300.0)));
        assert_eq!(size_panel_origin(low, screen, panel, None), point(px(200.0), px(300.0 - 6.0 - SIZE_PANEL_HEIGHT)));
        // 上方放不下，选区下方被工具栏占着：排到工具栏下面。
        let tall = Bounds::new(point(px(20.0), px(90.0)), size(px(1000.0), px(667.0)));
        let toolbar = Bounds::new(point(px(260.0), px(763.0)), size(px(760.0), px(40.0)));
        assert_eq!(size_panel_origin(tall, screen, panel, Some(toolbar)), point(px(20.0), px(809.0)));
        // 哪里都放不下：放进选区内左上角。
        let short = size(px(1920.0), px(856.0));
        assert_eq!(size_panel_origin(tall, short, panel, Some(toolbar)), point(px(20.0), px(96.0)));
        // 靠近右边缘：向左收回屏幕内。
        let right = Bounds::new(point(px(1700.0), px(500.0)), size(px(200.0), px(200.0)));
        assert_eq!(size_panel_origin(right, screen, panel, None).x, px(1920.0 - 4.0) - panel.width);
    }
}
