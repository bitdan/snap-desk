//! 截图会话：先等待可取消的延时，再冻结主屏画面，最后裁剪并导出。
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[path = "recorder/manager.rs"]
pub mod recording;
#[path = "scroll/mod.rs"]
pub mod scroll;

#[derive(Default)]
pub struct CaptureState {
    /// 从倒计时开始到导出/取消结束只允许一个会话。
    pub active: AtomicBool,
    operation: AtomicBool,
    next_id: AtomicU64,
    session: Mutex<Option<Session>>,
    scroll: Mutex<Option<scroll::ScrollSession>>,
    /// 全局快捷键只写原子停止标志，不等待后台拼接持有的图片锁。
    scroll_collecting: AtomicBool,
    scroll_stop: AtomicBool,
    scroll_cancel: AtomicBool,
    /// 0 表示不在倒计时；取消只设置标志，由原任务清理，防止旧任务干扰新截图。
    pub delay_remaining: AtomicU8,
    delay_cancelled: AtomicBool,
    /// 将“接受取消”和“结束倒计时”互斥，避免临界时刻回复取消成功却仍然截图。
    delay_gate: Mutex<()>,
}

struct Session {
    frame: Frame,
    image: RgbaImage,
    shown: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    id: u64,
    width: u32,
    height: u32,
    data_url: String,
    /// F3 进入录屏框选；随帧传递意图，窗口重新加载也不会丢失入口状态。
    recording: bool,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    fn validate(self, width: u32, height: u32) -> Result<Self, String> {
        if self.width == 0
            || self.height == 0
            || self
                .x
                .checked_add(self.width)
                .filter(|right| *right <= width)
                .is_none()
            || self
                .y
                .checked_add(self.height)
                .filter(|bottom| *bottom <= height)
                .is_none()
        {
            return Err("选区为空或超出截图边界，请重新框选。".into());
        }
        Ok(self)
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn overlay(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("capture")
        .ok_or_else(|| "截图窗口不可用，请重启应用。".into())
}
pub fn require_window(window: &WebviewWindow, label: &str) -> Result<(), String> {
    if window.label() == label {
        Ok(())
    } else {
        Err("当前窗口无权执行此操作。".into())
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 按钮、快捷键和托盘共用入口。读取设置与占用截图状态在同一把锁内完成，
/// 避免“正在保存快捷键”与“恰好开始截图”并发导致状态不一致。
pub async fn begin(app: AppHandle) -> Result<(), String> {
    begin_with_mode(app, false).await
}

/// 录屏快捷键复用截图的选区能力，确认范围后再交给录屏设置与采集器。
pub async fn begin_recording(app: AppHandle) -> Result<(), String> {
    begin_with_mode(app, true).await
}

async fn begin_with_mode(app: AppHandle, recording: bool) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    let delay = {
        let settings = app.state::<crate::settings::SettingsState>();
        let settings = settings.0.lock().map_err(err)?;
        if state
            .active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("截图正在进行中，请先完成或取消当前截图。".into());
        }
        state.delay_cancelled.store(false, Ordering::SeqCst);
        // 录屏有独立倒计时，选区入口不叠加截图延时。
        let delay = if recording {
            0
        } else {
            settings.value.delay_seconds
        };
        state.delay_remaining.store(delay, Ordering::SeqCst);
        delay
    };
    let worker_app = app.clone();
    // move 将句柄所有权转移给后台闭包；clone 只克隆应用句柄，不复制整个应用。
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<CaptureState>();
        let elapsed = wait_delay(
            Duration::from_secs(u64::from(delay)),
            &state.delay_cancelled,
            |remaining| {
                state.delay_remaining.store(remaining, Ordering::SeqCst);
                let _ = worker_app.emit_to("main", "capture-delay", remaining);
            },
        );
        let elapsed = {
            let _gate = state.delay_gate.lock().map_err(err)?;
            state.delay_remaining.store(0, Ordering::SeqCst);
            elapsed && !state.delay_cancelled.load(Ordering::SeqCst)
        };
        let _ = worker_app.emit_to("main", "capture-delay", 0u8);
        if !elapsed {
            // 倒计时期间尚未隐藏窗口或抓屏，因此取消无需改变其他应用的焦点。
            state.active.store(false, Ordering::SeqCst);
            let _ = worker_app.emit_to("main", "capture-result", "已取消延时截图。");
            return Ok(());
        }
        prepare(&worker_app, recording)
    })
    .await
    .map_err(err)
    .and_then(|result| result);
    if let Err(error) = &result {
        state.delay_remaining.store(0, Ordering::SeqCst);
        let _ = app.emit_to("main", "capture-delay", 0u8);
        if cleanup(&app).is_err() {
            state.active.store(false, Ordering::SeqCst);
        }
        log::error!("Capture preparation failed: {error}");
    }
    result
}

/// 使用单调时钟计时，避免系统时间调整影响延时；每 20ms 检查取消请求。
/// 返回 true 才允许抓屏。将等待逻辑与 Tauri 分离，便于测试“取消后不截图”。
fn wait_delay(duration: Duration, cancelled: &AtomicBool, mut tick: impl FnMut(u8)) -> bool {
    let deadline = Instant::now() + duration;
    let mut last = None;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let remaining = (deadline - now).as_secs_f64().ceil() as u8;
        if last != Some(remaining) {
            tick(remaining);
            last = Some(remaining);
        }
        std::thread::sleep(Duration::from_millis(20).min(deadline - now));
    }
}

/// 倒计时期间由重复快捷键、托盘菜单或主窗口取消按钮调用。
pub fn cancel_delay(app: &AppHandle) -> bool {
    let state = app.state::<CaptureState>();
    let Ok(_gate) = state.delay_gate.lock() else {
        return false;
    };
    if state.delay_remaining.load(Ordering::SeqCst) == 0 {
        return false;
    }
    state.delay_cancelled.store(true, Ordering::SeqCst);
    true
}

#[tauri::command]
pub fn cancel_pending_capture(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    if cancel_delay(&app) {
        Ok(())
    } else {
        Err("倒计时已结束；请在截图蒙层中按 Esc 取消。".into())
    }
}

fn prepare(app: &AppHandle, recording: bool) -> Result<(), String> {
    let window = overlay(app)?;
    window.hide().map_err(err)?;
    // 保留主窗口以支持截取 Snap Desk 自身；只等待旧截图蒙层从桌面移除。
    std::thread::sleep(Duration::from_millis(180));
    let monitor = window
        .primary_monitor()
        .map_err(err)?
        .ok_or("无法获取主显示器信息。")?;
    window.set_position(*monitor.position()).map_err(err)?;
    window.set_size(*monitor.size()).map_err(err)?;
    let image = crate::platform::capture_primary()?;
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).map_err(err)?;
    let state = app.state::<CaptureState>();
    let id = state.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    let frame = Frame {
        id,
        recording,
        width: image.width(),
        height: image.height(),
        data_url: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        ),
    };
    *state.session.lock().map_err(err)? = Some(Session {
        frame: frame.clone(),
        image,
        shown: false,
    });
    app.emit_to("capture", "capture-ready", &frame)
        .map_err(err)?;
    // WebView 未能加载图片时自动释放会话，防止看不见蒙层又无法重新截图。
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        let state = app.state::<CaptureState>();
        let Ok(_operation) = Operation::acquire(&state.operation) else {
            return;
        };
        let timed_out = state
            .session
            .lock()
            .map(|session| {
                session
                    .as_ref()
                    .is_some_and(|s| s.frame.id == id && !s.shown)
            })
            .unwrap_or(false);
        if timed_out {
            let _ = close_session(&app, id);
            report_error(&app, "截图窗口加载超时，请重新截图。".into());
        }
    });
    Ok(())
}

pub fn report_error(app: &AppHandle, message: String) {
    // 错误保留在日志和主界面提示中，不因后台操作失败抢走当前应用焦点。
    log::warn!("{message}");
    let _ = app.emit_to("main", "capture-error", &message);
}

#[tauri::command]
pub async fn start_capture(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    begin(app).await
}

#[tauri::command]
pub fn current_capture(app: AppHandle, window: WebviewWindow) -> Result<Option<Frame>, String> {
    require_window(&window, "capture")?;
    let state = app.state::<CaptureState>();
    let session = state.session.lock().map_err(err)?;
    Ok(session.as_ref().map(|session| session.frame.clone()))
}

#[tauri::command]
pub fn show_capture(app: AppHandle, window: WebviewWindow, id: u64) -> Result<(), String> {
    require_window(&window, "capture")?;
    let state = app.state::<CaptureState>();
    let _operation = Operation::acquire(&state.operation)?;
    let mut guard = state.session.lock().map_err(err)?;
    let session = guard
        .as_mut()
        .filter(|s| s.frame.id == id)
        .ok_or("截图已过期。")?;
    window.show().map_err(err)?;
    window.set_focus().map_err(err)?;
    session.shown = true;
    Ok(())
}

/// 完成、取消或失败后仅清理采集窗口和会话，不自动返回主界面。
fn cleanup(app: &AppHandle) -> Result<(), String> {
    // Keep the session recoverable if hiding fails; don't expose a dead overlay.
    overlay(app)?.hide().map_err(err)?;
    scroll::chrome::hide(app);
    let state = app.state::<CaptureState>();
    state.scroll_stop.store(true, Ordering::SeqCst);
    state.scroll_collecting.store(false, Ordering::SeqCst);
    *state.scroll.lock().map_err(err)? = None;
    *state.session.lock().map_err(err)? = None;
    state.active.store(false, Ordering::SeqCst);
    Ok(())
}

fn close_session(app: &AppHandle, id: u64) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    state
        .session
        .lock()
        .map_err(err)?
        .as_ref()
        .filter(|s| s.frame.id == id)
        .ok_or("截图已过期。")?;
    cleanup(app)
}

// 导出与取消互斥；Drop 在正常返回和 ? 提前返回时都会释放占用标志。
struct Operation<'a>(&'a AtomicBool);
impl<'a> Operation<'a> {
    fn acquire(flag: &'a AtomicBool) -> Result<Self, String> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "正在处理截图，请稍候。".to_string())?;
        Ok(Self(flag))
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub fn cancel_active(app: &AppHandle) -> Result<(), String> {
    if recording::request_stop(app, true) {
        return Ok(());
    }
    let state = app.state::<CaptureState>();
    let _operation = Operation::acquire(&state.operation)?;
    let id = state
        .session
        .lock()
        .map_err(err)?
        .as_ref()
        .map(|s| s.frame.id);
    if let Some(id) = id {
        close_session(app, id)?;
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_capture(app: AppHandle, window: WebviewWindow, id: u64) -> Result<(), String> {
    require_window(&window, "capture")?;
    let state = app.state::<CaptureState>();
    let _operation = Operation::acquire(&state.operation)?;
    close_session(&app, id)
}

/// 复制、保存与贴图共用合成入口，保证输出都包含相同标注。
fn crop(
    app: &AppHandle,
    id: u64,
    rect: Rect,
    annotations: &[crate::annotations::Annotation],
) -> Result<RgbaImage, String> {
    let state = app.state::<CaptureState>();
    if state.scroll.lock().map_err(err)?.is_some() {
        return Err("长截图进行中，请使用长截图预览中的导出按钮。".into());
    }
    let guard = state.session.lock().map_err(err)?;
    let session = guard
        .as_ref()
        .filter(|s| s.frame.id == id)
        .ok_or("截图已过期，请重新截图。")?;
    let rect = rect.validate(session.image.width(), session.image.height())?;
    crate::annotations::validate(annotations, session.image.width(), session.image.height())?;
    let mut image =
        image::imageops::crop_imm(&session.image, rect.x, rect.y, rect.width, rect.height)
            .to_image();
    drop(guard); // 裁剪完成后释放会话锁，绘制耗时不会阻塞其他读取。
    crate::annotations::render(
        &mut image,
        annotations,
        crate::annotations::Point {
            x: f64::from(rect.x),
            y: f64::from(rect.y),
        },
    )?;
    Ok(image)
}

#[tauri::command]
pub async fn recognize_capture_qr(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<crate::qr::QrResult, String> {
    require_window(&window, "capture")?;
    // 复用导出的裁剪与标注合成，但保留会话，允许识别后继续编辑。
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let cropped = crop(&app, id, rect, &annotations)?;
        crate::qr::recognize(&app, id, &cropped)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn copy_capture(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let cropped = crop(&app, id, rect, &annotations)?;
        crate::image_io::copy(&app, &cropped)?;
        close_session(&app, id)?;
        let _ = app.emit_to("main", "capture-result", "截图已复制到剪贴板。");
        Ok(())
    })
    .await
    .map_err(err)?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelPoint {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorFormat {
    Hex,
    Rgb,
}

fn pixel_color(
    image: &RgbaImage,
    point: PixelPoint,
    format: ColorFormat,
) -> Result<String, String> {
    if point.x >= image.width() || point.y >= image.height() {
        return Err("取色坐标超出截图边界。".into());
    }
    let p = image.get_pixel(point.x, point.y);
    Ok(match format {
        ColorFormat::Hex => format!("#{:02X}{:02X}{:02X}", p[0], p[1], p[2]),
        ColorFormat::Rgb => format!("rgb({}, {}, {})", p[0], p[1], p[2]),
    })
}

/// 取色只读取当前截图原始像素，不接受前端传来的任意剪贴板文本。
/// 仅复制色值，成功或失败都保留截图会话、选区及标注，可继续取色或框选。
#[tauri::command]
pub async fn copy_capture_color(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    point: PixelPoint,
    format: ColorFormat,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("长截图进行中，不能取色。".into());
        }
        let text = {
            let guard = state.session.lock().map_err(err)?;
            let session = guard
                .as_ref()
                .filter(|s| s.frame.id == id)
                .ok_or("截图已过期，请重新截图。")?;
            pixel_color(&session.image, point, format)?
        };
        app.clipboard()
            .write_text(text.clone())
            .map_err(|e| format!("复制色值失败，请重试：{e}"))?;
        let _ = app.emit_to("main", "capture-result", format!("色值 {text} 已复制。"));
        Ok(())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn save_capture(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<bool, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let cropped = crop(&app, id, rect, &annotations)?;
        let window = overlay(&app)?;
        // A topmost overlay must not cover the system save dialog.
        window.hide().map_err(err)?;
        let result = crate::pins::save_capture_image(&app, &cropped);
        match result {
            Ok(true) => {
                close_session(&app, id)?;
                let _ = app.emit_to("main", "capture-result", "截图已保存。");
                Ok(true)
            }
            other => {
                window.show().map_err(err)?;
                window.set_focus().map_err(err)?;
                other
            }
        }
    })
    .await
    .map_err(err)?
}

/// 托盘/全局贴图键优先交给截图窗口，由它提交当前选区和尚未导出的标注。
/// 返回 true 表示截图流程已占用入口；准备中或尚未框选时也不能误贴剪贴板。
pub fn request_pin(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<CaptureState>();
    if !state.active.load(Ordering::SeqCst) {
        return Ok(false);
    }
    let session = state.session.lock().map_err(err)?;
    if let Some(session) = session.as_ref() {
        if session.shown && !session.frame.recording {
            // 携带会话 ID，防止延迟事件提交下一次截图的选区。
            app.emit_to("capture", "capture-pin-requested", session.frame.id)
                .map_err(err)?;
        }
    }
    Ok(true)
}

/// 创建独立贴图后结束截图会话；创建失败时保留原选区供用户重试。
#[tauri::command]
pub async fn pin_capture(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let cropped = crop(&app, id, rect, &[])?;
        // 贴图保存裁剪后的底图与独立图形；将屏幕坐标平移为贴图局部像素坐标。
        let mut annotations = annotations;
        for shape in &mut annotations {
            for p in [&mut shape.start, &mut shape.end]
                .into_iter()
                .chain(shape.points.iter_mut())
            {
                p.x -= f64::from(rect.x);
                p.y -= f64::from(rect.y);
            }
        }
        let pin = crate::pins::create(&app, cropped, annotations)?;
        if let Err(error) = pin.show() {
            let _ = pin.destroy();
            return Err(err(error));
        }
        if let Err(error) = close_session(&app, id) {
            let _ = pin.destroy();
            return Err(error);
        }
        let _ = pin.set_focus();
        let _ = app.emit_to("main", "pins-changed", ());
        let _ = app.emit_to("main", "capture-result", "截图已贴到桌面。");
        Ok(())
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_picker_formats_exact_source_pixels_and_rejects_edges() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(1, 1, image::Rgba([0, 128, 255, 255]));
        assert_eq!(
            pixel_color(&image, PixelPoint { x: 1, y: 1 }, ColorFormat::Hex).unwrap(),
            "#0080FF"
        );
        assert_eq!(
            pixel_color(&image, PixelPoint { x: 1, y: 1 }, ColorFormat::Rgb).unwrap(),
            "rgb(0, 128, 255)"
        );
        assert!(pixel_color(&image, PixelPoint { x: 2, y: 1 }, ColorFormat::Hex).is_err());
        assert!(pixel_color(&image, PixelPoint { x: 1, y: u32::MAX }, ColorFormat::Hex).is_err());
    }
    #[test]
    fn delay_can_be_cancelled_before_or_during_wait() {
        let cancelled = AtomicBool::new(true);
        assert!(!wait_delay(Duration::from_secs(5), &cancelled, |_| panic!(
            "不应开始倒计时"
        )));
        cancelled.store(false, Ordering::SeqCst);
        assert!(!wait_delay(
            Duration::from_secs(3),
            &cancelled,
            |remaining| {
                assert_eq!(remaining, 3);
                cancelled.store(true, Ordering::SeqCst);
            }
        ));
        cancelled.store(false, Ordering::SeqCst);
        assert!(wait_delay(Duration::ZERO, &cancelled, |_| panic!(
            "立即截图不应倒计时"
        )));
    }
    #[test]
    fn rejects_empty_out_of_bounds_and_overflow() {
        for rect in [
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            Rect {
                x: 99,
                y: 0,
                width: 2,
                height: 1,
            },
            Rect {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
            Rect {
                x: 0,
                y: 99,
                width: 1,
                height: 2,
            },
        ] {
            assert!(rect.validate(100, 100).is_err());
        }
        assert!(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100
        }
        .validate(100, 100)
        .is_ok());
    }
    #[test]
    fn operation_prevents_double_export_and_releases_on_error() {
        let flag = AtomicBool::new(false);
        let operation = Operation::acquire(&flag).unwrap();
        assert!(Operation::acquire(&flag).is_err());
        drop(operation);
        assert!(Operation::acquire(&flag).is_ok());
    }
}
