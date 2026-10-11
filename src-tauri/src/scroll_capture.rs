//! 截图会话内的长截图工作流。采集时隐藏蒙层，让鼠标滚轮直接作用于原应用。
//! 截图快捷键/托盘可停止采集；自动模式仅在鼠标留在选区时发送滚轮。
use super::*;
#[path = "scroll_archive.rs"]
mod archive;
#[path = "scroll_driver.rs"]
mod driver;
use crate::stitch::{Match, Stitch};
use archive::Archive;
use driver::Driver;

#[derive(PartialEq)]
struct Screen {
    name: Option<String>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: u64,
}

fn screen(app: &AppHandle) -> Result<Screen, String> {
    let monitor = app
        .primary_monitor()
        .map_err(err)?
        .ok_or("主显示器不可用。")?;
    Ok(Screen {
        name: monitor.name().cloned(),
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale: monitor.scale_factor().to_bits(),
    })
}

pub(super) struct ScrollSession {
    id: u64,
    revision: u64,
    rect: Rect,
    screen: Screen,
    source_size: (u32, u32),
    stitch: Stitch,
    message: String,
    automatic: bool,
    preview_overlaps: bool,
    archive: Option<Archive>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollView {
    id: u64,
    revision: u64,
    collecting: bool,
    automatic: bool,
    archive_path: Option<String>,
    width: u32,
    height: u32,
    frames: usize,
    preview: String,
    frame_height: u32,
    message: String,
}

// 采集时只传分段缩略图，导出仍使用原始像素。
fn view(session: &ScrollSession, collecting: bool) -> Result<ScrollView, String> {
    Ok(ScrollView {
        id: session.id,
        revision: session.revision,
        collecting,
        automatic: session.automatic,
        archive_path: session
            .archive
            .as_ref()
            .map(|a| a.path.to_string_lossy().into_owned()),
        width: session.stitch.width,
        height: session.stitch.height,
        frames: session.stitch.count(),
        frame_height: session.stitch.frame_height,
        preview: if collecting {
            let mut bytes = Cursor::new(Vec::new());
            session
                .stitch
                .preview(240, 900)
                .write_to(&mut bytes, ImageFormat::Png)
                .map_err(err)?;
            format!(
                "data:image/png;base64,{}",
                STANDARD.encode(bytes.into_inner())
            )
        } else {
            String::new()
        },
        message: session.message.clone(),
    })
}

/// 优先右侧，其次左侧、下方、上方；无空位时靠屏幕右侧，并在取样时隐藏。
fn place_preview(app: &AppHandle, screen: &Screen, rect: Rect) -> Result<bool, String> {
    let window = app
        .get_webview_window("scroll-preview")
        .ok_or("实时预览窗口不可用。")?;
    let scale = f64::from_bits(screen.scale);
    let w = (220.0 * scale) as u32;
    let h = ((420.0 * scale) as u32).min(screen.height);
    let w = w.min(screen.width);
    let gap = (12.0 * scale) as u32;
    let candidates = [
        (
            i64::from(rect.x + rect.width + gap),
            i64::from(rect.y.min(screen.height - h)),
        ),
        (
            i64::from(rect.x) - i64::from(w + gap),
            i64::from(rect.y.min(screen.height - h)),
        ),
        (
            i64::from(rect.x.min(screen.width - w)),
            i64::from(rect.y + rect.height + gap),
        ),
        (
            i64::from(rect.x.min(screen.width - w)),
            i64::from(rect.y) - i64::from(h + gap),
        ),
    ];
    let position = candidates.into_iter().find(|&(x, y)| {
        x >= 0
            && y >= 0
            && x + i64::from(w) <= i64::from(screen.width)
            && y + i64::from(h) <= i64::from(screen.height)
    });
    let overlaps = position.is_none();
    let (x, y) = position.unwrap_or((i64::from(screen.width - w), 0));
    window
        .set_size(tauri::PhysicalSize::new(w, h))
        .map_err(err)?;
    window
        .set_position(tauri::PhysicalPosition::new(
            screen.x + x as i32,
            screen.y + y as i32,
        ))
        .map_err(err)?;
    Ok(overlaps)
}

/// 降级位置可能覆盖选区，取样前隐藏；离开函数的所有路径都会恢复预览。
struct PreviewSample(Option<WebviewWindow>);
impl Drop for PreviewSample {
    fn drop(&mut self) {
        if let Some(w) = &self.0 {
            let _ = w.show();
        }
    }
}

#[tauri::command]
pub fn control_scroll(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    cancel: bool,
) -> Result<(), String> {
    require_window(&window, "scroll-preview")?;
    let state = app.state::<CaptureState>();
    let guard = state.scroll.lock().map_err(err)?;
    if !guard.as_ref().is_some_and(|s| s.id == id) {
        return Err("长截图已结束。".into());
    }
    if cancel {
        state.scroll_cancel.store(true, Ordering::SeqCst);
    }
    state.scroll_stop.store(true, Ordering::SeqCst);
    Ok(())
}

/// 后台直接落盘并复制。匹配不可靠的候选帧不会进入成图，不要求用户修接缝。
fn complete(app: &AppHandle, id: u64, message: &str) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    let (image, result) = {
        let mut guard = state.scroll.lock().map_err(err)?;
        let Some(s) = guard.as_mut().filter(|s| s.id == id) else {
            return Ok(());
        };
        s.stitch.pending = None;
        s.revision += 1;
        s.message = message.into();
        (s.stitch.compose(), view(s, false)?)
    };
    let saved = (|| -> Result<std::path::PathBuf, String> {
        let root = app
            .path()
            .app_local_data_dir()
            .map_err(err)?
            .join("scroll-results");
        std::fs::create_dir_all(&root).map_err(err)?;
        let mut file = tempfile::Builder::new()
            .prefix("scroll-")
            .suffix(".png")
            .tempfile_in(root)
            .map_err(err)?;
        image.write_to(&mut file, ImageFormat::Png).map_err(err)?;
        file.as_file().sync_all().map_err(err)?;
        let (_, path) = file.keep().map_err(err)?;
        Ok(path)
    })();
    let copied = crate::image_io::copy(app, &image);
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
    close_session(app, id)?;
    let _ = app.emit_to("capture", "scroll-updated", &result);
    let _ = app.emit_to("main", "capture-result", format!("{message} {output}"));
    show_main(app);
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

fn wait(state: &CaptureState, millis: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        if state.scroll_stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn sample(app: &AppHandle, region: Rect, source_size: (u32, u32)) -> Result<RgbaImage, String> {
    let image = crate::platform::capture_primary()?;
    if image.dimensions() != source_size {
        return Err("显示器像素尺寸发生变化，已结束采集，仅输出已确认部分。".into());
    }
    region.validate(image.width(), image.height())?;
    Ok(
        image::imageops::crop_imm(&image, region.x, region.y, region.width, region.height)
            .to_image(),
    )
}

/// 调用方持有截图操作锁。结束前的最后取样忽略停止标志，但仍必须通过稳定性检查。
fn sample_pair(
    app: &AppHandle,
    state: &CaptureState,
    id: u64,
    finishing: bool,
) -> Result<Option<Match>, String> {
    let (rect, source_size, overlaps) = {
        let guard = state.scroll.lock().map_err(err)?;
        let session = guard
            .as_ref()
            .filter(|s| s.id == id)
            .ok_or("长截图会话已结束。")?;
        if screen(app)? != session.screen {
            return Err("主显示器位置、尺寸或 DPI 发生变化，已结束采集，仅输出已确认部分。".into());
        }
        (session.rect, session.source_size, session.preview_overlaps)
    };
    let preview = if overlaps {
        app.get_webview_window("scroll-preview")
    } else {
        None
    };
    if let Some(window) = &preview {
        window.hide().map_err(err)?;
        std::thread::sleep(Duration::from_millis(80));
    }
    let _preview_guard = PreviewSample(preview);
    let first = sample(app, rect, source_size)?;
    if finishing {
        std::thread::sleep(Duration::from_millis(160));
    } else if !wait(state, 160) {
        return Ok(None);
    }
    let second = sample(app, rect, source_size)?;
    if !finishing && state.scroll_stop.load(Ordering::SeqCst) {
        return Ok(None);
    }
    if !crate::stitch::stable(&first, &second) {
        return Ok(None);
    }
    let mut guard = state.scroll.lock().map_err(err)?;
    let session = guard
        .as_mut()
        .filter(|s| s.id == id)
        .ok_or("长截图会话已结束。")?;
    // 匹配之前保存候选帧，失败时也能排查；重复静止帧不重复写盘。
    if !crate::stitch::stable(session.stitch.last(), &second) {
        if let Some(archive) = &mut session.archive {
            archive.frame(&second)?;
        }
    }
    let result = session.stitch.consider(second)?;
    if let Some(archive) = &session.archive {
        archive.record(
            serde_json::json!({"event":"match", "result":format!("{result:?}"),
            "frames":session.stitch.count(), "height":session.stitch.height}),
        )?;
    }
    if matches!(result, Match::Overlap(_)) {
        session.revision += 1;
        let update = view(session, true)?;
        // 只有追加成功时推送，重复帧不会反复编码或发送。
        let _ = app.emit_to("scroll-preview", "scroll-updated", &update);
    }
    Ok(Some(result))
}

/// 选区以截图物理像素保存，转换到输入系统坐标后再检查指针。
fn check_driver(
    app: &AppHandle,
    state: &CaptureState,
    id: u64,
    driver: &mut Driver,
) -> Result<(), String> {
    let guard = state.scroll.lock().map_err(err)?;
    let s = guard
        .as_ref()
        .filter(|s| s.id == id)
        .ok_or("长截图已结束。")?;
    if screen(app)? != s.screen {
        return Err("显示器信息变化，已结束采集，仅输出已确认部分。".into());
    }
    let scale = if cfg!(target_os = "macos") {
        f64::from_bits(s.screen.scale)
    } else {
        1.0
    };
    let x = (f64::from(s.screen.x) + f64::from(s.rect.x)) / scale;
    // enigo 0.6.1 的 macOS location 用物理屏高减 NSEvent 逻辑 y，需补偿两种单位的差。
    let mac_origin = if cfg!(target_os = "macos") {
        f64::from(s.screen.height) * (1.0 - 1.0 / scale)
    } else {
        0.0
    };
    let y = (f64::from(s.screen.y) + f64::from(s.rect.y)) / scale + mac_origin;
    driver.check((
        x,
        y,
        x + f64::from(s.rect.width) / scale,
        y + f64::from(s.rect.height) / scale,
    ))
}

fn collect(app: AppHandle, id: u64, first_delay: u64) {
    let state = app.state::<CaptureState>();
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut message = "长截图采集结束。".to_string();
    let mut finish_latest = true;
    let automatic = state
        .scroll
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|s| s.automatic))
        .unwrap_or(false);
    let mut driver = if automatic {
        match Driver::new() {
            Ok(driver) => Some(driver),
            Err(error) => {
                message = format!("自动滚动初始化失败：{error}。已结束采集。");
                finish_latest = false;
                None
            }
        }
    } else {
        None
    };
    let mut delay = first_delay;
    let mut manual_retries = 0;
    while finish_latest && wait(&state, delay) {
        delay = 700;
        if Instant::now() >= deadline {
            message = "采集达到 5 分钟上限，已自动结束。".into();
            finish_latest = false;
            break;
        }
        let Ok(_operation) = Operation::acquire(&state.operation) else {
            continue;
        };
        let decision = (|| -> Result<(), String> {
            if let Some(driver) = &mut driver {
                check_driver(&app, &state, id, driver)?;
            }
            let result = sample_pair(&app, &state, id, false)?;
            if state.scroll_stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            if let Some(driver) = &mut driver {
                match result {
                    None => {
                        driver.moving += 1;
                        driver.unchanged = 0;
                        if driver.moving >= 12 {
                            return Err("画面持续变化，已自动结束，仅输出已确认部分。".into());
                        }
                        return Ok(());
                    }
                    Some(Match::Uncertain) => {
                        driver.retries += 1;
                        driver.unchanged = 0;
                        driver.step = 1;
                        delay = 1200;
                        if driver.retries <= 2 {
                            return Ok(());
                        }
                        return Err(
                            "同一位置重试 2 次后仍无法匹配，已自动结束，仅输出已确认部分。".into(),
                        );
                    }
                    Some(Match::Unchanged) => {
                        if driver.scrolled {
                            driver.unchanged += 1;
                        }
                        if driver.unchanged >= 4 {
                            return Err("连续 4 次滚动后画面未移动：可能已到底或窗口不接受滚轮，已自动停止。".into());
                        }
                        delay = 1200;
                    }
                    Some(Match::Overlap(overlap)) => {
                        let guard = state.scroll.lock().map_err(err)?;
                        let session = guard.as_ref().ok_or("长截图已结束。")?;
                        driver.adapt(
                            session.stitch.frame_height - overlap,
                            session.stitch.frame_height,
                        );
                    }
                }
                driver.retries = 0;
                driver.moving = 0;
                // 取样期间也可能移动鼠标，注入前再次检查；停止后不发送新滚轮。
                check_driver(&app, &state, id, driver)?;
                if !state.scroll_stop.load(Ordering::SeqCst) {
                    // 满屏降级预览可能位于鼠标下方；发滚轮时也隐藏，保证目标应用接收事件。
                    let overlaps = state
                        .scroll
                        .lock()
                        .map_err(err)?
                        .as_ref()
                        .is_some_and(|s| s.preview_overlaps);
                    let preview = if overlaps {
                        app.get_webview_window("scroll-preview")
                    } else {
                        None
                    };
                    if let Some(window) = &preview {
                        window.hide().map_err(err)?;
                        std::thread::sleep(Duration::from_millis(80));
                    }
                    let _preview_guard = PreviewSample(preview);
                    if !state.scroll_stop.load(Ordering::SeqCst) {
                        driver.scroll()?;
                    }
                }
            } else if result == Some(Match::Uncertain) {
                manual_retries += 1;
                delay = 1200;
                if manual_retries > 2 {
                    return Err("重叠匹配重试失败，已自动结束，仅输出已确认部分。".into());
                }
            } else {
                manual_retries = 0;
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
        let Ok(_operation) = Operation::acquire(&state.operation) else {
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
        if finish_latest {
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
        if let Some(preview) = app.get_webview_window("scroll-preview") {
            let _ = preview.hide();
        }
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
            report_error(&app, format!("长截图输出重试失败，内存结果仍保留：{error}"));
            show_main(&app);
        }
        break;
    }
}

fn launch(app: &AppHandle, id: u64, first_delay: u64) -> Result<(), String> {
    let app = app.clone();
    std::thread::Builder::new()
        .name("scroll-capture".into())
        .spawn(move || collect(app, id, first_delay))
        .map(|_| ())
        .map_err(err)
}

#[tauri::command]
pub async fn start_scroll(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    automatic: bool,
    save_frames: bool,
) -> Result<ScrollView, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("长截图会话已存在。".into());
        }
        let seed = crop(&app, id, rect, &[])?;
        let source_size = state
            .session
            .lock()
            .map_err(err)?
            .as_ref()
            .filter(|s| s.frame.id == id)
            .map(|s| s.image.dimensions())
            .ok_or("截图已过期。")?;
        // 先校验选区容量，非法选区不创建持久化目录。
        let stitch = Stitch::new(seed)?;
        let mut archive = if save_frames {
            Some(Archive::new(&app)?)
        } else {
            None
        };
        if let Some(archive) = &mut archive {
            archive.record(serde_json::json!({"event":"session", "version":1,
                "width":rect.width, "height":rect.height, "automatic":automatic}))?;
            archive.frame(stitch.last())?;
        }
        let screen = screen(&app)?;
        let preview_overlaps = place_preview(&app, &screen, rect)?;
        let session = ScrollSession {
            id,
            revision: 1,
            rect,
            screen,
            preview_overlaps,
            source_size,
            stitch,
            message: String::new(),
            automatic,
            archive,
        };
        let result = view(&session, true)?;
        window.hide().map_err(err)?;
        *state.scroll.lock().map_err(err)? = Some(session);
        state.scroll_cancel.store(false, Ordering::SeqCst);
        state.scroll_stop.store(false, Ordering::SeqCst);
        state.scroll_collecting.store(true, Ordering::SeqCst);
        if let Some(preview) = app.get_webview_window("scroll-preview") {
            let _ = app.emit_to("scroll-preview", "scroll-updated", &result);
            let _ = preview.show();
        }
        if let Err(error) = launch(&app, id, 2000) {
            state.scroll_collecting.store(false, Ordering::SeqCst);
            *state.scroll.lock().map_err(err)? = None;
            if let Some(preview) = app.get_webview_window("scroll-preview") {
                let _ = preview.hide();
            }
            let _ = window.show();
            return Err(error);
        }
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
    if window.label() != "capture" && window.label() != "scroll-preview" {
        return Err("窗口无权读取长截图。".into());
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
