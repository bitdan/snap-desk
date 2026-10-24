//! 录屏会话管理：命令、UI、WGC 采集和有界编码线程的生命周期。
use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, TrySendError},
};
mod audio;
mod capture;
mod compositor;
mod config;
#[cfg(test)]
#[path = "countdown_tests.rs"]
mod countdown_tests;
mod encoder;
mod frame;
mod gif;
mod input;
pub mod output;
pub mod recovery;
pub mod result;
pub mod editing;
mod stats;
mod timeline;
use super::scroll::chrome;
use config::{CaptureSource, OutputFormat, RecorderCapabilities, RecorderConfig};
use frame::QueuedFrame;
use timeline::Timeline;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Ready,
    Countdown,
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
    system_audio: bool,
    microphone: bool,
    /// push 的累计墙钟耗时，包含转换和同步提交，不代表 GPU 内部延迟。
    encoding_millis: f64,
    stats: stats::RecordingStats,
    target_bitrate: u32,
    encoder_kind: encoder::EncoderKind,
    encoder_fallback: Option<String>,
    countdown_remaining: u8,
    message: String,
    path: Option<String>,
}

/// 状态锁仅保护时间轴和帧提交；系统取样/编码在锁外进行，暂停通过代次淘汰旧帧。
struct Session {
    view: RecordingView,
    timeline: Timeline,
    fps_window: stats::FpsWindow,
    countdown_seconds: u8,
    countdown_until: Option<i64>,
    stop: bool,
    cancel: bool,
}
impl Session {
    fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.timeline.elapsed(capture::now()).max(0) as u64 * 100)
    }
    fn freeze(&mut self) {
        self.countdown_until = None;
        self.view.countdown_remaining = 0;
        self.timeline.stop(capture::now());
        self.view.seconds = self.elapsed().as_secs_f64();
    }

    /// 仅首次开始使用倒计时；恢复不延时。源时间轴在真正开始采集前保持停止。
    fn begin(&mut self, now: i64, delay: u8) {
        if delay == 0 {
            self.countdown_until = None;
            self.view.countdown_remaining = 0;
            self.timeline.resume(now);
            self.fps_window.reset(self.timeline.elapsed(now));
            self.view.phase = Phase::Recording;
            log::info!(
                "event=recording.started module=recording op=rec-{} fps={}",
                self.view.id,
                self.view.target_fps
            );
            self.view.message = "录制中".into();
        } else {
            self.countdown_until = Some(now + i64::from(delay) * 10_000_000);
            self.view.countdown_remaining = delay;
            self.view.phase = Phase::Countdown;
            self.view.message = format!("将在 {delay} 秒后开始录屏，可取消。");
        }
        self.view.revision += 1;
    }

    /// 工作线程短周期推进，不阻塞命令；停止/取消与到期转换在同一状态锁内裁决。
    fn advance_countdown(&mut self, now: i64) {
        if self.stop || self.cancel || self.view.phase != Phase::Countdown {
            return;
        }
        let Some(deadline) = self.countdown_until else {
            return;
        };
        if now >= deadline {
            // 使用实际推进时刻而非计划到期时刻，线程调度延迟不计入视频。
            self.begin(now, 0);
        } else {
            let remaining = ((deadline - now + 9_999_999) / 10_000_000) as u8;
            if remaining != self.view.countdown_remaining {
                self.view.countdown_remaining = remaining;
                self.view.message = format!("将在 {remaining} 秒后开始录屏，可取消。");
                self.view.revision += 1;
            }
        }
    }

    fn stop_requested(&mut self, cancel: bool) {
        self.cancel |= cancel || matches!(self.view.phase, Phase::Ready | Phase::Countdown);
        self.freeze();
        self.stop = true;
        self.view.phase = Phase::Encoding;
        self.view.message = if self.cancel {
            "正在取消录屏并释放设备…"
        } else {
            "已请求停止，正在排空编码队列并保存…"
        }
        .into();
        self.view.revision += 1;
    }
}

#[derive(Default)]
pub struct RecordingState(Mutex<Option<Session>>, Mutex<()>, AtomicBool);

/// 退出意图独立于录屏阶段：Completed 可能早于窗口和资源清理。
#[derive(Default)]
pub struct ExitState {
    requested: AtomicBool,
    workers: std::sync::atomic::AtomicUsize,
    progress: Mutex<Progress>,
}
impl ExitState {
    /// 最后一个任务释放时才允许正常退出，不能依赖可能提前更新的视频状态。
    fn release_worker(&self) -> bool {
        self.workers.fetch_sub(1, Ordering::SeqCst) == 1 && self.requested.load(Ordering::SeqCst)
    }
}

/// 准备任务与采集任务均持有守卫；异常返回或 panic 也会释放计数。
struct WorkerGuard(AppHandle);
impl WorkerGuard {
    fn acquire(app: &AppHandle) -> Result<Self, String> {
        let state = app.state::<ExitState>();
        state.workers.fetch_add(1, Ordering::SeqCst);
        let guard = Self(app.clone());
        if state.requested.load(Ordering::SeqCst) {
            return Err("程序正在退出，不能启动录屏。".into());
        }
        Ok(guard)
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let state = self.0.state::<ExitState>();
        if state.release_worker() {
            self.0.exit(0);
        }
    }
}

/// 记录真实处理进展，不能用定时心跳伪装编码器仍有进展。
struct Progress {
    stage: &'static str,
    updated: Instant,
    entered: Instant,
    path: Option<PathBuf>,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            stage: "idle",
            updated: Instant::now(),
            entered: Instant::now(),
            path: None,
        }
    }
}
fn progress(app: &AppHandle, stage: &'static str, path: Option<&std::path::Path>) {
    if let Ok(mut p) = app.state::<ExitState>().progress.lock() {
        if p.stage != stage {
            log::info!(
                "event=recording.stage_changed module=recording previous={} stage={stage} duration_ms={:.0}",
                p.stage,
                p.entered.elapsed().as_secs_f64() * 1000.0
            );
            p.entered = Instant::now();
        }
        p.stage = stage;
        p.updated = Instant::now();
        p.path = path.map(std::path::Path::to_path_buf);
    }
}
/// Finalize 属于不可细分的原生调用，给予更长无进展窗口；总等待仍有硬上限。
fn exit_timed_out(stage: &str, idle: Duration, total: Duration) -> bool {
    let limit = if stage == "finalizing" { 180 } else { 90 };
    idle >= Duration::from_secs(limit) || total >= Duration::from_secs(600)
}

/// 退出按真实帧提交、阶段切换及文件增长延长等待，保留超时源文件与恢复记录。
pub fn request_exit(app: &AppHandle) {
    let state = app.state::<ExitState>();
    if state.requested.swap(true, Ordering::SeqCst) {
        return;
    }
    if state.workers.load(Ordering::SeqCst) == 0 {
        app.exit(0);
        return;
    }
    let watchdog = app.clone();
    // 文件系统调用可能被掉线磁盘卡住，增长观察独立于超时裁决线程。
    let observer = app.clone();
    std::thread::spawn(move || {
        let mut observed: Option<(PathBuf, u64)> = None;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = observer.state::<ExitState>();
            if state.workers.load(Ordering::SeqCst) == 0 {
                return;
            }
            let path = state.progress.try_lock().ok().and_then(|p| p.path.clone());
            let Some(path) = path else {
                continue;
            };
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let size = metadata.len();
            if observed
                .as_ref()
                .is_some_and(|(old_path, old_size)| old_path == &path && size > *old_size)
            {
                if let Ok(mut p) = state.progress.try_lock() {
                    if p.path.as_ref() == Some(&path) {
                        p.updated = Instant::now();
                    }
                }
            }
            observed = Some((path, size));
        }
    });
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut last_progress = started;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = watchdog.state::<ExitState>();
            if state.workers.load(Ordering::SeqCst) == 0 {
                return;
            }
            let (stage, updated) = match state.progress.try_lock() {
                Ok(p) => (p.stage, p.updated),
                Err(_) => ("state-unavailable", started),
            };
            last_progress = last_progress.max(updated);
            if exit_timed_out(stage, last_progress.elapsed(), started.elapsed()) {
                log::error!("退出超时：阶段={stage}，无进展={:.1}s，总等待={:.1}s；源视频与恢复记录保留，下次启动可检查。", last_progress.elapsed().as_secs_f64(), started.elapsed().as_secs_f64());
                watchdog.exit(1);
                return;
            }
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        request_stop(&app, false);
    });
}

fn active(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Ready | Phase::Countdown | Phase::Recording | Phase::Paused | Phase::Encoding
    )
}

/// 托盘、录屏快捷键和退出入口只发停止信号，清理由录屏工作线程完成。
pub fn request_stop(app: &AppHandle, cancel: bool) -> bool {
    let state = app.state::<RecordingState>();
    let Ok(mut guard) = state.0.lock() else {
        return false;
    };
    let Some(s) = guard.as_mut().filter(|s| active(s.view.phase)) else {
        return false;
    };
    s.stop_requested(cancel);
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
        if matches!(action, RecordingAction::Cancel) {
            // 完成后的关闭只收起回放，不删除已保存的录像；旧会话 ID 仍在上方校验。
            drop(guard);
            chrome::hide_recording(app);
            return Ok(());
        }
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
            // 所有入口暂停都不展开主窗口，继续通过悬浮控制条恢复或停止。
            if id.is_none() {
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
            let delay = if expected_phase == Phase::Ready {
                s.countdown_seconds
            } else {
                0
            };
            s.begin(capture::now(), delay);
            return Ok(());
        }
        RecordingAction::Stop | RecordingAction::Cancel => {
            s.stop_requested(matches!(action, RecordingAction::Cancel));
            return Ok(());
        }
        _ => return Err("当前录屏状态不支持此操作。".into()),
    }
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
        view.stats.recent_fps =
            if matches!(view.phase, Phase::Ready | Phase::Countdown | Phase::Paused) {
                0.0
            } else {
                s.fps_window.fps(s.timeline.elapsed(capture::now()))
            };
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

/// 分别试写 MP4/GIF；H.264 不可用时仍可录 GIF，具体选区准备时再次探测。
#[tauri::command]
pub async fn recording_capabilities(window: WebviewWindow) -> Result<RecorderCapabilities, String> {
    if !matches!(window.label(), "main" | "capture") {
        return Err("当前窗口无权查询录屏能力。".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let result = capture::supported();
        let mut formats = Vec::new();
        if result.is_ok() {
            let root = tempfile::tempdir().map_err(err)?;
            for format in [OutputFormat::Mp4, OutputFormat::Gif] {
                if encoder::probe(
                    root.path(),
                    64,
                    64,
                    30,
                    1_000_000,
                    config::EncodingPreference::Auto,
                    format,
                    false,
                )
                .is_ok()
                {
                    formats.push(format);
                }
            }
        }
        let available = result.is_ok() && !formats.is_empty();
        // 能力查询只以合成静音试写 AAC；不开始麦克风或系统声音采集。
        let audio_available = cfg!(target_os = "windows")
            && formats.iter().any(|f| matches!(f, OutputFormat::Mp4))
            && (|| {
                let root = tempfile::tempdir().map_err(err)?;
                encoder::probe(
                    root.path(),
                    64,
                    64,
                    30,
                    1_000_000,
                    config::EncodingPreference::Auto,
                    OutputFormat::Mp4,
                    true,
                )
            })()
            .is_ok();
        Ok(RecorderCapabilities {
            available,
            reason: result
                .err()
                .or_else(|| (!available).then(|| "没有可用的录制编码器。".into())),
            sources: if available {
                vec![
                    CaptureSource::PrimaryMonitor,
                    CaptureSource::SelectedMonitor,
                    CaptureSource::SelectedWindow,
                ]
            } else {
                vec![]
            },
            frame_rates: if available {
                vec![10, 20, 30, 60]
            } else {
                vec![]
            },
            capture_cursor: available,
            system_audio: audio_available,
            microphone: audio_available,
            formats,
        })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    config: Option<RecorderConfig>,
    target: Option<usize>,
    annotations: Option<Vec<crate::annotations::Annotation>>,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _preparing = WorkerGuard::acquire(&app)?;
        #[cfg(windows)]
        let _dpi = crate::desktop_capture::PhysicalDpi::enter()?;
        progress(&app, "preparing", None);
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("请先结束滚动截图。".into());
        }
        let size = state
            .session
            .lock()
            .map_err(err)?
            .as_ref()
            .filter(|s| s.frame.id == id)
            .map(|s| s.image.dimensions())
            .ok_or("截图已过期。")?;
        let mut config = config.unwrap_or_else(|| RecorderConfig::for_region(rect));
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
        let annotations = annotations.unwrap_or_default();
        crate::annotations::validate(&annotations, size.0, size.1)?;
        // 标注仍使用虚拟桌面截图坐标；视频输出原点单独保存，不能使用显示器局部坐标。
        let mut annotation_origin = crate::annotations::Point {
            x: rect.x as f64,
            y: rect.y as f64,
        };
        // 跨屏窗口取最大可见交集用于控制条布局；不能把此交集作为视频裁剪范围。
        let placement_rect = if matches!(config.source, CaptureSource::SelectedWindow) {
            let guard = state.session.lock().map_err(err)?;
            let session = guard
                .as_ref()
                .filter(|s| s.frame.id == id)
                .ok_or("截图已过期。")?;
            crate::desktop_capture::display_intersection(
                rect,
                (session.frame.origin_x, session.frame.origin_y),
                &session.frame.displays,
            )
            .ok_or("窗口没有可见的显示器区域，请重新选择。")?
        } else {
            rect
        };
        let (mut rect, screen) = super::selected_region(&app, id, placement_rect)?;
        let placement = rect;
        let mut source_size = (screen.width, screen.height);
        let source = if matches!(config.source, CaptureSource::SelectedWindow) {
            let guard = state.session.lock().map_err(err)?;
            let session = guard
                .as_ref()
                .filter(|s| s.frame.id == id)
                .ok_or("截图已过期。")?;
            let candidate = target
                .and_then(|i| session.frame.targets.get(i))
                .filter(|t| t.kind == "window")
                .ok_or("请先点击吸附一个窗口，再选择窗口录制。")?;
            let native = session
                .desktop
                .windows
                .get(candidate.window_index)
                .ok_or("窗口已失效。")?;
            if native.is_minimized().map_err(err)? {
                return Err("窗口已最小化，请还原后重新选择。".into());
            }
            annotation_origin = crate::annotations::Point {
                x: candidate.rect.x as f64,
                y: candidate.rect.y as f64,
            };
            // 窗口录制使用整个原生来源，不把桌面裁剪坐标误用于窗口表面。
            source_size = (native.width().map_err(err)?, native.height().map_err(err)?);
            rect = Rect {
                x: 0,
                y: 0,
                width: source_size.0,
                height: source_size.1,
            };
            capture::NativeSource::Window {
                id: native.id().map_err(err)?,
                pid: native.pid().map_err(err)?,
            }
        } else {
            if matches!(config.source, CaptureSource::PrimaryMonitor) {
                let primary = app.primary_monitor().map_err(err)?.ok_or("主屏不可用。")?;
                if (primary.position().x, primary.position().y) != (screen.x, screen.y) {
                    return Err("旧版主屏录制配置不能用于副屏，请重新选择录制来源。".into());
                }
            }
            capture::NativeSource::Monitor {
                x: screen.x,
                y: screen.y,
            }
        };
        config.region = rect;
        config.validate(source_size)?;
        let annotation_layer =
            frame::AnnotationLayer::new(&annotations, rect.width, rect.height, annotation_origin)?;
        capture::supported()?;
        // 准备时固定本次目录；临时录制文件和最终文件同盘，停止时无需跨盘移动。
        let root = output::prepare(&app)?;
        let (encoder_kind, selected_encoding, encoder_fallback) = encoder::probe(
            &root,
            rect.width,
            rect.height,
            config.fps,
            config.target_bitrate(),
            config.encoding,
            config.format,
            config.system_audio || config.microphone,
        )?;
        config.encoding = selected_encoding;
        // 窗口本地原点与控制条位置无关，控制条仍以用户确认的屏幕选区放置。
        chrome::place_recording(&app, &screen, placement)?;
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
                system_audio: config.system_audio,
                microphone: config.microphone,
                encoding_millis: 0.0,
                stats: stats::RecordingStats::default(),
                target_bitrate: config.target_bitrate(),
                encoder_kind,
                encoder_fallback,
                countdown_remaining: 0,
                message: "录屏选区已准备，请点击下方“开始录制”。".into(),
                path: None,
            },
            timeline: Timeline::default(),
            fps_window: stats::FpsWindow::default(),
            countdown_seconds: config.countdown_seconds,
            countdown_until: None,
            stop: false,
            cancel: false,
        });
        // WGC 必须在所属线程初始化/释放。先握手确认，再显示可操作的控制条。
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (go_tx, go_rx) = mpsc::sync_channel(1);
        let worker_guard = WorkerGuard::acquire(&app)?;
        let worker_app = app.clone();
        let worker = std::thread::Builder::new()
            .name("screen-recording".into())
            .spawn(move || {
                let _worker = worker_guard;
                let prepared = capture::prepare(&config, source_size, source).and_then(|backend| {
                    let audio = if config.system_audio || config.microphone {
                        let audio_app = worker_app.clone();
                        // 初始化在确认选区阶段完成；只有 Recording 阶段才 Start 设备。
                        Some(audio::Worker::start(
                            config.system_audio,
                            config.microphone,
                            move || {
                                audio_app
                                    .state::<RecordingState>()
                                    .0
                                    .lock()
                                    .ok()
                                    .and_then(|guard| {
                                        guard.as_ref().map(|s| {
                                            s.view.id == id
                                                && s.view.phase == Phase::Recording
                                                && !s.stop
                                                && !s.cancel
                                        })
                                    })
                                    .unwrap_or(false)
                            },
                        )?)
                    } else {
                        None
                    };
                    Ok((backend, audio))
                });
                match prepared {
                    Ok((backend, audio)) => {
                        if ready_tx.send(Ok(())).is_ok() && go_rx.recv().unwrap_or(false) {
                            collect(
                                worker_app,
                                id,
                                config,
                                screen,
                                root,
                                backend,
                                audio,
                                annotation_layer,
                            );
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
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
        // 将选区会话所有权交给录屏；active 保持 true，阻止截图/滚动截图抢占屏幕。
        super::release_selection(&app, id)?;
        // 工作线程从这里起拥有全局占用与统一收尾责任。
        if app.state::<ExitState>().requested.load(Ordering::SeqCst) {
            request_stop(&app, false);
        }
        go_tx.send(true).map_err(err)?;
        Ok(())
    })
    .await
    .map_err(err)?
}

/// 单次取样在状态锁外计时；空取样也统计耗时，不包含 wait_for_frame 的空闲等待。
fn measured_capture(
    state: &RecordingState,
    id: u64,
    backend: &mut dyn capture::CaptureBackend,
) -> Result<Option<frame::VideoFrame>, String> {
    let started = Instant::now();
    let result = backend.next_frame();
    let millis = started.elapsed().as_secs_f64() * 1000.0;
    let mut guard = state.0.lock().map_err(err)?;
    let s = guard
        .as_mut()
        .filter(|s| s.view.id == id)
        .ok_or("录屏会话已过期。")?;
    s.view.stats.capture_polls += 1;
    s.view.stats.capture_millis += millis;
    if let Ok(Some(frame)) = &result {
        s.view.stats.captured_frames += 1;
        s.view.stats.capture_copy_bytes += frame.image.data.len() as u64;
    }
    s.view.revision += 1;
    result
}

/// 采集线程绝不等待编码队列；开启特效时额外保留一帧干净底图，动画与事件数量有界。
fn collect(
    app: AppHandle,
    id: u64,
    config: RecorderConfig,
    screen: super::scroll::Screen,
    root: PathBuf,
    mut backend: Box<dyn capture::CaptureBackend>,
    audio: Option<audio::Worker>,
    annotation_layer: Option<frame::AnnotationLayer>,
) {
    let state = app.state::<RecordingState>();
    let (sender, receiver) = mpsc::sync_channel::<QueuedFrame>(2);
    let abort = std::sync::Arc::new(AtomicBool::new(false));
    let writer_abort = abort.clone();
    let writer_app = app.clone();
    let writer = std::thread::Builder::new()
        .name("recording-encoder".into())
        .spawn(move || {
            write_video(
                &writer_app,
                id,
                config,
                root,
                receiver,
                &writer_abort,
                audio,
                annotation_layer,
            )
        });
    let result = match writer {
        Err(e) => Err(err(e)),
        Ok(writer) => {
            let mut last_sample = None;
            let mut check_screen = Instant::now();
            let mut waiting_since = None;
            let mut listener: Option<input::Listener> = None;
            let mut effects_generation = None;
            let mut compositor = compositor::Compositor::new(config.input_effects, config.region);
            compositor.set_origin((screen.x, screen.y));
            let mut base: Option<frame::BgraImage> = None;
            let mut base_dirty = false;
            let collected = (|| -> Result<(), String> {
                loop {
                    let (running, generation) = {
                        let mut guard = state.0.lock().map_err(err)?;
                        let s = guard
                            .as_mut()
                            .filter(|s| s.view.id == id)
                            .ok_or("录屏会话已过期。")?;
                        if s.cancel || s.stop {
                            break;
                        }
                        s.advance_countdown(capture::now());
                        // 不按录制时长自动停止；停止、取消和采集/编码异常仍走统一收尾。
                        (s.view.phase == Phase::Recording, s.timeline.generation)
                    };
                    if effects_generation != Some(generation) || !running {
                        // 先停止并 join 消息线程，再清缓存；恢复使用新监听器，不继承按住状态。
                        listener = None;
                        compositor.clear();
                        base = None;
                        base_dirty = false;
                        if running && config.input_effects.enabled() {
                            backend.refresh()?;
                        }
                        effects_generation = Some(generation);
                    }
                    if writer.is_finished() {
                        return Err("录屏编码线程已提前结束。".into());
                    }
                    if check_screen.elapsed() >= Duration::from_millis(500) {
                        backend.check_source()?;
                        if super::scroll::session::screen_at(&app, screen.x, screen.y)? != screen {
                            return Err("显示器位置、尺寸或 DPI 改变，请重新录屏。".into());
                        }
                        check_screen = Instant::now();
                    }
                    if running {
                        waiting_since.get_or_insert_with(Instant::now);
                        if config.input_effects.enabled() {
                            if listener.is_none() {
                                listener = Some(input::Listener::start(config.input_effects)?);
                            }
                            let frame = measured_capture(&state, id, backend.as_mut())?;
                            compositor.set_origin(backend.desktop_origin()?);
                            let events = listener.as_ref().unwrap().drain()?;
                            // 先取完源事件，再取 QPC，避免给未来事件分配更早的视频时间戳。
                            let now = capture::now();
                            let projected = {
                                let mut guard = state.0.lock().map_err(err)?;
                                let s = guard
                                    .as_mut()
                                    .filter(|s| s.view.id == id)
                                    .ok_or("录屏会话已过期。")?;
                                if s.stop
                                    || s.cancel
                                    || s.view.phase != Phase::Recording
                                    || s.timeline.generation != generation
                                {
                                    s.view.stats.rejected_capture_frames +=
                                        usize::from(frame.is_some());
                                    s.view.revision += 1;
                                    None
                                } else {
                                    let had_frame = frame.is_some();
                                    let frame = frame.filter(|f| {
                                        s.timeline.project(f.source_time).is_some()
                                            && f.source_time <= now
                                    });
                                    if had_frame && frame.is_none() {
                                        s.view.stats.rejected_capture_frames += 1;
                                        s.view.revision += 1;
                                    }
                                    let events = events
                                        .into_iter()
                                        .filter_map(|event| {
                                            s.timeline
                                                .project(event.source_time)
                                                .filter(|(_, pts)| *pts <= s.timeline.elapsed(now))
                                                .map(|(_, pts)| (pts, event.kind))
                                        })
                                        .collect::<Vec<_>>();
                                    s.timeline.project(now).map(|(_, pts)| (pts, frame, events))
                                }
                            };
                            if let Some((pts, frame, events)) = projected {
                                if let Some(frame) = frame {
                                    base = Some(frame.image);
                                    base_dirty = true;
                                    waiting_since = None;
                                }
                                for (time, event) in events {
                                    compositor.ingest(time, event);
                                }
                                if let Some(clean) = base.as_ref() {
                                    let needed = base_dirty || compositor.needs_frame();
                                    let accepted = needed
                                        && timeline::accept_sample(
                                            &mut last_sample,
                                            generation,
                                            pts,
                                            config.fps,
                                        );
                                    if needed && !accepted {
                                        let mut guard = state.0.lock().map_err(err)?;
                                        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id)
                                        {
                                            s.view.stats.throttled_samples += 1;
                                            s.view.revision += 1;
                                        }
                                    }
                                    if accepted {
                                        // 合成在锁外；重绘时只读未叠加的底图，帧时间用同源 QPC 的当前媒体时间。
                                        let composition_started = Instant::now();
                                        let image = compositor.render(clean, pts);
                                        let composition_millis =
                                            composition_started.elapsed().as_secs_f64() * 1000.0;
                                        let mut guard = state.0.lock().map_err(err)?;
                                        let s = guard
                                            .as_mut()
                                            .filter(|s| s.view.id == id)
                                            .ok_or("录屏会话已过期。")?;
                                        s.view.stats.composed_frames += 1;
                                        s.view.stats.composition_copy_bytes +=
                                            image.data.len() as u64;
                                        s.view.stats.composition_millis += composition_millis;
                                        s.view.revision += 1;
                                        if !s.stop
                                            && !s.cancel
                                            && s.view.phase == Phase::Recording
                                            && s.timeline.generation == generation
                                        {
                                            match sender.try_send(QueuedFrame {
                                                generation,
                                                pts,
                                                image,
                                            }) {
                                                Ok(()) => {
                                                    base_dirty = false;
                                                    s.view.stats.queued_frames += 1;
                                                }
                                                Err(TrySendError::Full(_)) => {
                                                    // 包括效果结束后的干净帧，拥堵后也必须重试，防止光圈冻结。
                                                    base_dirty = true;
                                                    s.view.stats.queue_dropped_frames += 1;
                                                    s.view.dropped_frames += 1;
                                                    s.view.revision += 1;
                                                }
                                                Err(TrySendError::Disconnected(_)) => {
                                                    return Err("录屏编码线程已停止。".into())
                                                }
                                            }
                                        } else {
                                            s.view.stats.rejected_composed_frames += 1;
                                        }
                                    }
                                }
                            }
                        } else if let Some(frame) = measured_capture(&state, id, backend.as_mut())?
                        {
                            let mut guard = state.0.lock().map_err(err)?;
                            let s = guard
                                .as_mut()
                                .filter(|s| s.view.id == id)
                                .ok_or("录屏会话已过期。")?;
                            s.view.revision += 1;
                            let projected = s
                                .timeline
                                .project(frame.source_time)
                                .filter(|(g, _)| !s.stop && !s.cancel && *g == generation);
                            if let Some((generation, pts)) = projected {
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
                                        Ok(()) => s.view.stats.queued_frames += 1,
                                        Err(TrySendError::Full(_)) => {
                                            s.view.stats.queue_dropped_frames += 1;
                                            s.view.dropped_frames += 1;
                                            s.view.revision += 1;
                                        }
                                        Err(TrySendError::Disconnected(_)) => {
                                            return Err("录屏编码线程已停止。".into())
                                        }
                                    }
                                } else {
                                    s.view.stats.throttled_samples += 1;
                                }
                            } else {
                                s.view.stats.rejected_capture_frames += 1;
                            }
                        }
                        // 从未收到有效帧时不能无限等待；已有帧的静态桌面允许无新帧。
                        if (if config.input_effects.enabled() {
                            base.is_none()
                        } else {
                            last_sample.is_none()
                        }) && waiting_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5))
                        {
                            return Err("屏幕采集未返回画面，请检查权限或图形驱动。".into());
                        }
                    } else {
                        waiting_since = None;
                    }
                    // 帧事件唤醒采集；超时使静态桌面也能检查停止和设备状态。
                    if running {
                        // Raw Input 独立消息线程不会触发 WGC；特效模式按帧率限制最长等待，
                        // 静态画面也能及时消费输入，并在动画结束后提交一帧干净底图。
                        let timeout = if config.input_effects.enabled() {
                            Duration::from_nanos(1_000_000_000 / u64::from(config.fps))
                        } else {
                            Duration::from_millis(50)
                        };
                        backend.wait_for_frame(timeout);
                    } else {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                Ok(())
            })();
            // 无论停止、取消、采集失败或编码线程提前退出，都在封装 MP4 前停止输入监听。
            drop(listener);
            compositor.clear();
            drop(base);
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
                    s.view.message = if s.cancel {
                        "正在取消录屏并清理临时文件…"
                    } else {
                        "录屏已停止，正在完成文件封装…"
                    }
                    .into();
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
    let mut completed = false;
    if let Ok(mut guard) = state.0.lock() {
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.freeze();
            let (phase, text, path) = match result {
                Ok(path) => (Phase::Completed, format!("录屏已保存：{path}"), Some(path)),
                Err(_) if s.cancel => (
                    Phase::Cancelled,
                    "录屏已取消；未能清理的残留可在录屏页查看并删除。".into(),
                    None,
                ),
                Err(error) => (
                    Phase::Failed,
                    format!(
                        "录屏失败：{error} 请在录屏页查看已保留的数据；未封装数据可能无法播放。"
                    ),
                    None,
                ),
            };
            // 只在统一收尾处记录汇总，不把含完整输出路径的 text/path 写盘。
            if phase == Phase::Failed {
                log::error!("event=recording.failed module=recording op=rec-{id} code=RECORDING_PIPELINE_FAILED");
            }
            let level = if phase == Phase::Failed {
                log::Level::Error
            } else {
                log::Level::Info
            };
            let outcome = match phase {
                Phase::Completed => "success",
                Phase::Cancelled => "cancelled",
                _ => "failed",
            };
            log::log!(level, "event=recording.finished module=recording op=rec-{id} result={outcome} duration_ms={} captured={} encoded={} dropped={} encoding_ms={}",
                (s.view.seconds * 1000.0) as u64, s.view.stats.captured_frames, s.view.frames, s.view.dropped_frames, s.view.encoding_millis);
            completed = phase == Phase::Completed;
            message = text.clone();
            s.view.phase = phase;
            s.view.message = text;
            s.view.path = path;
            s.view.revision += 1;
        }
    }
    chrome::hide_recording(&app);
    // 控制条原地切换为回放/裁剪窗口，成功后无需到主界面寻找录像。
    if completed {
        if let Err(error) = chrome::show_recording_preview(&app) {
            super::report_error(&app, format!("无法显示录屏回放：{error}"));
        }
    }
    app.state::<CaptureState>()
        .active
        .store(false, Ordering::SeqCst);
    let _ = app.emit_to("main", "capture-result", &message);
    // 主界面保持隐藏；成功由录屏窗口接管编辑，取消或失败沿用现有提示。
}

/// Finalize、sync_all 成功才发布；保存失败保留视频与恢复记录，明确取消才清理。
fn write_video(
    app: &AppHandle,
    id: u64,
    config: RecorderConfig,
    root: PathBuf,
    receiver: mpsc::Receiver<QueuedFrame>,
    abort: &AtomicBool,
    mut audio: Option<audio::Worker>,
    annotation_layer: Option<frame::AnnotationLayer>,
) -> Result<String, String> {
    let mut recovery = recovery::Pending::new(app, id, &root, config.format)?;
    let pending = recovery.path();
    progress(app, "recording", Some(&pending));
    let mut encoder = encoder::create(
        &pending,
        config.region.width,
        config.region.height,
        config.fps,
        config.target_bitrate(),
        config.encoding,
        config.format,
        config.system_audio || config.microphone,
    )?;
    let state = app.state::<RecordingState>();
    let encoder_kind = encoder.kind();
    {
        let mut guard = state.0.lock().map_err(err)?;
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.view.encoder_kind = encoder_kind;
            s.view.revision += 1;
        }
    }
    let mut mixer = audio::Mixer::default();
    loop {
        if abort.load(Ordering::Acquire) {
            return Err("录屏采集已中止。".into());
        }
        if let Some(worker) = audio.as_mut() {
            // 音频独立于 WGC 新帧；静态桌面和暂停期间仍排空已采集数据。
            pump_audio(worker, &mut mixer, encoder.as_mut(), &state, id, false)?;
        }
        let frame = match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
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
                s.view.stats.stale_queued_frames += 1;
                s.view.dropped_frames += 1;
                s.view.revision += 1;
                continue;
            }
            // 在锁内确认归属即视为提交；原生 WriteSample 不持有状态锁，按钮可及时响应。
        }
        let encoding_started = Instant::now();
        // 标注层只绘制一次；每帧在编码前混合，确保准备阶段的标注真正进入成品。
        let mut image = frame.image;
        if let Some(layer) = &annotation_layer {
            layer.apply(&mut image)?;
        }
        encoder.push(&image, frame.pts)?;
        progress(app, "encoding", Some(&pending));
        let encoding_millis = encoding_started.elapsed().as_secs_f64() * 1000.0;
        let mut guard = state.0.lock().map_err(err)?;
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.view.frames += 1;
            s.view.encoding_millis += encoding_millis;
            s.view.stats.nv12_output_bytes = encoder.converted_bytes();
            // push 期间仍可暂停；旧代次已提交帧计入累计值，但不污染恢复后的窗口。
            if frame.generation == s.timeline.generation {
                s.fps_window.submitted(frame.pts);
            }
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
    if let Some(worker) = audio.as_mut() {
        // 停止设备并 join 后再取尾包；停止时间之后的样本会被 Timeline 裁掉。
        worker.stop()?;
        pump_audio(worker, &mut mixer, encoder.as_mut(), &state, id, true)?;
    }
    recovery.stage("finalizing", false)?;
    progress(app, "finalizing", Some(&pending));
    encoder.finish(end)?;
    // 释放 MF 文件句柄后才能同步/重命名；取消期间仍由临时目录负责清理。
    drop(encoder);
    recovery.stage("syncing", true)?;
    progress(app, "syncing", Some(&pending));
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&pending)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    if file.metadata().map_err(err)?.len() == 0 {
        return Err("编码器未生成视频。".into());
    }
    drop(file);
    let guard = state.0.lock().map_err(err)?;
    guard
        .as_ref()
        .filter(|s| s.view.id == id && !s.cancel)
        .ok_or("已取消录屏。")?;
    // 最后确认取消；发布和取消共用状态锁，失败时 Pending 保留完整视频。
    let output = recovery.output(&root);
    progress(app, "publishing", Some(&pending));
    output::publish(&pending, &output)?;
    recovery.published(&output);
    result::remember(app, &output);
    progress(app, "completed", Some(&output));
    // 终态与成品路径由 collect 收尾时一起发布，避免回放读到 Completed 却没有路径。
    Ok(output.to_string_lossy().into_owned())
}

/// 诊断不触发试录/编码器探测，不采集视频或音频。
pub(crate) fn diagnostic_summary() -> serde_json::Value {
    serde_json::json!({"backend":std::env::consts::OS,"captureSupported":capture::supported().is_ok(),"encoderProbed":false,"audioBackend":if cfg!(target_os = "windows") { "wasapi" } else { "unsupported" },"audioProbed":false})
}

/// 原生编码放在状态锁外；与视频共用 Timeline，暂停不会在成品中留下音频空洞。
fn pump_audio(
    worker: &mut audio::Worker,
    mixer: &mut audio::Mixer,
    encoder: &mut dyn encoder::Encoder,
    state: &RecordingState,
    id: u64,
    finishing: bool,
) -> Result<(), String> {
    let frames = worker.drain()?;
    let (timeline, end) = {
        let guard = state.0.lock().map_err(err)?;
        let session = guard
            .as_ref()
            .filter(|s| s.view.id == id && !s.cancel)
            .ok_or("已取消录屏。")?;
        let elapsed = session.timeline.elapsed(capture::now());
        let end = if finishing {
            elapsed
        } else {
            elapsed.saturating_sub(audio::MIX_DELAY)
        };
        (session.timeline.clone(), end)
    };
    // 最多 128 个音频包可能累积大量逐样本运算；锁内只复制时间轴快照，
    // 在锁外混音，避免高负载时暂停/停止/取消按钮等待整批音频处理。
    for frame in frames {
        let cancelled = {
            let guard = state.0.lock().map_err(err)?;
            !guard.as_ref().is_some_and(|s| s.view.id == id && !s.cancel)
        };
        if cancelled {
            return Err("已取消录屏。".into());
        }
        mixer.ingest(frame, &timeline)?;
    }
    while let Some(packet) = mixer.next_packet(end) {
        // 长静音段可能需要补交很多包；每 10 ms 媒体数据重新检查取消，
        // 不让一次泵送在停止后继续占满编码线程。原生驱动调用本身仍不可抢占。
        {
            let guard = state.0.lock().map_err(err)?;
            if !guard.as_ref().is_some_and(|s| s.view.id == id && !s.cancel) {
                return Err("已取消录屏。".into());
            }
        }
        encoder.advance_video(packet.pts + packet.duration)?;
        encoder.push_audio(&packet)?;
    }
    Ok(())
}
