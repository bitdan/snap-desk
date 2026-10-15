//! 内存贴图与独立窗口。原图保留用于恢复，所有导出取当前处理后的像素。
//! 窗口标签由 Rust 生成；贴图窗口只能操作自身，主窗口只能执行明确的管理动作。
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
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
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
        serde_json::from_str(include_str!("../../src/lib/pin-layout.json"))
            .expect("内置贴图布局配置无效")
    }
    /// 与前端使用相同短边下限，原生命令也不能把小图缩成不可见的小点。
    fn minimum_zoom(&self, width: u32, height: u32, scale: f64) -> f64 {
        (self.min_visible_side * scale / f64::from(width.min(height).max(1))).max(0.1)
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
}

#[derive(Clone)]
struct Pin {
    original: Arc<RgbaImage>,
    image: Arc<RgbaImage>,
    view: PinView,
    annotations: Vec<crate::annotations::Annotation>,
    original_annotations: Vec<crate::annotations::Annotation>,
    /// 空字符串表示未分组；与图片一样仅保留到本次应用退出。
    group: String,
}

/// zoom 使用逻辑像素比例；1 表示一图像像素对应一 CSS 像素。
/// opacity 仅影响显示，locked 防止拖动、缩放和图像处理，仍允许复制/保存/关闭。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinView {
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
    width: u32,
    height: u32,
    data_url: String,
    view: PinView,
    transparency_supported: bool,
    annotations: Vec<crate::annotations::Annotation>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinSummary {
    label: String,
    width: u32,
    height: u32,
    visible: bool,
    group: String,
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
fn changed(app: &AppHandle) {
    let _ = app.emit_to("main", "pins-changed", ());
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
fn frame(label: &str, pin: &Pin) -> Result<PinFrame, String> {
    let mut png = Cursor::new(Vec::new());
    pin.image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(err)?;
    Ok(PinFrame {
        label: label.into(),
        width: pin.image.width(),
        height: pin.image.height(),
        data_url: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        ),
        view: pin.view.clone(),
        transparency_supported: !cfg!(target_os = "macos"),
        annotations: pin.annotations.clone(),
    })
}
fn validate_capacity(width: u32, height: u32, count: usize, used: u64) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_IMAGE_PIXELS {
        return Err("图片为空或超过 1600 万像素限制。".into());
    }
    if count >= MAX_PINS || used.saturating_add(pixels) > MAX_TOTAL_PIXELS {
        return Err("贴图最多 12 张，总计最多 3200 万像素；请先关闭部分贴图。".into());
    }
    Ok(())
}

/// 必须从后台线程调用，避免 Windows WebView 创建阻塞 UI 线程。
/// 先登记图片再创建窗口，保证页面首次请求能读到数据；失败时回收登记。
pub fn create(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<WebviewWindow, String> {
    crate::annotations::validate(&annotations, image.width(), image.height())?;
    let state = app.state::<PinState>();
    let _operation = Operation::acquire(&state)?;
    {
        let images = state.images.lock().map_err(err)?;
        let used = images
            .values()
            .map(|p| u64::from(p.image.width()) * u64::from(p.image.height()))
            .sum();
        validate_capacity(image.width(), image.height(), images.len(), used)?;
    }
    let monitor = app
        .primary_monitor()
        .map_err(err)?
        .ok_or("无法获取显示器信息。")?;
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
    }
    changed(app);
}

/// 主窗口创建贴图共用显示与失败清理，隐藏窗口创建成功并不代表贴图已显示。
pub(crate) fn create_visible(
    app: &AppHandle,
    image: RgbaImage,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<(), String> {
    let pin = create(app, image, annotations)?;
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
                        .set_title("打开图片贴图")
                        .add_filter("图片（PNG / JPEG / WebP）", &["png", "jpg", "jpeg", "webp"])
                        .blocking_pick_file();
                    path.map(|path| crate::pin_content::read_image(&path.into_path().map_err(err)?))
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

/// 主界面、托盘和 F2 共用的剪贴板图片贴图入口；直接读取内存像素，不要求磁盘文件。
pub fn clipboard_pin(app: &AppHandle) -> Result<(), String> {
    let clipboard = app
        .clipboard()
        .read_image()
        .map_err(|e| format!("剪贴板没有可读取的图片：{e}"))?;
    validate_capacity(clipboard.width(), clipboard.height(), 0, 0)?;
    let image = RgbaImage::from_raw(
        clipboard.width(),
        clipboard.height(),
        clipboard.rgba().to_vec(),
    )
    .ok_or("剪贴板图片数据无效。")?;
    create_visible(app, image, vec![])
}

#[tauri::command]
pub async fn current_pin(app: AppHandle, window: WebviewWindow) -> Result<PinFrame, String> {
    tauri::async_runtime::spawn_blocking(move || frame(window.label(), &own(&app, &window)?))
        .await
        .map_err(err)?
}

/// 前端每完成一次编辑就同步结构化标注；锁定/导出中不允许覆盖，错误时保留旧数据。
#[tauri::command]
pub async fn set_pin_annotations(
    app: AppHandle,
    window: WebviewWindow,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let pin = own(&app, &window)?;
        if pin.view.locked {
            return Err("请先解锁贴图再编辑标注。".into());
        }
        crate::annotations::validate(&annotations, pin.image.width(), pin.image.height())?;
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
    crate::annotations::render(
        &mut image,
        &pin.annotations,
        crate::annotations::Point { x: 0.0, y: 0.0 },
    )?;
    Ok(image)
}

#[tauri::command]
pub fn list_pins(app: AppHandle, window: WebviewWindow) -> Result<Vec<PinSummary>, String> {
    crate::capture::require_window(&window, "main")?;
    let images = app.state::<PinState>().images.lock().map_err(err)?.clone();
    images
        .into_iter()
        .map(|(label, pin)| {
            let visible = app
                .get_webview_window(&label)
                .map(|w| w.is_visible())
                .transpose()
                .map_err(err)?
                .unwrap_or(false);
            Ok(PinSummary {
                label,
                width: pin.image.width(),
                height: pin.image.height(),
                visible,
                group: pin.group,
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManageAction {
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
                    ManageAction::Show => pin.show().and_then(|_| pin.set_focus()),
                    ManageAction::Hide => pin.hide(),
                    ManageAction::Close => pin.destroy(),
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
        if let Err(error) = window
            .set_size(LogicalSize::new(width, height))
            .and_then(|_| {
                window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
            })
        {
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
    shapes: &[crate::annotations::Annotation],
    width: u32,
    height: u32,
    action: Transform,
) -> Result<Vec<crate::annotations::Annotation>, String> {
    use crate::annotations::{Kind, Point};
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
                let raster = crate::annotations::raster(source)?;
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
        pin.image = Arc::new(transform(&pin.image, &pin.original, action));
        crate::annotations::validate(&pin.annotations, pin.image.width(), pin.image.height())?;
        let result = frame(window.label(), &pin)?;
        state
            .images
            .lock()
            .map_err(err)?
            .insert(window.label().into(), pin);
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
                pin.hide().map_err(err)?;
                hidden.push(pin);
            }
        }
        task()
    })();
    let mut restore_errors = Vec::new();
    for pin in hidden {
        if let Err(error) = pin.show() {
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
        || crate::image_io::save(app, image),
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
            PinAction::Copy => crate::image_io::copy(&app, &composed(&pin)?)?,
            PinAction::Save => return save_with_hidden_pins(&app, Some(&window), &composed(&pin)?),
            PinAction::Hide => window.hide().map_err(err)?,
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
    fn grouped_pin(group: &str) -> Pin {
        let image = Arc::new(RgbaImage::new(1, 1));
        Pin {
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
        let lens: crate::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"magnify", "start":{"x":10,"y":5}, "end":{"x":30,"y":25},
            "color":"#ff0000", "width":3, "magnification":3,
            "raster":STANDARD.encode(png.into_inner())
        }))
        .unwrap();
        let spotlight: crate::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"spotlight", "start":{"x":10,"y":5}, "end":{"x":30,"y":25},
            "color":"#000000", "width":3, "opacity":0.7
        }))
        .unwrap();
        let source = vec![lens, spotlight];
        let rotated = transform_annotations(&source, 100, 50, Transform::RotateRight).unwrap();
        crate::annotations::validate(&rotated, 50, 100).unwrap();
        assert_eq!(rotated[0].magnification, Some(3.0));
        assert_eq!(
            crate::annotations::raster(&rotated[0]).unwrap(),
            image::imageops::rotate90(&pixels)
        );
        assert_eq!(rotated[1].start.x, 45.0);
        assert_eq!(rotated[1].end.x, 25.0);
        let flipped = transform_annotations(&source, 100, 50, Transform::FlipHorizontal).unwrap();
        crate::annotations::validate(&flipped, 100, 50).unwrap();
        assert_eq!(
            crate::annotations::raster(&flipped[0]).unwrap(),
            image::imageops::flip_horizontal(&pixels)
        );
        assert!(flipped[1].raster.is_none());
    }

    #[test]
    fn rotating_and_flipping_keep_annotations_editable_and_in_bounds() {
        let s: crate::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"pen", "start":{"x":10,"y":5}, "end":{"x":30,"y":20},
            "color":"#ff0000", "width":3, "points":[{"x":10,"y":5},{"x":20,"y":25},{"x":30,"y":20}]
        }))
        .unwrap();
        let source = vec![s];
        let rotated = transform_annotations(&source, 100, 50, Transform::RotateRight).unwrap();
        crate::annotations::validate(&rotated, 50, 100).unwrap();
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
        let s: crate::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"line", "start":{"x":2,"y":10}, "end":{"x":25,"y":10}, "color":"#ff0000", "width":3
        })).unwrap();
        let image = Arc::new(RgbaImage::new(30, 30));
        let pin = Pin {
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
}
