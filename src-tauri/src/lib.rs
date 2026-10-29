//! 原生应用模块地图；启动编排在 app，业务代码留在所属功能目录。
mod app;
mod capture;
mod clipboard;
mod diagnostics;
mod pins;
mod recognition;
mod resources;
mod settings;
mod shared;

pub use app::run;
