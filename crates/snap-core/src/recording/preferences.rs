//! 录制偏好（格式、帧率、画质、倒计时、光标、键鼠特效、声音）。
//!
//! 存在设置目录下的 `recording.json`（`{ "version": 1, "preferences": … }`）。
//! 旧的或损坏的字段逐项回退为默认值，不保存选区、窗口身份。
use super::config::{
    EncodingPreference, InputEffects, KeyPosition, OutputFormat, RecordingQuality,
};
use super::RecorderConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub format: OutputFormat,
    pub mp4_fps: u32,
    pub gif_fps: u32,
    pub encoding: EncodingPreference,
    pub quality: RecordingQuality,
    /// 自定义码率（Mbps，1–80）；None 使用画质预设。
    pub mbps: Option<f64>,
    pub countdown: u8,
    pub capture_cursor: bool,
    pub effects: InputEffects,
    pub system_audio: bool,
    pub microphone: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            format: OutputFormat::Mp4,
            mp4_fps: 60,
            gif_fps: 10,
            encoding: EncodingPreference::Auto,
            quality: RecordingQuality::Balanced,
            mbps: None,
            countdown: 0,
            capture_cursor: true,
            effects: InputEffects::default(),
            system_audio: false,
            microphone: false,
        }
    }
}

#[derive(Deserialize)]
struct Stored {
    version: u32,
    preferences: Value,
}

impl Preferences {
    /// 当前格式下的帧率。
    pub fn fps(&self) -> u32 {
        match self.format {
            OutputFormat::Mp4 => self.mp4_fps,
            OutputFormat::Gif => self.gif_fps,
        }
    }

    /// 本平台能否录制声音（Windows / macOS 的 MP4）。
    pub fn audio_supported(&self) -> bool {
        matches!(self.format, OutputFormat::Mp4)
            && cfg!(any(target_os = "windows", target_os = "macos"))
    }

    /// 逐项读取，非法值保留默认。
    pub fn normalize(value: &Value) -> Self {
        let mut result = Self::default();
        let Some(source) = value.as_object() else {
            return result;
        };
        let text = |key: &str| source.get(key).and_then(Value::as_str);
        let flag = |key: &str| source.get(key).and_then(Value::as_bool);
        let integer = |value: Option<&Value>, min: u64, max: u64| {
            value
                .and_then(Value::as_u64)
                .filter(|v| (min..=max).contains(v))
        };
        match text("format") {
            Some("mp4") => result.format = OutputFormat::Mp4,
            Some("gif") => result.format = OutputFormat::Gif,
            _ => {}
        }
        if let Some(fps) = integer(source.get("mp4Fps"), 30, 60).filter(|v| matches!(v, 30 | 60)) {
            result.mp4_fps = fps as u32;
        }
        if let Some(fps) = integer(source.get("gifFps"), 10, 20).filter(|v| matches!(v, 10 | 20)) {
            result.gif_fps = fps as u32;
        }
        match text("encoding") {
            Some("auto") => result.encoding = EncodingPreference::Auto,
            Some("software") => result.encoding = EncodingPreference::Software,
            _ => {}
        }
        match text("quality") {
            Some("economy") => result.quality = RecordingQuality::Economy,
            Some("balanced") => result.quality = RecordingQuality::Balanced,
            Some("high") => result.quality = RecordingQuality::High,
            _ => {}
        }
        if let Some(mbps) = source
            .get("mbps")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && (1.0..=80.0).contains(v))
        {
            result.mbps = Some(mbps);
        }
        if let Some(countdown) =
            integer(source.get("countdown"), 0, 5).filter(|v| matches!(v, 0 | 3 | 5))
        {
            result.countdown = countdown as u8;
        }
        if let Some(value) = flag("captureCursor") {
            result.capture_cursor = value;
        }
        if let Some(value) = flag("systemAudio") {
            result.system_audio = value;
        }
        if let Some(value) = flag("microphone") {
            result.microphone = value;
        }
        if let Some(effects) = source.get("effects").and_then(Value::as_object) {
            let e = &mut result.effects;
            for (key, field) in [
                ("keyboard", &mut e.keyboard),
                ("clicks", &mut e.clicks),
                ("movement", &mut e.movement),
                ("drag", &mut e.drag),
                ("wheel", &mut e.wheel),
            ] {
                if let Some(value) = effects.get(key).and_then(Value::as_bool) {
                    *field = value;
                }
            }
            if let Some(size) = integer(effects.get("size"), 12, 64) {
                e.size = size as u32;
            }
            if let Some(duration) = integer(effects.get("durationMs"), 200, 2000) {
                e.duration_ms = duration as u32;
            }
            match effects.get("position").and_then(Value::as_str) {
                Some("topLeft") => e.position = KeyPosition::TopLeft,
                Some("bottomLeft") => e.position = KeyPosition::BottomLeft,
                Some("bottomRight") => e.position = KeyPosition::BottomRight,
                _ => {}
            }
            for (key, field) in [
                ("leftColor", &mut e.left_color),
                ("rightColor", &mut e.right_color),
                ("doubleColor", &mut e.double_color),
            ] {
                let color = effects
                    .get(key)
                    .and_then(Value::as_array)
                    .filter(|c| c.len() == 3);
                let channels: Option<Vec<u8>> = color.and_then(|c| {
                    c.iter()
                        .map(|v| v.as_u64().filter(|v| *v <= 255).map(|v| v as u8))
                        .collect()
                });
                if let Some(channels) = channels {
                    *field = [channels[0], channels[1], channels[2]];
                }
            }
        }
        result
    }

    /// 读取偏好文件；不存在、版本不符或损坏时返回默认值。
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .filter(|stored| stored.version == 1)
            .map(|stored| Self::normalize(&stored.preferences))
            .unwrap_or_default()
    }

    /// 写入同目录临时文件后替换，失败保持原文件。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("录屏偏好路径无效。")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(
            file.as_file_mut(),
            &serde_json::json!({ "version": 1, "preferences": self }),
        )
        .map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 填入录制配置；音频只在支持时开启，Wayland 下关闭键鼠特效（无法读取全局输入）。
    pub fn apply(&self, config: &mut RecorderConfig) {
        config.format = self.format;
        config.fps = self.fps();
        config.encoding = self.encoding;
        config.quality = self.quality;
        config.bitrate = match self.format {
            OutputFormat::Mp4 => self.mbps.map(|mbps| (mbps * 1_000_000.0).round() as u32),
            OutputFormat::Gif => None,
        };
        config.countdown_seconds = self.countdown;
        config.capture_cursor = self.capture_cursor;
        config.input_effects = self.effects;
        let audio = self.audio_supported();
        config.system_audio = audio && self.system_audio;
        config.microphone = audio && self.microphone;
        if super::wayland_session() {
            let effects = &mut config.input_effects;
            effects.keyboard = false;
            effects.clicks = false;
            effects.movement = false;
            effects.drag = false;
            effects.wheel = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Rect;
    use serde_json::json;

    #[test]
    fn invalid_fields_fall_back_one_by_one() {
        let value = json!({
            "format": "gif", "mp4Fps": 45, "gifFps": 20, "encoding": "fast", "quality": "high",
            "mbps": 120, "countdown": 3, "captureCursor": false,
            "effects": { "keyboard": true, "size": 99, "durationMs": 800, "position": "bottomRight",
                         "leftColor": [1, 2, 3], "rightColor": [1, 2, 300] }
        });
        let p = Preferences::normalize(&value);
        assert!(matches!(p.format, OutputFormat::Gif));
        assert_eq!((p.mp4_fps, p.gif_fps, p.fps()), (60, 20, 20));
        assert_eq!(p.encoding, EncodingPreference::Auto);
        assert!(matches!(p.quality, RecordingQuality::High));
        assert_eq!((p.mbps, p.countdown, p.capture_cursor), (None, 3, false));
        assert!(p.effects.keyboard && !p.effects.clicks);
        assert_eq!((p.effects.size, p.effects.duration_ms), (20, 800));
        assert!(matches!(p.effects.position, KeyPosition::BottomRight));
        assert_eq!(p.effects.left_color, [1, 2, 3]);
        assert_eq!(p.effects.right_color, InputEffects::default().right_color);
        assert_eq!(
            Preferences::normalize(&json!("broken")),
            Preferences::default()
        );
    }

    #[test]
    fn saved_preferences_round_trip_and_apply_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.json");
        assert_eq!(Preferences::load(&path), Preferences::default());
        let preferences = Preferences {
            mp4_fps: 30,
            mbps: Some(12.5),
            countdown: 5,
            system_audio: true,
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), preferences);
        let mut config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        });
        preferences.apply(&mut config);
        assert_eq!(
            (config.fps, config.bitrate, config.countdown_seconds),
            (30, Some(12_500_000), 5)
        );
        assert_eq!(
            config.system_audio,
            cfg!(any(target_os = "windows", target_os = "macos"))
        );
        assert!(config.validate((640, 480)).is_ok());
        // GIF 不带声音和自定义码率。
        let gif = Preferences {
            format: OutputFormat::Gif,
            ..preferences
        };
        gif.apply(&mut config);
        assert_eq!(
            (config.fps, config.bitrate, config.system_audio),
            (10, None, false)
        );
        assert!(config.validate((640, 480)).is_ok());
        std::fs::write(&path, "{\"version\":2,\"preferences\":{}}").unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
    }
}
