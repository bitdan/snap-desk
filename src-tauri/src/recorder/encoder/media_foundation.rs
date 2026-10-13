//! Windows Media Foundation Sink Writer：NV12 → 系统 H.264 编码器 → MP4。
//! 所有 COM 对象只在创建它们的后台线程使用，先释放 writer，再关闭 MF 和 COM。
use super::{
    pixels::{nv12_into, nv12_len},
    BgraImage, EncoderKind, EncodingPreference,
};
use std::{marker::PhantomData, mem::MaybeUninit, path::Path, rc::Rc};
use windows::{
    core::{Interface, Result as WinResult, GUID, HSTRING},
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

pub struct MediaFoundation {
    // Rust 按声明顺序释放字段：writer 必须比 runtime 先释放。
    writer: IMFSinkWriter,
    stream: u32,
    preference: EncodingPreference,
    input_size: u32,
    converted_bytes: u64,
    original: (u32, u32),
    /// 每帧拥有独立 MF 缓冲；待下一帧到达后才确定持续时间，不借用 CPU 图像。
    pending: Option<(i64, IMFMediaBuffer)>,
    // 包括 pending 在内的全部 COM 对象均先于 MF / COM 运行时释放。
    _runtime: Runtime,
}

/// 即使转换失败或发生 panic，也先解锁再释放缓冲。显式解锁失败由调用者上报。
struct BufferLock<'a> {
    buffer: &'a IMFMediaBuffer,
    locked: bool,
}
impl BufferLock<'_> {
    fn unlock(mut self) -> WinResult<()> {
        self.locked = false;
        unsafe { self.buffer.Unlock() }
    }
}
impl Drop for BufferLock<'_> {
    fn drop(&mut self) {
        if self.locked {
            unsafe {
                let _ = self.buffer.Unlock();
            }
        }
    }
}

/// 分配后直接转换到 MF 独占内存；提交以后绝不再次写入或归还到 CPU 缓冲池。
/// Sink Writer 可以异步持有 sample/buffer 的 COM 引用，WriteSample 返回不表示已消费完。
fn converted_buffer(image: &BgraImage) -> Result<IMFMediaBuffer, String> {
    let len = nv12_len(image.width, image.height)?;
    if len > isize::MAX as usize {
        return Err("NV12 缓冲超过当前进程寻址范围。".into());
    }
    let length = u32::try_from(len).map_err(|_| "NV12 缓冲超过系统编码器容量。")?;
    unsafe {
        let buffer = MFCreateMemoryBuffer(length).map_err(native_error)?;
        let mut pointer = std::ptr::null_mut();
        let mut capacity = 0;
        buffer
            .Lock(&mut pointer, Some(&mut capacity), None)
            .map_err(native_error)?;
        let lock = BufferLock {
            buffer: &buffer,
            locked: true,
        };
        if pointer.is_null() || capacity < length {
            return Err("系统编码缓冲容量不足。".into());
        }
        {
            // MF 内存可能尚未初始化；使用 MaybeUninit 而不是先构造 &[u8]，
            // 转换函数只写不读，并覆盖全部 Y/UV（包括奇数尺寸补边），免去预清零。
            let data = std::slice::from_raw_parts_mut(pointer.cast::<MaybeUninit<u8>>(), len);
            nv12_into(image, data)?;
        }
        lock.unlock().map_err(native_error)?;
        buffer.SetCurrentLength(length).map_err(native_error)?;
        Ok(buffer)
    }
}

impl MediaFoundation {
    pub fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        preference: EncodingPreference,
    ) -> WinResult<Self> {
        let runtime = Runtime::new()?;
        let original = (width, height);
        let width = (width + 1) & !1;
        let height = (height + 1) & !1;
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2)?;
            let attributes = attributes.unwrap();
            // 采集线程负责节奏；写入线程不按播放速度限流，拥堵由有界队列丢帧。
            attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            attributes.SetUINT32(
                &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
                u32::from(preference == EncodingPreference::Auto),
            )?;
            let writer =
                MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attributes)?;
            let output = media_type(width, height, fps, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            let stream = writer.AddStream(&output)?;
            let input = media_type(width, height, fps, &MFVideoFormat_NV12)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width)?;
            input.SetUINT32(&MF_MT_SAMPLE_SIZE, width * height * 3 / 2)?;
            writer.SetInputMediaType(stream, &input, None)?;
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                _runtime: runtime,
                stream,
                preference,
                input_size: width * height * 3 / 2,
                converted_bytes: 0,
                original,
                pending: None,
            })
        }
    }

    fn write(&self, buffer: &IMFMediaBuffer, time: i64, duration: i64) -> WinResult<()> {
        unsafe {
            let sample = MFCreateSample()?;
            sample.AddBuffer(buffer)?;
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
    fps: u32,
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
    media.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps) << 32) | 1)?;
    media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
    media.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT601.0 as u32)?;
    media.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
    Ok(media)
}

impl super::Encoder for MediaFoundation {
    fn converted_bytes(&self) -> u64 {
        self.converted_bytes
    }

    fn kind(&self) -> EncoderKind {
        if self.preference == EncodingPreference::Software {
            return EncoderKind::Software;
        }
        // 开启硬件许可不等于实际使用硬件；无法枚举或未提供标识时不猜测。
        unsafe {
            if let Ok(writer) = self.writer.cast::<IMFSinkWriterEx>() {
                for index in 0..16 {
                    let mut category = GUID::zeroed();
                    let mut transform = None;
                    if writer
                        .GetTransformForStream(
                            self.stream,
                            index,
                            Some(&mut category),
                            &mut transform,
                        )
                        .is_err()
                    {
                        break;
                    }
                    if category == MFT_CATEGORY_VIDEO_ENCODER {
                        if let Some(transform) = transform {
                            if let Ok(attributes) = transform.GetAttributes() {
                                if attributes
                                    .GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute)
                                    .is_ok_and(|length| length > 0)
                                {
                                    return EncoderKind::Hardware;
                                }
                            }
                        }
                    }
                }
            }
        }
        EncoderKind::System
    }

    fn push(&mut self, image: &BgraImage, pts: i64) -> Result<(), String> {
        if image.dimensions() != self.original {
            return Err("编码帧尺寸发生变化。".into());
        }
        if let Some((time, buffer)) = self.pending.take() {
            if pts <= time {
                self.pending = Some((time, buffer));
                return Err("编码时间戳必须严格递增。".into());
            }
            self.write(&buffer, time, pts - time)
                .map_err(native_error)?;
            self.pending = Some((pts, converted_buffer(image)?));
        } else {
            // 首帧覆盖开始到首次采集之间的短间隔，输出时长与有效录制时间一致。
            self.pending = Some((0, converted_buffer(image)?));
        }
        self.converted_bytes += u64::from(self.input_size);
        Ok(())
    }

    fn finish(&mut self, end: i64) -> Result<(), String> {
        let (time, data) = self
            .pending
            .take()
            .ok_or("录制时间过短，没有可编码画面。")?;
        self.write(&data, time, (end - time).max(1))
            .map_err(native_error)?;
        MediaFoundation::finish(self).map_err(native_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(buffer: &IMFMediaBuffer) -> Vec<u8> {
        unsafe {
            let mut pointer = std::ptr::null_mut();
            let mut length = 0;
            buffer.Lock(&mut pointer, None, Some(&mut length)).unwrap();
            let lock = BufferLock {
                buffer,
                locked: true,
            };
            let result = std::slice::from_raw_parts(pointer, length as usize).to_vec();
            lock.unlock().unwrap();
            result
        }
    }

    #[test]
    fn later_conversion_does_not_overwrite_retained_mf_buffer() {
        let _runtime = Runtime::new().unwrap();
        let mut image = BgraImage::new(3, 3);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0, 0, 255, 255]);
        }
        let first = converted_buffer(&image).unwrap();
        let expected = bytes(&first);
        assert_eq!(expected.len(), 24);
        // 模拟编码器继续持有上一帧的 COM 引用，采集端已改写并释放原始图像。
        let retained = first.clone();
        drop(first);
        image.data.fill(0);
        let second = converted_buffer(&image).unwrap();
        drop(image);
        assert_eq!(bytes(&retained), expected);
        assert_ne!(bytes(&retained), bytes(&second));
        assert_eq!(&bytes(&second)[..16], &[16; 16]);
        assert_eq!(&bytes(&second)[16..], &[128; 8]);
    }
}
