//! 会话状态、命令、采集循环和资源生命周期；协调采集、恢复、拼接和输出。
use super::super::{
    close_session, crop, err, report_error, require_window, CaptureState, Operation, Rect,
};
use super::{
    archive::Archive,
    capture::{self, wait},
    chrome,
    input::Direction,
    matcher::{Bounds, Match, ScrollMode},
    recovery, stability,
    stitcher::Stitch,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use std::{
    io::Cursor,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(Clone, PartialEq)]
pub(in crate::capture) struct Screen {
    pub(in crate::capture) name: Option<String>,
    pub(in crate::capture) x: i32,
    pub(in crate::capture) y: i32,
    pub(in crate::capture) width: u32,
    pub(in crate::capture) height: u32,
    pub(in crate::capture) scale: u64,
    pub(in crate::capture) layout: Vec<(i32, i32, u32, u32, u64)>,
}

/// 通过物理桌面原点重查来源，不因主屏切换而悄悄换到另一块屏幕。
pub(in crate::capture) fn screen_at(app: &AppHandle, x: i32, y: i32) -> Result<Screen, String> {
    let monitors = app.available_monitors().map_err(err)?;
    let mut layout: Vec<_> = monitors
        .iter()
        .map(|m| {
            (
                m.position().x,
                m.position().y,
                m.size().width,
                m.size().height,
                u64::from((m.scale_factor() as f32).to_bits()),
            )
        })
        .collect();
    layout.sort_unstable();
    let monitor = monitors
        .into_iter()
        .find(|m| (m.position().x, m.position().y) == (x, y))
        .ok_or("来源显示器已移除或布局改变。")?;
    Ok(Screen {
        name: monitor.name().cloned(),
        x,
        y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale: monitor.scale_factor().to_bits(),
        layout,
    })
}

/// 空洞默认白底；透明选项仅影响导出，预览始终保留透明覆盖信息。
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportBackground {
    #[default]
    White,
    Transparent,
}

#[derive(Clone)]
pub(in crate::capture) struct ScrollSession {
    pub(in crate::capture) id: u64,
    revision: u64,
    rect: Rect,
    screen: Screen,
    source_size: (u32, u32),
    stitch: Stitch,
    direction: Direction,
    recovery: recovery::Recovery,
    message: String,
    preview: Arc<str>,
    status: &'static str,
    last_growth: Instant,
    background: ExportBackground,
    // 用户选择“保存已确认部分”时冻结当前可信画布，不再采集结束前的末帧。
    confirmed_only: bool,
    obscuring_windows: Vec<String>,
    archive: Option<Archive>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollView {
    id: u64,
    revision: u64,
    collecting: bool,
    archive_path: Option<String>,
    width: u32,
    height: u32,
    frames: usize,
    preview: String,
    frame_height: u32,
    canvas_bounds: Bounds,
    viewport_rect: Bounds,
    coverage_percent: f64,
    has_holes: bool,
    direction: &'static str,
    status: &'static str,
    message: String,
}

// 采集时只传图块缩略图，导出仍使用原始像素。
fn view(session: &ScrollSession, collecting: bool) -> Result<ScrollView, String> {
    Ok(ScrollView {
        id: session.id,
        revision: session.revision,
        collecting,
        archive_path: session
            .archive
            .as_ref()
            .map(|a| a.path.to_string_lossy().into_owned()),
        width: session.stitch.width,
        height: session.stitch.height,
        frames: session.stitch.count(),
        frame_height: session.stitch.frame_height,
        canvas_bounds: session.stitch.bounds,
        viewport_rect: session.stitch.viewport(),
        coverage_percent: session.stitch.coverage(),
        has_holes: session.stitch.has_holes(),
        direction: session.stitch.direction(),
        status: session.status,
        preview: if collecting {
            session.preview.to_string()
        } else {
            String::new()
        },
        message: session.message.clone(),
    })
}

#[tauri::command]
pub fn control_scroll(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    cancel: bool,
    confirmed_only: Option<bool>,
) -> Result<(), String> {
    if !matches!(window.label(), "scroll-preview" | "scroll-controls") {
        return Err("窗口无权控制滚动截图。".into());
    }
    if cancel {
        return if cancel_current(&app, id, "window") {
            Ok(())
        } else {
            Err("滚动截图已结束。".into())
        };
    }
    finish_current(&app, id, Some(confirmed_only.unwrap_or(false)))
}

/// 完成按钮与 Ctrl+C 共用停止入口；快捷键在恢复状态只导出已确认画布。
fn finish_current(app: &AppHandle, id: u64, confirmed_only: Option<bool>) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    let mut guard = state.scroll.lock().map_err(err)?;
    let session = guard.as_mut().filter(|s| s.id == id).ok_or("滚动截图已结束。")?;
    if state.scroll_cancel.load(Ordering::SeqCst) {
        return Err("滚动截图已取消。".into());
    }
    session.confirmed_only |= confirmed_only.unwrap_or(session.status == "recovering");
    state.scroll_stop.store(true, Ordering::SeqCst);
    Ok(())
}

/// Esc 和取消按钮共用入口；立即隐藏窗口，后台保留停止标志直到在途计算安全退出。
fn cancel_current(app: &AppHandle, id: u64, source: &'static str) -> bool {
    let state = app.state::<CaptureState>();
    let Ok(guard) = state.scroll.lock() else {
        return false;
    };
    if !guard.as_ref().is_some_and(|s| s.id == id) {
        return false;
    }
    state.scroll_cancel.store(true, Ordering::SeqCst);
    state.scroll_stop.store(true, Ordering::SeqCst);
    // 窗口操作不能持会话锁，避免等待 UI 时阻塞采集线程收尾。
    drop(guard);
    log::info!("event=scroll.cancel_requested module=scroll op=scroll-{id} source={source}");
    chrome::hide(app);
    true
}

/// 后台直接落盘并复制。匹配不可靠的候选帧不会进入成图，不要求用户修接缝。
fn complete(app: &AppHandle, id: u64, message: &str) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    // 取消无需分配最多 3200 万像素的连续导出画布。
    if state.scroll_cancel.load(Ordering::SeqCst) {
        return close_session(app, id);
    }
    let snapshot = {
        let mut guard = state.scroll.lock().map_err(err)?;
        let Some(s) = guard.as_mut().filter(|s| s.id == id) else {
            return Ok(());
        };
        s.revision += 1;
        s.message = message.into();
        s.status = "finishing";
        s.clone()
    };
    // 快照只克隆 Arc；整图分配与像素复制不占状态锁。
    let compose_started = Instant::now();
    let mut image = snapshot.stitch.compose();
    if matches!(snapshot.background, ExportBackground::White) {
        for pixel in image.pixels_mut() {
            if pixel[3] == 0 {
                *pixel = image::Rgba([255, 255, 255, 255]);
            }
        }
    }
    let compose_ms = compose_started.elapsed().as_secs_f64() * 1000.0;
    let result = view(&snapshot, false)?;
    if state.scroll_cancel.load(Ordering::SeqCst) {
        return close_session(app, id);
    }
    let mut png_ms = 0.0;
    let saved = (|| -> Result<std::path::PathBuf, String> {
        let root = app
            .path()
            .app_local_data_dir()
            .map_err(err)?
            .join("scroll-results");
        std::fs::create_dir_all(&root).map_err(err)?;
        let mut file =
            crate::shared::export_staging::TrackedFile::new(app, &root, "png", "滚动截图导出")?;
        let png_started = Instant::now();
        image
            .write_to(file.file_mut(), ImageFormat::Png)
            .map_err(err)?;
        png_ms = png_started.elapsed().as_secs_f64() * 1000.0;
        file.finalize()?;
        let path = root.join(format!("scroll-{}.png", file.id()));
        file.publish(&path, false)?;
        Ok(path)
    })();
    let clipboard_started = Instant::now();
    let copied = crate::shared::image_io::copy(app, &image);
    // 实际复制包含 PNG/DIB 准备及系统剪贴板重试；只记录数值，不记录输出路径或图像。
    log::debug!("event=scroll.export module=scroll op=scroll-{id} compose_ms={compose_ms:.2} png_ms={png_ms:.2} clipboard_ms={:.2} rgba_bytes={} width={} height={} frames={}",
        clipboard_started.elapsed().as_secs_f64()*1000.0, image.as_raw().len(), image.width(), image.height(), snapshot.stitch.count());
    // 两种输出都失败时保留内存会话，调用方自动重试；不假报完成。
    if let (Err(save), Err(copy)) = (&saved, &copied) {
        return Err(format!("保存失败：{save}；复制失败：{copy}"));
    }
    let output = match saved {
        Ok(path) => format!(
            "已保存：{}。{}",
            path.display(),
            if copied.is_ok() {
                "已复制到剪贴板。"
            } else {
                "剪贴板不可用，PNG 已保留。"
            }
        ),
        Err(error) => format!("已复制到剪贴板，本地保存失败：{error}"),
    };
    log::info!(
        "event=scroll.finished module=scroll op=scroll-{id} frames={} width={} height={}",
        result.frames,
        result.width,
        result.height
    );
    close_session(app, id)?;
    let _ = app.emit_to("capture", "scroll-updated", &result);
    let _ = app.emit_to("main", "capture-result", format!("{message} {output}"));
    Ok(())
}

/// 此函数由托盘和全局快捷键调用，不阻塞 UI；后台完成当前取样后直接输出成图。
pub fn stop(app: &AppHandle) -> bool {
    let state = app.state::<CaptureState>();
    if !state.scroll_collecting.load(Ordering::SeqCst) {
        return false;
    }
    state.scroll_stop.store(true, Ordering::SeqCst);
    true
}

/// 调用方持有截图操作锁。结束前的最后取样忽略停止标志，但仍必须通过稳定性检查。
fn sample_pair(
    app: &AppHandle,
    state: &CaptureState,
    id: u64,
    finishing: bool,
) -> Result<Option<Match>, String> {
    let mut snapshot = {
        let guard = state.scroll.lock().map_err(err)?;
        guard
            .as_ref()
            .filter(|s| s.id == id)
            .cloned()
            .ok_or("滚动截图会话已结束。")?
    };
    let revision = snapshot.revision;
    // Windows 以实际滚轮输入决定轴；还没滚动时无需抓屏和匹配。
    #[cfg(windows)]
    if snapshot.direction.mode() == ScrollMode::Auto {
        return Ok(Some(Match::Unchanged));
    }
    snapshot.stitch.lock_axis(snapshot.direction.mode());
    if screen_at(app, snapshot.screen.x, snapshot.screen.y)? != snapshot.screen {
        return Err("显示器布局、尺寸或 DPI 发生变化，已结束采集，仅输出已确认部分。".into());
    }
    let _preview_guard = chrome::hide_for_sample(app, id, &snapshot.obscuring_windows)?;
    let settled = capture::sample_pair(
        state,
        snapshot.rect,
        snapshot.source_size,
        &snapshot.screen,
        finishing,
    );
    if screen_at(app, snapshot.screen.x, snapshot.screen.y)? != snapshot.screen {
        return Err("显示器布局已变化，仅输出变化前已确认的部分。".into());
    }
    let before_pixels = snapshot.stitch.covered_pixels();
    let before_bounds = snapshot.stitch.bounds;
    let before_viewport = snapshot.stitch.viewport();
    let result = if let capture::SampleOutcome::Stable(second) = settled {
        let second = Arc::new(second);
        if !stability::stable(snapshot.stitch.last(), &second) {
            if let Some(archive) = &snapshot.archive {
                if let Err(_error) = archive.frame(second.clone()) {
                    log::warn!(
                        "event=scroll.archive_failed module=scroll code=ARCHIVE_FRAME_FAILED"
                    );
                    snapshot.archive = None;
                }
            }
        }
        let started = Instant::now();
        let result = snapshot.recovery.consider(&mut snapshot.stitch, second)?;
        if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
            log::debug!(
                "event=scroll.recovery_match module=scroll duration_ms={:.2}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(archive) = &snapshot.archive {
            if let Err(_error) = archive.record(
                serde_json::json!({"event":"match", "result":format!("{result:?}"),
                "frames":snapshot.stitch.count(), "height":snapshot.stitch.height}),
            ) {
                log::warn!("event=scroll.archive_failed module=scroll code=ARCHIVE_MATCH_FAILED");
                snapshot.archive = None;
            }
        }
        Some(result)
    } else {
        match settled {
            capture::SampleOutcome::CaptureFailure(error) => return Err(error),
            capture::SampleOutcome::Cancelled | capture::SampleOutcome::Stopped => return Ok(None),
            _ => None,
        }
    };
    let canvas_changed = snapshot.stitch.covered_pixels() != before_pixels
        || snapshot.stitch.bounds != before_bounds;
    if canvas_changed {
        snapshot.last_growth = Instant::now();
    }
    let no_progress = snapshot.last_growth.elapsed() >= Duration::from_secs(3);
    let status = if finishing {
        "finishing"
    } else if snapshot.recovery.is_recovering() {
        "recovering"
    } else if no_progress {
        "no_progress"
    } else if result.is_none() {
        "sampling"
    } else {
        "collecting"
    };
    let message = if status == "no_progress" {
        if result.is_none() {
            "页面还在动，请停一下或暂停动画。"
        } else {
            "暂无新增，可继续滚动或点“完成”。"
        }
    } else {
        snapshot.recovery.message(result)
    };
    let changed = snapshot.message != message
        || snapshot.status != status
        || canvas_changed
        || snapshot.stitch.viewport() != before_viewport;
    snapshot.message = message.into();
    snapshot.status = status;
    if canvas_changed {
        refresh_preview(&mut snapshot)?;
    }
    snapshot.revision += 1;
    // 计算期间控制命令可以立即设置停止/取消；过期快照不得覆盖新会话。
    let update = if changed {
        Some(view(&snapshot, true)?)
    } else {
        None
    };
    // 拼接计算不持锁；计算期间改变屏幕布局时，候选快照也不得提交。
    if screen_at(app, snapshot.screen.x, snapshot.screen.y)? != snapshot.screen {
        return Err("显示器布局已变化，仅输出变化前已确认的部分。".into());
    }
    {
        let mut guard = state.scroll.lock().map_err(err)?;
        let current = guard
            .as_mut()
            .filter(|s| s.id == id && s.revision == revision)
            .ok_or("滚动截图状态已更新，丢弃过期计算结果。")?;
        // 控制命令和提交共用会话锁：停止后不提交普通采样；部分保存连末帧也不追加。
        if state.scroll_cancel.load(Ordering::SeqCst)
            || current.confirmed_only
            || (!finishing && state.scroll_stop.load(Ordering::SeqCst))
        {
            return Ok(None);
        }
        *current = snapshot;
    }
    if let Some(update) = update {
        let _ = app.emit_to("scroll-preview", "scroll-updated", &update);
        let _ = app.emit_to("scroll-controls", "scroll-updated", &update);
    }
    Ok(result)
}

/// 缩略图仅在可信成图改变时编码；状态查询复用缓存，不反复编码 PNG。
fn refresh_preview(session: &mut ScrollSession) -> Result<(), String> {
    let started = Instant::now();
    let mut bytes = Cursor::new(Vec::new());
    session
        .stitch
        .preview(360, 600)
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(err)?;
    session.preview = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes.into_inner())
    )
    .into();
    if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
        log::debug!(
            "event=scroll.preview module=scroll duration_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

/// 锁争用也向两个窗口发布状态；以会话 ID 和 revision 防止旧结果覆盖新会话。
fn notify_waiting(app: &AppHandle, state: &CaptureState, id: u64, finishing: bool) {
    let Ok(mut guard) = state.scroll.lock() else {
        return;
    };
    let Some(session) = guard.as_mut().filter(|s| s.id == id) else {
        return;
    };
    let status = if finishing { "finishing" } else { "waiting" };
    let message = if finishing {
        "正在等待当前操作完成后保存。"
    } else {
        "正在处理其他截图，请稍候。"
    };
    if session.status == status && session.message == message {
        return;
    }
    session.status = status;
    session.message = message.into();
    session.revision += 1;
    if let Ok(update) = view(session, true) {
        for label in ["scroll-preview", "scroll-controls"] {
            let _ = app.emit_to(label, "scroll-updated", &update);
        }
    }
    log::warn!("event=scroll.operation_wait module=scroll op=scroll-{id} finishing={finishing}");
}

fn collect(app: AppHandle, id: u64, first_delay: u64) {
    let state = app.state::<CaptureState>();
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut message = "滚动截图采集结束。".to_string();
    let mut finish_latest = true;
    let mut delay = first_delay;
    let mut last_operation = Instant::now();
    while finish_latest && wait(&state, delay) {
        delay = 40;
        if Instant::now() >= deadline {
            message = "采集达到 5 分钟上限，已自动结束。".into();
            finish_latest = false;
            break;
        }
        let Ok(_operation) = Operation::acquire(&state.operation) else {
            if last_operation.elapsed() >= Duration::from_secs(1) {
                notify_waiting(&app, &state, id, false);
            }
            continue;
        };
        last_operation = Instant::now();
        let decision = (|| -> Result<(), String> {
            sample_pair(&app, &state, id, false)?;
            if state.scroll_stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            // 不确定匹配只退避重采，可信锚点始终保留，不按重试次数结束。
            let guard = state.scroll.lock().map_err(err)?;
            if let Some(session) = guard.as_ref().filter(|s| s.id == id) {
                delay = session.recovery.delay();
            }
            Ok(())
        })();
        if let Err(error) = decision {
            message = error;
            finish_latest = false;
            break;
        }
    }
    // 等待短暂的读取操作释放互斥；旧会话不得弹出窗口或覆盖新会话。
    loop {
        // 即使操作锁长期占用，旧线程也不应永久等待或污染新会话。
        if !state
            .scroll
            .lock()
            .ok()
            .is_some_and(|s| s.as_ref().is_some_and(|s| s.id == id))
        {
            break;
        }
        let Ok(_operation) = Operation::acquire(&state.operation) else {
            if last_operation.elapsed() >= Duration::from_secs(1) {
                notify_waiting(&app, &state, id, true);
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        // 旧采集线程不得关闭新会话的预览或重置其状态。
        if !state
            .scroll
            .lock()
            .ok()
            .is_some_and(|s| s.as_ref().is_some_and(|s| s.id == id))
        {
            break;
        }
        if state.scroll_cancel.load(Ordering::SeqCst) {
            let _ = close_session(&app, id);
            break;
        }
        let confirmed_only = state.scroll.lock().ok().is_some_and(|s| {
            s.as_ref()
                .is_some_and(|s| s.id == id && (s.confirmed_only || s.recovery.is_recovering()))
        });
        if confirmed_only {
            message.push_str(" 仅输出已确认部分；未匹配画面未追加。");
        }
        if finish_latest && !confirmed_only {
            // 等待托盘菜单收起；采集原窗口的最终静止画面，避免刚滚动完就结束而漏掉末尾。
            std::thread::sleep(Duration::from_millis(180));
            match sample_pair(&app, &state, id, true) {
                Ok(Some(Match::Uncertain)) => {
                    message = "末帧无法可靠拼接，已自动排除，仅输出已确认部分。".into()
                }
                Ok(Some(_)) => {}
                Ok(None) => message = "结束时画面仍在运动，末帧未追加，仅输出已确认部分。".into(),
                Err(error) => message = error,
            }
        }
        if state.scroll_cancel.load(Ordering::SeqCst) {
            let _ = close_session(&app, id);
            break;
        }
        chrome::hide(&app);
        state.scroll_collecting.store(false, Ordering::SeqCst);
        state.scroll_stop.store(true, Ordering::SeqCst);
        let mut failure = None;
        for _ in 0..3 {
            match complete(&app, id, &message) {
                Ok(()) => {
                    failure = None;
                    break;
                }
                Err(error) => {
                    failure = Some(error);
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        if let Some(error) = failure {
            log::error!(
                "event=scroll.failed module=scroll op=scroll-{id} code=SCROLL_OUTPUT_FAILED"
            );
            if let Ok(mut guard) = state.scroll.lock() {
                if let Some(session) = guard.as_mut().filter(|s| s.id == id) {
                    session.status = "failed";
                    session.message = format!("输出失败，结果仍保留：{error}");
                    session.revision += 1;
                    if let Ok(update) = view(session, false) {
                        let _ = app.emit_to("capture", "scroll-updated", &update);
                    }
                }
            }
            report_error(
                &app,
                format!("滚动截图输出重试失败，内存结果仍保留：{error}"),
            );
        }
        break;
    }
}

fn launch(app: &AppHandle, id: u64, first_delay: u64) -> Result<(), String> {
    #[cfg(windows)]
    let listener = {
        let state = app.state::<CaptureState>();
        let guard = state.scroll.lock().map_err(err)?;
        let s = guard
            .as_ref()
            .filter(|s| s.id == id)
            .ok_or("滚动截图会话已结束。")?;
        let x = i64::from(s.screen.x) + i64::from(s.rect.x);
        let y = i64::from(s.screen.y) + i64::from(s.rect.y);
        let cancel_app = app.clone();
        let finish_app = app.clone();
        super::input::Listener::start(
            s.direction.clone(),
            (
                x,
                y,
                x + i64::from(s.rect.width),
                y + i64::from(s.rect.height),
            ),
            move || cancel_current(&cancel_app, id, "escape"),
            move || finish_current(&finish_app, id, None).is_ok(),
        )?
    };
    let app = app.clone();
    std::thread::Builder::new()
        .name("scroll-capture".into())
        .spawn(move || {
            #[cfg(windows)]
            let _listener = listener;
            collect(app, id, first_delay)
        })
        .map(|_| ())
        .map_err(err)
}

#[tauri::command]
pub async fn start_scroll(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    save_frames: bool,
    background: Option<ExportBackground>,
) -> Result<ScrollView, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("滚动截图会话已存在。".into());
        }
        let seed = crop(&app, id, rect, &[])?;
        let (rect, screen) = super::super::selected_region(&app, id, rect)?;
        let source_size = (screen.width, screen.height);
        // 先校验选区容量，非法选区不创建持久化目录。
        let stitch = Stitch::with_mode(seed, ScrollMode::Auto)?;
        let mut archive = if save_frames {
            match Archive::new(&app) {
                Ok(archive) => Some(archive),
                Err(_error) => {
                    log::warn!(
                        "event=scroll.archive_failed module=scroll code=ARCHIVE_CREATE_FAILED"
                    );
                    None
                }
            }
        } else {
            None
        };
        if let Some(writer) = &archive {
            if let Err(_error) = writer
                .record(serde_json::json!({"event":"session", "version":1,
                "width":rect.width, "height":rect.height, "automatic":false}))
                .and_then(|_| writer.frame(stitch.last_shared()))
            {
                log::warn!("event=scroll.archive_failed module=scroll code=ARCHIVE_INIT_FAILED");
                archive = None;
            }
        }
        let mut pending_windows = chrome::PendingScrollWindows(&app, false);
        let obscuring_windows = chrome::place(&app, &screen, rect)?;
        let mut session = ScrollSession {
            id,
            revision: 1,
            rect,
            screen,
            obscuring_windows,
            source_size,
            stitch,
            direction: Direction::default(),
            recovery: recovery::Recovery::default(),
            message: "在选区内慢慢滚动，横向或纵向均可。".into(),
            archive,
            preview: Arc::from(""),
            status: "sampling",
            last_growth: Instant::now(),
            background: background.unwrap_or_default(),
            confirmed_only: false,
        };
        refresh_preview(&mut session)?;
        let result = view(&session, true)?;
        super::super::hide_overlay(&app, &window)?;
        *state.scroll.lock().map_err(err)? = Some(session);
        state.scroll_cancel.store(false, Ordering::SeqCst);
        state.scroll_stop.store(false, Ordering::SeqCst);
        state.scroll_collecting.store(true, Ordering::SeqCst);
        for label in ["scroll-preview", "scroll-controls"] {
            let _ = app.emit_to(label, "scroll-updated", &result);
        }
        if let Err(error) = chrome::show(&app).and_then(|_| launch(&app, id, 120)) {
            state.scroll_collecting.store(false, Ordering::SeqCst);
            *state.scroll.lock().map_err(err)? = None;
            chrome::hide(&app);
            let _ = window.show();
            return Err(error);
        }
        // 后续匹配仅使用选区帧/图块；不再常驻整桌面的 RGBA 和 Base64。
        // Operation 在本闭包结束前仍被持有，采集线程不会抢先结束会话。
        super::super::release_selection(&app, id)?;
        pending_windows.1 = true;
        log::info!(
            "event=scroll.started module=scroll op=scroll-{id} width={} height={}",
            result.width,
            result.height
        );
        Ok(result)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn current_scroll(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<ScrollView>, String> {
    if !matches!(
        window.label(),
        "capture" | "scroll-preview" | "scroll-controls"
    ) {
        return Err("窗口无权读取滚动截图。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let guard = state.scroll.lock().map_err(err)?;
        guard
            .as_ref()
            .map(|s| view(s, state.scroll_collecting.load(Ordering::SeqCst)))
            .transpose()
    })
    .await
    .map_err(err)?
}
