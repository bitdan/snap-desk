use image::RgbaImage;

/// 使用系统 URL 处理器打开固定地址，不接受前端传入路径或命令。
/// 在阻塞任务中等待启动器结束，避免阻塞主窗口，并将启动失败反馈给界面。
pub fn open_project_homepage() -> Result<(), String> {
    const URL: &str = "https://github.com/bitdan/snap-desk";
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("rundll32.exe");
        command
            .arg("url.dll,FileProtocolHandler")
            .creation_flags(0x08000000);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    let status = command
        .arg(URL)
        .status()
        .map_err(|e| format!("无法打开浏览器：{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("无法打开项目主页，请检查系统默认浏览器设置。".into())
    }
}

/// 只接收后端解析的目录，独立参数交给系统文件管理器，不经过 shell 拼接。
pub fn open_directory(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer.exe");
        command.creation_flags(0x08000000);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    // Explorer 可以把请求交给已有进程，其退出码不能作为打开失败的依据。
    command
        .arg(path)
        .spawn()
        .map_err(|e| format!("无法打开目录：{e}"))?;
    Ok(())
}

// Keep OS capture details behind one boundary; overlay, crop and export never depend on xcap.
pub fn capture_primary() -> Result<RgbaImage, String> {
    #[cfg(target_os = "linux")]
    if std::env::var("XDG_SESSION_TYPE").unwrap_or_default() == "wayland"
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
    {
        return Err("V0.1 暂不支持 Wayland，请在 X11 会话中运行。Portal 适配计划后续实现。".into());
    }

    let monitors = xcap::Monitor::all().map_err(capture_error)?;
    for monitor in monitors {
        if monitor.is_primary().map_err(capture_error)? {
            return monitor.capture_image().map_err(capture_error);
        }
    }
    Err("未找到主显示器，请检查显示器连接。".into())
}

fn capture_error(error: impl std::fmt::Display) -> String {
    #[cfg(target_os = "macos")]
    return format!("截图失败：{error}。请在系统设置 → 隐私与安全性 → 屏幕录制中授权后重启应用。");
    #[cfg(not(target_os = "macos"))]
    format!("截图失败：{error}")
}
