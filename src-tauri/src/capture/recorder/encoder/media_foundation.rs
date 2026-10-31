//! Windows Media Foundation Sink Writer：NV12 → H.264，PCM16 → AAC，共用 MP4 时间轴。
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
    format!("Windows 系统音视频编码失败：{error}。请检查系统媒体组件、所选分辨率及磁盘空间；Windows N 版本需启用 Media Feature Pack。")
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
    audio_stream: Option<u32>,
    audio_end: i64,
    preference: EncodingPreference,
    input_size: u32,
    converted_bytes: u64,
    original: (u32, u32),
    /// 100 ns；静止桌面复用像素，但仍按目标帧率提交短样本。
    frame_duration: i64,
    /// 区分乱序采集帧与因静态补帧而稍晚到达的有效帧。
    source_time: Option<i64>,
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
        audio: bool,
    ) -> WinResult<Self> {
        let runtime = Runtime::new()?;
        let original = (width, height);
        let width = (width + 1) & !1;
        let height = (height + 1) & !1;
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 3)?;
            let attributes = attributes.unwrap();
            // 临时输出路径不一定有扩展名；显式声明 MP4 容器，避免系统根据文件名猜测格式。
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
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
            let audio_stream = if audio {
                let output = audio_type(&MFAudioFormat_AAC)?;
                // AAC-LC，48 kHz 双声道、160 kbit/s；MP4 使用 raw AAC payload。
                output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 20_000)?;
                output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
                output.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)?;
                let index = writer.AddStream(&output)?;
                let input = audio_type(&MFAudioFormat_PCM)?;
                input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
                input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, super::super::audio::RATE * 4)?;
                writer.SetInputMediaType(index, &input, None)?;
                Some(index)
            } else { None };
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                _runtime: runtime,
                stream,
                audio_stream,
                audio_end: 0,
                preference,
                input_size: width * height * 3 / 2,
                converted_bytes: 0,
                original,
                frame_duration: (10_000_000 / i64::from(fps.max(1))).max(1),
                source_time: None,
                pending: None,
            })
        }
    }

    fn write(&self, buffer: &IMFMediaBuffer, time: i64, duration: i64) -> WinResult<()> {
        // WGC 仅在桌面变化时给帧；不能把几秒静止区间作为一个 H.264 样本。
        // 复用只读 NV12 缓冲，补齐媒体时间轴，无需重新转换或复制整幅图像。
        let mut offset = 0;
        while offset < duration {
            // 合并不足 1ms 的尾数，避免 MP4 时间刻度把尾帧舍入成零时长。
            let remaining = duration - offset;
            let span = if remaining <= self.frame_duration + 10_000 {
                remaining
            } else {
                self.frame_duration
            };
            unsafe {
                let sample = MFCreateSample()?;
                sample.AddBuffer(buffer)?;
                sample.SetSampleTime(time + offset)?;
                sample.SetSampleDuration(span)?;
                self.writer.WriteSample(self.stream, &sample)?;
            }
            offset += span;
        }
        Ok(())
    }

    fn finish(&self) -> WinResult<()> {
        unsafe { self.writer.Finalize() }
    }
}

/// PCM 与 AAC 的公共格式；系统 AAC 编码器要求 16-bit 输入。
unsafe fn audio_type(subtype: &GUID) -> WinResult<IMFMediaType> {
    let media = MFCreateMediaType()?;
    media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
    media.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, super::super::audio::RATE)?;
    media.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
    media.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    Ok(media)
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
    fn advance_video(&mut self, end: i64) -> Result<(), String> {
        if let Some((time, buffer)) = self.pending.take() {
            // 每 100 ms 推进一次，再按目标帧率提交短样本；MF 只读持有缓冲，无须重复转换。
            // 留下同一缓冲作为后续静态区间底图，同时避免音轨堆积和停止时集中补帧。
            if end - time >= 1_000_000 {
                self.write(&buffer, time, end - time).map_err(native_error)?;
                self.pending = Some((end, buffer));
            } else {
                self.pending = Some((time, buffer));
            }
        }
        Ok(())
    }

    fn push_audio(&mut self, packet: &super::AudioPacket) -> Result<(), String> {
        let stream = self.audio_stream.ok_or("本次录制未启用 AAC 音轨。")?;
        if packet.pts != self.audio_end || packet.duration <= 0
            || packet.samples.is_empty() || packet.samples.len() % 2 != 0
        {
            return Err("AAC 输入时间戳不连续或 PCM 数据无效。".into());
        }
        let length = u32::try_from(packet.samples.len() * 2).map_err(|_| "音频包过大。")?;
        unsafe {
            let buffer = MFCreateMemoryBuffer(length).map_err(native_error)?;
            let mut pointer = std::ptr::null_mut();
            let mut capacity = 0;
            buffer.Lock(&mut pointer, Some(&mut capacity), None).map_err(native_error)?;
            let lock = BufferLock { buffer: &buffer, locked: true };
            if pointer.is_null() || capacity < length {
                return Err("系统音频编码缓冲容量不足。".into());
            }
            for (index, value) in packet.samples.iter().enumerate() {
                std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), pointer.add(index * 2), 2);
            }
            lock.unlock().map_err(native_error)?;
            buffer.SetCurrentLength(length).map_err(native_error)?;
            let sample = MFCreateSample().map_err(native_error)?;
            sample.AddBuffer(&buffer).map_err(native_error)?;
            sample.SetSampleTime(packet.pts).map_err(native_error)?;
            sample.SetSampleDuration(packet.duration).map_err(native_error)?;
            self.writer.WriteSample(stream, &sample).map_err(native_error)?;
        }
        self.audio_end = packet.pts + packet.duration;
        Ok(())
    }

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
        if self.source_time.is_some_and(|time| pts <= time) {
            return Err("编码时间戳必须严格递增。".into());
        }
        let converted = converted_buffer(image)?;
        if let Some((time, buffer)) = self.pending.take() {
            if pts <= time {
                // 有声/无声录屏都可能先推进静止画面；有效新帧从尚未提交处生效。
                self.pending = Some((time, converted));
            } else {
                self.write(&buffer, time, pts - time)
                    .map_err(native_error)?;
                self.pending = Some((pts, converted));
            }
        } else {
            // 首帧覆盖开始到首次采集之间的短间隔，输出时长与有效录制时间一致。
            self.pending = Some((0, converted));
        }
        self.source_time = Some(pts);
        self.converted_bytes += u64::from(self.input_size);
        Ok(())
    }

    fn finish(&mut self, end: i64) -> Result<(), String> {
        let (time, data) = self
            .pending
            .take()
            .ok_or("录制时间过短，没有可编码画面。")?;
        // 音频泵可能已把静态视频恰好提交到停止时间，此时 pending 只是复用底图。
        // 不再追加 1 个 100 ns 的样本：MP4 时间刻度会将其舍入成零时长帧。
        if end > time || time == 0 {
            self.write(&data, time, (end - time).max(1))
                .map_err(native_error)?;
        }
        MediaFoundation::finish(self).map_err(native_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 重现短录屏只有几帧的输入：成品从零时刻可解码，静止区间保持短样本。
    #[test]
    fn sparse_silent_capture_has_visible_first_frame_and_bounded_sample_durations() {
        use super::super::Encoder;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sparse.mp4");
        let mut encoder = MediaFoundation::new(
            &path, 64, 64, 30, 1_000_000, EncodingPreference::Software, false,
        ).unwrap();
        let mut image = BgraImage::new(64, 64);
        image.data.fill(255);
        encoder.push(&image, 0).unwrap();
        encoder.advance_video(2_000_000).unwrap();
        // 模拟静止补帧刚推进后，采集队列中稍早的有效帧到达；不应报乱序错误。
        encoder.push(&image, 1_900_000).unwrap();
        assert!(encoder.push(&image, 1_800_000).is_err());
        encoder.push(&image, 50_000_000).unwrap();
        Encoder::finish(&mut encoder, 51_000_000).unwrap();
        drop(encoder);

        let _runtime = Runtime::new().unwrap();
        unsafe {
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None).unwrap();
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            let decoded = MFCreateMediaType().unwrap();
            decoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            decoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).unwrap();
            reader.SetCurrentMediaType(stream, None, &decoded).unwrap();
            let (mut count, mut end) = (0, 0);
            loop {
                let (mut flags, mut pts, mut sample) = (0, 0, None);
                reader.ReadSample(stream, 0, None, Some(&mut flags), Some(&mut pts), Some(&mut sample)).unwrap();
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 { break; }
                if let Some(sample) = sample {
                    if count == 0 {
                        assert_eq!(pts, 0);
                        let pixels = bytes(&sample.ConvertToContiguousBuffer().unwrap());
                        assert!(pixels[..64 * 64].iter().all(|value| *value > 200));
                    }
                    let duration = sample.GetSampleDuration().unwrap();
                    assert!(duration > 0 && duration <= 350_000);
                    assert!((pts - end).abs() <= 1_000);
                    end = pts + duration;
                    count += 1;
                }
            }
            assert!(count >= 150);
            assert!((end - 51_000_000).abs() <= 1_000);
        }
    }

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

    /// 合成音频验证真实 AAC/H.264 封装，不访问用户设备；需要 Windows 媒体组件。
    #[test]
    fn extensionless_output_muxes_aac_and_static_video_with_monotonic_timestamps() {
        use super::super::Encoder;
        let root = tempfile::tempdir().unwrap();
        // 临时输出也显式声明容器，不允许依赖 .mp4 后缀才能初始化。
        let path = root.path().join("snapdesk-pending-audio-video");
        assert!(path.extension().is_none());
        let mut encoder = MediaFoundation::new(&path, 64, 64, 30, 1_000_000,
            EncodingPreference::Software, true).unwrap();
        encoder.push(&BgraImage::new(64, 64), 0).unwrap();
        let mut mixer = super::super::super::audio::Mixer::default();
        while let Some(packet) = mixer.next_packet(5_000_000) {
            encoder.advance_video(packet.pts + packet.duration).unwrap();
            encoder.push_audio(&packet).unwrap();
        }
        Encoder::finish(&mut encoder, 5_000_000).unwrap();
        drop(encoder);
        let _runtime = Runtime::new().unwrap();
        unsafe {
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None).unwrap();
            // MP4 读取端的流序号不保证与 Sink Writer 的添加顺序一致；
            // 按媒体类别定位轨道，验证真实输出而不假定视频一定是第 0 轨。
            for (index, expected) in [
                (MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, MFMediaType_Video),
                (MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, MFMediaType_Audio),
            ] {
                assert_eq!(reader.GetNativeMediaType(index, 0).unwrap().GetGUID(&MF_MT_MAJOR_TYPE).unwrap(), expected);
                let (mut previous, mut count) = (None, 0);
                loop {
                    let (mut flags, mut pts, mut sample) = (0, 0, None);
                    reader.ReadSample(index, 0, None, Some(&mut flags), Some(&mut pts), Some(&mut sample)).unwrap();
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 { break; }
                    if let Some(sample) = sample {
                        assert!(previous.is_none_or(|last| pts > last));
                        assert!(sample.GetSampleDuration().unwrap() > 0);
                        previous = Some(pts);
                        count += 1;
                    }
                }
                assert!(count > 1);
            }
        }
    }
}
