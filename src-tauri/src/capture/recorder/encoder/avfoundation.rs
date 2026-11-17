//! macOS MP4 编码：AVAssetWriter + H.264（系统 VideoToolbox 自动选择硬件或软件）。
//!
//! 时间戳沿用流水线的 100 ns 单位，直接作为 CMTime（timescale = 10_000_000）传入，不做重采样。
//! 声音：混音器输出的 48 kHz 双声道 PCM16 包装成 CMSampleBuffer，交给 AAC 轨道编码。
use super::{AudioPacket, BgraImage, Encoder, EncoderKind};
use crate::shared::macos::{ns_string, rust_string, CMTime, Id, StreamDescription};
use objc2::{
    class,
    encode::{Encode, Encoding},
    msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, Bool},
};
use std::{ffi::c_void, path::Path, time::Duration};

#[repr(transparent)]
#[derive(Clone, Copy)]
struct CVPixelBufferRef(*mut c_void);
unsafe impl Encode for CVPixelBufferRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct CMSampleBufferRef(*mut c_void);
unsafe impl Encode for CMSampleBufferRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("opaqueCMSampleBuffer", &[]));
}

/// 'lpcm'；kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked。
const LINEAR_PCM: u32 = 0x6C70_636D;
const PCM16_FLAGS: u32 = 4 | 8;
/// 'aac '（kAudioFormatMPEG4AAC）。
const AAC: u32 = 0x6161_6320;
const AUDIO_BITRATE: i64 = 192_000;

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMAudioFormatDescriptionCreate(
        allocator: *const c_void,
        description: *const StreamDescription,
        layout_size: usize,
        layout: *const c_void,
        cookie_size: usize,
        cookie: *const c_void,
        extensions: *const c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn CMBlockBufferCreateWithMemoryBlock(
        allocator: *const c_void,
        memory: *mut c_void,
        length: usize,
        block_allocator: *const c_void,
        custom_source: *const c_void,
        offset: usize,
        data_length: usize,
        flags: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn CMBlockBufferReplaceDataBytes(
        source: *const c_void,
        buffer: *mut c_void,
        offset: usize,
        length: usize,
    ) -> i32;
    fn CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: *const c_void,
        data: *mut c_void,
        format: *mut c_void,
        samples: isize,
        pts: CMTime,
        packets: *const c_void,
        out: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        format: u32,
        attributes: *const c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
    fn CVPixelBufferRelease(buffer: *mut c_void);
}

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    static AVFileTypeMPEG4: Id;
    static AVMediaTypeVideo: Id;
    static AVVideoCodecKey: Id;
    static AVVideoCodecTypeH264: Id;
    static AVVideoWidthKey: Id;
    static AVVideoHeightKey: Id;
    static AVVideoCompressionPropertiesKey: Id;
    static AVVideoAverageBitRateKey: Id;
    static AVVideoExpectedSourceFrameRateKey: Id;
    static AVVideoMaxKeyFrameIntervalKey: Id;
    static AVMediaTypeAudio: Id;
}

#[link(name = "AVFAudio", kind = "framework")]
extern "C" {
    static AVFormatIDKey: Id;
    static AVSampleRateKey: Id;
    static AVNumberOfChannelsKey: Id;
    static AVEncoderBitRateKey: Id;
}

/// kCVPixelFormatType_32BGRA（'BGRA'），与流水线的帧格式一致，无需转换 NV12。
const BGRA: u32 = 0x4247_5241;
/// AVAssetWriterStatus：1 写入中，2 已完成。
const WRITING: isize = 1;
const COMPLETED: isize = 2;

pub struct AvFoundation {
    writer: Retained<AnyObject>,
    input: Retained<AnyObject>,
    adaptor: Retained<AnyObject>,
    /// AAC 轨道与其 PCM16 输入格式描述（CF 对象，Drop 时释放）。
    audio: Option<(Retained<AnyObject>, *mut c_void)>,
    width: u32,
    height: u32,
    /// 最近写入的视频帧（已保留引用）；静态画面时重复提交它推进视频轨，避免音轨等待视频而停滞。
    last_buffer: *mut c_void,
    /// 下一个音频包应有的起点，用于检查混音器输出连续。
    audio_end: Option<i64>,
    started: bool,
    last_pts: Option<i64>,
    finished: bool,
    converted: u64,
}

unsafe fn number(value: i64) -> Id {
    msg_send![class!(NSNumber), numberWithLongLong: value]
}

/// 混音器输出的交错 PCM16：48 kHz、双声道、每帧 4 字节。
fn pcm16_description() -> StreamDescription {
    StreamDescription {
        sample_rate: f64::from(super::super::audio::RATE),
        format_id: LINEAR_PCM,
        format_flags: PCM16_FLAGS,
        bytes_per_packet: 4,
        frames_per_packet: 1,
        bytes_per_frame: 4,
        channels_per_frame: 2,
        bits_per_channel: 16,
        reserved: 0,
    }
}

/// 创建 AAC 轨道并加入写入器；必须在 startWriting 之前调用。
unsafe fn audio_input(writer: &AnyObject) -> Result<(Retained<AnyObject>, *mut c_void), String> {
    let settings = dictionary(&[
        (AVFormatIDKey, number(i64::from(AAC))),
        (AVSampleRateKey, number(i64::from(super::super::audio::RATE))),
        (AVNumberOfChannelsKey, number(2)),
        (AVEncoderBitRateKey, number(AUDIO_BITRATE)),
    ]);
    let input: Option<Retained<AnyObject>> = msg_send![
        class!(AVAssetWriterInput),
        assetWriterInputWithMediaType: AVMediaTypeAudio,
        outputSettings: settings
    ];
    let input = input.ok_or("无法创建 AAC 音频轨道。")?;
    let _: () = msg_send![&*input, setExpectsMediaDataInRealTime: Bool::YES];
    let can_add: Bool = msg_send![writer, canAddInput: &*input];
    if !can_add.as_bool() {
        return Err("系统 AAC 编码器不支持 48 kHz 双声道。".into());
    }
    let _: () = msg_send![writer, addInput: &*input];
    let description = pcm16_description();
    let mut format: *mut c_void = std::ptr::null_mut();
    let status = CMAudioFormatDescriptionCreate(
        std::ptr::null(),
        &description,
        0,
        std::ptr::null(),
        0,
        std::ptr::null(),
        std::ptr::null(),
        &mut format,
    );
    if status != 0 || format.is_null() {
        return Err("无法创建音频格式描述。".into());
    }
    Ok((input, format))
}

unsafe fn dictionary(entries: &[(Id, Id)]) -> Id {
    let keys: Vec<Id> = entries.iter().map(|(k, _)| *k).collect();
    let values: Vec<Id> = entries.iter().map(|(_, v)| *v).collect();
    msg_send![
        class!(NSDictionary),
        dictionaryWithObjects: values.as_ptr(),
        forKeys: keys.as_ptr(),
        count: keys.len()
    ]
}

/// 写入器的系统错误描述（可能为空）。
unsafe fn writer_error(writer: &AnyObject) -> String {
    let error: Id = msg_send![writer, error];
    if error.is_null() {
        return "未知错误".into();
    }
    let text: Id = msg_send![error, localizedDescription];
    rust_string(text).unwrap_or_else(|| "未知错误".into())
}

impl AvFoundation {
    pub fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        audio: bool,
    ) -> Result<Self, String> {
        if width < 2 || height < 2 {
            return Err("录制尺寸无效。".into());
        }
        let path = path.to_str().ok_or("录制路径包含无法识别的字符。")?;
        objc2::rc::autoreleasepool(|_| unsafe {
            let url: Id = msg_send![class!(NSURL), fileURLWithPath: ns_string(path)];
            let mut error: Id = std::ptr::null_mut();
            let allocated: Allocated<AnyObject> = msg_send![class!(AVAssetWriter), alloc];
            let writer: Option<Retained<AnyObject>> = msg_send![
                allocated,
                initWithURL: url,
                fileType: AVFileTypeMPEG4,
                error: &mut error as *mut Id
            ];
            let writer = writer.ok_or("无法创建 MP4 写入器，请检查保存目录。")?;
            let fps = i64::from(fps.max(1));
            let compression = dictionary(&[
                (AVVideoAverageBitRateKey, number(i64::from(bitrate))),
                (AVVideoExpectedSourceFrameRateKey, number(fps)),
                // 每两秒一个关键帧，拖动进度条时定位更快。
                (AVVideoMaxKeyFrameIntervalKey, number(fps * 2)),
            ]);
            let settings = dictionary(&[
                (AVVideoCodecKey, AVVideoCodecTypeH264),
                (AVVideoWidthKey, number(i64::from(width))),
                (AVVideoHeightKey, number(i64::from(height))),
                (AVVideoCompressionPropertiesKey, compression),
            ]);
            let input: Option<Retained<AnyObject>> = msg_send![
                class!(AVAssetWriterInput),
                assetWriterInputWithMediaType: AVMediaTypeVideo,
                outputSettings: settings
            ];
            let input = input.ok_or("无法创建 H.264 视频轨道。")?;
            // 实时来源：编码跟不上时宁可丢帧，也不阻塞采集线程。
            let _: () = msg_send![&*input, setExpectsMediaDataInRealTime: Bool::YES];
            let can_add: Bool = msg_send![&*writer, canAddInput: &*input];
            if !can_add.as_bool() {
                return Err("所选分辨率或码率不受 H.264 编码器支持。".into());
            }
            let _: () = msg_send![&*writer, addInput: &*input];
            let attributes: Id = std::ptr::null_mut();
            let adaptor: Option<Retained<AnyObject>> = msg_send![
                class!(AVAssetWriterInputPixelBufferAdaptor),
                assetWriterInputPixelBufferAdaptorWithAssetWriterInput: &*input,
                sourcePixelBufferAttributes: attributes
            ];
            let adaptor = adaptor.ok_or("无法创建视频帧写入器。")?;
            let audio = if audio { Some(audio_input(&writer)?) } else { None };
            let started: Bool = msg_send![&*writer, startWriting];
            if !started.as_bool() {
                return Err(format!("无法开始写入 MP4：{}", writer_error(&writer)));
            }
            Ok(Self {
                writer,
                input,
                adaptor,
                audio,
                width,
                height,
                last_buffer: std::ptr::null_mut(),
                audio_end: None,
                started: false,
                last_pts: None,
                finished: false,
                converted: 0,
            })
        })
    }

    /// 新建 CVPixelBuffer 并逐行复制：系统缓冲的行宽可能带对齐填充。
    unsafe fn pixel_buffer(&self, image: &BgraImage) -> Result<*mut c_void, String> {
        let mut buffer: *mut c_void = std::ptr::null_mut();
        let status = CVPixelBufferCreate(
            std::ptr::null(),
            self.width as usize,
            self.height as usize,
            BGRA,
            std::ptr::null(),
            &mut buffer,
        );
        if status != 0 || buffer.is_null() {
            return Err("无法分配视频帧缓冲。".into());
        }
        CVPixelBufferLockBaseAddress(buffer, 0);
        let base = CVPixelBufferGetBaseAddress(buffer) as *mut u8;
        let stride = CVPixelBufferGetBytesPerRow(buffer);
        let row = self.width as usize * 4;
        if !base.is_null() && stride >= row {
            for y in 0..self.height as usize {
                std::ptr::copy_nonoverlapping(
                    image.data.as_ptr().add(y * row),
                    base.add(y * stride),
                    row,
                );
            }
        }
        CVPixelBufferUnlockBaseAddress(buffer, 0);
        if base.is_null() || stride < row {
            CVPixelBufferRelease(buffer);
            return Err("视频帧缓冲不可写。".into());
        }
        Ok(buffer)
    }
}

impl AvFoundation {
    /// 会话从第一份媒体数据（视频或音频）开始；更早的样本会被写入器裁掉。
    unsafe fn begin(&mut self, pts: i64) {
        if !self.started {
            let _: () = msg_send![&*self.writer, startSessionAtSourceTime: CMTime::from_ticks(pts)];
            self.started = true;
        }
    }

    /// 等待输入就绪，最多约 `attempts × 5 ms`。
    unsafe fn ready(input: &AnyObject, attempts: u32) -> bool {
        for attempt in 0..=attempts {
            let ready: Bool = msg_send![input, isReadyForMoreMediaData];
            if ready.as_bool() {
                return true;
            }
            if attempt < attempts {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        false
    }

    /// 提交像素缓冲；成功后保留为最近一帧（接管调用方的引用）。
    unsafe fn append(&mut self, buffer: *mut c_void, pts: i64) -> Result<(), String> {
        let appended: Bool = msg_send![
            &*self.adaptor,
            appendPixelBuffer: CVPixelBufferRef(buffer),
            withPresentationTime: CMTime::from_ticks(pts)
        ];
        if !appended.as_bool() {
            if buffer != self.last_buffer {
                CVPixelBufferRelease(buffer);
            }
            return Err(format!("写入视频帧失败：{}", writer_error(&self.writer)));
        }
        if buffer != self.last_buffer {
            if !self.last_buffer.is_null() {
                CVPixelBufferRelease(self.last_buffer);
            }
            self.last_buffer = buffer;
        }
        self.last_pts = Some(pts);
        Ok(())
    }
}

impl Encoder for AvFoundation {
    fn kind(&self) -> EncoderKind {
        EncoderKind::System
    }
    fn converted_bytes(&self) -> u64 {
        self.converted
    }
    fn push(&mut self, image: &BgraImage, pts: i64) -> Result<bool, String> {
        if image.width != self.width
            || image.height != self.height
            || image.data.len() < self.width as usize * self.height as usize * 4
        {
            return Err("视频帧尺寸与编码器不一致。".into());
        }
        if self.last_pts.is_some_and(|last| pts <= last) {
            return Ok(false);
        }
        objc2::rc::autoreleasepool(|_| unsafe {
            self.begin(pts);
            // 编码器短暂忙碌时最多等 50 ms，仍未就绪则丢弃这一帧，由下一帧延续画面。
            if !Self::ready(&self.input, 10) {
                return Ok(false);
            }
            let buffer = self.pixel_buffer(image)?;
            self.append(buffer, pts)?;
            self.converted += image.data.len() as u64;
            Ok(true)
        })
    }
    fn advance_video(&mut self, end: i64) -> Result<(), String> {
        // 每 100 ms 重复一次最近一帧；只读复用同一缓冲，无需再次复制像素。
        let Some(last) = self.last_pts else {
            return Ok(());
        };
        if self.last_buffer.is_null() || end - last < 1_000_000 {
            return Ok(());
        }
        objc2::rc::autoreleasepool(|_| unsafe {
            if Self::ready(&self.input, 0) {
                self.append(self.last_buffer, end)?;
            }
            Ok(())
        })
    }
    fn push_audio(&mut self, packet: &AudioPacket) -> Result<(), String> {
        let Some((input, format)) = self.audio.as_ref().map(|(i, f)| (i.clone(), *f)) else {
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
        objc2::rc::autoreleasepool(|_| unsafe {
            self.begin(packet.pts);
            // 混音器每包 10 ms；写入器为交织等待视频时最多等 200 ms，仍不就绪则丢弃这一包。
            if !Self::ready(&input, 40) {
                log::warn!("event=recording.audio_dropped module=recording reason=writer_busy");
                return Ok(());
            }
            let length = packet.samples.len() * 2;
            let mut block: *mut c_void = std::ptr::null_mut();
            // kCMBlockBufferAssureMemoryNowFlag：立即分配，随后复制 PCM。
            if CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null(),
                std::ptr::null_mut(),
                length,
                std::ptr::null(),
                std::ptr::null(),
                0,
                length,
                1,
                &mut block,
            ) != 0
                || block.is_null()
            {
                return Err("无法分配音频缓冲。".into());
            }
            let copied = CMBlockBufferReplaceDataBytes(
                packet.samples.as_ptr().cast(),
                block,
                0,
                length,
            );
            let mut sample: *mut c_void = std::ptr::null_mut();
            let status = if copied == 0 {
                CMAudioSampleBufferCreateReadyWithPacketDescriptions(
                    std::ptr::null(),
                    block,
                    format,
                    (packet.samples.len() / 2) as isize,
                    CMTime::from_ticks(packet.pts),
                    std::ptr::null(),
                    &mut sample,
                )
            } else {
                copied
            };
            CFRelease(block);
            if status != 0 || sample.is_null() {
                return Err("无法创建音频样本。".into());
            }
            let appended: Bool = msg_send![&*input, appendSampleBuffer: CMSampleBufferRef(sample)];
            CFRelease(sample);
            if !appended.as_bool() {
                return Err(format!("写入音频失败：{}", writer_error(&self.writer)));
            }
            Ok(())
        })
    }
    fn finish(&mut self, end: i64) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        objc2::rc::autoreleasepool(|_| unsafe {
            let _: () = msg_send![&*self.input, markAsFinished];
            if let Some((audio, _)) = &self.audio {
                let _: () = msg_send![&**audio, markAsFinished];
            }
            if self.started {
                let end = end.max(self.last_pts.unwrap_or(0) + 1);
                let _: () = msg_send![&*self.writer, endSessionAtSourceTime: CMTime::from_ticks(end)];
            }
            // 同步完成封装；本函数已在编码线程上运行，无需完成回调（避免引入 block 绑定）。
            let done: Bool = msg_send![&*self.writer, finishWriting];
            let status: isize = msg_send![&*self.writer, status];
            if !done.as_bool() || status != COMPLETED {
                return Err(format!("完成 MP4 封装失败：{}", writer_error(&self.writer)));
            }
            Ok(())
        })
    }
}

impl Drop for AvFoundation {
    /// 未完成即放弃：取消写入，由管理器删除临时文件。
    fn drop(&mut self) {
        unsafe {
            if !self.last_buffer.is_null() {
                CVPixelBufferRelease(self.last_buffer);
            }
            if let Some((_, format)) = &self.audio {
                CFRelease(*format);
            }
        }
        if !self.finished {
            unsafe {
                let status: isize = msg_send![&*self.writer, status];
                if status == WRITING {
                    let _: () = msg_send![&*self.writer, cancelWriting];
                }
            }
        }
    }
}
