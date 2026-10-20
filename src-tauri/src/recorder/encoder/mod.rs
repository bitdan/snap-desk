//! 平台编码器独占写入线程。Drop 即取消，临时文件由管理器清理；finish 才完成封装。
use super::{
    audio::AudioPacket,
    config::{EncodingPreference, OutputFormat},
    frame::BgraImage,
};
use serde::Serialize;
use std::path::Path;
#[cfg(target_os = "windows")]
mod media_foundation;
mod pixels;

/// 只有读取到硬件转换器标识才报告硬件；系统未公开信息时保留未知。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EncoderKind {
    Hardware,
    Software,
    System,
}

pub trait Encoder {
    fn kind(&self) -> EncoderKind;
    /// 成功转换并交给编码器拥有的 NV12 字节数，不计系统编码器内部拷贝。
    fn converted_bytes(&self) -> u64;
    fn push(&mut self, image: &BgraImage, pts: i64) -> Result<(), String>;
    /// 音视频交给同一个 Sink Writer，时间单位均为 100 ns；GIF 不允许声音。
    fn push_audio(&mut self, _: &AudioPacket) -> Result<(), String> {
        Err("当前编码格式不支持音频。".into())
    }
    /// 静态桌面也要推进视频流，否则 MP4 交织器可能长期缓存音频等待下一视频样本。
    fn advance_video(&mut self, _: i64) -> Result<(), String> { Ok(()) }
    fn finish(&mut self, end: i64) -> Result<(), String>;
}

pub fn create(
    path: &Path,
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    preference: EncodingPreference,
    format: OutputFormat,
    audio: bool,
) -> Result<Box<dyn Encoder>, String> {
    if matches!(format, OutputFormat::Gif) {
        if audio { return Err("GIF 不支持音频。".into()); }
        return Ok(Box::new(super::gif::GifEncoder::new(path, width, height)?));
    }
    #[cfg(target_os = "windows")]
    return Ok(Box::new(
        media_foundation::MediaFoundation::new(path, width, height, fps, bitrate, preference, audio)
            .map_err(|e| {
                format!("无法初始化系统 H.264 / AAC 编码器：{e}。请检查媒体组件、分辨率和磁盘空间。")
            })?,
    ));
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, width, height, fps, bitrate, preference, audio);
        Err("UnsupportedPlatform：当前平台尚未实现原生录屏编码。".into())
    }
}

/// 只在准备阶段回退，不在已经接收视频帧后重启编码器，避免丢失已录内容。
fn with_fallback<T>(
    preference: EncodingPreference,
    mut attempt: impl FnMut(EncodingPreference) -> Result<T, String>,
) -> Result<(T, EncodingPreference, Option<String>), String> {
    match attempt(preference) {
        Ok(value) => Ok((value, preference, None)),
        Err(first) if preference == EncodingPreference::Auto => {
            let value = attempt(EncodingPreference::Software).map_err(|second| {
                format!("自动编码试写失败：{first}；软件编码试写也失败：{second}")
            })?;
            Ok((
                value,
                EncodingPreference::Software,
                Some(format!("自动编码试写失败，已切换软件编码：{first}")),
            ))
        }
        Err(error) => Err(error),
    }
}

/// 使用独立临时文件验证所选尺寸的写入和封装；失败释放全部资源后再试软件。
pub fn probe(
    root: &Path,
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    preference: EncodingPreference,
    format: OutputFormat,
    audio: bool,
) -> Result<(EncoderKind, EncodingPreference, Option<String>), String> {
    with_fallback(preference, |mode| {
        let temp = tempfile::tempdir_in(root).map_err(|e| e.to_string())?;
        let mut encoder = create(
            &temp.path().join(format!("probe.{}", format.extension())),
            width,
            height,
            fps,
            bitrate,
            mode,
            format,
            audio,
        )?;
        encoder.push(&BgraImage::new(width, height), 0)?;
        if audio {
            // 试写 AAC 使用合成静音，不打开麦克风或录制系统播放。
            let mut mixer = super::audio::Mixer::default();
            while let Some(packet) = mixer.next_packet(10_000_000 / i64::from(fps)) {
                encoder.push_audio(&packet)?;
            }
        }
        encoder.finish(10_000_000 / i64::from(fps))?;
        Ok(encoder.kind())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_failure_retries_software_and_preserves_reason() {
        let mut modes = Vec::new();
        let (value, mode, reason) = with_fallback(EncodingPreference::Auto, |mode| {
            modes.push(mode);
            if mode == EncodingPreference::Auto {
                Err("driver".into())
            } else {
                Ok(7)
            }
        })
        .unwrap();
        assert_eq!(
            modes,
            [EncodingPreference::Auto, EncodingPreference::Software]
        );
        assert_eq!(value, 7);
        assert_eq!(mode, EncodingPreference::Software);
        assert!(reason.unwrap().contains("driver"));
    }
    #[test]
    fn successful_auto_and_forced_software_do_not_retry() {
        for mode in [EncodingPreference::Auto, EncodingPreference::Software] {
            let mut calls = 0;
            let (_, selected, reason) = with_fallback(mode, |_| {
                calls += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(selected, mode);
            assert!(reason.is_none());
        }
        let mut calls = 0;
        assert!(with_fallback::<()>(EncodingPreference::Software, |_| {
            calls += 1;
            Err("software".into())
        })
        .is_err());
        assert_eq!(calls, 1);
        let error = with_fallback::<()>(EncodingPreference::Auto, |mode| Err(format!("{mode:?}")))
            .unwrap_err();
        assert!(error.contains("Auto") && error.contains("Software"));
    }
}
