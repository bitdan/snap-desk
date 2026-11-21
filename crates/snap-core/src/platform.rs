
/// 使用系统 URL 处理器打开固定地址，不接受前端传入路径或命令。
/// 在阻塞任务中等待启动器结束，避免阻塞主窗口，并将启动失败反馈给界面。
pub fn open_project_homepage() -> Result<(), String> {
    open_web_page("https://github.com/bitdan/snap-desk")
}

/// 由功能模块传入受信的固定页面，复用系统浏览器启动方式，不经过 shell 拼接。
pub fn open_web_page(url: &'static str) -> Result<(), String> {
    open_trusted(url)
}

/// crate 内部拼出的受信地址（如发布页，版本号已校验）。
pub(crate) fn open_trusted(url: &str) -> Result<(), String> {
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
        .arg(url)
        .status()
        .map_err(|e| format!("无法打开浏览器：{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("无法打开网页，请检查系统默认浏览器设置。".into())
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

/// 目录直接打开；文件交给文件管理器定位，始终不调用文件关联程序。
pub fn reveal_path(path: &std::path::Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("此条目不是有效文件路径。".into());
    }
    let metadata = std::fs::metadata(path).map_err(|_| "路径不存在或无法访问。".to_string())?;
    if metadata.is_dir() {
        return open_directory(path);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // /select, 与路径使用独立参数，文件名中的空格由系统参数编码处理。
        std::process::Command::new("explorer.exe")
            .arg("/select,")
            .arg(path)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| format!("无法打开目录：{e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| format!("无法打开目录：{e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        open_directory(path.parent().ok_or("此条目不是有效文件路径。")?)
    }
}

#[cfg(target_os = "macos")]
pub fn capture_error(error: impl std::fmt::Display) -> String {
    format!("截图失败：{error}。请在系统设置 → 隐私与安全性 → 屏幕录制中授权后重启应用。")
}
