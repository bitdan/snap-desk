//! 内存贴图与独立窗口。原图保留用于恢复，所有导出取当前处理后的像素。
//! 窗口标签由 Rust 生成；贴图窗口只能操作自身，主窗口只能执行明确的管理动作。
#[cfg(test)]
mod benchmarks;
pub(crate) mod content;
mod text;
pub mod html;

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
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

const MAX_PINS: usize = 12;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_TOTAL_PIXELS: u64 = 32_000_000;

/// 与 Vue 共用同一份布局尺寸，单位均为逻辑像素，避免 DPI 下底栏挤占图片。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinLayout {
    padding: f64,
    gap: f64,
    border: f64,
    expanded_height: f64,
    collapsed_height: f64,
    min_visible_side: f64,
    min_image_width: f64,
    min_image_height: f64,
}
impl PinLayout {
    fn load() -> Self {
        // 编译时嵌入仓库内常量，不读取用户文件；前端直接导入相同 JSON。
        serde_json::from_str(include_str!("../../../src/features/pins/pin-layout.json"))
            .expect("内置贴图布局配置无效")
    }
    /// 与前端使用相同短边下限，原生命令也不能把小图缩成不可见的小点。
    fn minimum_zoom(&self, width: u32, height: u32, scale: f64) -> f64 {
        (self.min_visible_side * scale / f64::from(width.min(height).max(1))).max(0.1)
    }
    /// 图片首像素到窗口原点的物理距离；与前端 padding 取整规则一致。
    fn image_inset(&self, scale: f64) -> i32 {
        ((self.padding + self.border) * scale).round() as i32
    }

    /// 原位贴图不按工作区裁小窗口，避免底栏挤占选区或将大图缩放、居中。
    fn source_bounds(
        &self, origin: PhysicalPosition<i32>, width: u32, height: u32, scale: f64, zoom: f64,
    ) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
        let inset = self.image_inset(scale);
        let width = (f64::from(width) * zoom).max(self.min_image_width * scale);
        let height = (f64::from(height) * zoom).max(self.min_image_height * scale);
        (
            PhysicalPosition::new(origin.x - inset, origin.y - inset),
            PhysicalSize::new(
                (width + f64::from(inset) * 2.0).ceil() as u32,
                (height + f64::from(inset) * 2.0 + (self.gap + self.expanded_height) * scale).ceil() as u32,
            ),
        )
    }
    fn chrome_height(&self, expanded: bool) -> f64 {
        self.padding * 2.0
            + self.gap
            + self.border * 2.0
            + if expanded {
                self.expanded_height
            } else {
                self.collapsed_height
            }
    }
    fn window_size(
        &self,
        width: u32,
        height: u32,
        zoom: f64,
        max_width: f64,
        max_height: f64,
        expanded: bool,
    ) -> (f64, f64) {
        let width = (f64::from(width) * zoom).max(self.min_image_width)
            + (self.padding + self.border) * 2.0;
        let height =
            (f64::from(height) * zoom).max(self.min_image_height) + self.chrome_height(expanded);
        (width.min(max_width), height.min(max_height))
    }
}

#[derive(Default)]
pub struct PinState {
    pub operation: AtomicBool,
    next_id: AtomicU64,
    images: Mutex<BTreeMap<String, Pin>>,
    /// 管理页缩略图按像素版本缓存，显示/隐藏和分组更新不重复缩放原图。
    thumbnails: Mutex<BTreeMap<String, (u64, String)>>,
}

#[derive(Clone)]
struct Pin {
    /// 二维码仅属于当前贴图；关闭释放，图片变换后失效。
    qr_result: Option<crate::recognition::qr::QrResult>,
    /// 只在像素变化时递增；视图和标注更新不要求重新编码底图。
    revision: u64,
    original: Arc<RgbaImage>,
    image: Arc<RgbaImage>,
    view: PinView,
    annotations: Vec<crate::shared::annotations::Annotation>,
    original_annotations: Vec<crate::shared::annotations::Annotation>,
    /// 空字符串表示未分组；与图片一样仅保留到本次应用退出。
    group: String,
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

/// 只记录数量与字节数，不记录图片、标注、分组或文件名。
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
        return Err(
            "变换后原图与处理图将超过贴图像素缓存上限，请先关闭部分贴图（隐藏不会释放缓存）。"
                .into(),
        );
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
    /// 展开/收起底栏同时调整原生窗口高度，不改变图片缩放比例。
    toolbar_expanded: bool,
}
impl Default for PinView {
    fn default() -> Self {
        Self {
            click_through: false,
            zoom: 1.0,
            opacity: 1.0,
            locked: false,
            topmost: true,
            toolbar_expanded: true,
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
    data_url: String,
    view: PinView,
    appearance: PinAppearance,
    transparency_supported: bool,
    annotations: Vec<crate::shared::annotations::Annotation>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinSummary {
    click_through: bool,
    label: String,
    width: u32,
    height: u32,
    visible: bool,
    group: String,
    thumbnail: String,
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
    }
    window
        .app_handle()
        .emit_to(window.label(), "pin-visibility", visible)
}

fn changed(app: &AppHandle) {
    let _ = app.emit_to("main", "pins-changed", ());
}
pub(crate) fn require_pin(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    own(app, window).map(|_| ())
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
fn frame(label: &str, pin: &Pin, known_revision: Option<u64>) -> Result<PinFrame, String> {
    // 客户端仍持有此版本时仅更新元信息；不在 Rust 再常驻一份 Base64 缓存。
    let data_url = if known_revision == Some(pin.revision) {
        String::new()
    } else {
        let mut png = Cursor::new(Vec::new());
        pin.image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(err)?;
        // 直接写入最终字符串，避免 base64 临时字符串与 format! 结果同时驻留。
        let png = png.into_inner();
        let prefix = "data:image/png;base64,";
        let mut data_url = String::with_capacity(prefix.len() + png.len().div_ceil(3) * 4);
        data_url.push_str(prefix);
        STANDARD.encode_string(&png, &mut data_url);
        data_url
    };
    Ok(PinFrame {
        label: label.into(),
        revision: pin.revision,
        width: pin.image.width(),
        height: pin.image.height(),
        data_url,
        view: pin.view.clone(),
        appearance: PinAppearance::default(),
        transparency_supported: !cfg!(target_os = "macos"),
        annotations: pin.annotations.clone(),
    })
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
        return Err("贴图最多 12 张，原图与处理图的独立缓存合计最多 3200 万像素；请先关闭部分贴图（隐藏不会释放缓存）。".into());
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
    }.map_err(err)?.ok_or("无法获取显示器信息。")?;
    let layout = PinLayout::load();
    // 不再额外裁掉 20% 工作区：能完整显示的选区不应被迫出现滚动条。
    let max_width = f64::from(monitor.work_area().size.width) / monitor.scale_factor();
    let max_height = f64::from(monitor.work_area().size.height) / monitor.scale_factor();
    // 默认原尺寸；极小图片等比放大到短边 20px，超出屏幕的部分滚动查看。
    let zoom = PinView::default().zoom.max(layout.minimum_zoom(
        image.width(),
        image.height(),
        monitor.scale_factor(),
    ));
    let (width, height) = layout.window_size(
        image.width(),
        image.height(),
        // window_size 使用逻辑尺寸，zoom 本身始终表示物理像素的查看比例。
        zoom / monitor.scale_factor(),
        max_width,
        max_height,
        true,
    );
    let label = format!("pin-{}", state.next_id.fetch_add(1, Ordering::SeqCst) + 1);
    let image = Arc::new(image);
    state.images.lock().map_err(err)?.insert(
        label.clone(),
        Pin {
            qr_result: None,
            revision: 0,
            original: image.clone(),
            image,
            original_annotations: annotations.clone(),
            group: String::new(),
            annotations,
            view: PinView {
                zoom,
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
    match builder.build() {
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
                        origin, pin.image.width(), pin.image.height(), scale, pin.view.zoom,
                    );
                    drop(images);
                    window.set_position(position).map_err(err)?;
                    window.set_size(size).map_err(err)
                })();
                if let Err(error) = placed {
                    let _ = window.destroy();
                    state.images.lock().map_err(err)?.remove(&label);
                    return Err(error);
                }
            }
            if let Ok(images) = state.images.lock() {
                log_retained(&images, "created");
            }
            changed(app);
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
    if let Ok(mut thumbnails) = app.state::<PinState>().thumbnails.lock() {
        thumbnails.remove(label);
    }
    changed(app);
}

/// 主窗口创建贴图共用显示与失败清理，隐藏窗口创建成功并不代表贴图已显示。
pub(crate) fn create_visible(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::shared::annotations::Annotation>,
) -> Result<(), String> {
    let pin = create(app, image, annotations, None)?;
    if let Err(error) = pin.show().and_then(|_| pin.set_focus()) {
        let _ = pin.destroy();
        return Err(err(error));
    }
    changed(app);
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
    let clipboard = match app.clipboard().read_image() {
        Ok(image) => image,
        Err(_) => {
            if let Some(html) = self::html::read()? {
                return create_visible(app, self::html::render(app, html)?, vec![]);
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
    create_visible(app, self::text::render(app, text)?, vec![])
}

/// 识别当前处理后的原始像素，不合成标注；坐标直接对应贴图原图像素。
/// 先取得 Arc 快照再进入阻塞线程，推理不持有贴图集合锁，也不接收任意文件路径。
#[tauri::command]
pub async fn recognize_pin_text(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<crate::recognition::OcrResult, String> {
    let image = own(&app, &window)?.image;
    tauri::async_runtime::spawn_blocking(move || {
        let result = crate::recognition::provider("local-ppocr")?.recognize(&app, &image)?;
        let current = own(&app, &window)?;
        if !Arc::ptr_eq(&current.image, &image) {
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
    let image = own(&app, &window)?.image;
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
    let result = own(&app, &window)?.qr_result.ok_or("请先识别二维码。")?;
    tauri::async_runtime::spawn_blocking(move || result.act(&app, id, index, action))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn current_pin(
    app: AppHandle,
    window: WebviewWindow,
    known_revision: Option<u64>,
) -> Result<PinFrame, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut result = frame(window.label(), &own(&app, &window)?, known_revision)?;
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
        let pin = own(&app, &window)?;
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

#[tauri::command]
pub fn list_pins(app: AppHandle, window: WebviewWindow) -> Result<Vec<PinSummary>, String> {
    crate::capture::require_window(&window, "main")?;
    let state = app.state::<PinState>();
    let images = state.images.lock().map_err(err)?.clone();
    let mut thumbnails = state.thumbnails.lock().map_err(err)?;
    thumbnails.retain(|label, _| images.contains_key(label));
    images
        .into_iter()
        .map(|(label, pin)| {
            let visible = app
                .get_webview_window(&label)
                .map(|w| w.is_visible())
                .transpose()
                .map_err(err)?
                .unwrap_or(false);
            // 管理页只传最多 96×72 的底图缩略图，缓存最多随 12 张贴图保留。
            let thumbnail = match thumbnails.get(&label) {
                Some((revision, value)) if *revision == pin.revision => value.clone(),
                _ => {
                    let preview = image::imageops::thumbnail(pin.image.as_ref(), 96, 72);
                    let mut png = Cursor::new(Vec::new());
                    preview
                        .write_to(&mut png, image::ImageFormat::Png)
                        .map_err(err)?;
                    let value = format!(
                        "data:image/png;base64,{}",
                        STANDARD.encode(png.into_inner())
                    );
                    thumbnails.insert(label.clone(), (pin.revision, value.clone()));
                    value
                }
            };
            Ok(PinSummary {
                click_through: pin.view.click_through,
                label,
                width: pin.image.width(),
                height: pin.image.height(),
                visible,
                group: pin.group,
                thumbnail,
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManageAction {
    Interactive,
    Show,
    Hide,
    Close,
}
#[tauri::command]
pub async fn manage_pins(
    app: AppHandle,
    window: WebviewWindow,
    label: Option<String>,
    group: Option<String>,
    action: ManageAction,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let labels = management_labels(&*state.images.lock().map_err(err)?, label, group)?;
        // 即使部分窗口操作失败，也刷新管理列表，让已经成功的操作可见。
        let mut errors = Vec::new();
        for label in labels {
            if let Some(pin) = app.get_webview_window(&label) {
                let result = match action {
                    ManageAction::Interactive => restore_one(&app, &pin),
                    ManageAction::Show => set_visible(&pin, true).and_then(|_| pin.set_focus()).map_err(err),
                    ManageAction::Hide => set_visible(&pin, false).map_err(err),
                    ManageAction::Close => pin.destroy().map_err(err),
                };
                if let Err(e) = result {
                    errors.push(format!("{label}: {e}"));
                }
            }
        }
        changed(&app);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    })
    .await
    .map_err(err)?
}

/// 分组名称不对应文件路径；只限制长度和控制字符，支持中文名称。
fn normalize_group(group: &str) -> Result<String, String> {
    let group = group.trim();
    if group.chars().count() > 32 || group.chars().any(char::is_control) {
        return Err("分组名称最多 32 个字符，且不能包含换行或控制字符。".into());
    }
    Ok(group.to_string())
}

/// 管理页和托盘无需命中贴图本身；先恢复系统命中，再同步窗口内的视图状态。
fn restore_one(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    window.set_ignore_cursor_events(false).map_err(err)?;
    let view = {
        let state = app.state::<PinState>();
        let mut images = state.images.lock().map_err(err)?;
        let pin = images.get_mut(window.label()).ok_or("贴图已关闭。")?;
        pin.view.click_through = false;
        pin.view.clone()
    };
    app.emit_to(window.label(), "pin-view-changed", view).map_err(err)
}

/// 全局贴图键优先作为穿透逃生键；即使禁用全局快捷键，托盘和管理页仍可恢复。
pub fn restore_click_through(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<PinState>();
    let labels: Vec<_> = state.images.lock().map_err(err)?.iter()
        .filter(|(_, pin)| pin.view.click_through).map(|(label, _)| label.clone()).collect();
    if labels.is_empty() { return Ok(false); }
    let _operation = Operation::acquire(&state)?;
    let mut failures = Vec::new();
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(error) = restore_one(app, &window) { failures.push(error); }
        }
    }
    changed(app);
    if failures.is_empty() { Ok(true) } else { Err(failures.join("；")) }
}

fn management_labels(
    images: &BTreeMap<String, Pin>,
    label: Option<String>,
    group: Option<String>,
) -> Result<Vec<String>, String> {
    if label.is_some() && group.is_some() {
        return Err("单张贴图与分组不能同时指定。".into());
    }
    if let Some(label) = label {
        return if images.contains_key(&label) {
            Ok(vec![label])
        } else {
            Err("贴图已关闭。".into())
        };
    }
    let group = group.as_deref().map(normalize_group).transpose()?;
    Ok(images
        .iter()
        .filter(|(_, pin)| group.as_ref().is_none_or(|g| &pin.group == g))
        .map(|(label, _)| label.clone())
        .collect())
}

/// 先校验全部目标再修改，避免一张已关闭的贴图导致分组只更新一半。
fn assign_group(
    images: &mut BTreeMap<String, Pin>,
    labels: &[String],
    group: &str,
) -> Result<(), String> {
    let group = normalize_group(group)?;
    if labels.is_empty() || labels.len() > MAX_PINS {
        return Err("请选择 1～12 张贴图。".into());
    }
    if labels.iter().any(|label| !images.contains_key(label)) {
        return Err("部分贴图已关闭，请刷新后重试。".into());
    }
    for label in labels {
        if let Some(pin) = images.get_mut(label) {
            pin.group.clone_from(&group);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_pin_group(
    app: AppHandle,
    window: WebviewWindow,
    labels: Vec<String>,
    group: String,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let state = app.state::<PinState>();
    let _operation = Operation::acquire(&state)?;
    assign_group(&mut *state.images.lock().map_err(err)?, &labels, &group)?;
    changed(&app);
    Ok(())
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
        let pin = own(&app, &window)?;
        if pin.view.locked && view.zoom != pin.view.zoom {
            return Err("请先解锁贴图再缩放。".into());
        }
        let monitor = window
            .current_monitor()
            .map_err(err)?
            .ok_or("无法获取显示器信息。")?;
        let scale = monitor.scale_factor();
        let work_area = monitor.work_area();
        let layout = PinLayout::load();
        let minimum = layout.minimum_zoom(pin.image.width(), pin.image.height(), scale);
        // 通常最多 400%；仅极细图片允许超过此值，以满足 20px 的可见下限。
        view.zoom = view.zoom.clamp(minimum, 4.0_f64.max(minimum));
        let (width, height) = layout.window_size(
            pin.image.width(),
            pin.image.height(),
            view.zoom / scale,
            f64::from(work_area.size.width) / scale,
            f64::from(work_area.size.height) / scale,
            view.toolbar_expanded,
        );
        let old_size = window.inner_size().map_err(err)?;
        let old_position = window.outer_position().map_err(err)?;
        // 底栏展开或放大时，将窗口限制在显示器工作区内，避免按钮掉到任务栏下方。
        let x = f64::from(old_position.x).clamp(
            f64::from(work_area.position.x),
            f64::from(work_area.position.x) + f64::from(work_area.size.width)
                - (width * scale).ceil(),
        );
        let y = f64::from(old_position.y).clamp(
            f64::from(work_area.position.y),
            f64::from(work_area.position.y) + f64::from(work_area.size.height)
                - (height * scale).ceil(),
        );
        window.set_always_on_top(view.topmost).map_err(err)?;
        // 置顶、锁定和透明度不能顺带重排原位贴图；旋转改变图像宽高时仍需重新适配。
        let normal_size = LogicalSize::new(width, height).to_physical::<u32>(scale);
        let source_size = layout.source_bounds(
            PhysicalPosition::new(0, 0), pin.image.width(), pin.image.height(), scale, view.zoom,
        ).1;
        let needs_resize = view.zoom != pin.view.zoom
            || view.toolbar_expanded != pin.view.toolbar_expanded
            || (old_size != normal_size && old_size != source_size);
        let resized = if needs_resize {
            window.set_size(LogicalSize::new(width, height)).and_then(|_| {
                window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
            })
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
        window.set_ignore_cursor_events(view.click_through).map_err(err)?;
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

#[tauri::command]
pub fn drag_pin(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    if app.state::<PinState>().operation.load(Ordering::SeqCst) {
        return Err("正在处理贴图，请稍候。".into());
    }
    if own(&app, &window)?.view.locked {
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
        crate::shared::annotations::validate(
            &pin.annotations,
            pin.image.width(),
            pin.image.height(),
        )?;
        pin.revision = pin.revision.checked_add(1).ok_or("贴图版本超出限制。")?;
        let mut result = frame(window.label(), &pin, None)?;
        result.appearance = appearance(&app)?;
        commit_transformed_pin(&mut *state.images.lock().map_err(err)?, window.label(), pin)?;
        if let Ok(images) = state.images.lock() {
            log_retained(&images, "transformed");
        }
        changed(&app);
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
    Hide,
    Close,
}

/// 打开/保存期间临时隐藏所有可见贴图，防止其他置顶贴图盖住系统对话框。
/// 仅恢复本次隐藏的窗口，用户原本隐藏的贴图保持隐藏；失败也走恢复路径。
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
    changed(app);
    if restore_errors.is_empty() {
        return result;
    }
    let status = match result {
        Ok(value) => describe(&value).to_string(),
        Err(error) => error,
    };
    Err(format!(
        "{status}；恢复贴图窗口失败，可从贴图管理页重新显示：{}",
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
        let pin = own(&app, &window)?;
        match action {
            PinAction::Copy => crate::shared::image_io::copy(&app, &composed(&pin)?)?,
            PinAction::Save => return save_with_hidden_pins(&app, Some(&window), &composed(&pin)?),
            PinAction::Hide => set_visible(&window, false).map_err(err)?,
            PinAction::Close => window.destroy().map_err(err)?,
        }
        changed(&app);
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
                assert_eq!(size.width - inset as u32 * 2, 1233);
                assert!(f64::from(size.height) >= 626.0 + f64::from(inset) * 2.0
                    + (layout.gap + layout.expanded_height) * scale);
            }
        }
        // 小图在最小窗口内也从左上角开始，不因原生最小尺寸额外居中。
        let origin = PhysicalPosition::new(-10, 15);
        let (position, _) = layout.source_bounds(origin, 60, 40, 1.5, 1.0);
        assert_eq!(position.x + layout.image_inset(1.5), origin.x);
        assert_eq!(position.y + layout.image_inset(1.5), origin.y);
    }
    #[test]
    fn known_pixel_revision_skips_image_payload_and_new_revision_refreshes_it() {
        let mut pin = grouped_pin("");
        let initial = frame("pin-1", &pin, None).unwrap();
        assert!(initial.data_url.starts_with("data:image/png;base64,"));
        assert!(frame("pin-1", &pin, Some(0)).unwrap().data_url.is_empty());
        pin.view.zoom = 2.0;
        let metadata = frame("pin-1", &pin, Some(0)).unwrap();
        assert_eq!(metadata.view.zoom, 2.0);
        assert!(metadata.data_url.is_empty());
        pin.revision += 1;
        assert!(!frame("pin-1", &pin, Some(0)).unwrap().data_url.is_empty());
    }
    #[test]
    fn qr_results_stay_with_the_unchanged_live_pin() {
        let mut images = BTreeMap::from([
            ("pin-1".into(), grouped_pin("")),
            ("pin-2".into(), grouped_pin("")),
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
        let pin = grouped_pin("");
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
        images.insert("pin-1".into(), grouped_pin(""));
        let previous = Arc::downgrade(&images["pin-1"].image);
        commit_transformed_pin(&mut images, "pin-1", grouped_pin("new")).unwrap();
        assert_eq!(images["pin-1"].group, "new");
        assert!(previous.upgrade().is_none());
    }

    #[test]
    fn pixel_budget_counts_unique_buffers_and_reset_reuses_original() {
        let mut pin = grouped_pin("");
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
        let pin = grouped_pin("");
        assert!(validate_transform_budget(&pin, MAX_TOTAL_PIXELS - 1, Transform::Invert).is_err());
        assert!(validate_transform_budget(&pin, MAX_TOTAL_PIXELS - 1, Transform::Reset).is_ok());
        assert!(
            validate_transform_budget(&pin, MAX_TOTAL_PIXELS - 2, Transform::RotateRight).is_ok()
        );
        assert!(Arc::ptr_eq(&pin.image, &pin.original));
    }

    fn grouped_pin(group: &str) -> Pin {
        let image = Arc::new(RgbaImage::new(1, 1));
        Pin {
            qr_result: None,
            revision: 0,
            original: image.clone(),
            image,
            view: PinView::default(),
            annotations: vec![],
            original_annotations: vec![],
            group: group.into(),
        }
    }
    #[test]
    fn groups_distinguish_all_ungrouped_and_named_targets() {
        let images = BTreeMap::from([
            ("pin-1".into(), grouped_pin("")),
            ("pin-2".into(), grouped_pin("参考")),
        ]);
        assert_eq!(management_labels(&images, None, None).unwrap().len(), 2);
        assert_eq!(
            management_labels(&images, None, Some("".into())).unwrap(),
            vec!["pin-1"]
        );
        assert_eq!(
            management_labels(&images, None, Some("参考".into())).unwrap(),
            vec!["pin-2"]
        );
        assert!(management_labels(&images, Some("missing".into()), None).is_err());
        assert!(management_labels(&images, Some("pin-1".into()), Some("参考".into())).is_err());
    }
    #[test]
    fn assigning_groups_is_atomic_and_validates_names() {
        let mut images = BTreeMap::from([("pin-1".into(), grouped_pin("旧组"))]);
        assert!(assign_group(&mut images, &["pin-1".into(), "missing".into()], "新组").is_err());
        assert_eq!(images["pin-1"].group, "旧组");
        for name in ["字".repeat(33), "a\nb".into(), "a\u{0085}b".into()] {
            assert!(assign_group(&mut images, &["pin-1".into()], &name).is_err());
        }
        assign_group(&mut images, &["pin-1".into()], " 新组 ").unwrap();
        assert_eq!(images["pin-1"].group, "新组");
        assign_group(&mut images, &["pin-1".into()], "").unwrap();
        assert_eq!(images["pin-1"].group, "");
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
            qr_result: None,
            revision: 0,
            original: image.clone(),
            image,
            view: PinView::default(),
            annotations: vec![s],
            original_annotations: vec![],
            group: String::new(),
        };
        let output = composed(&pin).unwrap();
        assert!(output.get_pixel(10, 10)[3] > 0);
        assert_eq!(pin.image.get_pixel(10, 10)[3], 0);
        assert_eq!(pin.annotations.len(), 1);
    }
    #[test]
    fn bottom_toolbar_does_not_consume_image_height() {
        let layout = PinLayout::load();
        let (width, height) = layout.window_size(800, 600, 1.0, 1920.0, 1080.0, true);
        assert_eq!(width - (layout.padding + layout.border) * 2.0, 800.0);
        assert_eq!(height - layout.chrome_height(true), 600.0);
        let (_, collapsed) = layout.window_size(800, 600, 1.0, 1920.0, 1080.0, false);
        assert_eq!(collapsed - layout.chrome_height(false), 600.0);
        assert_eq!(
            height - collapsed,
            layout.expanded_height - layout.collapsed_height
        );
    }
    #[test]
    fn layout_handles_small_images_large_images_and_scaled_work_areas() {
        let layout = PinLayout::load();
        let (width, height) = layout.window_size(1, 1, 1.0, 1200.0, 800.0, true);
        assert_eq!(
            width,
            layout.min_image_width + (layout.padding + layout.border) * 2.0
        );
        assert_eq!(height, layout.min_image_height + layout.chrome_height(true));
        // 物理 1920×1080、150% DPI 的工作区换算为逻辑像素后再限制窗口。
        assert_eq!(
            layout.window_size(3000, 2000, 4.0, 1920.0 / 1.5, 1080.0 / 1.5, true),
            (1280.0, 720.0)
        );
        assert_eq!(
            layout.window_size(600, 800, 0.5, 1200.0, 800.0, true),
            (
                300.0 + (layout.padding + layout.border) * 2.0,
                400.0 + layout.chrome_height(true)
            )
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
