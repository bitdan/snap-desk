//! Snap Desk 与界面框架无关的业务核心，由桌面应用（`apps/desktop-gpui`）调用。
//!
//! 边界约定：本 crate 不依赖 `gpui` 或任何窗口/事件循环；需要通知界面或调用系统
//! 外壳能力（注册快捷键、发事件、读写窗口）时，由调用方以闭包或 trait 注入。
pub mod annotations;
pub mod autostart;
pub mod capture;
pub mod clipboard;
pub mod desktop_support;
pub mod diagnostics;
pub mod drag;
pub mod export;
pub mod i18n;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod paths;
pub mod pins;
pub mod platform;
pub mod recognition;
pub mod recording;
pub mod resources;
pub mod scroll;
pub mod settings;
pub mod shelf;
pub mod text_image;
pub mod updates;
#[cfg(target_os = "linux")]
pub mod wayland_probe;
#[cfg(target_os = "linux")]
pub mod x11;
