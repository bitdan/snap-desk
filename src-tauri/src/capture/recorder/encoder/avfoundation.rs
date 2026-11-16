//! macOS MP4 编码：AVAssetWriter + H.264（系统 VideoToolbox 自动选择硬件或软件）。
//!
//! 时间戳沿用流水线的 100 ns 单位，直接作为 CMTime（timescale = 10_000_000）传入，不做重采样。
//! 首版不录声音；音频轨道在能力中标为不可用。
use super::{BgraImage, Encoder, EncoderKind};
use crate::shared::macos::{ns_string, rust_string, Id};
use objc2::{
    class,
    encode::{Encode, Encoding},
    msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, Bool},
};
use std::{ffi::c_void, path::Path, time::Duration};

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}
// CMTime 是匿名 typedef 结构体，运行时类型编码为 {?=qiIq}。
unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING],
    );
}
impl CMTime {
    /// kCMTimeFlags_Valid = 1。
    fn ticks(value: i64) -> Self {
        Self {
            value,
            timescale: 10_000_000,
            flags: 1,
            epoch: 0,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct CVPixelBufferRef(*mut c_void);
unsafe impl Encode for CVPixelBufferRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
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
    width: u32,
    height: u32,
    started: bool,
    last_pts: Option<i64>,
    finished: bool,
    converted: u64,
}

unsafe fn number(value: i64) -> Id {
    msg_send![class!(NSNumber), numberWithLongLong: value]
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
    pub fn new(path: &Path, width: u32, height: u32, fps: u32, bitrate: u32) -> Result<Self, String> {
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
            let started: Bool = msg_send![&*writer, startWriting];
            if !started.as_bool() {
                return Err(format!("无法开始写入 MP4：{}", writer_error(&writer)));
            }
            Ok(Self {
                writer,
                input,
                adaptor,
                width,
                height,
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
            if !self.started {
                let _: () = msg_send![&*self.writer, startSessionAtSourceTime: CMTime::ticks(pts)];
                self.started = true;
            }
            // 编码器短暂忙碌时最多等 50 ms，仍未就绪则丢弃这一帧，由下一帧延续画面。
            let mut ready: Bool = msg_send![&*self.input, isReadyForMoreMediaData];
            for _ in 0..10 {
                if ready.as_bool() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
                ready = msg_send![&*self.input, isReadyForMoreMediaData];
            }
            if !ready.as_bool() {
                return Ok(false);
            }
            let buffer = self.pixel_buffer(image)?;
            let appended: Bool = msg_send![
                &*self.adaptor,
                appendPixelBuffer: CVPixelBufferRef(buffer),
                withPresentationTime: CMTime::ticks(pts)
            ];
            CVPixelBufferRelease(buffer);
            if !appended.as_bool() {
                return Err(format!("写入视频帧失败：{}", writer_error(&self.writer)));
            }
            self.last_pts = Some(pts);
            self.converted += image.data.len() as u64;
            Ok(true)
        })
    }
    fn finish(&mut self, end: i64) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        objc2::rc::autoreleasepool(|_| unsafe {
            let _: () = msg_send![&*self.input, markAsFinished];
            if self.started {
                let end = end.max(self.last_pts.unwrap_or(0) + 1);
                let _: () = msg_send![&*self.writer, endSessionAtSourceTime: CMTime::ticks(end)];
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
