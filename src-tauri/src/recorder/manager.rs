//! 录屏会话管理：命令、UI、WGC 采集和有界编码线程的生命周期。
use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, TrySendError},
};
mod capture;
mod config;
mod encoder;
mod frame;
pub mod output;
mod timeline;
use super::scroll_capture::chrome;
use config::{CaptureSource, OutputFormat, RecorderCapabilities, RecorderConfig};
use frame::QueuedFrame;
use timeline::Timeline;

const MAX_SECONDS: f64 = 180.0;
// 尚未完成长时间资源验收，保留 3 分钟限制；不再限制 PNG 字节数或 1800 帧。

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Ready,
    Recording,
    Paused,
    Encoding,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingView {
    id: u64,
    revision: u64,
    phase: Phase,
    seconds: f64,
    frames: usize,
    dropped_frames: usize,
    target_fps: u32,
    message: String,
    path: Option<String>,
}

/// 状态锁仅保护时间轴和帧提交；系统取样/编码在锁外进行，暂停通过代次淘汰旧帧。
struct Session {
    view: RecordingView,
    timeline: Timeline,
    stop: bool,
    cancel: bool,
}
impl Session {
    fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.timeline.elapsed(capture::now()).max(0) as u64 * 100)
    }
    fn freeze(&mut self) {
        self.timeline.stop(capture::now());
        self.view.seconds = self.elapsed().as_secs_f64();
    }
}

#[derive(Default)]
pub struct RecordingState(Mutex<Option<Session>>, Mutex<()>, AtomicBool);

fn active(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Ready | Phase::Recording | Phase::Paused | Phase::Encoding
    )
}

/// 托盘、截图快捷键和退出入口只发停止信号，清理始终由录屏工作线程完成。
pub fn request_stop(app: &AppHandle, cancel: bool) -> bool {
    let state = app.state::<RecordingState>();
    let Ok(mut guard) = state.0.lock() else {
        return false;
    };
    let Some(s) = guard.as_mut().filter(|s| active(s.view.phase)) else {
        return false;
    };
    s.freeze();
    s.stop = true;
    s.cancel |= cancel || s.view.phase == Phase::Ready;
    s.view.revision += 1;
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingAction {
    Start,
    Pause,
    Resume,
    Stop,
    Cancel,
}

/// 悬浮条暂停保留现场，托盘暂停可展开主页；恢复仅隐藏主页，边框和控制条不收起。
pub fn control(app: &AppHandle, id: Option<u64>, action: RecordingAction) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    // 控制动作串行，但窗口操作不得持有状态锁，避免 UI 查询状态时相互等待。
    let _control = state.1.lock().map_err(err)?;
    let mut guard = state.0.lock().map_err(err)?;
    let s = guard
        .as_mut()
        .filter(|s| id.is_none_or(|id| s.view.id == id))
        .ok_or("录屏会话已过期。")?;
    // 已完成/取消的同会话停止动作幂等；旧 ID 不能触及新会话。
    if !active(s.view.phase) {
        return if matches!(action, RecordingAction::Stop | RecordingAction::Cancel) {
            Ok(())
        } else {
            Err("录屏已结束。".into())
        };
    }
    match action {
        RecordingAction::Pause
            if matches!(s.view.phase, Phase::Recording | Phase::Paused) && !s.stop =>
        {
            s.timeline.pause(capture::now());
            s.view.seconds = s.elapsed().as_secs_f64();
            s.view.phase = Phase::Paused;
            s.view.message = "录屏已暂停，可恢复或停止导出。".into();
            s.view.revision += 1;
            drop(guard);
            // 悬浮条暂停不展开主窗口，录制范围及控制条持续可见；托盘仍可打开主页。
            if id.is_none() {
                show_main(app);
                let _ = app.emit_to("main", "capture-result", "录屏已暂停，可恢复或停止导出。");
            }
            return Ok(());
        }
        RecordingAction::Start | RecordingAction::Resume
            if !s.stop
                && ((matches!(action, RecordingAction::Start) && s.view.phase == Phase::Ready)
                    || (matches!(action, RecordingAction::Resume)
                        && s.view.phase == Phase::Paused)) =>
        {
            let session_id = s.view.id;
            let expected_phase = s.view.phase;
            drop(guard);
            if let Some(main) = app.get_webview_window("main") {
                main.hide().map_err(err)?;
            }
            // 此时仍是待开始/暂停状态；窗口完全移除后才开启计时和取样。
            std::thread::sleep(Duration::from_millis(250));
            let mut guard = state.0.lock().map_err(err)?;
            let s = guard
                .as_mut()
                .filter(|s| s.view.id == session_id && s.view.phase == expected_phase && !s.stop)
                .ok_or("录屏已停止。")?;
            s.timeline.resume(capture::now());
            s.view.phase = Phase::Recording;
            s.view.message = "录屏中；托盘可暂停，截图快捷键可停止。".into();
            s.view.revision += 1;
            return Ok(());
        }
        RecordingAction::Stop | RecordingAction::Cancel => {
            s.freeze();
            s.stop = true;
            s.cancel |= matches!(action, RecordingAction::Cancel) || s.view.phase == Phase::Ready;
        }
        _ => return Err("当前录屏状态不支持此操作。".into()),
    }
    s.view.revision += 1;
    Ok(())
}

#[tauri::command]
pub async fn control_recording(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    action: RecordingAction,
) -> Result<(), String> {
    require_control_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || control(&app, Some(id), action))
        .await
        .map_err(err)?
}

#[tauri::command]
pub fn current_recording(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<RecordingView>, String> {
    require_control_window(&window)?;
    let state = app.state::<RecordingState>();
    let guard = state.0.lock().map_err(err)?;
    if window.label() == "recording-controls" {
        state.2.store(true, Ordering::SeqCst);
    }
    Ok(guard.as_ref().map(|s| {
        let mut view = s.view.clone();
        view.seconds = s.elapsed().as_secs_f64();
        view
    }))
}

fn require_control_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "recording-controls") {
        Ok(())
    } else {
        Err("当前窗口无权控制录屏。".into())
    }
}

/// 能力查询会实际试写小尺寸 MP4；具体选区仍在准备时再次探测。
#[tauri::command]
pub async fn recording_capabilities(window: WebviewWindow) -> Result<RecorderCapabilities, String> {
    if !matches!(window.label(), "main" | "capture") {
        return Err("当前窗口无权查询录屏能力。".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let result = capture::supported().and_then(|_| {
            let root = tempfile::tempdir().map_err(err)?;
            encoder::probe(root.path(), 64, 64, 30)
        });
        let available = result.is_ok();
        RecorderCapabilities {
            available,
            reason: result.err(),
            sources: if available {
                vec![CaptureSource::PrimaryMonitor]
            } else {
                vec![]
            },
            frame_rates: if available { vec![30, 60] } else { vec![] },
            capture_cursor: available,
            system_audio: false,
            microphone: false,
            formats: if available {
                vec![OutputFormat::Mp4]
            } else {
                vec![]
            },
        }
    })
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    config: Option<RecorderConfig>,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("请先结束长截图。".into());
        }
        let size = state
            .session
            .lock()
            .map_err(err)?
            .as_ref()
            .filter(|s| s.frame.id == id)
            .map(|s| s.image.dimensions())
            .ok_or("截图已过期。")?;
        let config = config.unwrap_or_else(|| RecorderConfig::for_region(rect));
        if (
            config.region.x,
            config.region.y,
            config.region.width,
            config.region.height,
        ) != (rect.x, rect.y, rect.width, rect.height)
        {
            return Err("录屏配置与确认选区不一致。".into());
        }
        config.validate(size)?;
        capture::supported()?;
        // 准备时固定本次目录；临时 MP4 和最终文件同盘，停止时无需跨盘移动。
        let root = output::prepare(&app)?;
        encoder::probe(&root, rect.width, rect.height, config.fps)?;
        let screen = super::scroll_capture::screen(&app)?;
        chrome::place_recording(&app, &screen, rect)?;
        window.hide().map_err(err)?;
        let recording = app.state::<RecordingState>();
        *recording.0.lock().map_err(err)? = Some(Session {
            view: RecordingView {
                id,
                revision: 1,
                phase: Phase::Ready,
                seconds: 0.0,
                frames: 0,
                dropped_frames: 0,
                target_fps: config.fps,
                message: "录屏选区已准备，请点击下方“开始录制”。".into(),
                path: None,
            },
            timeline: Timeline::default(),
            stop: false,
            cancel: false,
        });
        // WGC 必须在所属线程初始化/释放。先握手确认，再显示可操作的控制条。
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (go_tx, go_rx) = mpsc::sync_channel(1);
        let worker_app = app.clone();
        let worker = std::thread::Builder::new()
            .name("screen-recording".into())
            .spawn(move || match capture::prepare(&config, size) {
                Ok(backend) => {
                    if ready_tx.send(Ok(())).is_ok() && go_rx.recv().unwrap_or(false) {
                        collect(worker_app, id, config, screen, root, backend);
                    }
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            });
        let ready = match worker {
            Ok(_) => ready_rx.recv().map_err(err).and_then(|result| result),
            Err(e) => Err(err(e)),
        };
        recording.2.store(false, Ordering::SeqCst);
        let ready = ready
            .and_then(|_| chrome::show_recording(&app))
            .and_then(|_| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !recording.2.load(Ordering::SeqCst) {
                    if Instant::now() >= deadline {
                        return Err("录屏按钮页未成功加载，请重启应用后重试。".into());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(())
            });
        if let Err(error) = ready {
            let _ = go_tx.send(false);
            chrome::hide_recording(&app);
            *recording.0.lock().map_err(err)? = None;
            let _ = window.show();
            return Err(error);
        }
        // 将选区会话所有权交给录屏；active 保持 true，阻止截图/长截图抢占屏幕。
        *state.session.lock().map_err(err)? = None;
        // 工作线程从这里起拥有全局占用与统一收尾责任。
        go_tx.send(true).map_err(err)?;
        Ok(())
    })
    .await
    .map_err(err)?
}

/// 采集线程绝不等待编码队列；内存最多为帧池、一个裁剪缓冲、两帧队列及编码当前帧。
fn collect(
    app: AppHandle,
    id: u64,
    config: RecorderConfig,
    screen: super::scroll_capture::Screen,
    root: PathBuf,
    mut backend: Box<dyn capture::CaptureBackend>,
) {
    let state = app.state::<RecordingState>();
    let (sender, receiver) = mpsc::sync_channel::<QueuedFrame>(2);
    let abort = std::sync::Arc::new(AtomicBool::new(false));
    let writer_abort = abort.clone();
    let writer_app = app.clone();
    let writer = std::thread::Builder::new()
        .name("recording-encoder".into())
        .spawn(move || write_video(&writer_app, id, config, root, receiver, &writer_abort));
    let result = match writer {
        Err(e) => Err(err(e)),
        Ok(writer) => {
            let mut last_sample = None;
            let mut check_screen = Instant::now();
            let mut waiting_since = None;
            let collected = (|| -> Result<(), String> {
                loop {
                    let tick = Instant::now();
                    let running = {
                        let mut guard = state.0.lock().map_err(err)?;
                        let s = guard
                            .as_mut()
                            .filter(|s| s.view.id == id)
                            .ok_or("录屏会话已过期。")?;
                        if s.cancel || s.stop {
                            break;
                        }
                        if s.elapsed().as_secs_f64() >= MAX_SECONDS {
                            s.freeze();
                            s.stop = true;
                            s.view.message = "达到 3 分钟验证期上限，正在保存。".into();
                            break;
                        }
                        s.view.phase == Phase::Recording
                    };
                    if writer.is_finished() {
                        return Err("录屏编码线程已提前结束。".into());
                    }
                    if check_screen.elapsed() >= Duration::from_millis(500) {
                        if super::scroll_capture::screen(&app)? != screen {
                            return Err("显示器位置、尺寸或 DPI 改变，请重新录屏。".into());
                        }
                        check_screen = Instant::now();
                    }
                    if running {
                        waiting_since.get_or_insert_with(Instant::now);
                        if let Some(frame) = backend.next_frame()? {
                            let mut guard = state.0.lock().map_err(err)?;
                            let s = guard
                                .as_mut()
                                .filter(|s| s.view.id == id)
                                .ok_or("录屏会话已过期。")?;
                            if !s.stop && !s.cancel {
                                if let Some((generation, pts)) =
                                    s.timeline.project(frame.source_time)
                                {
                                    // 帧池可能残留暂停前的帧；project 会拒绝恢复起点以前的数据。
                                    if timeline::accept_sample(
                                        &mut last_sample,
                                        generation,
                                        pts,
                                        config.fps,
                                    ) {
                                        waiting_since = None;
                                        match sender.try_send(QueuedFrame {
                                            generation,
                                            pts,
                                            image: frame.image,
                                        }) {
                                            Ok(()) => (),
                                            Err(TrySendError::Full(_)) => {
                                                s.view.dropped_frames += 1;
                                                s.view.revision += 1;
                                            }
                                            Err(TrySendError::Disconnected(_)) => {
                                                return Err("录屏编码线程已停止。".into())
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // 从未收到有效帧时不能无限等待；已有帧的静态桌面允许无新帧。
                        if last_sample.is_none()
                            && waiting_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5))
                        {
                            return Err("屏幕采集未返回画面，请检查权限或图形驱动。".into());
                        }
                    } else {
                        waiting_since = None;
                    }
                    // 用较短轮询排空 WGC 两帧缓冲，按源时间戳限制 30/60 FPS。
                    std::thread::sleep(Duration::from_millis(4).saturating_sub(tick.elapsed()));
                }
                Ok(())
            })();
            if collected.is_err() {
                abort.store(true, Ordering::Release);
            }
            // 先停止捕获，再关闭发送端，编码器按有效代次排空队列后 Finalize。
            drop(backend);
            if let Ok(mut guard) = state.0.lock() {
                if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
                    s.freeze();
                    s.stop = true;
                    s.view.phase = Phase::Encoding;
                    s.view.message = "录屏已停止，正在完成 MP4 封装…".into();
                    s.view.revision += 1;
                }
            }
            drop(sender);
            let encoded = writer
                .join()
                .map_err(|_| "录屏编码线程异常退出。".to_string())
                .and_then(|r| r);
            // 编码故障优先展示具体原生错误；采集故障不被内部中止提示覆盖。
            match (collected, encoded) {
                (Err(_), Err(writer_error)) if writer_error != "录屏采集已中止。" => {
                    Err(writer_error)
                }
                (Err(e), _) => Err(e),
                (Ok(()), result) => result,
            }
        }
    };
    let mut message = String::new();
    if let Ok(mut guard) = state.0.lock() {
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.freeze();
            let (phase, text, path) = match result {
                Ok(path) => {
                    let limit = if s.view.seconds >= MAX_SECONDS {
                        "（达到 3 分钟上限，已自动停止）"
                    } else {
                        ""
                    };
                    (
                        Phase::Completed,
                        format!("录屏已保存：{path}{limit}"),
                        Some(path),
                    )
                }
                Err(_) if s.cancel => (
                    Phase::Cancelled,
                    "录屏已取消，临时视频已清理。".into(),
                    None,
                ),
                Err(error) => (
                    Phase::Failed,
                    format!("录屏失败：{error} 临时视频已清理。"),
                    None,
                ),
            };
            message = text.clone();
            s.view.phase = phase;
            s.view.message = text;
            s.view.path = path;
            s.view.revision += 1;
        }
    }
    chrome::hide_recording(&app);
    app.state::<CaptureState>()
        .active
        .store(false, Ordering::SeqCst);
    let _ = app.emit_to("main", "capture-result", &message);
    show_main(&app);
}

/// 临时目录在 writer 释放后删除；只有 Finalize、sync_all 成功才原子发布最终文件。
fn write_video(
    app: &AppHandle,
    id: u64,
    config: RecorderConfig,
    root: PathBuf,
    receiver: mpsc::Receiver<QueuedFrame>,
    abort: &AtomicBool,
) -> Result<String, String> {
    let temp = tempfile::Builder::new()
        .prefix("recording-pending-")
        .tempdir_in(&root)
        .map_err(err)?;
    let pending = temp.path().join("video.mp4");
    let mut encoder = encoder::create(
        &pending,
        config.region.width,
        config.region.height,
        config.fps,
    )?;
    let state = app.state::<RecordingState>();
    for frame in receiver {
        if abort.load(Ordering::Acquire) {
            return Err("录屏采集已中止。".into());
        }
        {
            let mut guard = state.0.lock().map_err(err)?;
            let s = guard
                .as_mut()
                .filter(|s| s.view.id == id)
                .ok_or("录屏会话已过期。")?;
            if s.cancel {
                return Err("已取消录屏。".into());
            }
            if frame.generation != s.timeline.generation {
                s.view.dropped_frames += 1;
                s.view.revision += 1;
                continue;
            }
            // 在锁内确认归属即视为提交；原生 WriteSample 不持有状态锁，按钮可及时响应。
        }
        encoder.push(&frame.image, frame.pts)?;
        let mut guard = state.0.lock().map_err(err)?;
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.view.frames += 1;
            s.view.revision += 1;
        }
    }
    if abort.load(Ordering::Acquire) {
        return Err("录屏采集已中止。".into());
    }
    let end = {
        let guard = state.0.lock().map_err(err)?;
        let s = guard
            .as_ref()
            .filter(|s| s.view.id == id && !s.cancel)
            .ok_or("已取消录屏。")?;
        s.timeline.elapsed(capture::now())
    };
    encoder.finish(end)?;
    // 释放 MF 文件句柄后才能同步/重命名；取消期间仍由临时目录负责清理。
    drop(encoder);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&pending)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    if file.metadata().map_err(err)?.len() == 0 {
        return Err("编码器未生成视频。".into());
    }
    drop(file);
    let mut guard = state.0.lock().map_err(err)?;
    let s = guard
        .as_mut()
        .filter(|s| s.view.id == id && !s.cancel)
        .ok_or("已取消录屏。")?;
    // 临时目录随机名使输出唯一；发布与取消共享锁，不会已保留文件却报告取消。
    let name = temp
        .path()
        .file_name()
        .ok_or("临时录屏路径无效。")?
        .to_string_lossy();
    let output = root.join(format!("{name}.mp4"));
    fs::rename(&pending, &output).map_err(err)?;
    s.view.phase = Phase::Completed;
    Ok(output.to_string_lossy().into_owned())
}
