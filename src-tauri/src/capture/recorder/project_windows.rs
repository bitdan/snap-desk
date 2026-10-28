//! 流式解码干净 MP4 并重新合成标注；不将整段视频载入内存，声音使用原媒体 PTS 裁剪。
use super::{Layer, RecordingProject};
use crate::capture::recording::{
    audio::AudioPacket,
    config::{EncodingPreference, OutputFormat},
    encoder,
    frame::BgraImage,
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    core::HSTRING,
    Win32::{Media::MediaFoundation::*, System::Com::*},
};
fn error(e: impl std::fmt::Display) -> String {
    format!("录屏项目导出失败：{e}")
}
struct Runtime;
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
/// MF 连续缓冲在复制完成后立即解锁，不把 COM 内存跨采样或跨线程保留。
unsafe fn bytes(sample: &IMFSample) -> Result<Vec<u8>, String> {
    let buffer = sample.ConvertToContiguousBuffer().map_err(error)?;
    let mut ptr = std::ptr::null_mut();
    let mut len = 0;
    buffer.Lock(&mut ptr, None, Some(&mut len)).map_err(error)?;
    if ptr.is_null() || len > 512 * 1024 * 1024 {
        let _ = buffer.Unlock();
        return Err("媒体采样缓冲无效。".into());
    }
    let data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
    buffer.Unlock().map_err(error)?;
    Ok(data)
}
pub(super) fn export(
    input: &Path,
    output: &Path,
    project: &RecordingProject,
    layer: &Layer,
    start: u64,
    end: u64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(error)?;
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            CoUninitialize();
            return Err(error(e));
        }
        let _runtime = Runtime;
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1).map_err(error)?;
        let attrs = attrs.ok_or("媒体解码参数不可用。")?;
        attrs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(error)?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(input.as_os_str()), &attrs)
            .map_err(error)?;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        reader
            .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
            .map_err(error)?;
        reader.SetStreamSelection(video, true).map_err(error)?;
        let mt = MFCreateMediaType().map_err(error)?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(error)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(error)?;
        reader
            .SetCurrentMediaType(video, None, &mt)
            .map_err(error)?;
        let current = reader.GetCurrentMediaType(video).map_err(error)?;
        let size = current.GetUINT64(&MF_MT_FRAME_SIZE).map_err(error)?;
        let width = (size >> 32) as u32;
        let height = size as u32;
        if width < project.width
            || height < project.height
            || width > 16384
            || height > 16384
            || u64::from(width) * u64::from(height) > 67_108_864
        {
            return Err("干净原片尺寸与项目不一致。".into());
        }
        let stride = current
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .unwrap_or(width * 4) as i32;
        let has_audio = reader.GetNativeMediaType(audio, 0).is_ok();
        if has_audio {
            let mt = MFCreateMediaType().map_err(error)?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .map_err(error)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                .map_err(error)?;
            mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2).map_err(error)?;
            mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000)
                .map_err(error)?;
            mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                .map_err(error)?;
            mt.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)
                .map_err(error)?;
            mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192000)
                .map_err(error)?;
            reader
                .SetCurrentMediaType(audio, None, &mt)
                .map_err(error)?;
            reader.SetStreamSelection(audio, true).map_err(error)?;
        }
        let mut writer = encoder::create(
            output,
            project.width,
            project.height,
            project.fps,
            super::super::config::estimated_bitrate(
                project.width,
                project.height,
                project.fps,
                super::super::config::RecordingQuality::High,
            ),
            EncodingPreference::Auto,
            OutputFormat::Mp4,
            has_audio,
        )?;
        let mut clean: Option<BgraImage> = None;
        let mut frame_index = 0u64;
        let mut audio_cursor = 0i64;
        let mut video_end = false;
        let mut audio_end = !has_audio;
        let start_ticks = start as i64 * 10_000;
        let end_ticks = end as i64 * 10_000;
        let mut emit_until = |until: i64,
                              clean: &Option<BgraImage>,
                              writer: &mut dyn encoder::Encoder|
         -> Result<(), String> {
            if let Some(clean) = clean {
                loop {
                    let pts =
                        start_ticks + (frame_index * 10_000_000 / u64::from(project.fps)) as i64;
                    if pts >= until.min(end_ticks) {
                        break;
                    }
                    if cancel.load(Ordering::SeqCst) {
                        return Err("项目导出已取消。".into());
                    }
                    let mut frame = clean.clone();
                    layer.apply(&mut frame, pts)?;
                    writer.push(&frame, pts - start_ticks)?;
                    frame_index += 1;
                }
            }
            Ok(())
        };
        while !video_end || !audio_end {
            if cancel.load(Ordering::SeqCst) {
                return Err("项目导出已取消。".into());
            }
            let mut actual = 0;
            let mut flags = 0;
            let mut pts = 0;
            let mut sample = None;
            reader
                .ReadSample(
                    MF_SOURCE_READER_ANY_STREAM.0 as u32,
                    0,
                    Some(&mut actual),
                    Some(&mut flags),
                    Some(&mut pts),
                    Some(&mut sample),
                )
                .map_err(error)?;
            let major = reader
                .GetCurrentMediaType(actual)
                .map_err(error)?
                .GetGUID(&MF_MT_MAJOR_TYPE)
                .map_err(error)?;
            let is_video = major == MFMediaType_Video;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                if is_video {
                    video_end = true;
                } else {
                    audio_end = true;
                }
                reader.SetStreamSelection(actual, false).map_err(error)?;
                continue;
            }
            if flags
                & (MF_SOURCE_READERF_ERROR.0 as u32
                    | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32)
                != 0
            {
                return Err("原片媒体格式在解码期间发生变化。".into());
            }
            let Some(sample) = sample else {
                continue;
            };
            emit_until(pts, &clean, writer.as_mut())?;
            if pts >= end_ticks {
                break;
            }
            let data = bytes(&sample)?;
            if is_video {
                let row = stride.unsigned_abs() as usize;
                if row < width as usize * 4 || data.len() < row * height as usize {
                    return Err("视频行跨度无效。".into());
                }
                let mut frame = BgraImage::new(project.width, project.height);
                for y in 0..project.height as usize {
                    let source_y = if stride < 0 {
                        height as usize - 1 - y
                    } else {
                        y
                    };
                    frame.data
                        [y * project.width as usize * 4..(y + 1) * project.width as usize * 4]
                        .copy_from_slice(
                            &data[source_y * row..source_y * row + project.width as usize * 4],
                        );
                }
                clean = Some(frame);
            } else {
                if data.len() % 4 != 0 {
                    return Err("音频缓冲不符合 PCM16 双声道格式。".into());
                }
                let frames = data.len() / 4;
                let first = (((start_ticks - pts).max(0) as u128 * 48000).div_ceil(10_000_000)
                    as usize)
                    .min(frames);
                let last = (((end_ticks - pts).max(0) as u128 * 48000) / 10_000_000) as usize;
                let last = last.min(frames);
                if first < last {
                    let audio_pts = super::super::audio::sample_time(audio_cursor);
                    let audio_end =
                        super::super::audio::sample_time(audio_cursor + (last - first) as i64);
                    let packet = AudioPacket {
                        pts: audio_pts,
                        duration: audio_end - audio_pts,
                        samples: data[first * 4..last * 4]
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]))
                            .collect(),
                    };
                    writer.push_audio(&packet)?;
                    audio_cursor += (last - first) as i64;
                }
            }
        }
        emit_until(end_ticks, &clean, writer.as_mut())?;
        if clean.is_none() {
            return Err("原片没有可解码视频帧。".into());
        }
        writer.finish(end_ticks - start_ticks)?;
        Ok(())
    }
}
