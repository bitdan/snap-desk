//! Windows Media Foundation Sink Writer：NV12 → 系统 H.264 编码器 → MP4。
//! 所有 COM 对象只在创建它们的后台线程使用，先释放 writer，再关闭 MF 和 COM。
use super::nv12;
use std::{marker::PhantomData, path::Path, rc::Rc};
use windows::{
    core::{Result as WinResult, HSTRING},
    Win32::{Media::MediaFoundation::*, System::Com::*},
};

fn native_error(error: windows::core::Error) -> String {
    format!("Windows 系统视频编码失败：{error}。请检查系统媒体组件、所选分辨率及磁盘空间；Windows N 版本需启用 Media Feature Pack。")
}

/// 配对管理线程 COM 初始化以及进程 MF 引用计数，任何提前返回都会正确清理。
struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
    fn new() -> WinResult<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(error);
            }
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

struct Encoder {
    // Rust 按声明顺序释放字段：writer 必须比 runtime 先释放。
    writer: IMFSinkWriter,
    _runtime: Runtime,
    stream: u32,
    width: u32,
    height: u32,
}

impl Encoder {
    fn new(path: &Path, width: u32, height: u32) -> WinResult<Self> {
        let runtime = Runtime::new()?;
        let width = (width + 1) & !1;
        let height = (height + 1) & !1;
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.unwrap();
            // 离线编码不按播放速度限流，取消请求在相邻帧之间响应。
            attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            let writer =
                MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attributes)?;
            let output = media_type(width, height, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, 8_000_000)?;
            let stream = writer.AddStream(&output)?;
            let input = media_type(width, height, &MFVideoFormat_NV12)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width)?;
            input.SetUINT32(&MF_MT_SAMPLE_SIZE, width * height * 3 / 2)?;
            writer.SetInputMediaType(stream, &input, None)?;
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                _runtime: runtime,
                stream,
                width,
                height,
            })
        }
    }

    fn write(&self, data: &[u8], time: i64, duration: i64) -> WinResult<()> {
        debug_assert_eq!(data.len(), (self.width * self.height * 3 / 2) as usize);
        unsafe {
            let buffer = MFCreateMemoryBuffer(data.len() as u32)?;
            let mut pointer = std::ptr::null_mut();
            buffer.Lock(&mut pointer, None, None)?;
            // 容量由上面的 data.len() 指定，锁定期间复制等长字节，不保存裸指针。
            std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(data.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            self.writer.WriteSample(self.stream, &sample)
        }
    }

    fn finish(&self) -> WinResult<()> {
        unsafe { self.writer.Finalize() }
    }
}

/// MFSetAttributeSize / Ratio 是 SDK 内联函数；其等价编码为高 32 位 / 低 32 位。
unsafe fn media_type(
    width: u32,
    height: u32,
    subtype: &windows::core::GUID,
) -> WinResult<IMFMediaType> {
    let media = MFCreateMediaType()?;
    media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
    media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    media.SetUINT64(
        &MF_MT_FRAME_SIZE,
        (u64::from(width) << 32) | u64::from(height),
    )?;
    media.SetUINT64(&MF_MT_FRAME_RATE, (10u64 << 32) | 1)?;
    media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
    media.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT601.0 as u32)?;
    media.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
    Ok(media)
}

/// 按实际选区尺寸试写一帧，确认系统编码器支持；失败时保持原截图选区。
pub fn probe(root: &Path, width: u32, height: u32) -> Result<(), String> {
    let temp = tempfile::Builder::new()
        .prefix("probe-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let encoder =
        Encoder::new(&temp.path().join("probe.mp4"), width, height).map_err(native_error)?;
    let plane = (encoder.width * encoder.height) as usize;
    let mut black = vec![128; plane + plane / 2];
    black[..plane].fill(16);
    encoder.write(&black, 0, 1_000_000).map_err(native_error)?;
    encoder.finish().map_err(native_error)
}

/// 逐帧加载并提交到系统 API。取消在帧间检查；同步 WriteSample / Finalize 不强杀。
pub fn encode(
    output: &Path,
    frames: &Path,
    samples: &[(i64, i64)],
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    let mut encoder: Option<Encoder> = None;
    let mut original_size = None;
    for (index, &(time, duration)) in samples.iter().enumerate() {
        if cancelled() {
            return Err("已取消录屏。".into());
        }
        let image = image::open(frames.join(format!("{index:06}.png")))
            .map_err(|e| e.to_string())?
            .to_rgba8();
        if original_size.is_some_and(|size| size != image.dimensions()) {
            return Err("录屏缓存帧尺寸不一致。".into());
        }
        original_size = Some(image.dimensions());
        if encoder.is_none() {
            encoder =
                Some(Encoder::new(output, image.width(), image.height()).map_err(native_error)?);
        }
        let (_, _, data) = nv12(&image);
        encoder
            .as_ref()
            .unwrap()
            .write(&data, time, duration)
            .map_err(native_error)?;
    }
    if cancelled() {
        return Err("已取消录屏。".into());
    }
    encoder
        .as_ref()
        .ok_or("没有可编码的帧。")?
        .finish()
        .map_err(native_error)
}
