//! 平台监听线程只发送有界唤醒通知；读剪贴板、编码和 SQL 均在工作线程。
//! Windows 使用剪贴板消息窗口，macOS 轮询 NSPasteboard 计数，其他平台暂不支持。
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub struct Watcher;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl Watcher {
    pub fn start(_: std::sync::mpsc::SyncSender<()>) -> Result<Self, String> {
        Err("当前平台尚未支持自动记录剪贴板。".into())
    }
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn sequence() -> u32 {
    0
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn write_files(_: &[String]) -> Result<(), String> {
    Err("当前平台尚未支持复制文件引用。".into())
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn foreground_window() -> usize {
    0
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn activate(_: usize) -> Result<(), String> {
    Err("当前平台尚未支持直接粘贴。".into())
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn send_paste(_: usize) -> Result<(), String> {
    Err("当前平台尚未支持直接粘贴。".into())
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn snapshot(
    _: &super::model::ClipboardSettings,
) -> Result<Option<super::reader::Snapshot>, String> {
    Ok(None)
}
