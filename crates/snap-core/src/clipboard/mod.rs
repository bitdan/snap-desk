//! 剪贴板历史的存储层：配置与 IPC 模型、文本分类、文件引用校验和 SQLite 仓库。
//! 系统剪贴板监听、写回与图片文件仓库仍在 `src-tauri/src/clipboard`。
pub mod classification;
pub mod files;
pub mod model;
pub mod repository;
#[cfg(windows)]
pub mod windows_image;
