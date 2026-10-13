//! 录屏配置只表达当前已支持的能力；声音请求必须明确拒绝。
use crate::capture::Rect;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSource {
    PrimaryMonitor,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Mp4,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecorderConfig {
    pub source: CaptureSource,
    /// 相对主屏的物理像素；与截图 Rect 使用相同坐标系。
    pub region: Rect,
    pub fps: u32,
    pub capture_cursor: bool,
    pub format: OutputFormat,
    pub system_audio: bool,
    pub microphone: bool,
}

impl RecorderConfig {
    pub fn for_region(region: Rect) -> Self {
        Self {
            source: CaptureSource::PrimaryMonitor,
            region,
            fps: 30,
            capture_cursor: true,
            format: OutputFormat::Mp4,
            system_audio: false,
            microphone: false,
        }
    }

    pub fn validate(&self, size: (u32, u32)) -> Result<(), String> {
        let r = self.region;
        if !matches!(self.fps, 30 | 60) {
            return Err("录屏帧率仅支持 30 / 60 FPS。".into());
        }
        if self.system_audio || self.microphone {
            return Err("当前版本尚未支持系统声音或麦克风录制。".into());
        }
        if r.width < 2
            || r.height < 2
            || u64::from(r.width) * u64::from(r.height) > 16_000_000
            || r.x.checked_add(r.width).is_none_or(|v| v > size.0)
            || r.y.checked_add(r.height).is_none_or(|v| v > size.1)
        {
            return Err("录屏选区需在主屏内，至少 2×2，最多 1600 万像素。".into());
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderCapabilities {
    pub available: bool,
    pub reason: Option<String>,
    pub sources: Vec<CaptureSource>,
    pub frame_rates: Vec<u32>,
    pub capture_cursor: bool,
    pub system_audio: bool,
    pub microphone: bool,
    pub formats: Vec<OutputFormat>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_audio_bad_rates_and_region_overflow() {
        let mut config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 101,
            height: 99,
        });
        assert!(config.validate((1920, 1080)).is_ok());
        config.system_audio = true;
        assert!(config.validate((1920, 1080)).is_err());
        config.system_audio = false;
        config.fps = 10;
        assert!(config.validate((1920, 1080)).is_err());
        config.fps = 60;
        config.region.x = u32::MAX;
        assert!(config.validate((1920, 1080)).is_err());
    }
}
