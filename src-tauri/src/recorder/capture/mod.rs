//! 平台采集边界；非 Windows 明确拒绝，不以循环截图降级。
use super::{config::RecorderConfig, frame::VideoFrame};

pub trait CaptureBackend {
    /// 返回最新一帧；静态桌面可能暂时没有帧，编码端延长上一帧显示时间。
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String>;
    /// 等待帧到达；超时用于检查停止和设备状态，不无限阻塞控制操作。
    fn wait_for_frame(&self, timeout: std::time::Duration);
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{now, supported};

pub fn prepare(
    config: &RecorderConfig,
    size: (u32, u32),
) -> Result<Box<dyn CaptureBackend>, String> {
    #[cfg(target_os = "windows")]
    return Ok(Box::new(windows::Capture::new(config, size)?));
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (config, size);
        Err("UnsupportedPlatform：macOS / Linux 原生录屏尚未实现。".into())
    }
}

#[cfg(not(target_os = "windows"))]
pub fn supported() -> Result<(), String> {
    Err("UnsupportedPlatform：macOS / Linux 原生录屏尚未实现。".into())
}

#[cfg(not(target_os = "windows"))]
pub fn now() -> i64 {
    0
}
