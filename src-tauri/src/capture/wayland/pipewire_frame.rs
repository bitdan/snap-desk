//! 从门户授权的 PipeWire 流中取一帧：pipewiresrc → videoconvert → RGBA appsink。
//! 复用录屏已依赖的 GStreamer；pipewiresrc 由 gstreamer1.0-pipewire 提供。
//! GNOME 在 Start 响应后仍在播放授权对话框的关闭动画，首帧后再等一小段时间取最新帧，避免截到对话框。
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use image::RgbaImage;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(400);
const FIRST_FRAME: Duration = Duration::from_secs(5);

pub(super) fn grab(fd: &OwnedFd, node: u32) -> Result<RgbaImage, String> {
    gst::init().map_err(|e| format!("无法初始化 GStreamer：{e}"))?;
    if gst::ElementFactory::find("pipewiresrc").is_none() {
        return Err(
            "缺少 GStreamer PipeWire 插件（gstreamer1.0-pipewire），无法读取屏幕共享画面。".into(),
        );
    }
    // 门户返回的是已授权的 PipeWire 连接；pipewiresrc 会复制该描述符，管线结束前本端保持打开。
    let description = format!(
        "pipewiresrc fd={} path={node} do-timestamp=true ! videoconvert ! video/x-raw,format=RGBA ! appsink name=sink max-buffers=1 drop=true sync=false",
        fd.as_raw_fd()
    );
    let pipeline = gst::parse_launch(&description)
        .map_err(|e| e.to_string())?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "无法创建屏幕共享管线。".to_string())?;
    let result = pull(&pipeline);
    let _ = pipeline.set_state(gst::State::Null);
    result
}

fn pull(pipeline: &gst::Pipeline) -> Result<RgbaImage, String> {
    let sink = pipeline
        .by_name("sink")
        .and_then(|element| element.downcast::<gst_app::AppSink>().ok())
        .ok_or("无法创建屏幕共享管线。")?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| e.to_string())?;
    let first = sink
        .try_pull_sample(gst::ClockTime::from_nseconds(FIRST_FRAME.as_nanos() as u64))
        .ok_or_else(|| bus_error(pipeline).unwrap_or_else(|| "屏幕共享没有送来画面。".into()))?;
    let mut latest = first;
    let settle = Instant::now() + SETTLE;
    while let Some(left) = settle.checked_duration_since(Instant::now()) {
        match sink.try_pull_sample(gst::ClockTime::from_nseconds(left.as_nanos() as u64)) {
            Some(sample) => latest = sample,
            // 画面没有变化时 PipeWire 不再送帧，首帧就是最新画面。
            None => break,
        }
    }
    to_image(&latest)
}

fn bus_error(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus()?;
    while let Some(message) = bus.pop() {
        if let gst::MessageView::Error(error) = message.view() {
            return Some(error.error().to_string());
        }
    }
    None
}

fn to_image(sample: &gst::Sample) -> Result<RgbaImage, String> {
    let caps = sample.caps().ok_or("屏幕共享画面缺少格式信息。")?;
    let structure = caps.structure(0).ok_or("屏幕共享画面缺少格式信息。")?;
    let width = structure.get::<i32>("width").map_err(|e| e.to_string())?;
    let height = structure.get::<i32>("height").map_err(|e| e.to_string())?;
    let (width, height) = (
        u32::try_from(width).map_err(|_| "屏幕共享画面尺寸无效。")?,
        u32::try_from(height).map_err(|_| "屏幕共享画面尺寸无效。")?,
    );
    let buffer = sample.buffer().ok_or("屏幕共享画面为空。")?;
    let map = buffer.map_readable().map_err(|e| e.to_string())?;
    let stride = if height == 0 {
        0
    } else {
        map.size() / height as usize
    };
    super::to_rgba(map.as_slice(), width, height, stride, false, false)
}
