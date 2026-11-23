//! 应用数据目录。两个可执行程序必须解析到同一位置，才能共用设置、历史与资源。
use std::path::PathBuf;

/// 与安装包标识（`scripts/package-gpui.mjs` 的 `identifier`）一致；改动会让已安装用户的配置“消失”。
pub const APP_IDENTIFIER: &str = "io.snapdesk.app";

/// 与旧版（Tauri `app_config_dir()`）相同：Windows 为 `%APPDATA%\io.snapdesk.app`，
/// macOS 为 `~/Library/Application Support/io.snapdesk.app`，Linux 为 `$XDG_CONFIG_HOME/io.snapdesk.app`。
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// 与旧版（Tauri `app_data_dir()`）相同；下载的识别引擎、字体等资源放在这里。
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// 与旧版（Tauri `app_local_data_dir()`）相同（Windows 为 `%LOCALAPPDATA%\io.snapdesk.app`）。
pub fn local_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// 剪贴板历史目录（数据库与图片），`app_local_data_dir/clipboard`。
pub fn clipboard_dir() -> Option<PathBuf> {
    local_data_dir().map(|dir| dir.join("clipboard"))
}

/// 与旧版（Tauri `app_log_dir()`）相同：macOS 为 `~/Library/Logs/<标识>`，其他平台为本地数据目录
/// （Windows 为 `%LOCALAPPDATA%`）下的 `<标识>/logs`。
pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return dirs::home_dir().map(|dir| dir.join("Library/Logs").join(APP_IDENTIFIER));
    #[cfg(not(target_os = "macos"))]
    dirs::data_local_dir().map(|dir| dir.join(APP_IDENTIFIER).join("logs"))
}
