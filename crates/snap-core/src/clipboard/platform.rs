//! 平台监听线程只发送有界唤醒通知；读剪贴板、编码和 SQL 均在工作线程。
//! Windows 使用剪贴板消息窗口，macOS 轮询 NSPasteboard 计数，Linux（X11）订阅 XFixes 选区通知。
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod linux;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub use linux::*;
