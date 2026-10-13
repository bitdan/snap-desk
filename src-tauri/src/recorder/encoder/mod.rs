//! 平台编码器独占写入线程。Drop 即取消，临时文件由管理器清理；finish 才完成封装。
use std::path::Path;
#[cfg(target_os = "windows")]
mod media_foundation;
mod pixels;

pub trait Encoder {
    fn push(&mut self, image: &image::RgbaImage, pts: i64) -> Result<(), String>;
    fn finish(&mut self, end: i64) -> Result<(), String>;
}

pub fn create(path: &Path, width: u32, height: u32, fps: u32) -> Result<Box<dyn Encoder>, String> {
    #[cfg(target_os = "windows")]
    return Ok(Box::new(
        media_foundation::MediaFoundation::new(path, width, height, fps).map_err(|e| {
            format!("无法初始化系统 H.264 编码器：{e}。请检查媒体组件、分辨率和磁盘空间。")
        })?,
    ));
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, width, height, fps);
        Err("UnsupportedPlatform：当前平台尚未实现原生录屏编码。".into())
    }
}

/// 准备前试写并封装一帧；不以 API 存在就宣称所选尺寸可编码。
pub fn probe(root: &Path, width: u32, height: u32, fps: u32) -> Result<(), String> {
    let temp = tempfile::tempdir_in(root).map_err(|e| e.to_string())?;
    let mut encoder = create(&temp.path().join("probe.mp4"), width, height, fps)?;
    encoder.push(&image::RgbaImage::new(width, height), 0)?;
    encoder.finish(10_000_000 / i64::from(fps))
}
