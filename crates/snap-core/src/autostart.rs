//! 用户级登录自启：系统登录项是唯一状态源，不在设置文件中重复保存布尔值。
//!
//! 与 Tauri 版（tauri-plugin-autostart，同为 auto-launch 0.6）使用同一登录项名称与启动参数，
//! 两个版本共用一条登录项：后开启的一方生效，不会同时自启而争抢全局快捷键。
//! 注意 Windows 上的状态查询只看登录项名称是否存在，不核对其指向哪个可执行文件。
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use std::sync::Mutex;

/// 自启入口携带固定参数，让程序初始化托盘和快捷键但不弹出界面。
pub const STARTUP_ARG: &str = "--autostart";
/// 登录项名称（与 Tauri 版 productName 一致）。
pub const APP_NAME: &str = "Snap Desk";

/// 串行化系统查询、启用与禁用，避免并发请求返回相互矛盾的状态。
static LOCK: Mutex<()> = Mutex::new(());

/// 首次启动与单实例转交共用精确参数匹配，避免把相似参数当作登录启动。
pub fn is_autostart(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> bool {
    args.into_iter().any(|arg| arg.as_ref() == STARTUP_ARG)
}

fn launcher() -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法定位程序路径：{e}"))?;
    let mut builder = AutoLaunchBuilder::new();
    builder.set_app_name(APP_NAME);
    builder.set_args(&[STARTUP_ARG]);
    builder.set_app_path(&exe.display().to_string());
    #[cfg(target_os = "macos")]
    builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
    builder.build().map_err(|e| format!("开机自启不可用：{e}"))
}

/// 读取系统登录项状态。
pub fn enabled() -> Result<bool, String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    launcher()?
        .is_enabled()
        .map_err(|e| format!("读取开机自启状态失败：{e}"))
}

/// 启用或禁用后重新读取；系统未应用时报错，调用方只展示确认后的状态。
pub fn set(enabled: bool) -> Result<bool, String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    let manager = launcher()?;
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| format!("设置开机自启失败：{e}"))?;
    let actual = manager
        .is_enabled()
        .map_err(|e| format!("设置已提交，但读取自启状态失败，请刷新确认：{e}"))?;
    if actual != enabled {
        return Err("系统未应用开机自启设置，请刷新状态后重试。".into());
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_startup_flag_counts() {
        assert!(is_autostart(["snap-desk", "--autostart"]));
        assert!(!is_autostart(["snap-desk"]));
        assert!(!is_autostart(["snap-desk", "--autostart=false"]));
    }
}
