//! 多个功能复用的系统设施；平台适配、图像导出和原生窗口支持统一放在这里。
pub(crate) mod annotations;
pub(crate) mod auxiliary_windows;
#[cfg(test)]
pub(crate) mod benchmark;
pub(crate) mod dialog_windows;
pub(crate) mod export_staging;
pub(crate) mod image_io;
pub(crate) mod image_protocol;
pub(crate) mod native_i18n;
pub(crate) mod platform;

pub(crate) mod text_image;

pub(crate) mod native_drag;
