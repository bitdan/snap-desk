use image::RgbaImage;

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
