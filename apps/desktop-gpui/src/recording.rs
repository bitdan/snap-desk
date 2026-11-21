//! 录屏（对照 Tauri 版 `capture/recorder/manager.rs` 与 Vue 版 `RecordingControls.vue`）。
//!
//! 会话状态机、采集/编码流水线与平台后端都在 core（`snap_core::recording`）。这里负责：
//! 1. 截图浮层确认选区后，校验选区在单个显示器内、按录制偏好生成配置、试探编码器并在后台线程初始化采集；
//! 2. 收起浮层，打开选区红框与悬浮控制条（两者都不进入录像：Windows 用 WDA_EXCLUDEFROMCAPTURE）；
//! 3. 控制条每 200 ms 读取一次状态快照，提供暂停/继续/停止/取消，结束后显示成品并可打开目录、复制路径。
//!
//! 与 Tauri 版的差异：只录显示器区域（窗口录制未接入）；截图中画好的标注随视频输出，录制中不能再编辑标注；
//! 确认选区后直接开始（按偏好倒计时），没有单独的“准备就绪”步骤。
use gpui_kit::*;
use snap_core::{
    annotations::{Annotation, Point},
    capture::{Display, Rect},
    recording::{
        annotation_layer::{self, Layer},
        audio, capture as source, collect, encoder, frame, output,
        preferences::Preferences,
        stats, CaptureSource, ControlOutcome, Host, Phase, RecorderConfig, RecordingAction,
        RecordingState, RecordingView, Session, QUEUE_CAPACITY,
    },
    settings::{Language, RecordingExportSettings},
};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::Duration,
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
}

impl Global for Recorder {}

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
pub struct Request {
    pub rect: Rect,
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
}

/// 选区所在显示器与显示器内的局部选区；跨屏选区不支持。
fn locate(request: &Request) -> Result<(Display, Rect), String> {
    request
        .displays
        .iter()
        .find_map(|display| {
            snap_core::capture::within_display(request.rect, request.origin, display)
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
    let (display, local) = locate(&request)?;
    let mut config = RecorderConfig::for_region(local);
    config.source = CaptureSource::SelectedMonitor;
    // GPUI 版没有主界面需要收起。
    config.hide_main_window = false;
    request.preferences.apply(&mut config);
    let source_size = (display.width, display.height);
    config.validate(source_size)?;
    // 截图中的标注使用截图坐标；录制层以选区左上角为原点。
    let layer = if request.shapes.is_empty() {
        None
    } else {
        let origin = Point {
            x: f64::from(request.rect.x),
            y: f64::from(request.rect.y),
        };
        let shapes =
            annotation_layer::region_shapes(request.shapes, origin, local.width, local.height)?;
        let (layer, _, _) = annotation_layer::initial_layer(&shapes, local.width, local.height)?;
        Some(Arc::new(layer))
    };
    let export = cx.update(|cx| crate::shell::settings(cx).recording_export);
    export.validate()?;
    let (state, id) = cx.update(|cx| {
        let recorder = cx.default_global::<Recorder>();
        recorder.next_id += 1;
        (recorder.state.clone(), recorder.next_id)
    });
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
    let host = Arc::new(GpuiHost {
        state: state.clone(),
        layer,
        display: display.clone(),
        export,
        stage: Default::default(),
    });
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let (go_tx, go_rx) = mpsc::sync_channel::<bool>(1);
    let origin = (display.x, display.y);
    let monitor = source::NativeSource::Monitor {
        x: display.x,
        y: display.y,
    };
    let audio_state = state.clone();
    let worker = std::thread::Builder::new()
        .name("screen-recording".into())
        .spawn(move || {
            // 采集后端必须在所属线程初始化/释放；握手成功后等待开始信号。
            let prepared =
                source::prepare(&config, source_size, monitor, pool.clone()).and_then(|backend| {
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
    } = prepared;
    let state = cx.default_global::<Recorder>().state.clone();
    let logical = logical_region(cx, &display, region);
    let border = open_border(cx, logical);
    let controls = open_controls(cx, &display, logical, id, state.clone(), language);
    let recorder = cx.global_mut::<Recorder>();
    recorder.border = border;
    recorder.controls = controls;
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
fn logical_region(cx: &App, display: &Display, region: Rect) -> Bounds<Pixels> {
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

fn display_bounds(cx: &App, display: &Display) -> Bounds<Pixels> {
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

fn popup(bounds: Bounds<Pixels>) -> WindowOptions {
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

/// 录屏辅助窗口置顶（切换到其他应用后仍可见）且不进入录像；失败只记日志（控制条通常在选区外）。
fn protect(window: &Window) {
    if let Err(error) = crate::native::set_topmost(window, true) {
        log::warn!("event=gpui.recording_topmost_failed message={error}");
    }
    if let Err(error) = crate::native::exclude_from_capture(window) {
        log::warn!("event=gpui.recording_exclude_failed message={error}");
    }
}

const BAR_WIDTH: f32 = 460.0;
const BAR_HEIGHT: f32 = 44.0;

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

    fn button(
        &self,
        id: &'static str,
        icon: &'static [u8],
        label: String,
        primary: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .text_xs()
            .cursor_pointer()
            .text_color(rgb(crate::annotate::TEXT))
            .bg(rgb(if primary {
                0x2563eb
            } else {
                crate::annotate::HOVER
            }))
            .hover(|s| s.opacity(0.85))
            .child(crate::annotate::icon(icon, crate::annotate::TEXT))
            .child(label)
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
        let phase = self.view.as_ref().map(|v| v.phase);
        let seconds = self.view.as_ref().map_or(0.0, |v| v.seconds);
        let message = self.notice.clone().unwrap_or_else(|| self.status());
        let path = self
            .view
            .as_ref()
            .and_then(|v| v.path.clone())
            .map(PathBuf::from);
        let dot = match phase {
            Some(Phase::Recording) => RED,
            Some(Phase::Countdown) | Some(Phase::Paused) | Some(Phase::Ready) => 0xf59e0b,
            Some(Phase::Completed) => 0x22c55e,
            _ => crate::annotate::MUTED,
        };
        let mut buttons = div().flex().items_center().gap(px(6.0));
        match phase {
            Some(Phase::Ready) => {
                buttons =
                    buttons.child(
                        self.button(
                            "rec-start",
                            crate::annotate::icons::PLAY,
                            self.t("开始录制"),
                            true,
                        )
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, w, cx| v.act(RecordingAction::Start, w, cx),
                        )),
                    );
            }
            Some(Phase::Recording) | Some(Phase::Paused) | Some(Phase::Countdown) => {
                if phase == Some(Phase::Paused) {
                    buttons = buttons.child(
                        self.button(
                            "rec-resume",
                            crate::annotate::icons::PLAY,
                            self.t("继续"),
                            false,
                        )
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, w, cx| v.act(RecordingAction::Resume, w, cx),
                        )),
                    );
                } else if phase == Some(Phase::Recording) {
                    buttons = buttons.child(
                        self.button(
                            "rec-pause",
                            crate::annotate::icons::PAUSE,
                            self.t("暂停"),
                            false,
                        )
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, w, cx| v.act(RecordingAction::Pause, w, cx),
                        )),
                    );
                }
                buttons = buttons
                    .child(
                        self.button(
                            "rec-stop",
                            crate::annotate::icons::SQUARE,
                            self.t("停止并保存"),
                            true,
                        )
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, w, cx| v.act(RecordingAction::Stop, w, cx),
                        )),
                    )
                    .child(
                        self.button(
                            "rec-cancel",
                            crate::annotate::icons::CLOSE,
                            self.t("取消"),
                            false,
                        )
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, w, cx| v.act(RecordingAction::Cancel, w, cx),
                        )),
                    );
            }
            Some(Phase::Encoding) | None => {}
            Some(_) => {
                if let Some(path) = path.clone() {
                    let copy_path = path.clone();
                    buttons = buttons
                        .child(
                            self.button(
                                "rec-folder",
                                crate::annotate::icons::FOLDER_OPEN,
                                self.t("打开文件夹"),
                                false,
                            )
                            .on_click(cx.listener(
                                move |v, _: &ClickEvent, _, cx| {
                                    v.open_directory(&path);
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            self.button(
                                "rec-copy",
                                crate::annotate::icons::COPY,
                                self.t("复制路径"),
                                false,
                            )
                            .on_click(cx.listener(
                                move |v, _: &ClickEvent, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_path.to_string_lossy().into_owned(),
                                    ));
                                    v.notice = Some(v.t("已复制路径"));
                                    cx.notify();
                                },
                            )),
                        );
                }
                buttons =
                    buttons.child(
                        self.button(
                            "rec-close",
                            crate::annotate::icons::CLOSE,
                            self.t("关闭"),
                            false,
                        )
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, w, cx| v.act(RecordingAction::Cancel, w, cx),
                        )),
                    );
            }
        }
        let show_clock = matches!(
            phase,
            Some(Phase::Recording | Phase::Paused | Phase::Encoding | Phase::Completed)
        );
        // 成功后提示文字显示文件名，完整路径只在复制时使用。
        let message = match (phase, &path) {
            (Some(Phase::Completed), Some(path)) if self.notice.is_none() => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(message),
            _ => message,
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(12.0))
            .rounded(px(10.0))
            .bg(rgb(crate::annotate::SURFACE))
            .border_1()
            .border_color(rgb(crate::annotate::BORDER))
            .text_color(rgb(crate::annotate::TEXT))
            .child(div().size(px(10.0)).rounded_full().bg(rgb(dot)))
            .children(show_clock.then(|| {
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .min_w(px(44.0))
                    .child(clock(seconds))
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(rgb(crate::annotate::MUTED))
                    .child(message),
            )
            .child(buttons)
    }
}

/// GPUI 外壳为录屏流水线提供的能力：截图时画好的标注常驻视频，显示器变化时中止，状态由控制条轮询。
struct GpuiHost {
    state: Arc<RecordingState>,
    layer: Option<Arc<Layer>>,
    display: Display,
    export: RecordingExportSettings,
    /// 最近一次处理阶段；只在阶段变化时写日志（编码阶段每帧都会报告）。
    stage: std::sync::Mutex<&'static str>,
}

impl Host for GpuiHost {
    fn state(&self) -> &RecordingState {
        &self.state
    }
    fn annotations(&self, _id: u64) -> (u64, Option<Arc<Layer>>) {
        (u64::from(self.layer.is_some()), self.layer.clone())
    }
    fn screen_changed(&self) -> Result<bool, String> {
        snap_core::recording::display_unchanged(&self.display).map(|same| !same)
    }
    fn export_settings(&self) -> Result<RecordingExportSettings, String> {
        Ok(self.export.clone())
    }
    fn progress(&self, stage: &'static str, _path: Option<&Path>) {
        if let Ok(mut last) = self.stage.lock() {
            if *last != stage {
                log::info!(
                    "event=gpui.recording_stage previous={} stage={stage}",
                    *last
                );
                *last = stage;
            }
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
    use super::{
        bar_bounds, clock, locate, Display, Language, Preferences, Rect, Request, BAR_HEIGHT,
    };
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
        assert_eq!(below.origin.x, px(20.0));
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
        let request = |rect: Rect| Request {
            rect,
            origin: (-1920, 0),
            displays: vec![display(-1920), display(0)],
            shapes: Vec::new(),
            preferences: Preferences::default(),
            language: Language::default(),
        };
        let (screen, local) = locate(&request(Rect {
            x: 2000,
            y: 10,
            width: 100,
            height: 50,
        }))
        .unwrap();
        assert_eq!((screen.x, local.x, local.y), (0, 80, 10));
        assert!(locate(&request(Rect {
            x: 1900,
            y: 10,
            width: 100,
            height: 50
        }))
        .is_err());
    }
}
