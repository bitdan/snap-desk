//! 应用数据目录。两个可执行程序必须解析到同一位置，才能共用设置、历史与资源。
use std::path::PathBuf;

/// 与 `src-tauri/tauri.conf.json` 的 `identifier` 一致；改动会让已安装用户的配置“消失”。
pub const APP_IDENTIFIER: &str = "io.snapdesk.app";

/// 等同 Tauri `app_config_dir()`：Windows 为 `%APPDATA%\io.snapdesk.app`，
/// macOS 为 `~/Library/Application Support/io.snapdesk.app`，Linux 为 `$XDG_CONFIG_HOME/io.snapdesk.app`。
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// 等同 Tauri `app_data_dir()`；剪贴板历史、下载的识别引擎等放在这里。
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// 等同 Tauri `app_log_dir()`：macOS 为 `~/Library/Logs/<标识>`，其他平台为本地数据目录
/// （Windows 为 `%LOCALAPPDATA%`）下的 `<标识>/logs`。
pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return dirs::home_dir().map(|dir| dir.join("Library/Logs").join(APP_IDENTIFIER));
    #[cfg(not(target_os = "macos"))]
    dirs::data_local_dir().map(|dir| dir.join(APP_IDENTIFIER).join("logs"))
}
