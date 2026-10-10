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
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_clipboard_manager::ClipboardExt;

const MAX_PINS: usize = 12;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_TOTAL_PIXELS: u64 = 32_000_000;

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
}
impl Default for PinView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            opacity: 1.0,
            locked: false,
            topmost: true,
        }
    }
}
impl PinView {
    fn validate(&self) -> Result<(), String> {
        if !self.zoom.is_finite()
            || !(0.1..=4.0).contains(&self.zoom)
            || !self.opacity.is_finite()
            || !(0.2..=1.0).contains(&self.opacity)
        {
            return Err("缩放应为 10%～400%，透明度应为 20%～100%。".into());
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
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinSummary {
    label: String,
    width: u32,
    height: u32,
    visible: bool,
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
pub fn create(app: &AppHandle, image: RgbaImage) -> Result<WebviewWindow, String> {
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
    let max_width = f64::from(monitor.size().width) / monitor.scale_factor() * 0.8;
    let max_height = f64::from(monitor.size().height) / monitor.scale_factor() * 0.8;
    let zoom = (max_width / f64::from(image.width()))
        .min(max_height / f64::from(image.height()))
        .min(1.0)
        .max(0.1);
    let width = (f64::from(image.width()) * zoom).max(240.0).min(max_width);
    let height = (f64::from(image.height()) * zoom)
        .max(160.0)
        .min(max_height);
    let label = format!("pin-{}", state.next_id.fetch_add(1, Ordering::SeqCst) + 1);
    let image = Arc::new(image);
    state.images.lock().map_err(err)?.insert(
        label.clone(),
        Pin {
            original: image.clone(),
            image,
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

#[tauri::command]
pub async fn pin_clipboard(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
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
        let pin = create(&app, image)?;
        if let Err(error) = pin.show().and_then(|_| pin.set_focus()) {
            let _ = pin.destroy();
            return Err(err(error));
        }
        changed(&app);
        Ok(())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn current_pin(app: AppHandle, window: WebviewWindow) -> Result<PinFrame, String> {
    tauri::async_runtime::spawn_blocking(move || frame(window.label(), &own(&app, &window)?))
        .await
        .map_err(err)?
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
    action: ManageAction,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let labels: Vec<String> = {
            let images = state.images.lock().map_err(err)?;
            match label {
                Some(label) if images.contains_key(&label) => vec![label],
                Some(_) => return Err("贴图已关闭。".into()),
                None => images.keys().cloned().collect(),
            }
        };
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

#[tauri::command]
pub async fn set_pin_view(
    app: AppHandle,
    window: WebviewWindow,
    view: PinView,
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
        let width = (f64::from(pin.image.width()) * view.zoom)
            .max(240.0)
            .min(f64::from(monitor.size().width) / monitor.scale_factor() * 0.95);
        let height = (f64::from(pin.image.height()) * view.zoom)
            .max(160.0)
            .min(f64::from(monitor.size().height) / monitor.scale_factor() * 0.95);
        window.set_always_on_top(view.topmost).map_err(err)?;
        if let Err(error) = window.set_size(LogicalSize::new(width, height)) {
            let _ = window.set_always_on_top(pin.view.topmost);
            return Err(err(error));
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
        pin.image = Arc::new(transform(&pin.image, &pin.original, action));
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

/// 保存期间临时隐藏所有可见贴图，防止其他置顶贴图盖住系统对话框。
/// 仅恢复本次隐藏的窗口，用户原本隐藏的贴图保持隐藏；失败也走恢复路径。
fn save_with_hidden_pins(
    app: &AppHandle,
    focus: Option<&WebviewWindow>,
    image: &RgbaImage,
) -> Result<bool, String> {
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
        crate::image_io::save(app, image)
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
        Ok(true) => "图片已保存".to_string(),
        Ok(false) => "已取消保存".to_string(),
        Err(error) => error,
    };
    Err(format!(
        "{status}；恢复贴图窗口失败，可从贴图管理页重新显示：{}",
        restore_errors.join("；")
    ))
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
            PinAction::Copy => crate::image_io::copy(&app, &pin.image)?,
            PinAction::Save => return save_with_hidden_pins(&app, Some(&window), &pin.image),
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
