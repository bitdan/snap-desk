//! 录屏（对照 Tauri 版 `capture/recorder/manager.rs` 与 Vue 版 `RecordingControls.vue`）。
//!
//! 会话状态机、采集/编码流水线与平台后端都在 core（`snap_core::recording`）。这里负责：
//! 1. 截图浮层确认选区后，校验选区在单个显示器内、按录制偏好生成配置、试探编码器并在后台线程初始化采集；
//! 2. 收起浮层，打开选区红框与悬浮控制条（两者都不进入录像：Windows 用 WDA_EXCLUDEFROMCAPTURE）；
//! 3. 控制条每 200 ms 读取一次状态快照，提供暂停/继续/停止/取消，结束后显示成品并可打开目录、复制路径。
//!
//! 4. 托盘退出时先停止并保存录屏，等流水线封装完成再退出（无进展超时规则与 Tauri 版共用 core `recording::exit`）。
//!
//! 选区正好是吸附到的窗口时，可选“整个窗口”：直接采集该窗口表面（跟随窗口移动、不受遮挡），
//! 控制条放在窗口可见面积最大的显示器上，不画红框。
//!
//! 5. 控制条“标注”打开实时标注窗口（`live_annotate.rs`）；截图中画好的标注与录制中新增的标注都由
//!    core `recording::live::Live` 对账成媒体时间轨，编码线程按修订号换用新的标注层。
//!
//! 与 Tauri 版的差异：没有来源下拉（窗口只能从吸附选区进入）；窗口录制不支持实时标注；
//! 确认选区后直接开始（按偏好倒计时），没有单独的“准备就绪”步骤。
use gpui_kit::*;
use snap_core::{
    annotations::{Annotation, Point},
    capture::{Display, Rect},
    recording::{
        annotation_layer::{self, Layer},
        live::Live,
        audio, capture as source, collect, encoder, frame, output,
        preferences::Preferences,
        exit::{self, Progress},
        stats, CaptureSource, ControlOutcome, Host, Phase, RecorderConfig, RecordingAction,
        RecordingState, RecordingView, Session, QUEUE_CAPACITY,
    },
    settings::{Language, RecordingExportSettings},
};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

/// 控制条刷新间隔。
const POLL: Duration = Duration::from_millis(200);
/// 关闭截图浮层后等桌面合成器移除它，再开始取样。
const SETTLE: Duration = Duration::from_millis(250);
const RED: u32 = 0xef4444;

/// 录屏全局状态；同一时间只有一个会话（与截图互斥）。
#[derive(Default)]
struct Recorder {
    state: Arc<RecordingState>,
    next_id: u64,
    controls: Option<AnyWindowHandle>,
    border: Option<AnyWindowHandle>,
    /// 流水线报告的真实进展；退出等待据此判断是否卡死。
    progress: Arc<Mutex<Progress>>,
    /// 已请求退出：不再开始新录屏，准备中的录屏直接放弃。
    quitting: bool,
    /// 当前录屏的实时标注入口。
    annotation: Option<crate::live_annotate::Target>,
    annotator: Option<WindowHandle<crate::live_annotate::Annotator>>,
}

impl Global for Recorder {}

/// 退出等待的检查间隔。
const EXIT_TICK: Duration = Duration::from_secs(1);

/// 托盘“退出”：没有录屏时立即执行 `finish`；否则先停止并保存，等录屏线程封装、发布完成（或长时间
/// 无进展超时）后再执行。重复请求忽略。
pub fn quit_when_idle(cx: &mut App, finish: fn(&mut App)) {
    let recorder = cx.default_global::<Recorder>();
    if recorder.quitting {
        return;
    }
    recorder.quitting = true;
    let (state, progress) = (recorder.state.clone(), recorder.progress.clone());
    state.request_stop(false);
    if !state.is_active() {
        finish(cx);
        return;
    }
    log::info!("event=gpui.quit_waiting_recording");
    // 文件系统调用可能被掉线磁盘卡住：增长观察放在后台，独立于超时裁决。
    let executor = cx.background_executor().clone();
    let (observe_state, observe_progress) = (state.clone(), progress.clone());
    cx.background_executor()
        .spawn(async move {
            let mut observed = None;
            while observe_state.is_active() {
                executor.timer(EXIT_TICK).await;
                let Some(path) = observe_progress.lock().ok().and_then(|p| p.path.clone()) else {
                    continue;
                };
                let Ok(metadata) = std::fs::metadata(&path) else {
                    continue;
                };
                if exit::grew(&mut observed, &path, metadata.len()) {
                    if let Ok(mut p) = observe_progress.lock() {
                        p.file_grew(&path);
                    }
                }
            }
        })
        .detach();
    cx.spawn(async move |cx| {
        let started = Instant::now();
        let mut last = started;
        loop {
            cx.background_executor().timer(EXIT_TICK).await;
            if !state.is_active() {
                log::info!("event=gpui.quit_recording_done wait_ms={}", started.elapsed().as_millis());
                break;
            }
            let (stage, updated) = match progress.try_lock() {
                Ok(p) => (p.stage, p.updated),
                Err(_) => ("state-unavailable", started),
            };
            last = last.max(updated);
            if exit::exit_timed_out(stage, last.elapsed(), started.elapsed()) {
                // 未完成的录制不会发布为成品（临时目录随进程结束遗留，下次不会被当作成品）。
                log::error!(
                    "event=gpui.quit_recording_timeout stage={stage} idle_ms={} total_ms={}",
                    last.elapsed().as_millis(),
                    started.elapsed().as_millis()
                );
                break;
            }
        }
        cx.update(finish);
    })
    .detach();
}


/// 是否有未结束的录屏（含封装中）。
pub fn active(cx: &App) -> bool {
    cx.try_global::<Recorder>()
        .is_some_and(|r| r.state.is_active())
}

/// 录屏快捷键 / 托盘：录制中再按视为停止并保存；截图中切到录屏框选；否则开始录屏框选。
pub fn shortcut(cx: &mut App) {
    if active(cx) {
        stop(cx, false);
        return;
    }
    if crate::capture::active(cx) {
        crate::capture::enter_recording(cx);
        return;
    }
    // 上一次录屏结束后留着的结果条先收起。
    close_controls(cx);
    let mut settings = crate::shell::settings(cx);
    settings.delay_seconds = 0;
    crate::capture::start_recording(cx, &settings);
}

/// 设置页用：未结束录屏的阶段；没有录屏或已结束时为 None。
pub fn phase(cx: &App) -> Option<Phase> {
    cx.try_global::<Recorder>()?
        .state
        .current()
        .ok()
        .flatten()
        .map(|v| v.phase)
        .filter(|p| p.active())
}

/// 设置页的录屏控制（与控制条同一入口；结果为关闭时一并收起控制条）。
pub fn control(cx: &mut App, action: RecordingAction) -> Result<(), String> {
    let state = cx.try_global::<Recorder>().map(|r| r.state.clone()).ok_or("没有进行中的录屏。")?;
    if state.control(None, action, |_| Ok(()))? == ControlOutcome::Closed {
        close_controls(cx);
    }
    Ok(())
}

/// 停止（保存）或取消当前录屏；清理由录屏线程完成。
pub fn stop(cx: &mut App, cancel: bool) {
    if let Some(recorder) = cx.try_global::<Recorder>() {
        if recorder.state.request_stop(cancel) {
            log::info!("event=gpui.recording_stop_requested cancel={cancel}");
        }
    }
}

fn preferences_path() -> Option<PathBuf> {
    crate::shell::settings_dir().map(|dir| dir.join("recording.json"))
}

/// 读取录制偏好；文件不存在或损坏时为默认值。
pub fn preferences() -> Preferences {
    preferences_path()
        .map(|path| Preferences::load(&path))
        .unwrap_or_default()
}

pub fn save_preferences(preferences: &Preferences) {
    if let Some(path) = preferences_path() {
        if let Err(error) = preferences.save(&path) {
            log::warn!("event=gpui.recording_preferences_save_failed message={error}");
        }
    }
}

/// 截图浮层交给录屏的内容：选区（截图物理像素）、截图原点、显示器布局与已画的标注。
/// 截图时吸附到的顶层窗口：截图坐标中的边界与原生身份（只在本进程内使用，不写日志）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowTarget {
    pub rect: Rect,
    pub id: u32,
    pub pid: u32,
}

pub struct Request {
    pub rect: Rect,
    /// 录制整个窗口（选区即该窗口边界）；None 为录制显示器上的选区。
    pub window: Option<WindowTarget>,
    pub origin: (i32, i32),
    pub displays: Vec<Display>,
    pub shapes: Vec<Annotation>,
    pub preferences: Preferences,
    pub language: Language,
}

/// 采集线程已握手、等待开始信号的录屏。
pub struct Prepared {
    id: u64,
    display: Display,
    region: Rect,
    language: Language,
    go: mpsc::SyncSender<bool>,
    window: bool,
    live: Arc<Mutex<Live>>,
    /// 窗口录制：采集线程报告的窗口当前位置（全局物理像素），实时标注据此跟随窗口。
    follow: Arc<Mutex<Option<(i32, i32)>>>,
}

/// 选区所在显示器与显示器内的局部选区；跨屏选区不支持。
fn locate(rect: Rect, origin: (i32, i32), displays: &[Display]) -> Result<(Display, Rect), String> {
    displays
        .iter()
        .find_map(|display| {
            snap_core::capture::within_display(rect, origin, display)
                .map(|local| (display.clone(), local))
        })
        .ok_or_else(|| "请选择单个显示器内的区域；跨屏录制与滚动截图暂不支持。".into())
}

/// 导出设置与保存目录（目录记录与 Tauri 版共用同一文件）。
fn output_root(export: &RecordingExportSettings) -> Result<PathBuf, String> {
    let record = snap_core::paths::config_dir()
        .ok_or("无法定位配置目录。")?
        .join("recording-directory.json");
    let default = snap_core::paths::local_data_dir()
        .ok_or("无法定位数据目录。")?
        .join("recordings");
    output::prepare(&record, export.remember_directory, default)
}

/// 校验选区、试探编码器并在后台线程初始化采集与声音设备；成功时采集线程等待 [`launch`] 的开始信号。
/// 失败时不留下会话，截图浮层保持打开以便显示错误。
pub async fn prepare(cx: &mut AsyncApp, request: Request) -> Result<Prepared, String> {
    if cx.update(|cx| active(cx)) {
        return Err("录屏正在进行。".into());
    }
    if cx.update(|cx| crate::scroll::active(cx)) {
        return Err("正在滚动截图，请先结束。".into());
    }
    if cx.update(|cx| cx.try_global::<Recorder>().is_some_and(|r| r.quitting)) {
        return Err("程序正在退出，不能启动录屏。".into());
    }
    // 窗口录制：控制条按窗口可见面积最大的显示器放置（该交集只用于布局，视频取整个窗口表面）。
    let placement = match request.window {
        Some(window) => {
            snap_core::capture::display_intersection(window.rect, request.origin, &request.displays)
                .ok_or("窗口没有可见的显示器区域，请重新选择。")?
        }
        None => request.rect,
    };
    let (display, local) = locate(placement, request.origin, &request.displays)?;
    let native = match request.window {
        Some(window) => source::NativeSource::Window { id: window.id, pid: window.pid },
        None => source::NativeSource::Monitor { x: display.x, y: display.y },
    };
    // 编码器、区域复制和帧池均使用采集表面的实际尺寸（窗口最小化或已关闭时在这里报错）。
    let (source_size, region) = match request.window {
        Some(_) => {
            let size = cx
                .background_executor()
                .spawn(async move { source::source_dimensions(native) })
                .await?;
            (size, Rect { x: 0, y: 0, width: size.0, height: size.1 })
        }
        None => ((display.width, display.height), local),
    };
    let mut config = RecorderConfig::for_region(region);
    config.source = if request.window.is_some() {
        CaptureSource::SelectedWindow
    } else {
        CaptureSource::SelectedMonitor
    };
    // GPUI 版没有主界面需要收起。
    config.hide_main_window = false;
    request.preferences.apply(&mut config);
    config.validate(source_size)?;
    // 截图中的标注使用截图坐标；录制层以选区左上角为原点。
    let shapes = if request.shapes.is_empty() {
        Vec::new()
    } else {
        let origin = Point {
            x: f64::from(request.rect.x),
            y: f64::from(request.rect.y),
        };
        annotation_layer::region_shapes(request.shapes, origin, region.width, region.height)?
    };
    let export = cx.update(|cx| crate::shell::settings(cx).recording_export);
    export.validate()?;
    let (state, id, progress) = cx.update(|cx| {
        let recorder = cx.default_global::<Recorder>();
        recorder.next_id += 1;
        (recorder.state.clone(), recorder.next_id, recorder.progress.clone())
    });
    let live = Arc::new(Mutex::new(Live::new(id, shapes, region.width, region.height)?));
    let probe_config = config;
    let probe_export = export.clone();
    let (root, kind, encoding, fallback) = cx
        .background_executor()
        .spawn(async move {
            source::supported()?;
            let root = output_root(&probe_export)?;
            let c = probe_config;
            let (kind, encoding, fallback) = encoder::probe(
                &root,
                c.region.width,
                c.region.height,
                c.fps,
                c.target_bitrate(),
                c.encoding,
                c.format,
                c.system_audio || c.microphone,
            )?;
            Ok::<_, String>((root, kind, encoding, fallback))
        })
        .await?;
    config.encoding = encoding;
    let queue = Arc::new(stats::QueueGauge::default());
    // 峰值同时持有底图、两帧队列和正在合成的帧；多留一帧减少分配抖动。
    let pool = frame::FramePool::new(QUEUE_CAPACITY + 2);
    *state.0.lock().map_err(|e| e.to_string())? = Some(Session::prepared(
        id,
        &config,
        kind,
        fallback,
        queue.clone(),
    ));
    let follow: Arc<Mutex<Option<(i32, i32)>>> = Arc::default();
    let host = Arc::new(GpuiHost {
        state: state.clone(),
        follow: follow.clone(),
        live: live.clone(),
        display: display.clone(),
        export,
        progress,
    });
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let (go_tx, go_rx) = mpsc::sync_channel::<bool>(1);
    let origin = (display.x, display.y);
    let audio_state = state.clone();
    let worker = std::thread::Builder::new()
        .name("screen-recording".into())
        .spawn(move || {
            // 采集后端必须在所属线程初始化/释放；握手成功后等待开始信号。
            let prepared =
                source::prepare(&config, source_size, native, pool.clone()).and_then(|backend| {
                    let audio = if config.system_audio || config.microphone {
                        // 设备在此初始化，只在 Recording 阶段启动。
                        Some(audio::Worker::start(
                            config.system_audio,
                            config.microphone,
                            move || audio_state.recording(id),
                        )?)
                    } else {
                        None
                    };
                    Ok((backend, audio))
                });
            match prepared {
                Ok((backend, audio)) => {
                    if ready_tx.send(Ok(())).is_ok() && go_rx.recv().unwrap_or(false) {
                        collect(host, id, config, origin, root, backend, audio, queue, pool);
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        });
    let ready = match worker {
        Ok(_) => {
            cx.background_executor()
                .spawn(async move {
                    ready_rx
                        .recv()
                        .map_err(|e| e.to_string())
                        .and_then(|result| result)
                })
                .await
        }
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = ready {
        let _ = go_tx.send(false);
        if let Ok(mut session) = state.0.lock() {
            *session = None;
        }
        log::error!("event=gpui.recording_prepare_failed message={error}");
        return Err(error);
    }
    log::info!(
        "event=gpui.recording_prepared id={id} width={} height={} fps={} format={:?}",
        config.region.width,
        config.region.height,
        config.fps,
        config.format
    );
    Ok(Prepared {
        id,
        display,
        region: local,
        language: request.language,
        go: go_tx,
        window: request.window.is_some(),
        live,
        follow,
    })
}

/// 截图浮层已关闭：打开红框与控制条，等桌面合成后开始录制（偏好中的倒计时由会话处理）。
pub fn launch(cx: &mut App, prepared: Prepared) {
    let Prepared {
        id,
        display,
        region,
        language,
        go,
        window,
        live,
        follow,
    } = prepared;
    let recorder = cx.default_global::<Recorder>();
    let state = recorder.state.clone();
    // 准备期间请求了退出：放弃这次录屏，让退出等待尽快结束。
    if recorder.quitting {
        let _ = go.send(false);
        if let Ok(mut session) = state.0.lock() {
            *session = None;
        }
        crate::shelf::resume(cx);
        log::info!("event=gpui.recording_abandoned_for_quit id={id}");
        return;
    }
    let logical = logical_region(cx, &display, region);
    // 窗口录制跟随窗口移动，固定位置的红框会误导，不显示（Windows 采集本身会给窗口加系统边框）。
    let border = if window { None } else { open_border(cx, logical) };
    let screen = display_bounds(cx, &display);
    let bar = bar_bounds(screen, logical);
    // 窗口录制也可标注：画布铺满显示器，录制区域随窗口移动（Tauri `update_origin`）。
    let annotation = Some(crate::live_annotate::Target {
        follow: window.then_some(follow),
        id,
        live,
        state: state.clone(),
        language: language.clone(),
        display_id: crate::capture::matched_display(cx, &display).map(|d| d.id()),
        screen,
        region: logical,
        bar,
        scale: display.scale.max(0.1) as f32,
    });
    // 截图中画好的标注在屏幕上也显示出来（鼠标穿透），与录像一致。
    let annotator = annotation
        .clone()
        .filter(|target| target.has_shapes())
        .and_then(|target| crate::live_annotate::open(cx, target, false));
    let controls = open_controls(cx, &display, logical, id, state.clone(), language, annotation.is_some());
    let recorder = cx.global_mut::<Recorder>();
    recorder.border = border;
    recorder.controls = controls;
    recorder.annotation = annotation;
    recorder.annotator = annotator;
    if go.send(true).is_err() {
        log::error!("event=gpui.recording_worker_gone id={id}");
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SETTLE).await;
        match state.control(Some(id), RecordingAction::Start, |_| Ok(())) {
            Ok(_) => log::info!("event=gpui.recording_started id={id}"),
            Err(error) => log::warn!("event=gpui.recording_start_failed message={error}"),
        }
        let _ = cx.update(|_| ());
    })
    .detach();
}

/// 选区在 GPUI 全局逻辑坐标中的边界（显示器原点按“物理像素 ÷ 缩放”换算，与截图浮层一致）。
pub(crate) fn logical_region(cx: &App, display: &Display, region: Rect) -> Bounds<Pixels> {
    let scale = display.scale.max(0.1) as f32;
    let fallback = point(px(display.x as f32 / scale), px(display.y as f32 / scale));
    let origin = crate::capture::matched_display(cx, display)
        .map(|d| d.bounds().origin)
        .unwrap_or(fallback);
    Bounds::new(
        point(
            origin.x + px(region.x as f32 / scale),
            origin.y + px(region.y as f32 / scale),
        ),
        size(
            px(region.width as f32 / scale),
            px(region.height as f32 / scale),
        ),
    )
}

pub(crate) fn display_bounds(cx: &App, display: &Display) -> Bounds<Pixels> {
    let scale = display.scale.max(0.1) as f32;
    crate::capture::matched_display(cx, display)
        .map(|d| d.bounds())
        .unwrap_or_else(|| {
            Bounds::new(
                point(px(display.x as f32 / scale), px(display.y as f32 / scale)),
                size(
                    px(display.width as f32 / scale),
                    px(display.height as f32 / scale),
                ),
            )
        })
}

pub(crate) fn popup(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

/// 选区外侧的红框：鼠标穿透，不进入录像。
struct Border;

impl Render for Border {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().border_2().border_color(rgb(RED))
    }
}

fn open_border(cx: &mut App, region: Bounds<Pixels>) -> Option<AnyWindowHandle> {
    let inset = px(2.0);
    let bounds = Bounds::new(
        point(region.origin.x - inset, region.origin.y - inset),
        size(
            region.size.width + inset * 2.0,
            region.size.height + inset * 2.0,
        ),
    );
    match cx.open_window(popup(bounds), |window, cx| {
        if let Err(error) = crate::native::set_click_through(window, true) {
            log::warn!("event=gpui.recording_border_click_through_failed message={error}");
        }
        protect(window);
        cx.new(|_| Border)
    }) {
        Ok(handle) => Some(handle.into()),
        Err(error) => {
            log::warn!("event=gpui.recording_border_failed message={error}");
            None
        }
    }
}

/// 控制条“标注”：打开实时标注窗口或切换编辑状态；控制条随后重新置顶，保持在标注窗口之上。
pub fn toggle_annotations(cx: &mut App) {
    let Some(recorder) = cx.try_global::<Recorder>() else {
        return;
    };
    let (target, annotator, controls) =
        (recorder.annotation.clone(), recorder.annotator, recorder.controls);
    let Some(target) = target.filter(|t| t.state.is_active()) else {
        return;
    };
    let reused = annotator.is_some_and(|handle| {
        handle
            .update(cx, |view, window, cx| {
                let editing = !view.editing;
                view.set_editing(editing, window, cx);
            })
            .is_ok()
    });
    if !reused {
        let handle = crate::live_annotate::open(cx, target, true);
        cx.global_mut::<Recorder>().annotator = handle;
    }
    if let Some(controls) = controls {
        let _ = controls.update(cx, |_, window, _| {
            let _ = crate::native::set_topmost(window, true);
        });
    }
}

/// 标注窗口自行关闭（结束编辑时没有标注、标注全部到期）。
pub fn annotator_closed(cx: &mut App, id: u64) {
    if let Some(recorder) = cx.has_global::<Recorder>().then(|| cx.global_mut::<Recorder>()) {
        if recorder.annotation.as_ref().is_some_and(|t| t.id == id) {
            recorder.annotator = None;
        }
    }
}

/// 录屏结束：关闭标注窗口并清除入口。
fn close_annotations(cx: &mut App) {
    let Some(recorder) = cx.has_global::<Recorder>().then(|| cx.global_mut::<Recorder>()) else {
        return;
    };
    if let Some(target) = recorder.annotation.take() {
        if let Ok(mut live) = target.live.lock() {
            live.editing = false;
        }
    }
    if let Some(handle) = recorder.annotator.take() {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// 录屏辅助窗口置顶（切换到其他应用后仍可见）且不进入录像；失败只记日志（控制条通常在选区外）。
pub fn protect(window: &Window) {
    if let Err(error) = crate::native::set_topmost(window, true) {
        log::warn!("event=gpui.recording_topmost_failed message={error}");
    }
    if let Err(error) = crate::native::exclude_from_capture(window) {
        log::warn!("event=gpui.recording_exclude_failed message={error}");
    }
}

/// 与 Tauri 版录屏控制条窗口一致（480×48）。
const BAR_WIDTH: f32 = 480.0;
const BAR_HEIGHT: f32 = 48.0;

/// 控制条放在选区下方；放不下时放在上方，再不行放在选区内底部。
fn bar_bounds(screen: Bounds<Pixels>, region: Bounds<Pixels>) -> Bounds<Pixels> {
    let gap = px(8.0);
    let (width, height) = (px(BAR_WIDTH), px(BAR_HEIGHT));
    let below = region.origin.y + region.size.height + gap;
    let screen_bottom = screen.origin.y + screen.size.height;
    let y = if below + height <= screen_bottom {
        below
    } else if region.origin.y - gap - height >= screen.origin.y {
        region.origin.y - gap - height
    } else {
        (region.origin.y + region.size.height - height - gap).max(screen.origin.y)
    };
    let centered = region.origin.x + region.size.width / 2.0 - width / 2.0;
    let x = centered
        .max(screen.origin.x)
        .min(screen.origin.x + screen.size.width - width);
    Bounds::new(point(x, y), size(width, height))
}

fn open_controls(
    cx: &mut App,
    display: &Display,
    region: Bounds<Pixels>,
    id: u64,
    state: Arc<RecordingState>,
    language: Language,
    annotations: bool,
) -> Option<AnyWindowHandle> {
    let bounds = bar_bounds(display_bounds(cx, display), region);
    let result = cx.open_window(popup(bounds), |window, cx| {
        protect(window);
        cx.new(|cx| {
            let view = Controls {
                id,
                state,
                language,
                view: None,
                finished: false,
                notice: None,
                annotations,
            };
            cx.spawn(
                async move |this: WeakEntity<Controls>, cx: &mut AsyncApp| loop {
                    cx.background_executor().timer(POLL).await;
                    if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                        break;
                    }
                },
            )
            .detach();
            view
        })
    });
    match result {
        Ok(handle) => Some(handle.into()),
        Err(error) => {
            log::error!("event=gpui.recording_controls_failed message={error}");
            None
        }
    }
}

fn close_border(cx: &mut App) {
    let border = cx.try_global::<Recorder>().and_then(|r| r.border);
    if let Some(handle) = border {
        cx.global_mut::<Recorder>().border = None;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// 收起已结束录屏的结果条；仍在录制时不关闭。
fn close_controls(cx: &mut App) {
    if active(cx) {
        return;
    }
    let controls = cx.try_global::<Recorder>().and_then(|r| r.controls);
    if let Some(handle) = controls {
        cx.global_mut::<Recorder>().controls = None;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// 悬浮控制条。
struct Controls {
    id: u64,
    state: Arc<RecordingState>,
    language: Language,
    view: Option<RecordingView>,
    /// 终态的收尾（收起红框、恢复暂存架）只做一次。
    finished: bool,
    /// 操作反馈（已复制路径、打开目录失败等）。
    notice: Option<String>,
    /// 可在录制中标注（显示器录制）。
    annotations: bool,
}

impl Controls {
    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    /// 状态文案按阶段生成（core 的提示文本含动态数字与路径，不能直接查英文词条）；失败原因原样显示。
    fn status(&self) -> String {
        let Some(view) = self.view.as_ref() else {
            return self.t("正在准备录屏…");
        };
        match view.phase {
            Phase::Ready => self.t("录屏选区已准备"),
            Phase::Countdown => format!("{} {}", view.countdown_remaining, self.t("秒后开始录制")),
            Phase::Recording => self.t("录制中"),
            Phase::Paused => self.t("已暂停"),
            Phase::Encoding if view.message.contains("取消") => self.t("正在取消录屏…"),
            Phase::Encoding => self.t("正在保存录屏…"),
            Phase::Completed => self.t("录屏已保存"),
            Phase::Cancelled => self.t("已取消录屏。"),
            Phase::Failed => view.message.clone(),
        }
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let view = self
            .state
            .current()
            .ok()
            .flatten()
            .filter(|v| v.id == self.id);
        let terminal = view.as_ref().is_some_and(|v| !v.phase.active());
        self.view = view;
        if terminal && !self.finished {
            self.finished = true;
            let phase = self.view.as_ref().map(|v| v.phase);
            log::info!(
                "event=gpui.recording_finished id={} phase={phase:?}",
                self.id
            );
            close_border(cx);
            close_annotations(cx);
            crate::shelf::resume(cx);
        }
        cx.notify();
    }

    fn act(&mut self, action: RecordingAction, window: &mut Window, cx: &mut Context<Self>) {
        match self.state.control(Some(self.id), action, |_| Ok(())) {
            Ok(ControlOutcome::Closed) => {
                cx.global_mut::<Recorder>().controls = None;
                window.remove_window();
                return;
            }
            Ok(_) => {}
            Err(error) => self.notice = Some(error),
        }
        self.poll(cx);
    }

    fn open_directory(&mut self, path: &Path) {
        let result = path
            .parent()
            .filter(|p| p.is_dir())
            .ok_or_else(|| "录屏目录已被移动或删除。".to_owned())
            .and_then(snap_core::platform::open_directory);
        if let Err(error) = result {
            self.notice = Some(error);
        }
    }
}

/// 时长显示为 分:秒（超过 1 小时为 时:分:秒）。
fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 还原 Vue `RecordingControls.vue`：红点、时长、状态、分隔线、帧率，右侧图标按钮；与截图工具栏同款胶囊。
        use crate::annotate::{border, hover, icon, icons, muted, surface, text};
        let view = self.view.clone();
        let phase = view.as_ref().map(|v| v.phase);
        let seconds = view.as_ref().map_or(0.0, |v| v.seconds);
        let path = view.as_ref().and_then(|v| v.path.clone()).map(PathBuf::from);
        let live = matches!(phase, Some(Phase::Ready | Phase::Countdown | Phase::Recording | Phase::Paused));
        let recording = phase == Some(Phase::Recording);
        let status = match (phase, &path) {
            (Some(Phase::Completed), Some(path)) if self.notice.is_none() => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.status()),
            _ => self.notice.clone().unwrap_or_else(|| self.status()),
        };
        let tool = |id: &'static str, data: &'static [u8], tip: String, color: u32, enabled: bool| {
            let button = div()
                .id(id)
                .h(px(30.0))
                .min_w(px(30.0))
                .px(px(6.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(4.0))
                .rounded(px(6.0))
                .child(icon(data, color).size(px(17.0)))
                .tooltip(move |_, cx| cx.new(|_| crate::annotate::Tooltip(tip.clone().into())).into());
            if enabled {
                button.cursor_pointer().hover(|s| s.bg(rgb(hover())))
            } else {
                button.opacity(0.4)
            }
        };
        let mut buttons = div().flex().flex_none().items_center().gap(px(2.0));
        match phase {
            Some(Phase::Ready) => {
                buttons = buttons.child(
                    div()
                        .id("rec-start")
                        .h(px(30.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .rounded(px(6.0))
                        .bg(rgb(0x2563eb))
                        .text_color(rgb(0xffffff))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x1d4ed8)))
                        .child(icon(icons::PLAY, 0xffffff).size(px(15.0)))
                        .child(self.t("开始"))
                        .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.act(RecordingAction::Start, w, cx))),
                );
            }
            Some(Phase::Countdown) => {
                buttons = buttons.child(
                    div()
                        .h(px(30.0))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .opacity(0.4)
                        .child(icon(icons::PLAY, text()).size(px(15.0)))
                        .child(self.t("等待开始")),
                );
            }
            Some(Phase::Paused) => {
                buttons = buttons.child(
                    tool("rec-resume", icons::PLAY, self.t("继续录制"), text(), true)
                        .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.act(RecordingAction::Resume, w, cx))),
                );
            }
            Some(Phase::Recording) => {
                buttons = buttons.child(
                    tool("rec-pause", icons::PAUSE, self.t("暂停录制"), text(), true)
                        .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.act(RecordingAction::Pause, w, cx))),
                );
            }
            _ => {}
        }
        if live {
            let stoppable = matches!(phase, Some(Phase::Recording | Phase::Paused));
            let mut stop = tool("rec-stop", icons::SQUARE, self.t("结束录制并保存"), 0xe5484d, stoppable);
            if stoppable {
                stop = stop.on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.act(RecordingAction::Stop, w, cx)));
            }
            buttons = buttons
                .child(stop)
                .child(
                    tool("rec-cancel", icons::CLOSE, self.t("取消并丢弃本次录屏"), text(), true)
                        .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.act(RecordingAction::Cancel, w, cx))),
                );
            if self.annotations {
                buttons = buttons.child(
                    tool("rec-annotate", icons::PENCIL, self.t("录制中标注"), text(), true)
                        .on_click(|_, _, cx| toggle_annotations(cx)),
                );
            }
        } else if phase.is_some() && phase != Some(Phase::Encoding) {
            // 结束后的结果：打开文件夹、复制路径、关闭。
            if let Some(path) = path.clone() {
                let copy_path = path.clone();
                buttons = buttons
                    .child(
                        tool("rec-folder", icons::FOLDER_OPEN, self.t("打开文件夹"), text(), true).on_click(cx.listener(
                            move |v, _: &ClickEvent, _, cx| {
                                v.open_directory(&path);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        tool("rec-copy", icons::COPY, self.t("复制路径"), text(), true).on_click(cx.listener(
                            move |v, _: &ClickEvent, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_path.to_string_lossy().into_owned()));
                                v.notice = Some(v.t("已复制路径"));
                                cx.notify();
                            },
                        )),
                    );
            }
            buttons = buttons.child(
                tool("rec-close", icons::CLOSE, self.t("关闭"), text(), true)
                    .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.act(RecordingAction::Cancel, w, cx))),
            );
        }
        let fps = view.as_ref().filter(|_| phase != Some(Phase::Countdown) && live).map(|v| {
            let dropped = v.dropped_frames > 0;
            let mut label = format!("{} FPS", v.target_fps);
            if dropped {
                label = format!("{label} · {}", self.t("丢帧 {0}").replace("{0}", &v.dropped_frames.to_string()));
            }
            div()
                .text_size(px(11.0))
                .whitespace_nowrap()
                .text_color(rgb(if dropped { 0xb45309 } else { muted() }))
                .child(label)
        });
        let dot = div()
            .flex_none()
            .size(px(10.0))
            .mr(px(6.0))
            .rounded_full()
            .bg(rgb(if recording { 0xe5484d } else if phase == Some(Phase::Completed) { 0x22c55e } else { 0xa1a1aa }));
        let dot = if recording { dot.border_3().border_color(rgba(0xe5484d33)).size(px(16.0)).mx(px(-3.0)) } else { dot };
        div()
            // 录屏不能直接隐藏：Esc 录制中结束并保存，就绪/倒计时时取消（Vue `RecordingControls.vue`）。
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != "escape" {
                    return;
                }
                match phase {
                    Some(Phase::Recording | Phase::Paused) => stop(cx, false),
                    Some(Phase::Ready | Phase::Countdown) => stop(cx, true),
                    _ => return,
                }
                cx.stop_propagation();
            })
            .size_full()
            .flex()
            .items_center()
            .gap(px(2.0))
            .pl(px(12.0))
            .pr(px(6.0))
            .rounded(px(10.0))
            .overflow_hidden()
            .bg(rgb(surface()))
            .border_1()
            .border_color(rgb(border()))
            .text_xs()
            .text_color(rgb(text()))
            .child(dot)
            .children((phase != Some(Phase::Ready)).then(|| {
                div().mr(px(6.0)).text_size(px(14.0)).whitespace_nowrap().child(clock(seconds))
            }))
            .child(
                div()
                    .min_w(px(42.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(rgb(muted()))
                    .child(status),
            )
            .child(div().flex_none().w(px(1.0)).h(px(18.0)).mx(px(6.0)).bg(rgb(border())))
            .children(fps)
            .child(div().flex_1())
            .child(buttons)
    }
}

/// GPUI 外壳为录屏流水线提供的能力：标注层来自实时标注会话，显示器变化时中止，状态由控制条轮询。
struct GpuiHost {
    state: Arc<RecordingState>,
    follow: Arc<Mutex<Option<(i32, i32)>>>,
    live: Arc<Mutex<Live>>,
    display: Display,
    export: RecordingExportSettings,
    /// 真实处理进展（阶段变化时记录日志；退出等待据此判断是否卡死）。
    progress: Arc<Mutex<Progress>>,
}

impl Host for GpuiHost {
    fn state(&self) -> &RecordingState {
        &self.state
    }
    fn annotations(&self, id: u64) -> (u64, Option<Arc<Layer>>) {
        self.live
            .lock()
            .ok()
            .filter(|live| live.id == id)
            .map(|live| {
                let (revision, layer) = live.snapshot();
                (revision, Some(layer))
            })
            .unwrap_or((0, None))
    }
    /// 编辑标注时不录入键鼠特效，避免录下标注工具栏的按键与点击。
    fn annotation_editing(&self, id: u64) -> bool {
        self.live.lock().is_ok_and(|live| live.id == id && live.editing)
    }
    fn update_origin(&self, _id: u64, origin: (i32, i32)) {
        if let Ok(mut follow) = self.follow.lock() {
            *follow = Some(origin);
        }
    }
    fn timed_annotations(&self, id: u64) -> bool {
        let ms = self.state.media_time_ms().unwrap_or(0);
        self.live.lock().is_ok_and(|live| live.id == id && live.has_timed(ms))
    }
    fn screen_changed(&self) -> Result<bool, String> {
        snap_core::recording::display_unchanged(&self.display).map(|same| !same)
    }
    fn export_settings(&self) -> Result<RecordingExportSettings, String> {
        Ok(self.export.clone())
    }
    fn progress(&self, stage: &'static str, path: Option<&Path>) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.record(stage, path);
        }
    }
    fn published(&self, _path: &Path) {
        log::info!("event=gpui.recording_published");
    }
    fn changed(&self) {}
    fn finished(&self, id: u64, _message: &str) {
        log::info!("event=gpui.recording_pipeline_finished id={id}");
    }
}

#[cfg(test)]
mod tests {
    use super::{bar_bounds, clock, locate, Display, Rect, BAR_HEIGHT, BAR_WIDTH};
    use gpui_kit::{point, px, size, Bounds};

    #[test]
    fn clock_formats_minutes_and_hours() {
        assert_eq!(clock(0.4), "00:00");
        assert_eq!(clock(75.9), "01:15");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn bar_prefers_below_then_above_then_inside() {
        let screen = Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(800.0)));
        let region = |y: f32, h: f32| Bounds::new(point(px(100.0), px(y)), size(px(300.0), px(h)));
        let below = bar_bounds(screen, region(100.0, 200.0));
        assert_eq!(below.origin.y, px(308.0));
        let above = bar_bounds(screen, region(400.0, 390.0));
        assert_eq!(above.origin.y, px(400.0 - 8.0 - BAR_HEIGHT));
        let inside = bar_bounds(screen, region(10.0, 785.0));
        assert_eq!(inside.origin.y, px(10.0 + 785.0 - BAR_HEIGHT - 8.0));
        // 水平居中于选区，且不超出屏幕左边。
        assert_eq!(below.origin.x, px((250.0 - BAR_WIDTH / 2.0).max(0.0)));
        let left = bar_bounds(
            screen,
            Bounds::new(point(px(0.0), px(100.0)), size(px(40.0), px(40.0))),
        );
        assert_eq!(left.origin.x, px(0.0));
    }

    #[test]
    fn region_must_fit_one_display() {
        let display = |x: i32| Display {
            x,
            y: 0,
            width: 1920,
            height: 1080,
            primary: x == 0,
            scale: 1.0,
        };
        let displays = vec![display(-1920), display(0)];
        let rect = |x: u32| Rect { x, y: 10, width: 100, height: 50 };
        let (screen, local) = locate(rect(2000), (-1920, 0), &displays).unwrap();
        assert_eq!((screen.x, local.x, local.y), (0, 80, 10));
        assert!(locate(rect(1900), (-1920, 0), &displays).is_err());
    }
}
