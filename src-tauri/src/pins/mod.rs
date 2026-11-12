//! 内存贴图与独立窗口。原图保留用于恢复，所有导出取当前处理后的像素。
//! 窗口标签由 Rust 生成；贴图窗口只能操作自身，主窗口只能执行明确的管理动作。
#[cfg(test)]
pub(crate) mod benchmarks;
pub(crate) mod content;
pub(crate) mod drag;
pub mod html;
mod text;

use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Cursor,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

const MAX_PINS: usize = 12;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_TOTAL_PIXELS: u64 = 32_000_000;

/// 与 Vue 共用同一份布局尺寸，单位均为逻辑像素。
/// 窗口只包住图片；工具栏显示时才在图片上方或下方扩出一条，图片不让位、不被裁。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinLayout {
    padding: f64,
    gap: f64,
    border: f64,
    min_visible_side: f64,
    /// 工具栏单行加样式行的高度；显示时整体放在图片外侧。
    toolbar_height: f64,
    /// 工具栏完整显示所需宽度；仅在工具栏显示时撑宽窗口。
    toolbar_width: f64,
    /// 空闲窗口的透明下限，供右键菜单展开；多出的透明部分由 set_pin_region 裁掉。
    min_window_width: f64,
    min_window_height: f64,
}

/// 一次布局结果：窗口外框（物理像素）、图片首像素相对窗口的偏移、工具栏是否在图片上方。
#[derive(Debug, PartialEq)]
struct Placement {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    image: PhysicalPosition<i32>,
    toolbar_above: bool,
}

/// 显示器工作区，物理像素。
#[derive(Clone, Copy)]
struct WorkArea {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}
impl From<&tauri::Monitor> for WorkArea {
    fn from(monitor: &tauri::Monitor) -> Self {
        let area = monitor.work_area();
        Self {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width as i32,
            height: area.size.height as i32,
        }
    }
}

impl PinLayout {
    fn load() -> &'static Self {
        // 编译时嵌入仓库内常量，不读取用户文件；前端直接导入相同 JSON。
        // 内容在进程生命周期内不变，只解析一次；缩放、滚轮等高频命令直接复用。
        static LAYOUT: std::sync::OnceLock<PinLayout> = std::sync::OnceLock::new();
        LAYOUT.get_or_init(|| {
            serde_json::from_str(include_str!("../../../src/features/pins/pin-layout.json"))
                .expect("内置贴图布局配置无效")
        })
    }
    /// 与前端使用相同短边下限，原生命令也不能把小图缩成不可见的小点。
    fn minimum_zoom(&self, width: u32, height: u32, scale: f64) -> f64 {
        (self.min_visible_side * scale / f64::from(width.min(height).max(1))).max(0.1)
    }
    /// 图片首像素到窗口原点的物理距离；与前端 padding 取整规则一致。
    fn image_inset(&self, scale: f64) -> i32 {
        ((self.padding + self.border) * scale).round() as i32
    }
    /// 已测得的工具栏宽度优先；未测量时按布局常量预留。
    fn toolbar_width_for(&self, measured: f64) -> f64 {
        if measured > 0.0 {
            measured
        } else {
            self.toolbar_width
        }
    }
    /// 工具栏占用的高度（含与图片的间隙），物理像素。
    fn toolbar_band(&self, scale: f64) -> i32 {
        ((self.gap + self.toolbar_height) * scale).ceil() as i32
    }
    /// 新贴图默认原尺寸；放不进工作区（含为工具栏预留的一条）时等比缩小到能完整显示。
    fn fit_zoom(&self, width: u32, height: u32, scale: f64, work: WorkArea) -> f64 {
        let inset = f64::from(self.image_inset(scale) * 2);
        let room_width = f64::from(work.width) - inset;
        let room_height = f64::from(work.height - self.toolbar_band(scale)) - inset;
        let fit = (room_width / f64::from(width.max(1)))
            .min(room_height / f64::from(height.max(1)))
            .min(1.0);
        // 保留两位小数，滚轮按 10% 步进时仍从可读的比例出发。
        ((fit * 100.0).floor() / 100.0).max(self.minimum_zoom(width, height, scale))
    }
    /// 图片按缩放比例显示的物理尺寸。
    fn displayed(&self, width: u32, height: u32, zoom: f64) -> (i32, i32) {
        (
            (f64::from(width) * zoom).round().max(1.0) as i32,
            (f64::from(height) * zoom).round().max(1.0) as i32,
        )
    }

    /// 原位贴图：图片首像素对准选区，不按工作区裁小窗口。
    fn source_bounds(
        &self,
        origin: PhysicalPosition<i32>,
        width: u32,
        height: u32,
        scale: f64,
        zoom: f64,
    ) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
        let inset = self.image_inset(scale);
        let (width, height) = self.displayed(width, height, zoom);
        (
            PhysicalPosition::new(origin.x - inset, origin.y - inset),
            self.window_size(width, height, scale, None),
        )
    }

    /// `toolbar` 为显示中的工具栏宽度（逻辑像素），`None` 表示工具栏隐藏。
    fn window_size(
        &self,
        width: i32,
        height: i32,
        scale: f64,
        toolbar: Option<f64>,
    ) -> PhysicalSize<u32> {
        let inset = self.image_inset(scale);
        let content = if let Some(toolbar) = toolbar {
            width.max((toolbar * scale).ceil() as i32)
        } else {
            width
        };
        let band = if toolbar.is_some() {
            self.toolbar_band(scale)
        } else {
            0
        };
        PhysicalSize::new(
            (content + inset * 2).max((self.min_window_width * scale).ceil() as i32) as u32,
            (height + inset * 2 + band).max((self.min_window_height * scale).ceil() as i32) as u32,
        )
    }

    /// 以图片当前屏幕位置为锚点重新布局：图片保持在工作区内，工具栏优先放下方，
    /// 下方放不下放上方，两侧都放不下时才把图片上移；横向超出右缘时窗口向左展开。
    /// `image` 为图片首像素的屏幕坐标，`width`/`height` 为图片显示尺寸。
    fn place(
        &self,
        image: PhysicalPosition<i32>,
        width: i32,
        height: i32,
        scale: f64,
        work: WorkArea,
        toolbar: Option<f64>,
    ) -> Placement {
        let inset = self.image_inset(scale);
        let band = if toolbar.is_some() {
            self.toolbar_band(scale)
        } else {
            0
        };
        let (left, top) = (work.x, work.y);
        let (right, bottom) = (work.x + work.width, work.y + work.height);
        // 超出工作区的大图由前端在边框内滚动，显示区域按工作区收窄。
        let width = width.min(work.width - inset * 2).max(1);
        let height = height.min(work.height - inset * 2 - band).max(1);
        let x = image.x.clamp(left, left.max(right - width));
        let mut y = image.y.clamp(top, top.max(bottom - height));
        let size = self.window_size(width, height, scale, toolbar);
        let mut toolbar_above = false;
        if toolbar.is_some() && y + height + inset + band > bottom {
            if y - inset - band >= top {
                toolbar_above = true;
            } else {
                y = top.max(bottom - inset - band - height);
            }
        }
        let window_y = if toolbar_above {
            y - inset - band
        } else {
            y - inset
        };
        let mut window_x = x - inset;
        if window_x + size.width as i32 > right {
            window_x = window_x.min((left - inset).max(right - size.width as i32));
        }
        Placement {
            position: PhysicalPosition::new(window_x, window_y),
            size,
            image: PhysicalPosition::new(x - window_x, y - window_y),
            toolbar_above,
        }
    }
}

#[derive(Default)]
pub struct PinState {
    pub operation: AtomicBool,
    next_id: AtomicU64,
    images: Mutex<BTreeMap<String, Pin>>,
}

#[derive(Clone)]
struct Pin {
    /// 文字、文件路径及颜色保留语义内容；None 表示拖出当前合成图像。
    drag_text: Option<Arc<str>>,
    /// 二维码仅属于当前贴图；关闭释放，图片变换后失效。
    qr_result: Option<crate::recognition::qr::QrResult>,
    /// 只在像素变化时递增；视图和标注更新不要求重新编码底图。
    revision: u64,
    /// 当前 revision 的快速 PNG，供只读协议按需生成并在标注栅格化时复用；
    /// 克隆共享同一缓存，像素变换必须换成新缓存，窗口隐藏时释放。
    encoded: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    original: Arc<RgbaImage>,
    image: Arc<RgbaImage>,
    view: PinView,
    annotations: Vec<crate::shared::annotations::Annotation>,
    original_annotations: Vec<crate::shared::annotations::Annotation>,
}

impl Pin {
    /// 新贴图的原图/当前图共享 Arc；变换后才是两份独立 RGBA 缓冲。
    /// 这里只计 Rust 常驻像素，不把 WebView 解码、标注和操作临时副本伪装成已受限。
    fn retained_pixels(&self) -> u64 {
        let original = u64::from(self.original.width()) * u64::from(self.original.height());
        original
            + if Arc::ptr_eq(&self.original, &self.image) {
                0
            } else {
                u64::from(self.image.width()) * u64::from(self.image.height())
            }
    }

    /// 恢复原图直接共享原缓冲，不再次复制整张图片。
    fn transformed_image(&self, action: Transform) -> Arc<RgbaImage> {
        if matches!(action, Transform::Reset) {
            Arc::clone(&self.original)
        } else {
            Arc::new(transform(&self.image, &self.original, action))
        }
    }
}

/// 只记录数量与字节数，不记录图片、标注或文件名。
fn log_retained(images: &BTreeMap<String, Pin>, reason: &str) {
    let bytes: u64 = images.values().map(|pin| pin.retained_pixels() * 4).sum();
    log::info!(
        "event=pins.memory module=capture reason={} count={} rgba_bytes={}",
        reason,
        images.len(),
        bytes
    );
}

/// 预检变换后的常驻预算；所有现有变换保持像素总数，旋转只交换宽高。
fn validate_transform_budget(
    pin: &Pin,
    other_pixels: u64,
    action: Transform,
) -> Result<(), String> {
    let original = u64::from(pin.original.width()) * u64::from(pin.original.height());
    let next = original
        + if matches!(action, Transform::Reset) {
            0
        } else {
            u64::from(pin.image.width()) * u64::from(pin.image.height())
        };
    if other_pixels.saturating_add(next) > MAX_TOTAL_PIXELS {
        return Err("变换后原图与处理图将超过贴图像素缓存上限，请先关闭部分贴图。".into());
    }
    Ok(())
}

/// 全局贴图外观只影响显示；阴影由 WebView 按尺寸和焦点计算，原图像素保持独立。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PinAppearance {
    pub shadow: PinShadow,
    pub corner_radius: u8,
    /// 只接受 #RRGGBB，旧配置缺少字段时沿用默认中性灰。
    pub border_color: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinShadow {
    #[default]
    Auto,
    Soft,
    None,
}
impl Default for PinAppearance {
    fn default() -> Self {
        Self {
            shadow: PinShadow::Auto,
            corner_radius: 8,
            border_color: "#a1a1aa".into(),
        }
    }
}
impl PinAppearance {
    pub fn validate(&self) -> Result<(), String> {
        let color = self.border_color.as_bytes();
        if color.len() != 7 || color[0] != b'#' || !color[1..].iter().all(u8::is_ascii_hexdigit) {
            return Err("边框颜色需为六位十六进制颜色。".into());
        }
        if self.corner_radius > 24 {
            return Err("贴图圆角需为 0～24 像素。".into());
        }
        Ok(())
    }
}
fn appearance(app: &AppHandle) -> Result<PinAppearance, String> {
    let state = app.state::<crate::settings::SettingsState>();
    let current = state.0.lock().map_err(err)?;
    Ok(current.value.pin_appearance.clone())
}

/// zoom 使用逻辑像素比例；1 表示一图像像素对应一 CSS 像素。
/// opacity 仅影响显示，locked 防止拖动、缩放和图像处理，仍允许复制/保存/关闭。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinView {
    /// 穿透仅影响鼠标命中；锁定、透明度和图片像素保持独立。
    #[serde(default)]
    click_through: bool,
    zoom: f64,
    opacity: f64,
    locked: bool,
    topmost: bool,
    /// 工具栏显示时窗口在图片外侧扩出一条，不改变图片缩放比例和屏幕位置。
    toolbar_expanded: bool,
    /// 以下为原生布局结果，前端只读；回传的值会被忽略并重新计算。
    /// 图片首像素相对窗口左上角的物理像素偏移；0 表示尚未布局，按默认内边距处理。
    #[serde(default)]
    image_x: i32,
    #[serde(default)]
    image_y: i32,
    #[serde(default)]
    toolbar_above: bool,
    /// 前端实测的工具栏宽度（逻辑像素）；0 表示尚未测量，按布局常量预留。
    #[serde(default)]
    toolbar_width: f64,
}
impl Default for PinView {
    fn default() -> Self {
        Self {
            click_through: false,
            zoom: 1.0,
            opacity: 1.0,
            locked: false,
            topmost: true,
            toolbar_expanded: false,
            image_x: 0,
            image_y: 0,
            toolbar_above: false,
            toolbar_width: 0.0,
        }
    }
}
impl PinView {
    fn validate(&self) -> Result<(), String> {
        if !self.zoom.is_finite()
            // 上限依赖图片短边和 DPI，由 set_pin_view 在取得图片后钳制。
            || self.zoom < 0.1
            || !self.opacity.is_finite()
            || !(0.2..=1.0).contains(&self.opacity)
            || !self.toolbar_width.is_finite()
            || !(0.0..=4000.0).contains(&self.toolbar_width)
        {
            return Err("缩放比例无效，透明度应为 20%～100%。".into());
        }
        if cfg!(target_os = "macos") && self.opacity != 1.0 {
            return Err("macOS 当前未启用透明贴图窗口。".into());
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinFrame {
    label: String,
    revision: u64,
    width: u32,
    height: u32,
    view: PinView,
    appearance: PinAppearance,
    transparency_supported: bool,
    annotations: Vec<crate::shared::annotations::Annotation>,
}

/// 导出、图像变换与窗口管理串行执行；Drop 在错误提前返回时也会释放占用。
struct Operation<'a>(&'a AtomicBool);
impl<'a> Operation<'a> {
    fn acquire(state: &'a PinState) -> Result<Self, String> {
        state
            .operation
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "正在处理贴图，请稍候。".to_string())?;
        Ok(Self(&state.operation))
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn set_visible(window: &WebviewWindow, visible: bool) -> Result<(), tauri::Error> {
    if visible {
        window.show()?;
    } else {
        window.hide()?;
        // 前端隐藏五秒后才释放解码图；原生编码缓存立即释放，重新显示时按需再编码一次。
        if let Ok(images) = window.app_handle().state::<PinState>().images.lock() {
            if let Some(pin) = images.get(window.label()) {
                if let Ok(mut encoded) = pin.encoded.lock() {
                    *encoded = None;
                }
            }
        }
    }
    window
        .app_handle()
        .emit_to(window.label(), "pin-visibility", visible)
}

pub(crate) fn require_pin(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    peek(app, window, |_| ())
}

/// 在集合锁内只读出调用方需要的字段。`own` 会克隆两份标注，文字和滤镜标注带 Base64 像素
/// （合计可达数 MB）；拖动、缩放、区域裁剪等高频命令只看视图或图片尺寸，不应为此复制它们。
/// 闭包内不得调用窗口 API 或做耗时计算，锁只覆盖一次字段读取。
fn peek<T>(
    app: &AppHandle,
    window: &WebviewWindow,
    read: impl FnOnce(&Pin) -> T,
) -> Result<T, String> {
    app.state::<PinState>()
        .images
        .lock()
        .map_err(err)?
        .get(window.label())
        .map(read)
        .ok_or_else(|| "贴图已关闭或当前窗口无权访问。".into())
}

/// 视图状态与底图的轻量快照：图片只增加 Arc 引用计数，不复制像素。
struct PinGlance {
    view: PinView,
    image: Arc<RgbaImage>,
}
fn glance(app: &AppHandle, window: &WebviewWindow) -> Result<PinGlance, String> {
    peek(app, window, |pin| PinGlance {
        view: pin.view.clone(),
        image: Arc::clone(&pin.image),
    })
}

fn own(app: &AppHandle, window: &WebviewWindow) -> Result<Pin, String> {
    app.state::<PinState>()
        .images
        .lock()
        .map_err(err)?
        .get(window.label())
        .cloned()
        .ok_or_else(|| "贴图已关闭或当前窗口无权访问。".into())
}
/// 只返回元信息；底图由前端按 label 与 revision 经 `snap-image` 协议读取。
fn frame(label: &str, pin: &Pin) -> PinFrame {
    PinFrame {
        label: label.into(),
        revision: pin.revision,
        width: pin.image.width(),
        height: pin.image.height(),
        view: pin.view.clone(),
        appearance: PinAppearance::default(),
        transparency_supported: !cfg!(target_os = "macos"),
        annotations: pin.annotations.clone(),
    }
}

/// 协议读取当前版本底图：集合锁内只确认版本并取 Arc，编码在锁外进行；
/// 同一贴图的并发请求在缓存锁上排队，只编码一次。过期版本返回 None。
pub(crate) fn protocol_image(
    app: &AppHandle,
    label: &str,
    revision: u64,
) -> Result<Option<Arc<Vec<u8>>>, String> {
    let Some(state) = app.try_state::<PinState>() else {
        return Ok(None);
    };
    let found = state
        .images
        .lock()
        .map_err(err)?
        .get(label)
        .filter(|pin| pin.revision == revision)
        .map(|pin| (Arc::clone(&pin.image), Arc::clone(&pin.encoded)));
    found
        .map(|(image, encoded)| encoded_png(&image, &encoded))
        .transpose()
}

fn encoded_png(
    image: &RgbaImage,
    cache: &Mutex<Option<Arc<Vec<u8>>>>,
) -> Result<Arc<Vec<u8>>, String> {
    let mut cache = cache.lock().map_err(err)?;
    if let Some(png) = cache.as_ref() {
        return Ok(Arc::clone(png));
    }
    let png = crate::shared::image_protocol::fast_png(image)?;
    *cache = Some(Arc::clone(&png));
    Ok(png)
}

/// 变换/编码期间用户可能关闭窗口；只替换仍存在的贴图，禁止重新插入无窗口的大图。
fn commit_transformed_pin(
    images: &mut BTreeMap<String, Pin>,
    label: &str,
    pin: Pin,
) -> Result<(), String> {
    *images.get_mut(label).ok_or("贴图已关闭。")? = pin;
    Ok(())
}
fn validate_capacity(width: u32, height: u32, count: usize, used: u64) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_IMAGE_PIXELS {
        return Err("图片为空或超过 1600 万像素限制。".into());
    }
    if count >= MAX_PINS || used.saturating_add(pixels) > MAX_TOTAL_PIXELS {
        return Err(
            "贴图最多 12 张，原图与处理图的独立缓存合计最多 3200 万像素；请先关闭部分贴图。".into(),
        );
    }
    Ok(())
}

/// 必须从后台线程调用，避免 Windows WebView 创建阻塞 UI 线程。
/// 先登记图片再创建窗口，保证页面首次请求能读到数据；失败时回收登记。
pub fn create(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::shared::annotations::Annotation>,
    image_origin: Option<PhysicalPosition<i32>>,
) -> Result<WebviewWindow, String> {
    create_with_text(app, image, annotations, image_origin, None)
}

/// 内容类型在显示窗口前绑定，避免首次拖拽把文字误当图像导出。
fn create_with_text(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::shared::annotations::Annotation>,
    image_origin: Option<PhysicalPosition<i32>>,
    drag_text: Option<Arc<str>>,
) -> Result<WebviewWindow, String> {
    #[cfg(windows)]
    let _dpi = crate::capture::desktop::PhysicalDpi::enter()?;
    crate::shared::annotations::validate(&annotations, image.width(), image.height())?;
    let state = app.state::<PinState>();
    let _operation = Operation::acquire(&state)?;
    {
        let images = state.images.lock().map_err(err)?;
        let used = images.values().map(Pin::retained_pixels).sum();
        validate_capacity(image.width(), image.height(), images.len(), used)?;
    }
    let monitor = match image_origin {
        Some(origin) => app.monitor_from_point(f64::from(origin.x), f64::from(origin.y)),
        None => app.primary_monitor(),
    }
    .map_err(err)?
    .ok_or("无法获取显示器信息。")?;
    let layout = PinLayout::load();
    let scale = monitor.scale_factor();
    let work = WorkArea::from(&monitor);
    // 原位贴图保持选区原尺寸；其他来源默认原尺寸，放不下工作区时等比缩小到完整可见。
    let zoom = match image_origin {
        Some(_) => 1.0_f64.max(layout.minimum_zoom(image.width(), image.height(), scale)),
        None => layout.fit_zoom(image.width(), image.height(), scale, work),
    };
    let (shown_width, shown_height) = layout.displayed(image.width(), image.height(), zoom);
    // 非原位贴图在工作区居中；原位贴图建窗后按实际 DPI 重新对齐选区。
    let centered = layout.place(
        PhysicalPosition::new(
            work.x + (work.width - shown_width) / 2,
            work.y + (work.height - shown_height) / 2,
        ),
        shown_width,
        shown_height,
        scale,
        work,
        None,
    );
    let (width, height) = (
        f64::from(centered.size.width) / scale,
        f64::from(centered.size.height) / scale,
    );
    let label = format!("pin-{}", state.next_id.fetch_add(1, Ordering::SeqCst) + 1);
    let image = Arc::new(image);
    state.images.lock().map_err(err)?.insert(
        label.clone(),
        Pin {
            drag_text,
            qr_result: None,
            revision: 0,
            encoded: Default::default(),
            original: image.clone(),
            image,
            original_annotations: annotations.clone(),
            annotations,
            view: PinView {
                zoom,
                image_x: centered.image.x,
                image_y: centered.image.y,
                ..PinView::default()
            },
        },
    );
    let builder =
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html?window=pin".into()))
            .title(format!("Snap Desk · {label}"))
            .inner_size(width, height)
            .center()
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .visible(false);
    // macOS 的透明 WebView 依赖私有 API，本期保持普通背景，不引入该平台开关。
    #[cfg(not(target_os = "macos"))]
    let builder = builder.transparent(true);
    match crate::shared::dialog_windows::attach(builder, app, &label).build() {
        Ok(window) => {
            if let Some(origin) = image_origin {
                // 先进入来源显示器再读取实际 DPI；移动、定尺寸均在隐藏期间完成。
                // 屏幕坐标允许为负，不将副屏选区钳制到主屏工作区。
                let placed = (|| -> Result<(), String> {
                    window.set_position(origin).map_err(err)?;
                    let scale = window.scale_factor().map_err(err)?;
                    let images = state.images.lock().map_err(err)?;
                    let pin = images.get(&label).ok_or("贴图已关闭。")?;
                    let (position, size) = layout.source_bounds(
                        origin,
                        pin.image.width(),
                        pin.image.height(),
                        scale,
                        pin.view.zoom,
                    );
                    drop(images);
                    window.set_position(position).map_err(err)?;
                    window.set_size(size).map_err(err)?;
                    let inset = layout.image_inset(scale);
                    let mut images = state.images.lock().map_err(err)?;
                    let view = &mut images.get_mut(&label).ok_or("贴图已关闭。")?.view;
                    (view.image_x, view.image_y) = (inset, inset);
                    Ok(())
                })();
                if let Err(error) = placed {
                    let _ = window.destroy();
                    state.images.lock().map_err(err)?.remove(&label);
                    return Err(error);
                }
            }
            if image_origin.is_none() {
                // 物理像素定位，避免逻辑尺寸取整后图片偏离预先计算的偏移。
                let placed = window
                    .set_size(centered.size)
                    .and_then(|_| window.set_position(centered.position));
                if let Err(error) = placed {
                    let _ = window.destroy();
                    state.images.lock().map_err(err)?.remove(&label);
                    return Err(err(error));
                }
            }
            if let Ok(images) = state.images.lock() {
                log_retained(&images, "created");
            }
            Ok(window)
        }
        Err(error) => {
            state.images.lock().map_err(err)?.remove(&label);
            Err(err(error))
        }
    }
}

/// 关闭事件发生时移除原图及当前图，释放内存；不在持锁期间调用原生窗口 API。
pub fn destroyed(app: &AppHandle, label: &str) {
    if let Ok(mut images) = app.state::<PinState>().images.lock() {
        images.remove(label);
        log_retained(&images, "destroyed");
    }
}

/// 主窗口创建贴图共用显示与失败清理，隐藏窗口创建成功并不代表贴图已显示。
pub(crate) fn create_visible(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::shared::annotations::Annotation>,
) -> Result<(), String> {
    create_content_visible(app, image, annotations, None)
}

fn create_content_visible(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::shared::annotations::Annotation>,
    drag_text: Option<Arc<str>>,
) -> Result<(), String> {
    let pin = create_with_text(app, image, annotations, None, drag_text)?;
    if let Err(error) = pin.show().and_then(|_| pin.set_focus()) {
        let _ = pin.destroy();
        return Err(err(error));
    }
    Ok(())
}

/// 只允许主窗口唤起原生文件选择器；取消不创建贴图、不清除现有列表。
#[tauri::command]
pub async fn pin_file(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let image = {
            let state = app.state::<PinState>();
            let _operation = Operation::acquire(&state)?;
            with_hidden_pins(
                &app,
                None,
                || {
                    let path = app
                        .dialog()
                        .file()
                        .set_title(crate::shared::native_i18n::text(&app, "打开图片贴图"))
                        .add_filter(
                            crate::shared::native_i18n::text(&app, "图片（PNG / JPEG / WebP）"),
                            &["png", "jpg", "jpeg", "webp"],
                        )
                        .blocking_pick_file();
                    path.map(|path| {
                        crate::pins::content::read_image(&path.into_path().map_err(err)?)
                    })
                    .transpose()
                },
                |image| {
                    if image.is_some() {
                        "图片已读取，但尚未创建贴图"
                    } else {
                        "已取消打开图片"
                    }
                },
            )?
        };
        // create 会获取同一把操作锁，必须先结束对话框的临界区。
        let Some(image) = image else { return Ok(false) };
        create_visible(&app, image, vec![])?;
        Ok(true)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn pin_clipboard(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || clipboard_pin(&app))
        .await
        .map_err(err)?
}

/// 主界面、托盘和 F2 共用入口；优先读取图片，否则将剪贴板纯文字排版成贴图。
pub fn clipboard_pin(app: &AppHandle) -> Result<(), String> {
    if let Some(paths) = crate::clipboard::file_paths_for_pin()? {
        return create_text_visible(app, &paths.join("\n"));
    }
    let clipboard = match app.clipboard().read_image() {
        Ok(image) => image,
        Err(_) => {
            if let Some(html) = self::html::read()? {
                let text = app.clipboard().read_text().ok();
                return create_content_visible(
                    app,
                    self::html::render(app, html)?,
                    vec![],
                    text.map(|s| Arc::from(self::text::drag_text(&s))),
                );
            }
            let text = app
                .clipboard()
                .read_text()
                .map_err(|_| "剪贴板没有可读取的图片或文字。")?;
            return create_text_visible(app, &text);
        }
    };
    validate_capacity(clipboard.width(), clipboard.height(), 0, 0)?;
    let image = RgbaImage::from_raw(
        clipboard.width(),
        clipboard.height(),
        clipboard.rgba().to_vec(),
    )
    .ok_or("剪贴板图片数据无效。")?;
    create_visible(app, image, vec![])
}

/// 历史与即时剪贴板共用文字贴图入口，沿用窗口容量、显示及失败清理逻辑。
pub(crate) fn create_text_visible(app: &AppHandle, text: &str) -> Result<(), String> {
    create_content_visible(
        app,
        self::text::render(app, text)?,
        vec![],
        Some(Arc::from(self::text::drag_text(text))),
    )
}

/// 识别当前处理后的原始像素，不合成标注；坐标直接对应贴图原图像素。
/// 先取得 Arc 快照再进入阻塞线程，推理不持有贴图集合锁，也不接收任意文件路径。
#[tauri::command]
pub async fn recognize_pin_text(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<crate::recognition::OcrResult, String> {
    let image = glance(&app, &window)?.image;
    tauri::async_runtime::spawn_blocking(move || {
        let result = crate::recognition::recognize_selected(&app, &image)?;
        if !peek(&app, &window, |pin| Arc::ptr_eq(&pin.image, &image))? {
            return Err("图片已更新，请重新识别。".into());
        }
        Ok(result)
    })
    .await
    .map_err(err)?
}

/// 每张贴图独立保存最近结果，避免多窗口与截图识别互相覆盖。
#[tauri::command]
pub async fn recognize_pin_qr(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<crate::recognition::qr::QrResult, String> {
    static NEXT_RESULT: AtomicU64 = AtomicU64::new(1);
    let image = glance(&app, &window)?.image;
    tauri::async_runtime::spawn_blocking(move || {
        let result = crate::recognition::qr::QrResult::from_image(
            NEXT_RESULT.fetch_add(1, Ordering::Relaxed),
            &image,
        )?;
        let state = app.state::<PinState>();
        let mut images = state.images.lock().map_err(err)?;
        commit_pin_qr(&mut images, window.label(), &image, result.clone())?;
        Ok(result)
    })
    .await
    .map_err(err)?
}

/// 提交时再次确认贴图和图像身份；异步解码不能复活已关闭的贴图或污染新图。
fn commit_pin_qr(
    images: &mut BTreeMap<String, Pin>,
    label: &str,
    image: &Arc<RgbaImage>,
    result: crate::recognition::qr::QrResult,
) -> Result<(), String> {
    let pin = images.get_mut(label).ok_or("贴图已关闭。")?;
    if !Arc::ptr_eq(&pin.image, image) {
        return Err("图片已更新，请重新识别。".into());
    }
    pin.qr_result = Some(result);
    Ok(())
}

/// 链接须经界面明确确认；原生只接受本窗口最近结果的 ID 与索引。
#[tauri::command]
pub async fn pin_qr_action(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    index: usize,
    action: crate::recognition::qr::QrAction,
) -> Result<(), String> {
    let result = peek(&app, &window, |pin| pin.qr_result.clone())?.ok_or("请先识别二维码。")?;
    tauri::async_runtime::spawn_blocking(move || result.act(&app, id, index, action))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn current_pin(app: AppHandle, window: WebviewWindow) -> Result<PinFrame, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut result = frame(window.label(), &own(&app, &window)?);
        result.appearance = appearance(&app)?;
        Ok(result)
    })
    .await
    .map_err(err)?
}

/// 前端每完成一次编辑就同步结构化标注；锁定/导出中不允许覆盖，错误时保留旧数据。
#[tauri::command]
pub async fn set_pin_annotations(
    app: AppHandle,
    window: WebviewWindow,
    annotations: Vec<crate::shared::annotations::Annotation>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let pin = glance(&app, &window)?;
        if pin.view.locked {
            return Err("请先解锁贴图再编辑标注。".into());
        }
        crate::shared::annotations::validate(&annotations, pin.image.width(), pin.image.height())?;
        state
            .images
            .lock()
            .map_err(err)?
            .get_mut(window.label())
            .ok_or("贴图已关闭。")?
            .annotations = annotations;
        Ok(())
    })
    .await
    .map_err(err)?
}

/// 导出只在底图副本上合成，复制/保存不改变可编辑对象，也不破坏“恢复原图”。
fn composed(pin: &Pin) -> Result<RgbaImage, String> {
    let mut image = (*pin.image).clone();
    crate::shared::annotations::render(
        &mut image,
        &pin.annotations,
        crate::shared::annotations::Point { x: 0.0, y: 0.0 },
    )?;
    Ok(image)
}

/// 托盘和快捷键无需命中贴图本身；先恢复系统命中，再同步窗口内的视图状态。
fn restore_one(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    window.set_ignore_cursor_events(false).map_err(err)?;
    let view = {
        let state = app.state::<PinState>();
        let mut images = state.images.lock().map_err(err)?;
        let pin = images.get_mut(window.label()).ok_or("贴图已关闭。")?;
        pin.view.click_through = false;
        pin.view.clone()
    };
    app.emit_to(window.label(), "pin-view-changed", view)
        .map_err(err)
}

/// 全局贴图键优先作为穿透逃生键；即使禁用全局快捷键，托盘仍可恢复。
pub fn restore_click_through(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<PinState>();
    let labels: Vec<_> = state
        .images
        .lock()
        .map_err(err)?
        .iter()
        .filter(|(_, pin)| pin.view.click_through)
        .map(|(label, _)| label.clone())
        .collect();
    if labels.is_empty() {
        return Ok(false);
    }
    let _operation = Operation::acquire(&state)?;
    let mut failures = Vec::new();
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(error) = restore_one(app, &window) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        Ok(true)
    } else {
        Err(failures.join("；"))
    }
}

#[tauri::command]
pub async fn set_pin_view(
    app: AppHandle,
    window: WebviewWindow,
    mut view: PinView,
) -> Result<PinView, String> {
    view.validate()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let pin = glance(&app, &window)?;
        if pin.view.locked && view.zoom != pin.view.zoom {
            return Err("请先解锁贴图再缩放。".into());
        }
        let monitor = window
            .current_monitor()
            .map_err(err)?
            .ok_or("无法获取显示器信息。")?;
        let scale = monitor.scale_factor();
        let layout = PinLayout::load();
        let minimum = layout.minimum_zoom(pin.image.width(), pin.image.height(), scale);
        // 通常最多 400%；仅极细图片允许超过此值，以满足 20px 的可见下限。
        view.zoom = view.zoom.clamp(minimum, 4.0_f64.max(minimum));
        let old_size = window.inner_size().map_err(err)?;
        let old_position = window.outer_position().map_err(err)?;
        let inset = layout.image_inset(scale);
        let offset = |value: i32| if value > 0 { value } else { inset };
        // 以图片当前的屏幕位置为锚点，显示/隐藏工具栏和缩放都不挪动图片。
        let anchor = PhysicalPosition::new(
            old_position.x + offset(pin.view.image_x),
            old_position.y + offset(pin.view.image_y),
        );
        let (shown_width, shown_height) =
            layout.displayed(pin.image.width(), pin.image.height(), view.zoom);
        let placement = layout.place(
            anchor,
            shown_width,
            shown_height,
            scale,
            WorkArea::from(&monitor),
            view.toolbar_expanded
                .then(|| layout.toolbar_width_for(view.toolbar_width)),
        );
        window.set_always_on_top(view.topmost).map_err(err)?;
        // 置顶、锁定和透明度不能顺带重排原位贴图；旋转改变图像宽高时仍需重新适配。
        let source_size = layout
            .source_bounds(
                PhysicalPosition::new(0, 0),
                pin.image.width(),
                pin.image.height(),
                scale,
                view.zoom,
            )
            .1;
        let needs_resize = view.zoom != pin.view.zoom
            || view.toolbar_expanded != pin.view.toolbar_expanded
            || (view.toolbar_expanded && view.toolbar_width != pin.view.toolbar_width)
            || (old_size != placement.size && old_size != source_size);
        (view.image_x, view.image_y, view.toolbar_above) =
            (pin.view.image_x, pin.view.image_y, pin.view.toolbar_above);
        let resized = if needs_resize {
            (view.image_x, view.image_y, view.toolbar_above) = (
                placement.image.x,
                placement.image.y,
                placement.toolbar_above,
            );
            window
                .set_size(placement.size)
                .and_then(|_| window.set_position(placement.position))
        } else {
            Ok(())
        };
        if let Err(error) = resized {
            let mut failures = Vec::new();
            for result in [
                window.set_size(old_size),
                window.set_position(old_position),
                window.set_always_on_top(pin.view.topmost),
            ] {
                if let Err(e) = result {
                    failures.push(err(e));
                }
            }
            return Err(if failures.is_empty() {
                err(error)
            } else {
                format!("{error}；恢复窗口失败：{}", failures.join("；"))
            });
        }
        window
            .set_ignore_cursor_events(view.click_through)
            .map_err(err)?;
        state
            .images
            .lock()
            .map_err(err)?
            .get_mut(window.label())
            .ok_or("贴图已关闭。")?
            .view = view.clone();
        Ok(view)
    })
    .await
    .map_err(err)?
}

/// 空闲贴图只保留图片（含阴影边距）区域，其余透明部分交给系统直接命中下层窗口；
/// `None` 恢复完整窗口，供工具栏、菜单和识别面板使用。坐标为相对窗口左上角的物理像素。
#[tauri::command]
pub async fn set_pin_region(
    app: AppHandle,
    window: WebviewWindow,
    region: Option<crate::capture::Rect>,
) -> Result<(), String> {
    require_pin(&app, &window)?;
    // SetWindowRgn 同步向窗口线程发消息，不能在 IPC 线程池以外的主线程回调里调用。
    tauri::async_runtime::spawn_blocking(move || apply_region(&window, region))
        .await
        .map_err(err)?
}

#[cfg(windows)]
fn apply_region(
    window: &WebviewWindow,
    region: Option<crate::capture::Rect>,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ},
    };
    let hwnd = HWND(window.hwnd().map_err(err)?.0);
    unsafe {
        let Some(rect) = region else {
            return if SetWindowRgn(hwnd, None, true) == 0 {
                Err("无法恢复贴图窗口区域。".into())
            } else {
                Ok(())
            };
        };
        let size = window.inner_size().map_err(err)?;
        let right = rect.x.saturating_add(rect.width).min(size.width);
        let bottom = rect.y.saturating_add(rect.height).min(size.height);
        if rect.x >= right || rect.y >= bottom {
            return Err("贴图交互区域无效。".into());
        }
        let handle = CreateRectRgn(rect.x as i32, rect.y as i32, right as i32, bottom as i32);
        if handle.0.is_null() {
            return Err("无法创建贴图交互区域。".into());
        }
        // 成功后区域归系统所有；失败时自行释放，避免泄漏 GDI 对象。
        if SetWindowRgn(hwnd, Some(handle), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(handle.0));
            return Err("无法设置贴图交互区域。".into());
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn apply_region(_: &WebviewWindow, _: Option<crate::capture::Rect>) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub fn drag_pin(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    if app.state::<PinState>().operation.load(Ordering::SeqCst) {
        return Err("正在处理贴图，请稍候。".into());
    }
    // 同步命令在主线程执行，只读锁定标志，不克隆标注。
    if peek(&app, &window, |pin| pin.view.locked)? {
        return Err("贴图已锁定。".into());
    }
    window.start_dragging().map_err(err)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transform {
    RotateRight,
    RotateLeft,
    FlipHorizontal,
    FlipVertical,
    Grayscale,
    Invert,
    Reset,
}
/// 变换只作用于图片像素；反色保留 alpha，恢复原图会同时清除之前的全部变换。
fn transform(image: &RgbaImage, original: &RgbaImage, action: Transform) -> RgbaImage {
    match action {
        Transform::RotateRight => image::imageops::rotate90(image),
        Transform::RotateLeft => image::imageops::rotate270(image),
        Transform::FlipHorizontal => image::imageops::flip_horizontal(image),
        Transform::FlipVertical => image::imageops::flip_vertical(image),
        Transform::Reset => original.clone(),
        Transform::Grayscale | Transform::Invert => {
            let mut result = image.clone();
            for pixel in result.pixels_mut() {
                if matches!(action, Transform::Invert) {
                    for channel in &mut pixel.0[..3] {
                        *channel = 255 - *channel;
                    }
                } else {
                    let gray = ((u32::from(pixel[0]) * 299
                        + u32::from(pixel[1]) * 587
                        + u32::from(pixel[2]) * 114
                        + 500)
                        / 1000) as u8;
                    pixel.0[..3].fill(gray);
                }
            }
            result
        }
    }
}
/// 整图变换同步变换标注坐标及字形/滤镜像素，保留可编辑对象而不是合并图层。
fn transform_annotations(
    shapes: &[crate::shared::annotations::Annotation],
    width: u32,
    height: u32,
    action: Transform,
) -> Result<Vec<crate::shared::annotations::Annotation>, String> {
    use crate::shared::annotations::{Kind, Point};
    let map = |p: Point| -> Point {
        match action {
            Transform::RotateRight => Point {
                x: f64::from(height) - p.y,
                y: p.x,
            },
            Transform::RotateLeft => Point {
                x: p.y,
                y: f64::from(width) - p.x,
            },
            Transform::FlipHorizontal => Point {
                x: f64::from(width) - p.x,
                y: p.y,
            },
            Transform::FlipVertical => Point {
                x: p.x,
                y: f64::from(height) - p.y,
            },
            _ => p,
        }
    };
    shapes
        .iter()
        .map(|source| {
            let mut s = source.clone();
            s.start = map(s.start);
            s.end = map(s.end);
            s.points = s.points.into_iter().map(map).collect();
            if matches!(
                s.kind,
                Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify
            ) {
                let a = s.start;
                let b = s.end;
                s.start = Point {
                    x: a.x.min(b.x),
                    y: a.y.min(b.y),
                };
                s.end = Point {
                    x: a.x.max(b.x),
                    y: a.y.max(b.y),
                };
                let raster = crate::shared::annotations::raster(source)?;
                let mut png = Cursor::new(Vec::new());
                transform(&raster, &raster, action)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(err)?;
                s.raster = Some(STANDARD.encode(png.into_inner()));
            }
            if matches!(action, Transform::Grayscale | Transform::Invert) {
                let value = u32::from_str_radix(&s.color[1..], 16).map_err(err)?;
                let pixel = RgbaImage::from_pixel(
                    1,
                    1,
                    image::Rgba([(value >> 16) as u8, (value >> 8) as u8, value as u8, 255]),
                );
                let converted = transform(&pixel, &pixel, action);
                let p = converted.get_pixel(0, 0);
                s.color = format!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2]);
            }
            Ok(s)
        })
        .collect()
}

#[tauri::command]
pub async fn transform_pin(
    app: AppHandle,
    window: WebviewWindow,
    action: Transform,
) -> Result<PinFrame, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let mut pin = own(&app, &window)?;
        if pin.view.locked {
            return Err("请先解锁贴图再处理图片。".into());
        }
        {
            let images = state.images.lock().map_err(err)?;
            let other_pixels = images
                .iter()
                .filter(|(label, _)| label.as_str() != window.label())
                .map(|(_, value)| value.retained_pixels())
                .sum();
            validate_transform_budget(&pin, other_pixels, action)?;
        }
        if matches!(action, Transform::Reset) {
            pin.annotations = pin.original_annotations.clone();
        } else {
            pin.annotations = transform_annotations(
                &pin.annotations,
                pin.image.width(),
                pin.image.height(),
                action,
            )?;
        }
        pin.qr_result = None;
        pin.image = pin.transformed_image(action);
        pin.encoded = Default::default();
        crate::shared::annotations::validate(
            &pin.annotations,
            pin.image.width(),
            pin.image.height(),
        )?;
        pin.revision = pin.revision.checked_add(1).ok_or("贴图版本超出限制。")?;
        let mut result = frame(window.label(), &pin);
        result.appearance = appearance(&app)?;
        commit_transformed_pin(&mut *state.images.lock().map_err(err)?, window.label(), pin)?;
        if let Ok(images) = state.images.lock() {
            log_retained(&images, "transformed");
        }
        Ok(result)
    })
    .await
    .map_err(err)?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinAction {
    Copy,
    Save,
    Close,
}

/// 打开/保存期间临时隐藏所有可见贴图，防止其他置顶贴图盖住系统对话框。
/// 仅恢复本次隐藏的窗口；失败也走恢复路径。
fn with_hidden_pins<T>(
    app: &AppHandle,
    focus: Option<&WebviewWindow>,
    task: impl FnOnce() -> Result<T, String>,
    describe: impl FnOnce(&T) -> &'static str,
) -> Result<T, String> {
    let mut hidden = Vec::new();
    let result = (|| {
        for pin in app
            .webview_windows()
            .into_values()
            .filter(|w| w.label().starts_with("pin-"))
        {
            if pin.is_visible().map_err(err)? {
                set_visible(&pin, false).map_err(err)?;
                hidden.push(pin);
            }
        }
        task()
    })();
    let mut restore_errors = Vec::new();
    for pin in hidden {
        if let Err(error) = set_visible(&pin, true) {
            restore_errors.push(format!("{}: {error}", pin.label()));
        }
    }
    if let Some(window) = focus {
        if let Err(error) = window.set_focus() {
            restore_errors.push(err(error));
        }
    }
    if restore_errors.is_empty() {
        return result;
    }
    let status = match result {
        Ok(value) => describe(&value).to_string(),
        Err(error) => error,
    };
    Err(format!(
        "{status}；恢复贴图窗口失败：{}",
        restore_errors.join("；")
    ))
}

fn save_with_hidden_pins(
    app: &AppHandle,
    focus: Option<&WebviewWindow>,
    image: &RgbaImage,
) -> Result<bool, String> {
    with_hidden_pins(
        app,
        focus,
        || crate::shared::image_io::save(app, image),
        |saved| {
            if *saved {
                "图片已保存"
            } else {
                "已取消保存"
            }
        },
    )
}

/// 截图保存也需避让已有置顶贴图；获取同一操作锁，避免保存期间发生批量显示。
pub fn save_capture_image(app: &AppHandle, image: &RgbaImage) -> Result<bool, String> {
    let state = app.state::<PinState>();
    let _operation = Operation::acquire(&state)?;
    save_with_hidden_pins(app, None, image)
}

#[tauri::command]
pub async fn pin_action(
    app: AppHandle,
    window: WebviewWindow,
    action: PinAction,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        match action {
            PinAction::Copy => {
                crate::shared::image_io::copy(&app, &composed(&own(&app, &window)?)?)?
            }
            PinAction::Save => {
                return save_with_hidden_pins(&app, Some(&window), &composed(&own(&app, &window)?)?)
            }
            // 关闭只需确认贴图仍属于本窗口，不为此克隆图片和标注。
            PinAction::Close => {
                require_pin(&app, &window)?;
                window.destroy().map_err(err)?
            }
        }
        Ok(true)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_pin_keeps_image_origin_on_primary_negative_and_scaled_displays() {
        let layout = PinLayout::load();
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for origin in [
                PhysicalPosition::new(0, 0),
                PhysicalPosition::new(320, 180),
                PhysicalPosition::new(-1920, -600),
            ] {
                let inset = layout.image_inset(scale);
                let (position, size) = layout.source_bounds(origin, 1233, 626, scale, 1.0);
                assert_eq!(position.x + inset, origin.x);
                assert_eq!(position.y + inset, origin.y);
                // 空闲窗口只包住图片，不为工具栏预留宽度或高度。
                assert_eq!(size.width as i32, 1233 + inset * 2);
                assert_eq!(size.height as i32, 626 + inset * 2);
            }
        }
        // 小图在最小窗口内也从左上角开始，不因原生最小尺寸额外居中。
        let origin = PhysicalPosition::new(-10, 15);
        let (position, _) = layout.source_bounds(origin, 60, 40, 1.5, 1.0);
        assert_eq!(position.x + layout.image_inset(1.5), origin.x);
        assert_eq!(position.y + layout.image_inset(1.5), origin.y);
    }
    #[test]
    fn frame_carries_metadata_only_and_encoding_is_cached_per_revision() {
        let pin = sample_pin();
        let json = serde_json::to_value(frame("pin-1", &pin)).unwrap();
        assert!(json.get("dataUrl").is_none());
        assert_eq!(json["revision"], 0);
        let first = encoded_png(&pin.image, &pin.encoded).unwrap();
        assert!(first.starts_with(b"\x89PNG"));
        assert!(Arc::ptr_eq(
            &first,
            &encoded_png(&pin.image, &pin.encoded).unwrap()
        ));
        // 克隆共享缓存；变换换成新缓存后不会读到旧像素。
        let mut transformed = pin.clone();
        assert!(transformed.encoded.lock().unwrap().is_some());
        transformed.encoded = Default::default();
        assert!(transformed.encoded.lock().unwrap().is_none());
        assert!(pin.encoded.lock().unwrap().is_some());
    }
    #[test]
    fn qr_results_stay_with_the_unchanged_live_pin() {
        let mut images = BTreeMap::from([
            ("pin-1".into(), sample_pin()),
            ("pin-2".into(), sample_pin()),
        ]);
        let source = images["pin-1"].image.clone();
        commit_pin_qr(
            &mut images,
            "pin-1",
            &source,
            crate::recognition::qr::QrResult::default(),
        )
        .unwrap();
        assert!(images["pin-1"].qr_result.is_some());
        assert!(images["pin-2"].qr_result.is_none());
        let replacement = Arc::new(RgbaImage::new(1, 1));
        images.get_mut("pin-1").unwrap().image = replacement;
        assert!(commit_pin_qr(
            &mut images,
            "pin-1",
            &source,
            crate::recognition::qr::QrResult::default()
        )
        .is_err());
        images.remove("pin-1");
        assert!(commit_pin_qr(
            &mut images,
            "pin-1",
            &source,
            crate::recognition::qr::QrResult::default()
        )
        .is_err());
        assert_eq!(images.len(), 1);
    }

    #[test]
    fn closed_pin_is_not_resurrected_by_an_in_flight_transform() {
        let mut images = BTreeMap::new();
        let pin = sample_pin();
        let original = Arc::downgrade(&pin.original);
        images.insert("pin-1".into(), pin.clone());
        let mut transformed = pin;
        transformed.image = transformed.transformed_image(Transform::Invert);
        let pixels = Arc::downgrade(&transformed.image);
        // 模拟变换拿到 Arc 快照后，窗口销毁回调先移除了管理表中的贴图。
        images.remove("pin-1");
        assert!(commit_transformed_pin(&mut images, "pin-1", transformed).is_err());
        assert!(images.is_empty());
        assert!(pixels.upgrade().is_none());
        assert!(original.upgrade().is_none());
    }

    #[test]
    fn open_pin_accepts_transform_and_releases_previous_pixels() {
        let mut images = BTreeMap::new();
        images.insert("pin-1".into(), sample_pin());
        let previous = Arc::downgrade(&images["pin-1"].image);
        let mut replacement = sample_pin();
        replacement.revision = 7;
        commit_transformed_pin(&mut images, "pin-1", replacement).unwrap();
        assert_eq!(images["pin-1"].revision, 7);
        assert!(previous.upgrade().is_none());
    }

    #[test]
    fn pixel_budget_counts_unique_buffers_and_reset_reuses_original() {
        let mut pin = sample_pin();
        assert_eq!(pin.retained_pixels(), 1);
        pin.image = pin.transformed_image(Transform::Invert);
        assert!(!Arc::ptr_eq(&pin.image, &pin.original));
        assert_eq!(pin.retained_pixels(), 2);
        let transformed = Arc::downgrade(&pin.image);
        pin.image = pin.transformed_image(Transform::Reset);
        assert!(Arc::ptr_eq(&pin.image, &pin.original));
        assert_eq!(pin.retained_pixels(), 1);
        assert!(transformed.upgrade().is_none());
    }

    #[test]
    fn transformation_budget_allows_reset_but_rejects_extra_buffer_at_limit() {
        let pin = sample_pin();
        assert!(validate_transform_budget(&pin, MAX_TOTAL_PIXELS - 1, Transform::Invert).is_err());
        assert!(validate_transform_budget(&pin, MAX_TOTAL_PIXELS - 1, Transform::Reset).is_ok());
        assert!(
            validate_transform_budget(&pin, MAX_TOTAL_PIXELS - 2, Transform::RotateRight).is_ok()
        );
        assert!(Arc::ptr_eq(&pin.image, &pin.original));
    }

    fn sample_pin() -> Pin {
        let image = Arc::new(RgbaImage::new(1, 1));
        Pin {
            drag_text: None,
            qr_result: None,
            revision: 0,
            encoded: Default::default(),
            original: image.clone(),
            image,
            view: PinView::default(),
            annotations: vec![],
            original_annotations: vec![],
        }
    }
    #[test]
    fn effects_survive_pin_rotation_and_flip() {
        let mut png = Cursor::new(Vec::new());
        let pixels = RgbaImage::from_fn(2, 3, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 60, 20, 255])
        });
        pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let lens: crate::shared::annotations::Annotation =
            serde_json::from_value(serde_json::json!({
                "kind":"magnify", "start":{"x":10,"y":5}, "end":{"x":30,"y":25},
                "color":"#ff0000", "width":3, "magnification":3,
                "raster":STANDARD.encode(png.into_inner())
            }))
            .unwrap();
        let spotlight: crate::shared::annotations::Annotation =
            serde_json::from_value(serde_json::json!({
                "kind":"spotlight", "start":{"x":10,"y":5}, "end":{"x":30,"y":25},
                "color":"#000000", "width":3, "opacity":0.7
            }))
            .unwrap();
        let source = vec![lens, spotlight];
        let rotated = transform_annotations(&source, 100, 50, Transform::RotateRight).unwrap();
        crate::shared::annotations::validate(&rotated, 50, 100).unwrap();
        assert_eq!(rotated[0].magnification, Some(3.0));
        assert_eq!(
            crate::shared::annotations::raster(&rotated[0]).unwrap(),
            image::imageops::rotate90(&pixels)
        );
        assert_eq!(rotated[1].start.x, 45.0);
        assert_eq!(rotated[1].end.x, 25.0);
        let flipped = transform_annotations(&source, 100, 50, Transform::FlipHorizontal).unwrap();
        crate::shared::annotations::validate(&flipped, 100, 50).unwrap();
        assert_eq!(
            crate::shared::annotations::raster(&flipped[0]).unwrap(),
            image::imageops::flip_horizontal(&pixels)
        );
        assert!(flipped[1].raster.is_none());
    }

    #[test]
    fn rotating_and_flipping_keep_annotations_editable_and_in_bounds() {
        let s: crate::shared::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"pen", "start":{"x":10,"y":5}, "end":{"x":30,"y":20},
            "color":"#ff0000", "width":3, "points":[{"x":10,"y":5},{"x":20,"y":25},{"x":30,"y":20}]
        }))
        .unwrap();
        let source = vec![s];
        let rotated = transform_annotations(&source, 100, 50, Transform::RotateRight).unwrap();
        crate::shared::annotations::validate(&rotated, 50, 100).unwrap();
        assert_eq!(rotated[0].start.x, 45.0);
        assert_eq!(rotated[0].points[1].y, 20.0);
        let restored = transform_annotations(&rotated, 50, 100, Transform::RotateLeft).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(&source).unwrap()
        );
        let flipped = transform_annotations(&source, 100, 50, Transform::FlipHorizontal).unwrap();
        assert_eq!(flipped[0].start.x, 90.0);
        let inverted = transform_annotations(&source, 100, 50, Transform::Invert).unwrap();
        assert_eq!(inverted[0].color, "#00ffff");
    }
    #[test]
    fn exporting_annotations_does_not_flatten_the_editable_source() {
        let s: crate::shared::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"line", "start":{"x":2,"y":10}, "end":{"x":25,"y":10}, "color":"#ff0000", "width":3
        })).unwrap();
        let image = Arc::new(RgbaImage::new(30, 30));
        let pin = Pin {
            drag_text: None,
            qr_result: None,
            revision: 0,
            encoded: Default::default(),
            original: image.clone(),
            image,
            view: PinView::default(),
            annotations: vec![s],
            original_annotations: vec![],
        };
        let output = composed(&pin).unwrap();
        assert!(output.get_pixel(10, 10)[3] > 0);
        assert_eq!(pin.image.get_pixel(10, 10)[3], 0);
        assert_eq!(pin.annotations.len(), 1);
    }
    const WORK: WorkArea = WorkArea {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    #[test]
    fn toolbar_sits_outside_the_image_and_never_moves_it() {
        let layout = PinLayout::load();
        let inset = layout.image_inset(1.0);
        let band = layout.toolbar_band(1.0);
        let image = PhysicalPosition::new(300, 200);
        let hidden = layout.place(image, 800, 400, 1.0, WORK, None);
        assert_eq!(hidden.image, PhysicalPosition::new(inset, inset));
        assert_eq!(
            hidden.size,
            PhysicalSize::new(800 + inset as u32 * 2, 400 + inset as u32 * 2)
        );
        // 下方放得下：窗口向下扩出一条，图片屏幕位置不变。
        let below = layout.place(image, 800, 400, 1.0, WORK, Some(layout.toolbar_width));
        assert!(!below.toolbar_above);
        assert_eq!(below.position.x + below.image.x, 300);
        assert_eq!(below.position.y + below.image.y, 200);
        assert_eq!(below.size.height as i32, 400 + inset * 2 + band);
        // 贴近底部：工具栏翻到上方，图片仍不动。
        let low = PhysicalPosition::new(300, 1040 - 400 - 4);
        let above = layout.place(low, 800, 400, 1.0, WORK, Some(layout.toolbar_width));
        assert!(above.toolbar_above);
        assert_eq!(above.position.y + above.image.y, low.y);
        assert_eq!(above.image.y, inset + band);
        // 小图靠右：窗口为工具栏向左展开，不越过工作区右缘。
        let right = PhysicalPosition::new(1800, 200);
        let narrow = layout.place(right, 100, 80, 1.0, WORK, Some(layout.toolbar_width));
        assert_eq!(narrow.position.x + narrow.image.x, 1800);
        assert!(narrow.position.x + narrow.size.width as i32 <= WORK.width);
        assert!(narrow.size.width as i32 >= layout.toolbar_width as i32);
    }
    #[test]
    fn tall_images_fit_the_work_area_with_room_for_the_toolbar() {
        let layout = PinLayout::load();
        for scale in [1.0, 1.25, 1.5] {
            let zoom = layout.fit_zoom(640, 1340, scale, WORK);
            assert!(zoom < 1.0);
            let (width, height) = layout.displayed(640, 1340, zoom);
            let inset = layout.image_inset(scale);
            assert!(height + inset * 2 + layout.toolbar_band(scale) <= WORK.height);
            // 显示工具栏时整张图仍完整可见，只把图片上移让出一条。
            let top = PhysicalPosition::new(100, 0);
            let shown = layout.place(top, width, height, scale, WORK, Some(layout.toolbar_width));
            // 透明内边距允许越过屏幕边缘，图片和工具栏本身留在工作区内。
            let image_top = shown.position.y + shown.image.y;
            assert!(image_top >= 0);
            assert!(image_top + height + layout.toolbar_band(scale) <= WORK.height);
        }
        assert_eq!(layout.fit_zoom(800, 600, 1.0, WORK), 1.0);
        // 极细图片仍满足 20px 的可见下限。
        assert!(layout.fit_zoom(2, 100_000, 1.0, WORK) >= layout.minimum_zoom(2, 100_000, 1.0));
    }
    #[test]
    fn idle_window_keeps_room_for_the_context_menu() {
        let layout = PinLayout::load();
        let tiny = layout.place(PhysicalPosition::new(50, 50), 20, 20, 1.0, WORK, None);
        assert_eq!(tiny.size.width, layout.min_window_width as u32);
        assert_eq!(tiny.size.height, layout.min_window_height as u32);
        assert_eq!(
            tiny.image,
            PhysicalPosition::new(layout.image_inset(1.0), layout.image_inset(1.0))
        );
    }
    #[test]
    fn rejects_empty_oversized_and_excess_pins() {
        assert!(validate_capacity(0, 1, 0, 0).is_err());
        assert!(validate_capacity(u32::MAX, u32::MAX, 0, 0).is_err());
        assert!(validate_capacity(1, 1, MAX_PINS, 0).is_err());
        assert!(validate_capacity(1, 1, 0, MAX_TOTAL_PIXELS).is_err());
        assert!(validate_capacity(4000, 4000, 1, 16_000_000).is_ok());
    }
    #[test]
    fn transformations_preserve_alpha_and_restore_original() {
        let image = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 7, 0, 100, 200, 99]).unwrap();
        let flipped = transform(&image, &image, Transform::FlipHorizontal);
        assert_eq!(flipped.get_pixel(0, 0), image.get_pixel(1, 0));
        let rotated = transform(&image, &image, Transform::RotateRight);
        assert_eq!(rotated.dimensions(), (1, 2));
        assert_eq!(transform(&rotated, &image, Transform::RotateLeft), image);
        assert_eq!(transform(&flipped, &image, Transform::Reset), image);
        assert_eq!(
            transform(&image, &image, Transform::Invert)
                .get_pixel(0, 0)
                .0,
            [0, 255, 255, 7]
        );
        assert_eq!(
            transform(&image, &image, Transform::Grayscale)
                .get_pixel(0, 0)
                .0,
            [76, 76, 76, 7]
        );
    }
    #[test]
    fn rejects_invalid_display_values_and_releases_operation() {
        assert!(PinView {
            zoom: f64::NAN,
            ..PinView::default()
        }
        .validate()
        .is_err());
        assert!(PinView {
            opacity: 0.0,
            ..PinView::default()
        }
        .validate()
        .is_err());
        let state = PinState::default();
        let guard = Operation::acquire(&state).unwrap();
        assert!(Operation::acquire(&state).is_err());
        drop(guard);
        assert!(Operation::acquire(&state).is_ok());
    }
    #[test]
    fn pin_appearance_defaults_and_validation_keep_old_settings_compatible() {
        let value: PinAppearance = serde_json::from_str("{}").unwrap();
        assert_eq!(value, PinAppearance::default());
        assert!(PinAppearance {
            corner_radius: 25,
            ..value
        }
        .validate()
        .is_err());
        assert!(serde_json::from_str::<PinAppearance>(r#"{"shadow":"unknown"}"#).is_err());
    }
}
