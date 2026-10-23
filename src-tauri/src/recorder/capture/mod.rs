//! 平台采集边界；非 Windows 明确拒绝，不以循环截图降级。
use super::{config::RecorderConfig, frame::VideoFrame};

/// 仅由后端从当前截图会话解析，前端不能直接指定 HWND 或进程。
#[derive(Clone, Copy)]
pub enum NativeSource {
    Monitor { x: i32, y: i32 },
    Window { id: u32, pid: u32 },
}

pub trait CaptureBackend {
    /// 当前来源的桌面物理原点；窗口移动后特效也随来源移动。
    fn desktop_origin(&self) -> Result<(i32, i32), String> {
        Ok((0, 0))
    }
    /// 暂停期间也检查来源；窗口最小化、关闭或尺寸变化时明确结束录制。
    fn check_source(&self) -> Result<(), String> {
        Ok(())
    }
    /// 返回最新一帧；静态桌面可能暂时没有帧，编码端延长上一帧显示时间。
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String>;
    /// 等待帧到达；超时用于检查停止和设备状态，不无限阻塞控制操作。
    fn wait_for_frame(&self, timeout: std::time::Duration);
    /// 恢复特效录制时请求一张新底图，静态桌面也不能重放暂停前的捕获帧。
    fn refresh(&mut self) -> Result<(), String>;
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{now, supported};

pub fn prepare(
    config: &RecorderConfig,
    size: (u32, u32),
    source: NativeSource,
) -> Result<Box<dyn CaptureBackend>, String> {
    #[cfg(target_os = "windows")]
    return Ok(Box::new(windows::Capture::new(config, size, source)?));
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (config, size, source);
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
