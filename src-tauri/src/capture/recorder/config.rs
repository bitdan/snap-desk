//! 录屏配置只表达当前已支持的能力；Windows MP4 可选系统声音与麦克风。
use crate::capture::Rect;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSource {
    PrimaryMonitor,
    SelectedMonitor,
    SelectedWindow,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Mp4,
    Gif,
}

impl OutputFormat {
    /// 扩展名来自格式白名单。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Gif => "gif",
        }
    }
}

/// 自动模式允许系统选择硬件编码器；软件模式用于驱动兼容性排查。
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EncodingPreference {
    #[default]
    Auto,
    Software,
}

/// 按分辨率、帧率和质量估算目标码率，不保证输出恒定码率。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingQuality {
    Economy,
    #[default]
    Balanced,
    High,
}

/// 按物理分辨率、帧率和画质预设估算编码预算；录制与裁剪共用同一尺度。
pub(super) fn estimated_bitrate(width: u32, height: u32, fps: u32, quality: RecordingQuality) -> u32 {
    let weight = match quality {
        RecordingQuality::Economy => 8,
        RecordingQuality::Balanced => 13,
        RecordingQuality::High => 20,
    };
    (u64::from(width) * u64::from(height) * u64::from(fps) * weight / 100)
        .clamp(1_000_000, 80_000_000) as u32
}

/// 默认关闭；所有样式采用输出视频物理像素，颜色为 RGB，避免前端传任意样式表达式。
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct InputEffects {
    pub keyboard: bool,
    pub clicks: bool,
    pub movement: bool,
    pub drag: bool,
    pub wheel: bool,
    pub size: u32,
    pub duration_ms: u32,
    pub position: KeyPosition,
    pub left_color: [u8; 3],
    pub right_color: [u8; 3],
    pub double_color: [u8; 3],
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyPosition {
    TopLeft,
    BottomLeft,
    BottomRight,
}

impl Default for InputEffects {
    fn default() -> Self {
        Self {
            keyboard: false,
            clicks: false,
            movement: false,
            drag: false,
            wheel: false,
            size: 20,
            duration_ms: 400,
            position: KeyPosition::TopLeft,
            left_color: [64, 160, 255],
            right_color: [255, 112, 80],
            double_color: [190, 100, 255],
        }
    }
}
impl InputEffects {
    pub fn enabled(&self) -> bool {
        self.keyboard || self.clicks || self.movement || self.drag || self.wheel
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(12..=64).contains(&self.size) || !(200..=2000).contains(&self.duration_ms) {
            return Err("键鼠特效尺寸需为 12–64 像素，持续时间需为 200–2000 毫秒。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecorderConfig {
    pub source: CaptureSource,
    /// IPC 使用虚拟桌面选区；准备完成后转换为来源局部物理像素。
    pub region: Rect,
    pub fps: u32,
    #[serde(default)]
    pub quality: RecordingQuality,
    /// 自定义目标码率，单位 bit/s；缺省或 null 沿用画质预设，兼容旧客户端。
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub encoding: EncodingPreference,
    pub capture_cursor: bool,
    #[serde(default)]
    pub input_effects: InputEffects,
    /// 点击开始后的倒计时，与截图自身的延时设置相互独立；旧调用默认立即开始。
    #[serde(default)]
    pub countdown_seconds: u8,
    pub format: OutputFormat,
    pub system_audio: bool,
    pub microphone: bool,
    /// 本次会话开始/恢复时是否隐藏主界面；旧配置保持原有的自动收起行为。
    #[serde(default = "default_hide_main_window")]
    pub hide_main_window: bool,
}

fn default_hide_main_window() -> bool {
    true
}

impl RecorderConfig {
    pub fn for_region(region: Rect) -> Self {
        Self {
            source: CaptureSource::PrimaryMonitor,
            region,
            fps: 60,
            quality: RecordingQuality::Balanced,
            bitrate: None,
            encoding: EncodingPreference::Auto,
            capture_cursor: true,
            input_effects: InputEffects::default(),
            countdown_seconds: 0,
            format: OutputFormat::Mp4,
            system_audio: false,
            microphone: false,
            hide_main_window: true,
        }
    }

    /// 自定义值优先；自动模式按 0.08/0.13/0.20 bit/pixel/frame 计算，限制在 1–80 Mbps。
    /// 调用前由 validate 拒绝越界配置，不静默修改用户指定的码率。
    pub fn target_bitrate(&self) -> u32 {
        if matches!(self.format, OutputFormat::Gif) {
            return 0;
        }
        if let Some(bitrate) = self.bitrate {
            return bitrate;
        }
        estimated_bitrate(self.region.width, self.region.height, self.fps, self.quality)
    }

    pub fn validate(&self, size: (u32, u32)) -> Result<(), String> {
        self.input_effects.validate()?;
        if self
            .bitrate
            .is_some_and(|value| !(1_000_000..=80_000_000).contains(&value))
        {
            return Err("自定义录屏码率需为 1–80 Mbps。".into());
        }
        let r = self.region;
        if !matches!(self.countdown_seconds, 0 | 3 | 5) {
            return Err("录屏倒计时仅支持立即、3 秒或 5 秒。".into());
        }
        if !match self.format {
            OutputFormat::Mp4 => matches!(self.fps, 30 | 60),
            OutputFormat::Gif => matches!(self.fps, 10 | 20),
        } {
            return Err("MP4 支持 30 / 60 FPS，GIF 支持 10 / 20 FPS。".into());
        }
        if (self.system_audio || self.microphone)
            && (matches!(self.format, OutputFormat::Gif) || !cfg!(target_os = "windows"))
        {
            return Err("音频录制仅支持 Windows MP4，GIF 不支持声音。".into());
        }
        if matches!(self.format, OutputFormat::Gif) && (r.width > 65535 || r.height > 65535) {
            return Err("GIF 单边不能超过 65535 像素。".into());
        }
        if r.width < 2
            || r.height < 2
            || u64::from(r.width) * u64::from(r.height) > 16_000_000
            || r.x.checked_add(r.width).is_none_or(|v| v > size.0)
            || r.y.checked_add(r.height).is_none_or(|v| v > size.1)
        {
            return Err("录屏选区需在来源范围内，至少 2×2，最多 1600 万像素。".into());
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
    fn main_window_preference_keeps_explicit_false_and_defaults_legacy_configs() {
        let config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        });
        let mut json = serde_json::to_value(config).unwrap();
        json["hideMainWindow"] = serde_json::json!(false);
        assert!(
            !serde_json::from_value::<RecorderConfig>(json.clone())
                .unwrap()
                .hide_main_window
        );
        json.as_object_mut().unwrap().remove("hideMainWindow");
        assert!(
            serde_json::from_value::<RecorderConfig>(json)
                .unwrap()
                .hide_main_window
        );
    }
    #[test]
    fn source_contract_keeps_legacy_and_accepts_only_known_sources() {
        for source in ["primaryMonitor", "selectedMonitor", "selectedWindow"] {
            let mut json = serde_json::to_value(RecorderConfig::for_region(Rect {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
            }))
            .unwrap();
            json["source"] = serde_json::json!(source);
            let config: RecorderConfig = serde_json::from_value(json).unwrap();
            assert!(config.validate((1920, 1080)).is_ok());
            assert_eq!(serde_json::to_value(config).unwrap()["source"], source);
        }
        assert!(serde_json::from_str::<CaptureSource>(r#""arbitraryWindowHandle""#).is_err());
    }
    #[test]
    fn input_effects_default_compatibility_and_validation() {
        let config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        let mut json = serde_json::to_value(config).unwrap();
        json.as_object_mut().unwrap().remove("inputEffects");
        let restored: RecorderConfig = serde_json::from_value(json).unwrap();
        assert!(!restored.input_effects.enabled());
        let effects: InputEffects = serde_json::from_str(r#"{"keyboard":true}"#).unwrap();
        assert!(effects.keyboard && !effects.clicks);
        // 与前端 defaultInputEffects 保持一致；旧配置缺字段时使用当前默认样式。
        assert_eq!(effects.size, 20);
        assert_eq!(effects.duration_ms, 400);
        assert!(matches!(effects.position, KeyPosition::TopLeft));
        assert!(effects.validate().is_ok());
        for (size, duration_ms) in [(0, 700), (65, 700), (28, 199), (28, 2001)] {
            assert!(InputEffects {
                size,
                duration_ms,
                ..effects
            }
            .validate()
            .is_err());
        }
        assert!(serde_json::from_str::<InputEffects>(r#"{"showText":true}"#).is_err());
        assert!(serde_json::from_str::<InputEffects>(r#"{"leftColor":[999,0,0]}"#).is_err());
    }
    #[test]
    fn custom_bitrate_validation_and_legacy_compatibility() {
        let mut config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        });
        for bitrate in [1_000_000, 8_500_000, 80_000_000] {
            config.bitrate = Some(bitrate);
            assert!(config.validate((1920, 1080)).is_ok());
            assert_eq!(config.target_bitrate(), bitrate);
        }
        // 改帧率和画质不会覆盖显式配置；恢复自动才重新应用预设。
        config.fps = 60;
        config.quality = RecordingQuality::High;
        assert_eq!(config.target_bitrate(), 80_000_000);
        for bitrate in [0, 999_999, 80_000_001, u32::MAX] {
            config.bitrate = Some(bitrate);
            assert!(config.validate((1920, 1080)).is_err());
        }
        config.bitrate = None;
        assert_eq!(config.target_bitrate(), 24_883_200);
        let mut json = serde_json::to_value(config).unwrap();
        assert!(serde_json::from_value::<RecorderConfig>(json.clone())
            .unwrap()
            .bitrate
            .is_none());
        json.as_object_mut().unwrap().remove("bitrate");
        let legacy: RecorderConfig = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(legacy.target_bitrate(), 24_883_200);
        // IPC 不接受负数、小数 bit/s 或字符串，不能绕过 Option<u32> 契约。
        for invalid in [
            serde_json::json!(-1),
            serde_json::json!(1_000_000.5),
            serde_json::json!("8000000"),
        ] {
            json["bitrate"] = invalid;
            assert!(serde_json::from_value::<RecorderConfig>(json.clone()).is_err());
        }
    }

    #[test]
    fn bitrate_scales_with_size_rate_and_quality() {
        let mut config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        });
        // 此场景比较 30/60 FPS，不依赖 for_region 当前的默认帧率。
        config.fps = 30;
        assert_eq!(config.target_bitrate(), 8_087_040);
        config.fps = 60;
        assert_eq!(config.target_bitrate(), 16_174_080);
        config.quality = RecordingQuality::High;
        assert_eq!(config.target_bitrate(), 24_883_200);
        config.region.width = 3840;
        config.region.height = 2160;
        assert_eq!(config.target_bitrate(), 80_000_000);
        config.region.width = 2;
        config.region.height = 2;
        assert_eq!(config.target_bitrate(), 1_000_000);
    }

    #[test]
    fn validates_platform_audio_bad_rates_and_region_overflow() {
        let mut config = RecorderConfig::for_region(Rect {
            x: 0,
            y: 0,
            width: 101,
            height: 99,
        });
        assert!(config.validate((1920, 1080)).is_ok());
        for delay in [0, 3, 5] {
            config.countdown_seconds = delay;
            assert!(config.validate((1920, 1080)).is_ok());
        }
        config.countdown_seconds = 2;
        assert!(config.validate((1920, 1080)).is_err());
        let mut old = serde_json::to_value(config).unwrap();
        old.as_object_mut().unwrap().remove("countdownSeconds");
        assert_eq!(
            serde_json::from_value::<RecorderConfig>(old)
                .unwrap()
                .countdown_seconds,
            0
        );
        let mut old = serde_json::to_value(config).unwrap();
        old.as_object_mut().unwrap().remove("encoding");
        assert_eq!(
            serde_json::from_value::<RecorderConfig>(old)
                .unwrap()
                .encoding,
            EncodingPreference::Auto
        );
        config.countdown_seconds = 0;
        let mut old = serde_json::to_value(config).unwrap();
        old.as_object_mut().unwrap().remove("quality");
        assert!(matches!(
            serde_json::from_value::<RecorderConfig>(old)
                .unwrap()
                .quality,
            RecordingQuality::Balanced
        ));
        config.system_audio = true;
        assert_eq!(
            config.validate((1920, 1080)).is_ok(),
            cfg!(target_os = "windows")
        );
        config.microphone = true;
        assert_eq!(
            config.validate((1920, 1080)).is_ok(),
            cfg!(target_os = "windows")
        );
        config.format = OutputFormat::Gif;
        config.fps = 10;
        assert!(config.validate((1920, 1080)).unwrap_err().contains("GIF"));
        config.format = OutputFormat::Mp4;
        config.microphone = false;
        config.system_audio = false;
        config.fps = 10;
        assert!(config.validate((1920, 1080)).is_err());
        config.fps = 60;
        config.region.x = u32::MAX;
        assert!(config.validate((1920, 1080)).is_err());
    }
}
