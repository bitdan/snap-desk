//! Windows 专属消息线程只发送有界唤醒通知；读剪贴板、编码和 SQL 均在工作线程。
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(not(target_os = "windows"))]
pub struct Watcher;
#[cfg(not(target_os = "windows"))]
impl Watcher {
    pub fn start(_: std::sync::mpsc::SyncSender<()>) -> Result<Self, String> {
        Err("当前平台尚未支持自动记录剪贴板。".into())
    }
}
#[cfg(not(target_os = "windows"))]
pub fn sequence() -> u32 {
    0
}
#[cfg(not(target_os = "windows"))]
pub fn snapshot(
    _: &super::model::ClipboardSettings,
) -> Result<Option<super::reader::Snapshot>, String> {
    Ok(None)
}
