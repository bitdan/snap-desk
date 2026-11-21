//! Wayland 录制来源：屏幕共享门户（ScreenCast）授权一块显示器，GStreamer `pipewiresrc` 读取视频流，
//! 缩放到截图时冻结的物理尺寸后按选区裁剪，交给与 X11 共用的编码管线。
//!
//! 授权与截图共用同一份恢复令牌；门户授权的显示器必须与截图时选中的显示器一致，否则拒绝录制，
//! 避免把另一块屏幕的画面按错误的坐标裁剪。Wayland 下应用无法读取全局键鼠，也无法从画面中去掉
//! 本应用的窗口：控制条与标注画布会出现在录像中，请把它们移出录制区域。
use super::{now, BgraImage, CaptureBackend, FramePool, NativeSource, RecorderConfig, VideoFrame};
use crate::capture::wayland::{self, portal};
use crate::capture::Rect;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MISSING_PLUGIN: &str =
    "缺少 GStreamer PipeWire 插件（gstreamer1.0-pipewire），无法读取屏幕共享画面。";
const NOT_FROZEN: &str = "录制的显示器已断开或布局已改变，请重新选择。";
const WRONG_OUTPUT: &str =
    "屏幕共享授权的显示器与所选显示器不一致，请重新录屏并在系统对话框中选择同一块显示器。";
const STOPPED: &str = "屏幕共享已停止，录制已结束。";

pub fn supported() -> Result<(), String> {
    let support = crate::desktop_support::current();
    if !support.protocols().portal_screencast {
        return Err(
            "当前 Wayland 桌面没有屏幕共享门户（xdg-desktop-portal ScreenCast），无法录屏。".into(),
        );
    }
    gst::init().map_err(|e| format!("无法初始化 GStreamer：{e}"))?;
    if gst::ElementFactory::find("pipewiresrc").is_none() {
        return Err(MISSING_PLUGIN.into());
    }
    Ok(())
}

/// 录制来源只能是截图时冻结的那块显示器（Wayland 下没有窗口来源）。
fn frozen(source: NativeSource) -> Result<wayland::Frozen, String> {
    let NativeSource::Monitor { x, y } = source else {
        return Err("Wayland 下暂不支持单窗口录制，请录制显示器区域。".into());
    };
    wayland::last_frozen()
        .filter(|frozen| frozen.origin == (x, y))
        .ok_or_else(|| NOT_FROZEN.into())
}

pub fn source_dimensions(source: NativeSource) -> Result<(u32, u32), String> {
    frozen(source).map(|frozen| frozen.size)
}

pub fn prepare(
    config: &RecorderConfig,
    size: (u32, u32),
    source: NativeSource,
    pool: Arc<FramePool>,
) -> Result<Box<dyn CaptureBackend>, String> {
    Ok(Box::new(Capture::new(config, size, source, pool)?))
}

/// 选区必须完整位于来源画面内。
pub(super) fn region_fits(region: Rect, size: (u32, u32)) -> bool {
    region.width > 0
        && region.height > 0
        && u64::from(region.x) + u64::from(region.width) <= u64::from(size.0)
        && u64::from(region.y) + u64::from(region.height) <= u64::from(size.1)
}

/// 从紧密或带行尾填充的 BGRA 帧中复制选区。
pub(super) fn crop(
    data: &[u8],
    size: (u32, u32),
    region: Rect,
    target: &mut BgraImage,
) -> Result<(), String> {
    let stride = if size.1 == 0 {
        0
    } else {
        data.len() / size.1 as usize
    };
    let row = region.width as usize * 4;
    if stride < size.0 as usize * 4 || !region_fits(region, size) {
        return Err("屏幕共享画面尺寸与录制来源不一致。".into());
    }
    for y in 0..region.height as usize {
        let start = (region.y as usize + y) * stride + region.x as usize * 4;
        target.data[y * row..(y + 1) * row].copy_from_slice(&data[start..start + row]);
    }
    Ok(())
}

pub struct Capture {
    // 字段按声明顺序析构：先停管线，再关 PipeWire 连接，最后结束门户会话。
    pipeline: gst::Pipeline,
    sink: gst_app::AppSink,
    _remote: OwnedFd,
    _session: portal::Session,
    origin: (i32, i32),
    size: (u32, u32),
    region: Rect,
    pool: Arc<FramePool>,
    /// 等待到达但尚未取走的帧；`wait_for_frame` 只有共享引用，因此用锁保存。
    pending: Mutex<Option<gst::Sample>>,
    /// 最近一帧；静态画面没有新帧时，恢复录制据此重新送出底图。
    last: Option<gst::Sample>,
}

impl Capture {
    pub fn new(
        config: &RecorderConfig,
        size: (u32, u32),
        source: NativeSource,
        pool: Arc<FramePool>,
    ) -> Result<Self, String> {
        supported()?;
        let frozen = frozen(source)?;
        if frozen.size != size || !region_fits(config.region, size) {
            return Err("录制来源尺寸已变化，请重新选择录屏区域。".into());
        }
        let conn = portal::connect()?;
        let session = portal::start_with_token(
            &conn,
            portal::SCREENCAST_TOKEN,
            config.capture_cursor,
            false,
        )?;
        let (node, position, _) = session.streams[0];
        let (lx, ly, _, _) = frozen.logical;
        if position.is_some_and(|position| position != (lx, ly)) {
            // 令牌指向另一块显示器；清除后下一次录屏重新选择。
            portal::forget_token(portal::SCREENCAST_TOKEN);
            return Err(WRONG_OUTPUT.into());
        }
        let remote = session.open_pipewire()?;
        // 门户流通常是逻辑尺寸或物理尺寸；统一缩放到冻结截图的物理尺寸，选区坐标无需换算。
        let description = format!(
            "pipewiresrc fd={} path={node} do-timestamp=true ! videoconvert ! videoscale ! video/x-raw,format=BGRA,width={},height={} ! appsink name=sink max-buffers=2 drop=true sync=false",
            remote.as_raw_fd(),
            size.0,
            size.1
        );
        let pipeline = gst::parse_launch(&description)
            .map_err(|e| e.to_string())?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "无法创建屏幕共享管线。".to_string())?;
        let sink = pipeline
            .by_name("sink")
            .and_then(|element| element.downcast::<gst_app::AppSink>().ok())
            .ok_or("无法创建屏幕共享管线。")?;
        if let Err(error) = pipeline.set_state(gst::State::Playing) {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(error.to_string());
        }
        if let Some(value) = &session.restore_token {
            portal::write_token(portal::SCREENCAST_TOKEN, value);
        }
        Ok(Self {
            pipeline,
            sink,
            _remote: remote,
            _session: session,
            origin: frozen.origin,
            size,
            region: config.region,
            pool,
            pending: Mutex::new(None),
            last: None,
        })
    }

    /// 管线报错或结束（用户在系统托盘里停止共享）时返回原因。
    fn stopped(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Error(error) => {
                    log::warn!(
                        "event=recording.wayland_stream_failed module=capture error={}",
                        error.error()
                    );
                    return Some(STOPPED.into());
                }
                gst::MessageView::Eos(_) => return Some(STOPPED.into()),
                _ => {}
            }
        }
        None
    }

    fn take(&self) -> Option<gst::Sample> {
        let pending = self.pending.lock().ok()?.take();
        pending.or_else(|| self.sink.try_pull_sample(gst::ClockTime::ZERO))
    }

    fn frame(&self, sample: &gst::Sample) -> Result<BgraImage, String> {
        let buffer = sample.buffer().ok_or("屏幕共享画面为空。")?;
        let map = buffer.map_readable().map_err(|e| e.to_string())?;
        let mut image = self.pool.acquire(self.region.width, self.region.height);
        crop(map.as_slice(), self.size, self.region, &mut image)?;
        Ok(image)
    }
}

impl CaptureBackend for Capture {
    fn desktop_origin(&self) -> Result<(i32, i32), String> {
        Ok(self.origin)
    }
    fn check_source(&self) -> Result<(), String> {
        match self.stopped() {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String> {
        let Some(sample) = self.take() else {
            if self.sink.is_eos() {
                return Err(STOPPED.into());
            }
            return Ok(None);
        };
        let image = self.frame(&sample)?;
        self.last = Some(sample);
        Ok(Some(VideoFrame {
            source_time: now(),
            image,
        }))
    }
    fn wait_for_frame(&self, timeout: Duration) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        if pending.is_none() {
            *pending = self
                .sink
                .try_pull_sample(gst::ClockTime::from_nseconds(timeout.as_nanos() as u64));
        }
    }
    fn refresh(&mut self) -> Result<(), String> {
        if let (Some(last), Ok(mut pending)) = (self.last.clone(), self.pending.lock()) {
            if pending.is_none() {
                *pending = Some(last);
            }
        }
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn region_must_fit_inside_the_source() {
        assert!(region_fits(rect(0, 0, 4, 2), (4, 2)));
        assert!(!region_fits(rect(1, 0, 4, 2), (4, 2)));
        assert!(!region_fits(rect(0, 0, 0, 2), (4, 2)));
    }

    #[test]
    fn crops_padded_rows() {
        // 2×2 帧，每行带 4 字节填充。
        let data = [
            1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0, //
            3, 3, 3, 3, 4, 4, 4, 4, 0, 0, 0, 0,
        ];
        let mut target = BgraImage::new(1, 2);
        crop(&data, (2, 2), rect(1, 0, 1, 2), &mut target).unwrap();
        assert_eq!(target.data, [2, 2, 2, 2, 4, 4, 4, 4]);
        assert!(crop(&data[..8], (2, 2), rect(0, 0, 1, 1), &mut target).is_err());
    }
}
