//! Linux 编码：GStreamer 管线 appsrc → videoconvert → H.264 → mp4mux → filesink，
//! 有声音时另接 appsrc → audioconvert → AAC → 同一 mp4mux。
//!
//! 编码插件来自系统：自动模式优先硬件（VA-API、NVENC），其次软件（x264、OpenH264）；
//! 缺少插件时明确说明需要安装的软件包，不静默改用其他格式。
//! 时间戳按 100 ns 换算为纳秒；每帧暂存到下一帧到达时才写出，以便给出准确时长，
//! 静止画面每 100 ms 复用同一缓冲推进一次视频流，避免封装器为等待视频而积压音频。
use super::{AudioPacket, BgraImage, Encoder, EncoderKind, EncodingPreference};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::path::Path;

/// 帧重复间隔（100 ns）。
const REPEAT: i64 = 1_000_000;
/// 结束封装的最长等待；写出 moov 一般在一秒内完成。
const FINISH_SECONDS: u64 = 60;

/// 一个 H.264 编码插件：工厂名、码率单位是否为 kbit/s、需要的原始格式。
struct Candidate {
    factory: &'static str,
    kbps: bool,
    format: &'static str,
    kind: EncoderKind,
}

const HARDWARE: &[Candidate] = &[
    Candidate {
        factory: "vah264enc",
        kbps: true,
        format: "NV12",
        kind: EncoderKind::Hardware,
    },
    Candidate {
        factory: "vaapih264enc",
        kbps: true,
        format: "NV12",
        kind: EncoderKind::Hardware,
    },
    Candidate {
        factory: "nvh264enc",
        kbps: true,
        format: "NV12",
        kind: EncoderKind::Hardware,
    },
];

const SOFTWARE: &[Candidate] = &[
    Candidate {
        factory: "x264enc",
        kbps: true,
        format: "I420",
        kind: EncoderKind::Software,
    },
    Candidate {
        factory: "openh264enc",
        kbps: false,
        format: "I420",
        kind: EncoderKind::Software,
    },
];

const AAC: &[&str] = &["avenc_aac", "fdkaacenc", "voaacenc", "faac"];

fn init() -> Result<(), String> {
    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| gst::init().map_err(|e| format!("无法初始化 GStreamer：{e}")))
        .clone()
}

fn available(factory: &str) -> bool {
    gst::ElementFactory::find(factory).is_some()
}

/// 自动模式先试硬件再试软件；指定软件时只看软件插件。
fn choose(preference: EncodingPreference) -> Option<&'static Candidate> {
    let software = SOFTWARE.iter().find(|c| available(c.factory));
    match preference {
        EncodingPreference::Auto => HARDWARE.iter().find(|c| available(c.factory)).or(software),
        EncodingPreference::Software => software,
    }
}

fn make(factory: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| missing(factory))
}

/// 缺少的插件对应 Debian / Ubuntu 软件包名，其他发行版名称相近。
fn missing(factory: &str) -> String {
    let package = match factory {
        "mp4mux" | "aacparse" => "gstreamer1.0-plugins-good",
        "h264parse" | "vah264enc" | "openh264enc" | "fdkaacenc" | "voaacenc" | "faac" => {
            "gstreamer1.0-plugins-bad"
        }
        "x264enc" => "gstreamer1.0-plugins-ugly",
        "avenc_aac" => "gstreamer1.0-libav",
        "vaapih264enc" => "gstreamer1.0-vaapi",
        _ => "gstreamer1.0-plugins-base",
    };
    format!("缺少 GStreamer 插件 {factory}，请安装 {package}。")
}

/// 视频编码要求偶数宽高；奇数边补一列 / 一行黑色，与 Windows 的 NV12 补边一致。
fn even(value: u32) -> u32 {
    value.saturating_add(1) & !1
}

/// BGRA 帧复制为补边后的 BGRx 缓冲；alpha 不参与编码。
fn padded_pixels(image: &BgraImage, width: u32, height: u32) -> Vec<u8> {
    let (source_width, target_width) = (image.width as usize * 4, width as usize * 4);
    let mut data = vec![0u8; target_width * height as usize];
    for (row, line) in image.data.chunks_exact(source_width).enumerate() {
        data[row * target_width..row * target_width + source_width].copy_from_slice(line);
    }
    data
}

fn ns(ticks: i64) -> gst::ClockTime {
    gst::ClockTime::from_nseconds(u64::try_from(ticks).unwrap_or(0).saturating_mul(100))
}

fn message_error(message: &gst::Message) -> Option<String> {
    match message.view() {
        gst::MessageView::Error(error) => Some(format!(
            "{}{}",
            error.error(),
            error
                .src()
                .map(|source| format!("（{}）", source.name()))
                .unwrap_or_default()
        )),
        _ => None,
    }
}

pub struct GStreamer {
    pipeline: gst::Pipeline,
    video: gst_app::AppSrc,
    audio: Option<gst_app::AppSrc>,
    kind: EncoderKind,
    width: u32,
    height: u32,
    padded: (u32, u32),
    frame: i64,
    /// 尚未写出的最新一帧及其时间戳；下一帧到达或结束时确定时长。
    held: Option<(gst::Buffer, i64)>,
    last_pts: Option<i64>,
    audio_end: Option<i64>,
    converted: u64,
}

impl GStreamer {
    pub fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        preference: EncodingPreference,
        audio: bool,
    ) -> Result<Self, String> {
        init()?;
        let location = path.to_str().ok_or("录屏文件路径编码无效。")?;
        let candidate = choose(preference).ok_or(
            "未找到 H.264 编码插件，请安装 gstreamer1.0-plugins-ugly（x264）或 gstreamer1.0-plugins-bad（OpenH264）。",
        )?;
        let padded = (even(width), even(height));
        let fps = fps.max(1);
        let pipeline = gst::Pipeline::builder().build();

        let video_caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRx")
            .field("width", padded.0 as i32)
            .field("height", padded.1 as i32)
            .field("framerate", gst::Fraction::new(fps as i32, 1))
            .build();
        let frame_bytes = u64::from(padded.0) * u64::from(padded.1) * 4;
        // 最多排队 3 帧：编码跟不上时阻塞写入线程，由上游队列水位统计丢帧，而不是无限占用内存。
        let source = gst::ElementFactory::make("appsrc")
            .property("caps", video_caps)
            .property("format", gst::Format::Time)
            .property("block", true)
            .property("max-bytes", frame_bytes * 3)
            .build()
            .map_err(|_| missing("appsrc"))?;
        let convert = make("videoconvert")?;
        convert.set_property_from_str("n-threads", "0");
        let raw = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-raw")
                    .field("format", candidate.format)
                    .build(),
            )
            .build()
            .map_err(|_| missing("capsfilter"))?;
        let encoder = make(candidate.factory)?;
        let rate = if candidate.kbps {
            (bitrate / 1000).max(1)
        } else {
            bitrate.max(1)
        };
        encoder.set_property_from_str("bitrate", &rate.to_string());
        if candidate.factory == "x264enc" {
            // 录屏优先实时性：veryfast 在画质与 CPU 占用之间折中，两秒一个关键帧便于拖动进度。
            encoder.set_property_from_str("speed-preset", "veryfast");
            encoder.set_property_from_str("key-int-max", &(fps * 2).to_string());
        }
        let mux = make("mp4mux")?;
        let sink = gst::ElementFactory::make("filesink")
            .property("location", location)
            .build()
            .map_err(|_| missing("filesink"))?;
        let mut video_chain = vec![source.clone(), convert, raw, encoder];
        // OpenH264 只输出 byte-stream，需要解析器转换为 MP4 所需的 avc 格式。
        if let Ok(parse) = gst::ElementFactory::make("h264parse").build() {
            video_chain.push(parse);
        } else if candidate.factory == "openh264enc" {
            return Err(missing("h264parse"));
        }
        video_chain.push(queue()?);
        pipeline
            .add_many(video_chain.iter().chain([&mux, &sink]))
            .map_err(|e| e.to_string())?;
        gst::Element::link_many(video_chain.iter().chain([&mux, &sink]))
            .map_err(|e| format!("无法连接视频编码管线：{e}"))?;

        let audio_source = if audio {
            let factory = AAC.iter().copied().find(|f| available(f)).ok_or(
                "未找到 AAC 编码插件，请安装 gstreamer1.0-libav（avenc_aac）或 gstreamer1.0-plugins-bad。",
            )?;
            let caps = gst::Caps::builder("audio/x-raw")
                .field("format", "S16LE")
                .field("layout", "interleaved")
                .field("rate", super::super::audio::RATE as i32)
                .field("channels", super::super::audio::CHANNELS as i32)
                .build();
            let source = gst::ElementFactory::make("appsrc")
                .property("caps", caps)
                .property("format", gst::Format::Time)
                .property("block", true)
                .property("max-bytes", 1_000_000u64)
                .build()
                .map_err(|_| missing("appsrc"))?;
            let encoder = make(factory)?;
            encoder.set_property_from_str("bitrate", "192000");
            let mut chain = vec![source.clone(), make("audioconvert")?, encoder];
            if let Ok(parse) = gst::ElementFactory::make("aacparse").build() {
                chain.push(parse);
            }
            chain.push(queue()?);
            pipeline.add_many(chain.iter()).map_err(|e| e.to_string())?;
            gst::Element::link_many(chain.iter().chain([&mux]))
                .map_err(|e| format!("无法连接音频编码管线：{e}"))?;
            Some(downcast(source)?)
        } else {
            None
        };

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| format!("无法启动 {} 编码管线。", candidate.factory))?;
        let encoder = Self {
            video: downcast(source)?,
            audio: audio_source,
            pipeline,
            kind: candidate.kind,
            width,
            height,
            padded,
            frame: 10_000_000 / i64::from(fps),
            held: None,
            last_pts: None,
            audio_end: None,
            converted: 0,
        };
        encoder.check()?;
        Ok(encoder)
    }

    /// 管线线程报告的错误（驱动初始化失败、磁盘写满等）在下一次调用时返回。
    fn check(&self) -> Result<(), String> {
        let Some(bus) = self.pipeline.bus() else {
            return Ok(());
        };
        match bus
            .pop_filtered(&[gst::MessageType::Error])
            .and_then(|message| message_error(&message))
        {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn push_video(&self, mut buffer: gst::Buffer, pts: i64, end: i64) -> Result<(), String> {
        {
            let buffer = buffer.make_mut();
            buffer.set_pts(ns(pts));
            buffer.set_duration(ns((end - pts).max(1)));
        }
        if self.video.push_buffer(buffer).is_err() {
            self.check()?;
            return Err("视频编码管线已停止。".into());
        }
        self.check()
    }
}

fn queue() -> Result<gst::Element, String> {
    // 两路之间允许 5 秒偏差：封装器按时间交织，一路暂时没有数据时另一路先在此排队。
    gst::ElementFactory::make("queue")
        .property("max-size-buffers", 0u32)
        .property("max-size-bytes", 0u32)
        .property("max-size-time", 5_000_000_000u64)
        .build()
        .map_err(|_| missing("queue"))
}

fn downcast(element: gst::Element) -> Result<gst_app::AppSrc, String> {
    element
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| "appsrc 类型不符。".to_string())
}

impl Encoder for GStreamer {
    fn kind(&self) -> EncoderKind {
        self.kind
    }
    fn converted_bytes(&self) -> u64 {
        self.converted
    }
    fn push(&mut self, image: &BgraImage, pts: i64) -> Result<bool, String> {
        if image.width != self.width
            || image.height != self.height
            || image.data.len() != self.width as usize * self.height as usize * 4
        {
            return Err("视频帧尺寸与编码器不一致。".into());
        }
        if self.last_pts.is_some_and(|last| pts <= last) {
            return Ok(false);
        }
        let data = padded_pixels(image, self.padded.0, self.padded.1);
        let bytes = data.len() as u64;
        if let Some((held, held_pts)) = self.held.take() {
            self.push_video(held, held_pts, pts)?;
        }
        self.held = Some((gst::Buffer::from_mut_slice(data), pts));
        self.last_pts = Some(pts);
        self.converted += bytes;
        Ok(true)
    }
    fn advance_video(&mut self, end: i64) -> Result<(), String> {
        let Some((held, held_pts)) = self.held.as_ref() else {
            return Ok(());
        };
        if end - held_pts < REPEAT {
            return Ok(());
        }
        // 浅拷贝只复制元数据，像素内存由两个缓冲共享。
        let repeat = held.copy();
        let (held, held_pts) = self.held.take().ok_or("视频帧状态异常。")?;
        self.push_video(held, held_pts, end)?;
        self.held = Some((repeat, end));
        self.last_pts = Some(end);
        Ok(())
    }
    fn push_audio(&mut self, packet: &AudioPacket) -> Result<(), String> {
        let Some(audio) = self.audio.as_ref() else {
            return Err("本次录制未启用 AAC 音轨。".into());
        };
        if self.audio_end.is_some_and(|end| end != packet.pts)
            || packet.duration <= 0
            || packet.samples.is_empty()
            || packet.samples.len() % 2 != 0
        {
            return Err("AAC 输入时间戳不连续或 PCM 数据无效。".into());
        }
        self.audio_end = Some(packet.pts + packet.duration);
        let bytes = packet
            .samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<u8>>();
        let mut buffer = gst::Buffer::from_mut_slice(bytes);
        {
            let buffer = buffer.make_mut();
            buffer.set_pts(ns(packet.pts));
            buffer.set_duration(ns(packet.duration));
        }
        if audio.push_buffer(buffer).is_err() {
            self.check()?;
            return Err("音频编码管线已停止。".into());
        }
        self.check()
    }
    fn finish(&mut self, end: i64) -> Result<(), String> {
        if let Some((held, held_pts)) = self.held.take() {
            self.push_video(held, held_pts, end.max(held_pts + self.frame))?;
        }
        let _ = self.video.end_of_stream();
        if let Some(audio) = self.audio.as_ref() {
            let _ = audio.end_of_stream();
        }
        let bus = self.pipeline.bus().ok_or("编码管线没有消息总线。")?;
        let result = match bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(FINISH_SECONDS),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) {
            Some(message) => match message_error(&message) {
                Some(error) => Err(error),
                None => Ok(()),
            },
            None => Err("写入 MP4 超时，请检查磁盘空间后重试。".into()),
        };
        let _ = self.pipeline.set_state(gst::State::Null);
        result
    }
}

impl Drop for GStreamer {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_frames_are_padded_with_black_to_even_sizes() {
        assert_eq!((even(3), even(4), even(1)), (4, 4, 2));
        let image = BgraImage {
            width: 1,
            height: 1,
            data: vec![1, 2, 3, 255],
        };
        let data = padded_pixels(&image, 2, 2);
        assert_eq!(data.len(), 16);
        assert_eq!(&data[..4], &[1, 2, 3, 255]);
        assert!(data[4..].iter().all(|value| *value == 0));
    }

    #[test]
    fn ticks_convert_to_nanoseconds_without_going_negative() {
        assert_eq!(ns(12).nseconds(), 1200);
        assert_eq!(ns(-5).nseconds(), 0);
    }

    #[test]
    fn missing_plugins_name_the_package_to_install() {
        assert!(missing("x264enc").contains("gstreamer1.0-plugins-ugly"));
        assert!(missing("avenc_aac").contains("gstreamer1.0-libav"));
        assert!(missing("mp4mux").contains("gstreamer1.0-plugins-good"));
    }
}
