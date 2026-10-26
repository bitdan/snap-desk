//! 截图会话：先等待可取消的延时，再冻结桌面画面，最后裁剪并导出。
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[path = "recorder/manager.rs"]
pub mod recording;
#[path = "scroll/mod.rs"]
pub mod scroll;

#[path = "capture_preview.rs"]
pub mod preview;

/// PNG 原始字节留在会话中，由只读协议传输；元信息事件不再携带整屏 Base64。
fn preview_png(image: &RgbaImage) -> Result<Arc<Vec<u8>>, String> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).map_err(err)?;
    Ok(Arc::new(png.into_inner()))
}

#[derive(Default)]
pub struct CaptureState {
    /// 从倒计时开始到导出/取消结束只允许一个会话。
    pub active: AtomicBool,
    operation: AtomicBool,
    next_id: AtomicU64,
    session: Mutex<Option<Session>>,
    /// 最近一次隐藏蒙层的时刻；快速连续截图仍需等待桌面合成器移除上一帧。
    hidden_at: Mutex<Option<Instant>>,
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
    desktop: crate::desktop_capture::Desktop,
    /// 协议请求只克隆 Arc，离开锁后再复制响应体；取消会话后不接受新的读取。
    preview: Arc<Vec<u8>>,
    prepared_at: Instant,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    id: u64,
    width: u32,
    height: u32,
    /// 原生窗口重抓后递增，令 WebView 重新加载图片，避免复用同一 URL 的旧像素。
    preview_revision: u64,
    /// F3 进入录屏框选；随帧传递意图，窗口重新加载也不会丢失入口状态。
    recording: bool,
    origin_x: i32,
    origin_y: i32,
    displays: Vec<crate::desktop_capture::Display>,
    targets: Vec<crate::desktop_capture::Target>,
    layout_key: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
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
    if let Err(_error) = &result {
        state.delay_remaining.store(0, Ordering::SeqCst);
        let _ = app.emit_to("main", "capture-delay", 0u8);
        if cleanup(&app).is_err() {
            state.active.store(false, Ordering::SeqCst);
        }
        log::error!("event=capture.prepare_failed module=capture code=CAPTURE_PREPARE_FAILED");
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

/// 所有截图蒙层隐藏入口共用时间记录，包含录屏/滚动截图接管和退出截图。
fn hide_overlay(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let visible = window.is_visible().map_err(err)?;
    window.hide().map_err(err)?;
    if visible {
        *app.state::<CaptureState>().hidden_at.lock().map_err(err)? = Some(Instant::now());
    }
    Ok(())
}
fn state_hide_remaining(app: &AppHandle) -> Result<Duration, String> {
    let state = app.state::<CaptureState>();
    let hidden = state.hidden_at.lock().map_err(err)?;
    Ok(hide_remaining(*hidden, Instant::now()))
}
/// 使用单调时间计算剩余等待，首次启动（没有旧蒙层）无需等待。
fn hide_remaining(hidden: Option<Instant>, now: Instant) -> Duration {
    hidden
        .map(|time| Duration::from_millis(180).saturating_sub(now.saturating_duration_since(time)))
        .unwrap_or_default()
}

fn prepare(app: &AppHandle, recording: bool) -> Result<(), String> {
    let window = overlay(app)?;
    let started = Instant::now();
    hide_overlay(app, &window)?;
    // 已经隐藏超过 180ms 的常驻蒙层不再固定等待；刚关闭后立刻重截仍等待剩余时间。
    let remaining = state_hide_remaining(app)?;
    if !remaining.is_zero() {
        std::thread::sleep(remaining);
    }
    let capture_started = Instant::now();
    let (include_cursor, snap) = {
        let settings = app.state::<crate::settings::SettingsState>();
        let guard = settings.0.lock().map_err(err)?;
        (
            !recording && guard.value.screenshot_cursor,
            guard.value.screenshot_snap,
        )
    };
    let mut desktop = crate::desktop_capture::capture(false, include_cursor, recording || snap)?;
    #[cfg(not(windows))]
    {
        let monitor = window
            .primary_monitor()
            .map_err(err)?
            .ok_or("主显示器不可用。")?;
        desktop.x = monitor.position().x;
        desktop.y = monitor.position().y;
        desktop.displays = vec![crate::desktop_capture::Display {
            x: desktop.x,
            y: desktop.y,
            width: desktop.image.width(),
            height: desktop.image.height(),
            primary: true,
            scale: monitor.scale_factor(),
        }];
    }
    window
        .set_position(tauri::PhysicalPosition::new(desktop.x, desktop.y))
        .map_err(err)?;
    window
        .set_size(tauri::PhysicalSize::new(
            desktop.image.width(),
            desktop.image.height(),
        ))
        .map_err(err)?;
    let image = std::mem::take(&mut desktop.image);
    let capture_ms = capture_started.elapsed().as_secs_f64() * 1000.0;
    let encode_started = Instant::now();
    let preview = preview_png(&image)?;
    let encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
    let state = app.state::<CaptureState>();
    let id = state.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    let frame = Frame {
        id,
        recording,
        origin_x: desktop.x,
        origin_y: desktop.y,
        displays: desktop.displays.clone(),
        targets: desktop.targets.clone(),
        layout_key: serde_json::to_string(&desktop.displays).map_err(err)?,
        width: image.width(),
        height: image.height(),
        preview_revision: 0,
    };
    *state.session.lock().map_err(err)? = Some(Session {
        frame: frame.clone(),
        image,
        shown: false,
        desktop,
        preview,
        prepared_at: started,
    });
    log::debug!("event=capture.prepared module=capture op=cap-{id} capture_ms={capture_ms:.2} encode_ms={encode_ms:.2} duration_ms={:.2}", started.elapsed().as_secs_f64() * 1000.0);
    app.emit_to("capture", "capture-ready", &frame)
        .map_err(err)?;
    // WebView 未能加载图片时自动释放会话，防止看不见蒙层又无法重新截图。
    let app = app.clone();
    // Tokio 定时器不占用休眠 OS 线程；会话 ID 校验避免旧任务关闭新截图。
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
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
    // 错误详情保留在主界面提示中，不因后台操作失败抢走当前应用焦点。
    // message 可能含路径、OCR 或网络错误正文，只送 UI，不持久化。
    log::warn!("event=app.operation_failed module=app code=OPERATION_FAILED");
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

/// 原生窗口抓取由“原生窗口截图”按钮触发；尺寸或位置变化时保留旧截图供用户重选。
#[tauri::command]
pub async fn capture_native_window(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    target: usize,
) -> Result<Frame, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("请先结束滚动截图。".into());
        }
        let (native, old, include_cursor, origin) = {
            let guard = state.session.lock().map_err(err)?;
            let session = guard
                .as_ref()
                .filter(|s| s.frame.id == id)
                .ok_or("截图已过期。")?;
            let candidate = session
                .frame
                .targets
                .get(target)
                .filter(|t| t.kind == "window")
                .ok_or("请选择已识别窗口。")?;
            (
                session.desktop.windows[candidate.window_index].clone(),
                candidate.rect,
                session.desktop.include_cursor,
                (session.frame.origin_x, session.frame.origin_y),
            )
        };
        hide_overlay(&app, &window)?;
        let result = (|| {
            let (image, x, y) = crate::desktop_capture::native_window(&native, include_cursor)?;
            let mut guard = state.session.lock().map_err(err)?;
            let session = guard
                .as_mut()
                .filter(|s| s.frame.id == id)
                .ok_or("截图已过期。")?;
            let rect = crate::desktop_capture::clipped(
                x,
                y,
                image.width(),
                image.height(),
                origin,
                session.image.dimensions(),
            )
            .ok_or("窗口已移出截图范围。")?;
            if (rect.x, rect.y, rect.width, rect.height) != (old.x, old.y, old.width, old.height) {
                return Err("窗口位置或尺寸已变化，请重新截图。".into());
            }
            // 编码成功后才提交新像素；失败时原会话和选区仍可继续操作。
            let mut composed = session.image.clone();
            image::imageops::replace(
                &mut composed,
                &image,
                i64::from(x) - i64::from(origin.0),
                i64::from(y) - i64::from(origin.1),
            );
            let preview = preview_png(&composed)?;
            session.preview = preview;
            session.frame.preview_revision += 1;
            session.image = composed;
            Ok(session.frame.clone())
        })();
        // 无论成功失败均恢复窗口；相同 PNG 不会再次触发前端 load，不能依赖它解锁显示。
        window.show().map_err(err)?;
        window.set_focus().map_err(err)?;
        result
    })
    .await
    .map_err(err)?
}

/// 将虚拟桌面物理选区绑定到唯一显示器；拒绝跨屏，并核对冻结时的 DPI 和布局。
fn selected_region(app: &AppHandle, id: u64, rect: Rect) -> Result<(Rect, scroll::Screen), String> {
    let state = app.state::<CaptureState>();
    let guard = state.session.lock().map_err(err)?;
    let session = guard
        .as_ref()
        .filter(|s| s.frame.id == id)
        .ok_or("截图已过期。")?;
    rect.validate(session.image.width(), session.image.height())?;
    let display = session
        .frame
        .displays
        .iter()
        .find(|d| {
            crate::desktop_capture::within_display(
                rect,
                (session.frame.origin_x, session.frame.origin_y),
                d,
            )
            .is_some()
        })
        .ok_or("请选择单个显示器内的区域；跨屏录制与滚动截图暂不支持。")?;
    let current = scroll::session::screen_at(app, display.x, display.y)?;
    // xcap 将 DPI 保存为 f32，Tauri 为 f64；统一到 f32 后比较，避免 110% 等比例误报。
    let mut frozen_layout: Vec<_> = session
        .frame
        .displays
        .iter()
        .map(|d| {
            (
                d.x,
                d.y,
                d.width,
                d.height,
                u64::from((d.scale as f32).to_bits()),
            )
        })
        .collect();
    frozen_layout.sort_unstable();
    if current.layout != frozen_layout {
        return Err("显示器布局或 DPI 已变化，请重新截图。".into());
    }
    let local = crate::desktop_capture::within_display(
        rect,
        (session.frame.origin_x, session.frame.origin_y),
        display,
    )
    .ok_or("选区已超出显示器。")?;
    Ok((local, current))
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
    if !session.shown {
        log::debug!(
            "event=capture.visible module=capture op=cap-{id} duration_ms={:.2}",
            session.prepared_at.elapsed().as_secs_f64() * 1000.0
        );
    }
    session.shown = true;
    Ok(())
}

/// 完成、取消或失败后仅清理采集窗口和会话，不自动返回主界面。
fn cleanup(app: &AppHandle) -> Result<(), String> {
    crate::translation::cancel_window(app, "capture");
    // Keep the session recoverable if hiding fails; don't expose a dead overlay.
    hide_overlay(app, &overlay(app)?)?;
    scroll::chrome::hide(app);
    let state = app.state::<CaptureState>();
    state.scroll_stop.store(true, Ordering::SeqCst);
    state.scroll_collecting.store(false, Ordering::SeqCst);
    let scroll_id = state.scroll.lock().map_err(err)?.take().map(|s| s.id);
    let session = state.session.lock().map_err(err)?.take();
    let closed_id = session.as_ref().map(|session| session.frame.id);
    // 先释放原生 RGBA/PNG，再通知常驻 WebView 清空图片和标注历史。
    // 关闭窗口/全局取消也会经过这里，不能只依赖前端按钮的成功回调。
    drop(session);
    if let Some(id) = closed_id.or(scroll_id) {
        if let Err(error) = app.emit_to("capture", "capture-closed", id) {
            log::warn!("通知截图窗口释放会话 {id} 失败: {error}");
        }
        // 辅助 WebView 常驻隐藏；结束/取消后也必须释放其缩略图。
        for label in ["scroll-preview", "scroll-controls"] {
            let _ = app.emit_to(label, "capture-closed", id);
        }
    }
    state.active.store(false, Ordering::SeqCst);
    Ok(())
}

fn close_session(app: &AppHandle, id: u64) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    let capture_matches = state
        .session
        .lock()
        .map_err(err)?
        .as_ref()
        .is_some_and(|s| s.frame.id == id);
    // 滚动采集接管后，冻结桌面已释放；结束仍按滚动会话编号校验。
    let scroll_matches = state
        .scroll
        .lock()
        .map_err(err)?
        .as_ref()
        .is_some_and(|s| s.id == id);
    if !capture_matches && !scroll_matches {
        return Err("截图已过期。".into());
    }
    cleanup(app)
}

/// 录屏/滚动采集准备成功后接管选区，只释放冻结桌面，不解除 active 占用。
/// 调用方持有 Operation；失败准备仍保留截图，供用户重选或重试。
fn release_selection(app: &AppHandle, id: u64) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    let mut guard = state.session.lock().map_err(err)?;
    if !guard.as_ref().is_some_and(|s| s.frame.id == id) {
        return Err("截图已过期。".into());
    }
    let session = guard.take();
    drop(guard);
    drop(session);
    if let Err(error) = app.emit_to("capture", "capture-closed", id) {
        log::warn!("通知截图窗口释放选区 {id} 失败: {error}");
    }
    Ok(())
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
    let id = id.or(state.scroll.lock().map_err(err)?.as_ref().map(|s| s.id));
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
        return Err("滚动截图进行中，请使用滚动截图预览中的导出按钮。".into());
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

/// 空标注沿用原有会话、选区和滚动截图校验，不让标注污染 OCR 输入。
pub(crate) fn crop_source(app: &AppHandle, id: u64, rect: Rect) -> Result<RgbaImage, String> {
    crop(app, id, rect, &[])
}

/// 结构化识别只对截图窗口开放；推理在阻塞线程执行，不阻塞界面。
#[tauri::command]
pub async fn recognize_capture_v2(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
) -> Result<crate::recognition::OcrResult, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let cropped = crop_source(&app, id, rect)?;
        crate::recognition::provider("local-ppocr")?.recognize(&app, &cropped)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn recognize_capture_text(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    annotations: Vec<crate::annotations::Annotation>,
) -> Result<String, String> {
    require_window(&window, "capture")?;
    // 与保存图片使用同一裁剪/标注坐标，阻塞推理放后台线程，保留截图会话。
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let cropped = crop(&app, id, rect, &annotations)?;
        crate::ocr::recognize(&app, &cropped)
    })
    .await
    .map_err(err)?
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
        log::info!(
            "event=capture.copied module=capture op=cap-{id} width={} height={}",
            cropped.width(),
            cropped.height()
        );
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
            return Err("滚动截图进行中，不能取色。".into());
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
        hide_overlay(&app, &window)?;
        let result = crate::pins::save_capture_image(&app, &cropped);
        match result {
            Ok(true) => {
                log::info!(
                    "event=capture.saved module=capture op=cap-{id} width={} height={}",
                    cropped.width(),
                    cropped.height()
                );
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
    fn hidden_overlay_only_waits_for_unfinished_composition() {
        let now = Instant::now();
        assert_eq!(hide_remaining(None, now), Duration::ZERO);
        assert_eq!(hide_remaining(Some(now), now), Duration::from_millis(180));
        assert_eq!(
            hide_remaining(Some(now), now + Duration::from_millis(50)),
            Duration::from_millis(130)
        );
        assert_eq!(
            hide_remaining(Some(now), now + Duration::from_secs(1)),
            Duration::ZERO
        );
    }

    #[test]
    fn preview_encoding_preserves_rgba_pixels() {
        let image = RgbaImage::from_fn(17, 9, |x, y| {
            image::Rgba([x as u8 * 13, y as u8 * 27, 123, (x + y) as u8 * 9])
        });
        let bytes = preview_png(&image).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded, image);
    }
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
