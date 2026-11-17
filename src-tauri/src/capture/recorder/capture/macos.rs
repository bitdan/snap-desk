//! macOS 显示器录制：复用 xcap 的 AVCaptureScreenInput 录制器取整屏帧，再裁剪为选区。
//!
//! 坐标约定与截图一致：`NativeSource::Monitor` 的 x/y 与选区都是“点坐标 × 该屏缩放”的物理像素。
//! xcap 每帧给出整屏 RGBA；这里只复制选区并转回 BGRA，交给与 Windows 共用的合成与编码流水线。
//! xcap 录制器固定包含光标，且不支持单窗口录制，这两项在能力中如实标为不可用。
use super::{
    super::frame::{BgraImage, FramePool},
    CaptureBackend, NativeSource, RecorderConfig, VideoFrame,
};
use crate::capture::Rect;
use std::{
    sync::{mpsc::Receiver, Arc, Mutex},
    time::Duration,
};

/// 录屏时间轴的单位是 100 ns。使用主机时钟，与系统音频、麦克风回调的时间戳同源，
/// 音画按同一时间轴对齐。
pub fn now() -> i64 {
    crate::shared::macos::host_ticks()
}

/// 录屏需要“屏幕录制”授权；未授权时系统只返回桌面壁纸，必须提前拒绝而不是录出空白视频。
pub fn supported() -> Result<(), String> {
    if !crate::shared::macos::screen_capture_granted() {
        return Err("录屏需要“屏幕录制”权限，请在“设置 → 通用 → 系统权限”中授权后重启应用。".into());
    }
    Ok(())
}

pub fn source_dimensions(source: NativeSource) -> Result<(u32, u32), String> {
    match source {
        NativeSource::Monitor { x, y } => {
            let monitor = monitor_at(x, y)?;
            let scale = f64::from(monitor.scale_factor().map_err(|e| e.to_string())?);
            let px = |v: u32| (f64::from(v) * scale).round() as u32;
            Ok((
                px(monitor.width().map_err(|e| e.to_string())?),
                px(monitor.height().map_err(|e| e.to_string())?),
            ))
        }
        NativeSource::Window { .. } => {
            Err("UnsupportedPlatform：macOS 暂不支持单窗口录制，请录制显示器区域。".into())
        }
    }
}

/// 按物理原点找回显示器；允许 2 像素取整误差（缩放比例以 f32 保存）。
fn monitor_at(x: i32, y: i32) -> Result<xcap::Monitor, String> {
    xcap::Monitor::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| {
            let (Ok(mx), Ok(my), Ok(scale)) = (m.x(), m.y(), m.scale_factor()) else {
                return false;
            };
            let scale = f64::from(scale);
            (f64::from(mx) * scale - f64::from(x)).abs() <= 2.0
                && (f64::from(my) * scale - f64::from(y)).abs() <= 2.0
        })
        .ok_or_else(|| "录制的显示器已断开或布局已改变，请重新选择。".into())
}

pub struct Capture {
    recorder: xcap::VideoRecorder,
    /// Drop 时先于停止录制释放：xcap 用无缓冲通道发送帧，回调可能正阻塞在发送上，
    /// 先断开接收端才能让回调返回，否则 stopRunning 等待回调结束会形成死锁。
    frames: Option<Receiver<xcap::Frame>>,
    /// wait_for_frame 只借用 self，等到的帧暂存于此，由下一次 next_frame 取走。
    pending: Mutex<Option<xcap::Frame>>,
    region: Rect,
    source: (i32, i32),
    /// 源屏的物理像素尺寸；录制器输出若是点分辨率，按比例换算选区。
    source_size: (u32, u32),
    pool: Arc<FramePool>,
    started: bool,
}

impl Capture {
    pub fn new(
        config: &RecorderConfig,
        size: (u32, u32),
        source: NativeSource,
        pool: Arc<FramePool>,
    ) -> Result<Self, String> {
        supported()?;
        let NativeSource::Monitor { x, y } = source else {
            return Err("UnsupportedPlatform：macOS 暂不支持单窗口录制，请录制显示器区域。".into());
        };
        let monitor = monitor_at(x, y)?;
        let (recorder, frames) = monitor
            .video_recorder()
            .map_err(|e| format!("无法初始化 macOS 录屏：{e}"))?;
        Ok(Self {
            recorder,
            frames: Some(frames),
            pending: Mutex::new(None),
            region: config.region,
            source: (x, y),
            source_size: size,
            pool,
            started: false,
        })
    }

    /// 取最新一帧并丢弃积压的旧帧：编码端按时间戳补帧，追赶旧画面只会增加延迟。
    fn latest(&self) -> Option<xcap::Frame> {
        let mut latest = self.pending.lock().ok()?.take();
        if let Some(frames) = &self.frames {
            while let Ok(frame) = frames.try_recv() {
                latest = Some(frame);
            }
        }
        latest
    }

    /// 从整屏 RGBA 帧裁剪选区并转为 BGRA。录制器输出与源屏尺寸不同（点分辨率）时按比例取样。
    fn crop(&self, frame: &xcap::Frame) -> Result<BgraImage, String> {
        let r = self.region;
        let (fw, fh) = (frame.width as usize, frame.height as usize);
        if fw == 0 || fh == 0 || frame.raw.len() < fw * fh * 4 {
            return Err("录屏帧数据不完整。".into());
        }
        let sx = fw as f64 / f64::from(self.source_size.0.max(1));
        let sy = fh as f64 / f64::from(self.source_size.1.max(1));
        let mut image = self.pool.acquire(r.width, r.height);
        let exact = (sx - 1.0).abs() < 1e-6 && (sy - 1.0).abs() < 1e-6;
        for row in 0..r.height as usize {
            let source_row = if exact {
                r.y as usize + row
            } else {
                ((f64::from(r.y) + row as f64 + 0.5) * sy) as usize
            }
            .min(fh - 1);
            let target = &mut image.data[row * r.width as usize * 4..(row + 1) * r.width as usize * 4];
            for column in 0..r.width as usize {
                let source_column = if exact {
                    r.x as usize + column
                } else {
                    ((f64::from(r.x) + column as f64 + 0.5) * sx) as usize
                }
                .min(fw - 1);
                let s = (source_row * fw + source_column) * 4;
                let t = column * 4;
                target[t] = frame.raw[s + 2];
                target[t + 1] = frame.raw[s + 1];
                target[t + 2] = frame.raw[s];
                target[t + 3] = 255;
            }
        }
        Ok(image)
    }
}

impl CaptureBackend for Capture {
    fn desktop_origin(&self) -> Result<(i32, i32), String> {
        Ok(self.source)
    }
    fn check_source(&self) -> Result<(), String> {
        let (x, y) = self.source;
        monitor_at(x, y).map(|_| ())
    }
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String> {
        // 与 Windows 一致：用户点击开始后第一次拉帧才启动系统采集。
        if !self.started {
            self.recorder
                .start()
                .map_err(|e| format!("无法开始 macOS 录屏：{e}"))?;
            self.started = true;
        }
        let Some(frame) = self.latest() else {
            return Ok(None);
        };
        let image = self.crop(&frame)?;
        Ok(Some(VideoFrame {
            source_time: now(),
            image,
        }))
    }
    fn wait_for_frame(&self, timeout: Duration) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        if pending.is_some() {
            return;
        }
        if let Some(frame) = self.frames.as_ref().and_then(|f| f.recv_timeout(timeout).ok()) {
            *pending = Some(frame);
        }
    }
    fn refresh(&mut self) -> Result<(), String> {
        // 丢弃暂停前的帧，恢复后只使用新到达的画面。
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
        if let Some(frames) = &self.frames {
            while frames.try_recv().is_ok() {}
        }
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.frames.take();
        if self.started {
            let _ = self.recorder.stop();
        }
    }
}
