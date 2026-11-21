//! 多个功能复用的系统设施；平台适配、图像导出和原生窗口支持统一放在这里。
//! 与界面无关的平台层（platform、desktop_support、x11、wayland_probe、macos 系统函数）在 core，此处按原路径重导出。
pub(crate) use snap_core::annotations;
pub(crate) mod auxiliary_windows;
#[cfg(test)]
pub(crate) mod benchmark;
pub(crate) use snap_core::desktop_support;
pub(crate) mod dialog_windows;
pub(crate) mod export_staging;
pub(crate) mod idle_release;
pub(crate) mod image_io;
pub(crate) mod image_protocol;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
pub(crate) mod native_i18n;
pub(crate) use snap_core::platform;

#[cfg(target_os = "linux")]
pub(crate) use snap_core::{wayland_probe, x11};

pub(crate) mod native_drag;
