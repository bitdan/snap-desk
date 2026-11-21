//! 录屏会话管理：命令、控制条与实时标注窗口。会话状态机、采集/编码流水线与平台后端在 core
//! （`snap_core::recording`，GPUI 版共用），这里通过 [`TauriHost`] 为流水线提供标注快照与收尾。
use super::*;
use std::{path::Path, sync::mpsc};
#[cfg(test)]
#[path = "countdown_tests.rs"]
mod countdown_tests;
pub mod live_annotations;
pub mod output;
pub mod result;
mod lifecycle;
pub use lifecycle::{ExitState, request_exit, request_restart};
use lifecycle::{progress, WorkerGuard};
use super::scroll::chrome;
pub use snap_core::recording::{
    annotation_layer, annotation_track, diagnostic_summary, Phase, RecordingAction, RecordingState,
    RecordingView,
};
use snap_core::recording::{
    audio, capture, collect, encoder, frame, stats, wayland_session, CaptureSource,
    ControlOutcome, Host, RecorderCapabilities, RecorderConfig, Session, QUEUE_CAPACITY,
};

/// 阶段变化只发失效通知；各窗口按权限重新查询，统计数据仍由低频快照校准。
fn notify_changed(app: &AppHandle) {
    for label in ["main", "capture", "recording-controls"] {
        let _ = app.emit_to(label, "recording-changed", ());
    }
}

/// 是否存在未结束的录屏会话；截图蒙层兜底清理时用来避免误关录屏画布。
pub fn is_active(app: &AppHandle) -> bool {
    app.state::<RecordingState>().is_active()
}

/// 托盘、录屏快捷键和退出入口只发停止信号，清理由录屏工作线程完成。
pub fn request_stop(app: &AppHandle, cancel: bool) -> bool {
    if !app.state::<RecordingState>().request_stop(cancel) {
        return false;
    }
    live_annotations::hide_interaction(app);
    notify_changed(app);
    true
}

/// Alt+F4 只关闭已经结束的控制条，不能意外中止仍在录制/封装的视频。
pub fn dismiss_completed_controls(app: &AppHandle) {
    // 进入新框选前隐藏上一轮已经结束的控制条。
    if app.state::<RecordingState>().completed_id().is_some() {
        chrome::hide_recording(app);
    }
}

/// Alt+F4 关闭已经结束的控制条，成品文件不受影响。
pub fn close_completed_controls(app: &AppHandle) {
    if let Some(id) = app.state::<RecordingState>().completed_id() {
        let _ = control(app, Some(id), RecordingAction::Cancel);
    }
}

/// 暂停保留现场；开始和恢复按本次会话偏好处理主界面，边框和控制条不收起。
pub fn control(app: &AppHandle, id: Option<u64>, action: RecordingAction) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    let outcome = state.control(id, action, |hide_main_window| {
        if hide_main_window {
            if let Some(main) = app.get_webview_window("main") {
                main.hide().map_err(err)?;
            }
            // 仅自动收起时等待桌面合成移除主窗口，再开启计时和取样。
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    })?;
    match outcome {
        ControlOutcome::Closed => {
            chrome::hide_recording(app);
            chrome::release_recording(app);
        }
        ControlOutcome::Unchanged => {}
        ControlOutcome::Paused => {
            notify_changed(app);
            // 所有入口暂停都不展开主窗口，继续通过悬浮控制条恢复或停止。
            if id.is_none() {
                let _ = app.emit_to("main", "capture-result", "录屏已暂停，可恢复或停止导出。");
            }
        }
        ControlOutcome::Started => notify_changed(app),
        ControlOutcome::Stopping => {
            live_annotations::hide_interaction(app);
            // 控制窗继续显示封装进度，完成后原位切换预览；停止请求不能迟到隐藏预览。
            notify_changed(app);
        }
    }
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
    app.state::<RecordingState>().current()
}

fn require_control_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "capture" | "recording-controls") {
        Ok(())
    } else {
        Err("当前窗口无权控制录屏。".into())
    }
}

/// 页面查询只读取平台/采集支持，不初始化编码器、音频设备或生成临时试录。
/// 格式和声音字段表示可选的平台能力；实际设备、尺寸及编码组合在确认选区时试写。
#[tauri::command]
pub async fn recording_capabilities(window: WebviewWindow) -> Result<RecorderCapabilities, String> {
    if !matches!(window.label(), "main" | "capture") {
        return Err("当前窗口无权查询录屏能力。".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        RecorderCapabilities::from_capture_support(capture::supported())
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
        if state.scroll.session.lock().map_err(err)?.is_some() {
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
            // Wayland 下 GDK 报告的主屏不可靠，且来源固定为截图时冻结的显示器。
            if matches!(config.source, CaptureSource::PrimaryMonitor) && !wayland_session() {
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
        // Wayland 不允许读取全局键鼠；旧配置里开启的特效在这里关闭，不让录制因此失败。
        if wayland_session() {
            let effects = &mut config.input_effects;
            effects.keyboard = false;
            effects.clicks = false;
            effects.movement = false;
            effects.drag = false;
            effects.wheel = false;
        }
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
        // 采集线程与查询共用同一水位；编码线程只改原子计数，不持录屏状态锁。
        let queue = std::sync::Arc::new(stats::QueueGauge::default());
        // 峰值同时持有底图、两帧队列和正在合成的帧；多留一帧减少分配抖动。
        let pool = frame::FramePool::new(QUEUE_CAPACITY + 2);
        let recording = app.state::<RecordingState>();
        *recording.0.lock().map_err(err)? = Some(Session::prepared(
            id,
            &config,
            encoder_kind,
            encoder_fallback,
            queue.clone(),
        ));
        // WGC 必须在所属线程初始化/释放。握手成功后由原选区页接管，等待明确开始。
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (go_tx, go_rx) = mpsc::sync_channel(1);
        let worker_guard = WorkerGuard::acquire(&app)?;
        let worker_app = app.clone();
        let worker = std::thread::Builder::new()
            .name("screen-recording".into())
            .spawn(move || {
                let _worker = worker_guard;
                let prepared = capture::prepare(&config, source_size, source, pool.clone()).and_then(|backend| {
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
                            move || audio_app.state::<RecordingState>().recording(id),
                        )?)
                    } else {
                        None
                    };
                    Ok((backend, audio))
                });
                match prepared {
                    Ok((backend, audio)) => {
                        if ready_tx.send(Ok(())).is_ok() && go_rx.recv().unwrap_or(false) {
                            let origin = (screen.x, screen.y);
                            let host = Arc::new(TauriHost { app: worker_app, screen });
                            collect(host, id, config, origin, root, backend, audio, queue, pool);
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

/// Tauri 外壳为流水线提供的能力：实时标注来自录制浮层，终态收起控制条并通知主界面。
struct TauriHost {
    app: AppHandle,
    screen: super::scroll::Screen,
}

impl Host for TauriHost {
    fn state(&self) -> &RecordingState {
        self.app.state::<RecordingState>().inner()
    }
    fn annotations(&self, id: u64) -> (u64, Option<Arc<annotation_layer::Layer>>) {
        live_annotations::snapshot(&self.app, id)
    }
    fn annotation_editing(&self, id: u64) -> bool {
        live_annotations::editing(&self.app, id)
    }
    fn timed_annotations(&self, id: u64) -> bool {
        live_annotations::has_timed_annotations(&self.app, id)
    }
    fn update_origin(&self, id: u64, origin: (i32, i32)) {
        live_annotations::update_origin(&self.app, id, origin);
    }
    fn screen_changed(&self) -> Result<bool, String> {
        Ok(super::scroll::session::screen_at(&self.app, self.screen.x, self.screen.y)? != self.screen)
    }
    fn export_settings(&self) -> Result<crate::settings::RecordingExportSettings, String> {
        output::preferences(&self.app)
    }
    fn progress(&self, stage: &'static str, path: Option<&Path>) {
        progress(&self.app, stage, path);
    }
    fn published(&self, path: &Path) {
        result::remember(&self.app, path);
    }
    fn release_annotations(&self, id: u64) {
        live_annotations::clear(&self.app, id);
    }
    fn changed(&self) {
        notify_changed(&self.app);
    }
    fn finished(&self, _id: u64, message: &str) {
        // 成功后直接保留已经发布的成品，所有终态都收起录制交互窗口。
        chrome::release_recording(&self.app);
        self.app
            .state::<CaptureState>()
            .active
            .store(false, Ordering::SeqCst);
        crate::shelf::resume_after_capture(&self.app);
        let _ = self.app.emit_to("main", "capture-result", message);
        // 不主动改变主界面的显示状态；成功路径已在结果消息和原生成品状态中发布。
    }
}
