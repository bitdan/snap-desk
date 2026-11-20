//! 剪贴板历史：配置与 IPC 模型、文本分类、文件引用校验、SQLite 仓库、图片仓库、
//! 系统剪贴板监听/读取/写回（各平台）与记录服务。导出（PDF/长图）与界面命令仍在各外壳。
pub mod blob_store;
pub mod classification;
pub mod files;
pub mod model;
pub mod platform;
pub mod reader;
pub mod repository;
pub mod service;
#[cfg(windows)]
pub mod windows_image;
