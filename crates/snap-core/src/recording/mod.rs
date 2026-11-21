//! 录屏引擎：会话状态机、采集/编码流水线与平台后端（WGC / ScreenCaptureKit / X11·PipeWire 采集，
//! Media Foundation / AVFoundation / GStreamer 编码，WASAPI / ScreenCaptureKit / PulseAudio 声音，键鼠特效）。
//!
//! 与界面无关，Tauri 与 GPUI 两个外壳共用。外壳只负责框选、控制条、实时标注浮层和结果展示，
//! 通过 [`Host`] 向流水线提供标注快照、显示器变化检查、输出设置与收尾通知。
pub mod annotation_layer;
pub mod annotation_track;
pub mod audio;
pub mod capture;
pub mod compositor;
pub mod config;
pub mod encoder;
pub mod exit;
pub mod frame;
pub mod gif;
pub mod input;
pub mod output;
mod pipeline;
pub mod preferences;
pub mod stats;
pub mod timeline;
#[cfg(test)]
mod session_tests;

pub use config::{CaptureSource, RecorderCapabilities, RecorderConfig};
pub use pipeline::{collect, QUEUE_CAPACITY};

use crate::settings::RecordingExportSettings;
use annotation_layer::Layer;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use timeline::Timeline;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

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

impl Phase {
    /// 会话仍占用屏幕与设备（含封装中）。
    pub fn active(self) -> bool {
        matches!(
            self,
            Phase::Ready | Phase::Countdown | Phase::Recording | Phase::Paused | Phase::Encoding
        )
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingView {
    pub id: u64,
    pub revision: u64,
    pub phase: Phase,
    pub seconds: f64,
    pub frames: usize,
    pub dropped_frames: usize,
    pub target_fps: u32,
    pub system_audio: bool,
    pub microphone: bool,
    /// push 的累计墙钟耗时，包含转换和同步提交，不代表 GPU 内部延迟。
    pub encoding_millis: f64,
    pub stats: stats::RecordingStats,
    pub target_bitrate: u32,
    pub encoder_kind: encoder::EncoderKind,
    pub encoder_fallback: Option<String>,
    pub countdown_remaining: u8,
    pub message: String,
    pub path: Option<String>,
}

/// 状态锁仅保护时间轴和帧提交；系统取样/编码在锁外进行，暂停通过代次淘汰旧帧。
pub struct Session {
    /// 在确认录制时固定；开始和继续共用，避免中途改变本次会话的窗口行为。
    pub hide_main_window: bool,
    pub view: RecordingView,
    pub timeline: Timeline,
    pub fps_window: stats::FpsWindow,
    /// 与采集/编码线程共享，查询时刷新诊断中的队列水位，不参与命令裁决。
    pub queue: Arc<stats::QueueGauge>,
    pub countdown_seconds: u8,
    pub countdown_until: Option<i64>,
    pub stop: bool,
    pub cancel: bool,
}

impl Session {
    /// 选区确认后、采集握手前的会话：Ready 阶段，等待用户点击开始。
    pub fn prepared(
        id: u64,
        config: &RecorderConfig,
        encoder_kind: encoder::EncoderKind,
        encoder_fallback: Option<String>,
        queue: Arc<stats::QueueGauge>,
    ) -> Self {
        Self {
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
            queue,
            countdown_seconds: config.countdown_seconds,
            countdown_until: None,
            stop: false,
            cancel: false,
        }
    }

    pub fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.timeline.elapsed(capture::now()).max(0) as u64 * 100)
    }

    pub fn freeze(&mut self) {
        self.countdown_until = None;
        self.view.countdown_remaining = 0;
        self.timeline.stop(capture::now());
        self.view.seconds = self.elapsed().as_secs_f64();
    }

    /// 仅首次开始使用倒计时；恢复不延时。源时间轴在真正开始采集前保持停止。
    pub fn begin(&mut self, now: i64, delay: u8) {
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
    pub fn advance_countdown(&mut self, now: i64) {
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

    pub fn stop_requested(&mut self, cancel: bool) {
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingAction {
    Start,
    Pause,
    Resume,
    Stop,
    Cancel,
}

/// 控制动作的结果；窗口操作由外壳按结果执行，不在状态锁内进行。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlOutcome {
    /// 已结束会话的关闭：只收起控制条，成品不受影响。
    Closed,
    /// 已结束会话上的幂等停止。
    Unchanged,
    Paused,
    /// 开始（可能进入倒计时）或恢复。
    Started,
    /// 已请求停止或取消，流水线随后排空并收尾。
    Stopping,
}

/// 唯一的录屏会话；第二把锁使控制动作串行。
#[derive(Default)]
pub struct RecordingState(pub Mutex<Option<Session>>, Mutex<()>);

impl RecordingState {
    /// 是否存在未结束的录屏会话；锁异常时保守地视为仍在录制。
    pub fn is_active(&self) -> bool {
        self.0
            .lock()
            .map(|guard| guard.as_ref().is_some_and(|s| s.view.phase.active()))
            .unwrap_or(true)
    }

    /// 已结束（完成/取消/失败）但控制条尚未关闭的会话 ID。
    pub fn completed_id(&self) -> Option<u64> {
        self.0.lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .filter(|s| !s.view.phase.active())
                .map(|s| s.view.id)
        })
    }

    /// 托盘、快捷键和退出入口只发停止信号，清理由录屏工作线程完成。
    pub fn request_stop(&self, cancel: bool) -> bool {
        let Ok(mut guard) = self.0.lock() else {
            return false;
        };
        let Some(s) = guard.as_mut().filter(|s| s.view.phase.active()) else {
            return false;
        };
        s.stop_requested(cancel);
        true
    }

    /// 查询快照：刷新时长、近期帧率与队列水位。
    pub fn current(&self) -> Result<Option<RecordingView>, String> {
        let guard = self.0.lock().map_err(err)?;
        Ok(guard.as_ref().map(|s| {
            let mut view = s.view.clone();
            view.seconds = s.elapsed().as_secs_f64();
            view.stats.recent_fps =
                if matches!(view.phase, Phase::Ready | Phase::Countdown | Phase::Paused) {
                    0.0
                } else {
                    s.fps_window.fps(s.timeline.elapsed(capture::now()))
                };
            // 水位直接读取无锁计数；阶段切换期间队列可能仍在排空，保留真实值。
            view.stats.queue_depth = s.queue.depth();
            view.stats.queue_peak = s.queue.peak();
            view
        }))
    }

    /// 录制标注只能在未停止的会话中修改。
    pub fn editable(&self, id: u64) -> bool {
        self.0.lock().ok().is_some_and(|session| {
            session.as_ref().is_some_and(|s| {
                s.view.id == id
                    && !s.stop
                    && !s.cancel
                    && matches!(
                        s.view.phase,
                        Phase::Ready | Phase::Countdown | Phase::Recording | Phase::Paused
                    )
            })
        })
    }

    /// 录屏 Timeline 的媒体时间（毫秒）；Ready/暂停期间不推进。
    pub fn media_time_ms(&self) -> Result<u64, String> {
        let current = self.0.lock().map_err(err)?;
        Ok(current.as_ref().map_or(0, |s| {
            s.timeline.elapsed(capture::now()).max(0) as u64 / 10_000
        }))
    }

    /// 音频设备只在 Recording 阶段启动。
    pub fn recording(&self, id: u64) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|guard| {
                guard.as_ref().map(|s| {
                    s.view.id == id && s.view.phase == Phase::Recording && !s.stop && !s.cancel
                })
            })
            .unwrap_or(false)
    }

    /// 暂停保留现场；开始和恢复前调用 `before_start(hide_main_window)`（在状态锁外），
    /// 由外壳收起主界面并等待桌面合成，再开启计时和取样。
    pub fn control(
        &self,
        id: Option<u64>,
        action: RecordingAction,
        before_start: impl FnOnce(bool) -> Result<(), String>,
    ) -> Result<ControlOutcome, String> {
        // 控制动作串行，但窗口操作不得持有状态锁，避免 UI 查询状态时相互等待。
        let _control = self.1.lock().map_err(err)?;
        let mut guard = self.0.lock().map_err(err)?;
        let s = guard
            .as_mut()
            .filter(|s| id.is_none_or(|id| s.view.id == id))
            .ok_or("录屏会话已过期。")?;
        // 已完成/取消的同会话停止动作幂等；旧 ID 不能触及新会话。
        if !s.view.phase.active() {
            return match action {
                // 完成后的关闭只收起控制条，不删除已保存的录像；旧会话 ID 仍在上方校验。
                RecordingAction::Cancel => Ok(ControlOutcome::Closed),
                RecordingAction::Stop => Ok(ControlOutcome::Unchanged),
                _ => Err("录屏已结束。".into()),
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
                Ok(ControlOutcome::Paused)
            }
            RecordingAction::Start | RecordingAction::Resume
                if !s.stop
                    && ((matches!(action, RecordingAction::Start)
                        && s.view.phase == Phase::Ready)
                        || (matches!(action, RecordingAction::Resume)
                            && s.view.phase == Phase::Paused)) =>
            {
                let session_id = s.view.id;
                let expected_phase = s.view.phase;
                let hide_main_window = s.hide_main_window;
                drop(guard);
                before_start(hide_main_window)?;
                let mut guard = self.0.lock().map_err(err)?;
                let s = guard
                    .as_mut()
                    .filter(|s| {
                        s.view.id == session_id && s.view.phase == expected_phase && !s.stop
                    })
                    .ok_or("录屏已停止。")?;
                let delay = if expected_phase == Phase::Ready {
                    s.countdown_seconds
                } else {
                    0
                };
                s.begin(capture::now(), delay);
                Ok(ControlOutcome::Started)
            }
            RecordingAction::Stop | RecordingAction::Cancel => {
                s.stop_requested(matches!(action, RecordingAction::Cancel));
                Ok(ControlOutcome::Stopping)
            }
            _ => Err("当前录屏状态不支持此操作。".into()),
        }
    }
}

/// 单次取样在状态锁外计时；空取样也统计耗时，不包含 wait_for_frame 的空闲等待。
pub fn measured_capture(
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

/// 外壳为流水线提供的能力。方法在采集或编码线程上调用，实现不得阻塞在界面线程上。
pub trait Host: Send + Sync + 'static {
    fn state(&self) -> &RecordingState;
    /// 实时标注快照：(版本号, 标注层)。没有标注时返回 (0, None)。
    fn annotations(&self, _id: u64) -> (u64, Option<Arc<Layer>>) {
        (0, None)
    }
    /// 正在编辑标注时，工具栏上的按键与点击不进入键鼠特效。
    fn annotation_editing(&self, _id: u64) -> bool {
        false
    }
    /// 是否有限时显示的标注仍在动画中（需要持续出帧）。
    fn timed_annotations(&self, _id: u64) -> bool {
        false
    }
    /// 采集来源的桌面原点变化（窗口录制时窗口移动）。
    fn update_origin(&self, _id: u64, _origin: (i32, i32)) {}
    /// 录制所在显示器的位置、尺寸或 DPI 是否已改变。
    fn screen_changed(&self) -> Result<bool, String>;
    fn export_settings(&self) -> Result<RecordingExportSettings, String>;
    /// 真实处理进展（退出看门狗据此判断是否卡死）。
    fn progress(&self, _stage: &'static str, _path: Option<&Path>) {}
    /// 成品已发布。
    fn published(&self, _path: &Path) {}
    /// 流水线结束、标注会话应当释放（在写入终态之前）。
    fn release_annotations(&self, _id: u64) {}
    /// 状态变化（阶段、统计）；外壳刷新控制条。
    fn changed(&self);
    /// 流水线完全结束（设备已释放、终态已写入），`message` 为面向用户的结果文案。
    fn finished(&self, id: u64, message: &str);
}

/// 诊断不触发试录/编码器探测，不采集视频或音频。
/// 声音分三层报告：后端是否内置（audioBackend）、系统授权（*Permission）、设备是否实际可用
/// （audioProbed=false 表示未打开设备检测，不能据此认为设备不可用）。
pub fn diagnostic_summary() -> serde_json::Value {
    let (system_audio_permission, microphone_permission) = audio_permissions();
    serde_json::json!({
        "backend": std::env::consts::OS,
        "captureSupported": capture::supported().is_ok(),
        "encoderProbed": false,
        "audioBackend": audio_backend(),
        "audioBackendSupported": audio_backend() != "unsupported",
        "systemAudioPermission": system_audio_permission,
        "microphonePermission": microphone_permission,
        "audioProbed": false,
    })
}

/// 与 `audio::Worker` 的编译目标一一对应。
fn audio_backend() -> &'static str {
    if cfg!(target_os = "windows") {
        "wasapi"
    } else if cfg!(target_os = "linux") {
        "pulseaudio"
    } else if cfg!(target_os = "macos") {
        // 系统声音走 ScreenCaptureKit 音频流，麦克风走 AVCaptureSession。
        "screencapturekit+avfoundation"
    } else {
        "unsupported"
    }
}

/// 只读系统授权状态，不弹出授权框；不需要应用内授权的平台报告 not_required。
fn audio_permissions() -> (&'static str, &'static str) {
    #[cfg(target_os = "macos")]
    {
        let system = if crate::macos::screen_capture_granted() {
            "granted"
        } else {
            "not_granted"
        };
        (system, audio::microphone_status())
    }
    #[cfg(not(target_os = "macos"))]
    {
        ("not_required", "not_required")
    }
}

/// Wayland 不允许读取全局键鼠；旧配置里开启的特效在准备时关闭。
pub fn wayland_session() -> bool {
    #[cfg(target_os = "linux")]
    return crate::x11::wayland();
    #[cfg(not(target_os = "linux"))]
    false
}

/// 录制所在显示器是否仍与开始时一致（位置、物理尺寸、缩放）；拔插或改分辨率后应停止录制。
/// Windows 以外的平台由采集后端自行检测来源变化，这里总是返回 true。
pub fn display_unchanged(display: &crate::capture::Display) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let _dpi = crate::capture::PhysicalDpi::enter()?;
        let monitors = xcap::Monitor::all().map_err(err)?;
        Ok(monitors.iter().any(|m| {
            m.x().ok() == Some(display.x)
                && m.y().ok() == Some(display.y)
                && m.width().ok() == Some(display.width)
                && m.height().ok() == Some(display.height)
                // xcap 以 f32 报告缩放；统一到 f32 比较，避免 110% 等比例误报。
                && m.scale_factor().ok().map(f32::to_bits) == Some((display.scale as f32).to_bits())
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = display;
        Ok(true)
    }
}
