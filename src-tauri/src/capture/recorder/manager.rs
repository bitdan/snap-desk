//! 录屏会话管理：命令、UI、WGC 采集和有界编码线程的生命周期。
use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, TrySendError},
};
mod audio;
mod annotation_track;
mod annotation_layer;
pub mod project;
mod capture;
mod compositor;
mod config;
#[cfg(test)]
#[path = "countdown_tests.rs"]
mod countdown_tests;
pub mod editing;
mod encoder;
mod frame;
mod gif;
mod input;
pub mod live_annotations;
pub mod output;
pub mod recovery;
pub mod result;
mod stats;
mod timeline;
mod lifecycle;
mod pipeline;
pub use lifecycle::{ExitState, request_exit};
use lifecycle::{progress, WorkerGuard};
#[cfg(test)]
use lifecycle::exit_timed_out;
use pipeline::collect;
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
    /// 在确认录制时固定；开始和继续共用，避免中途改变本次会话的窗口行为。
    hide_main_window: bool,
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
pub struct RecordingState(Mutex<Option<Session>>, Mutex<()>);

/// 阶段变化只发失效通知；各窗口按权限重新查询，统计数据仍由低频快照校准。
fn notify_changed(app: &AppHandle) {
    for label in ["main", "capture", "recording-controls"] {
        let _ = app.emit_to(label, "recording-changed", ());
    }
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
    drop(guard);
    live_annotations::hide_interaction(app);
    notify_changed(app);
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

/// Alt+F4 只关闭已经完成的回放，不能意外中止仍在录制/封装的视频。
pub fn dismiss_completed_preview(app: &AppHandle) {
    // 进入新框选前暂时隐藏；截图成功才销毁，失败仍能返回原回放。
    let state = app.state::<RecordingState>();
    let completed = state.0.lock().ok().is_some_and(|guard| {
        guard.as_ref().is_some_and(|s| !active(s.view.phase))
    });
    if completed { chrome::hide_recording(app); }
}

/// Alt+F4 明确关闭回放；与新框选前的临时隐藏分开处理。
pub fn close_completed_preview(app: &AppHandle) {
    let id = app.state::<RecordingState>().0.lock().ok().and_then(|guard| {
        guard.as_ref().filter(|s| !active(s.view.phase)).map(|s| s.view.id)
    });
    if let Some(id) = id { let _ = control(app, Some(id), RecordingAction::Cancel); }
}

/// 暂停保留现场；开始和恢复按本次会话偏好处理主界面，边框和控制条不收起。
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
            let closed_id = s.view.id;
            drop(guard);
            chrome::hide_recording(app);
            // 保留模式通知前端卸载播放器；释放模式还会销毁整个 WebView。
            let _ = app.emit_to("recording-controls", "recording-preview-closed", closed_id);
            chrome::release_recording(app);
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
            notify_changed(app);
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
            let hide_main_window = s.hide_main_window;
            drop(guard);
            if hide_main_window {
                if let Some(main) = app.get_webview_window("main") {
                    main.hide().map_err(err)?;
                }
                // 仅自动收起时等待桌面合成移除主窗口，再开启计时和取样。
                std::thread::sleep(Duration::from_millis(250));
            }
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
            drop(guard);
            notify_changed(app);
            return Ok(());
        }
        RecordingAction::Stop | RecordingAction::Cancel => {
            s.stop_requested(matches!(action, RecordingAction::Cancel));
            drop(guard);
            live_annotations::hide_interaction(app);
            // 控制窗继续显示封装进度，完成后原位切换预览；停止请求不能迟到隐藏预览。
            notify_changed(app);
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
    if matches!(window.label(), "main" | "capture" | "recording-controls") {
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
    annotations: Option<Vec<crate::shared::annotations::Annotation>>,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _preparing = WorkerGuard::acquire(&app)?;
        #[cfg(windows)]
        let _dpi = crate::capture::desktop::PhysicalDpi::enter()?;
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
        crate::shared::annotations::validate(&annotations, size.0, size.1)?;
        // 标注仍使用虚拟桌面截图坐标；视频输出原点单独保存，不能使用显示器局部坐标。
        let mut annotation_origin = crate::shared::annotations::Point {
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
            crate::capture::desktop::display_intersection(
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
            annotation_origin = crate::shared::annotations::Point {
                x: candidate.rect.x as f64,
                y: candidate.rect.y as f64,
            };
            // 窗口录制使用整个原生来源，不把桌面裁剪坐标误用于窗口表面。
            let source = capture::NativeSource::Window {
                id: native.id().map_err(err)?,
                pid: native.pid().map_err(err)?,
            };
            // 编码器、区域复制和帧池均使用 WGC 的实际尺寸；DWM 仅负责桌面定位。
            source_size = capture::source_dimensions(source)?;
            rect = Rect {
                x: 0,
                y: 0,
                width: source_size.0,
                height: source_size.1,
            };
            source
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
        chrome::hide_recording(&app);
        chrome::place_recording(&app, &screen, placement)?;
        super::hide_overlay(&app, &window)?;
        let recording = app.state::<RecordingState>();
        *recording.0.lock().map_err(err)? = Some(Session {
            hide_main_window: config.hide_main_window,
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
        // WGC 必须在所属线程初始化/释放。握手成功后由原选区页接管，等待明确开始。
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (go_tx, go_rx) = mpsc::sync_channel(1);
        let worker_guard = WorkerGuard::acquire(&app)?;
        let worker_app = app.clone();
        let worker = std::thread::Builder::new()
            .name("screen-recording".into())
            .spawn(move || {
                let _worker = worker_guard;
                let prepared = capture::prepare(&config, source_size, source).and_then(|backend| {
                    live_annotations::initialize(
                        &worker_app,
                        id,
                        &screen,
                        config.region,
                        backend.desktop_origin()?,
                        annotations,
                        annotation_origin,
                    )?;
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
                            collect(worker_app, id, config, screen, root, backend, audio);
                        }
                    }
                    Err(e) => {
                        live_annotations::clear(&worker_app, id);
                        let _ = ready_tx.send(Err(e));
                    }
                }
            });
        let ready = match worker {
            Ok(_) => ready_rx.recv().map_err(err).and_then(|result| result),
            Err(e) => Err(err(e)),
        };
        if let Err(error) = ready {
            let _ = go_tx.send(false);
            chrome::release_recording(&app);
            live_annotations::clear(&app, id);
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
        notify_changed(&app);
        Ok(())
    })
    .await
    .map_err(err)?
}

/// 诊断不触发试录/编码器探测，不采集视频或音频。
pub(crate) fn diagnostic_summary() -> serde_json::Value {
    serde_json::json!({"backend":std::env::consts::OS,"captureSupported":capture::supported().is_ok(),"encoderProbed":false,"audioBackend":if cfg!(target_os = "windows") { "wasapi" } else { "unsupported" },"audioProbed":false})
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
