//! 独立 EXE 更新：先验证并暂存到同一磁盘，外部助手等待退出后原子替换。

#[cfg(windows)]
pub(super) fn install(
    app: &tauri::AppHandle,
    update: &tauri_plugin_updater::Update,
    bytes: &[u8],
) -> Result<(), String> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::Write,
        os::windows::process::CommandExt,
        process::Command,
        time::{Duration, Instant},
    };

    update.verify(bytes).map_err(|e| e.to_string())?;
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::shared::export_staging::no_links(&target)?;
    let parent = target.parent().ok_or("程序目录无效。")?;
    // 创建失败（只读目录/权限不足）时保持应用运行，不申请提权后悄悄写入其他位置。
    let directory = tempfile::Builder::new()
        .prefix(".snapdesk-update-")
        .tempdir_in(parent)
        .map_err(|_| "程序目录不可写，请手动下载新版本并替换。")?;
    let source = directory.path().join("new.exe");
    let mut file = fs::File::create(&source).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let script = include_str!("portable.ps1");
    let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let system = std::env::var_os("SystemRoot").ok_or("无法定位 Windows 系统目录。")?;
    let mut helper = Command::new(
        std::path::Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-EncodedCommand",
    ])
    .arg(base64::engine::general_purpose::STANDARD.encode(encoded))
    // 路径通过环境变量传入固定脚本，包含引号、空格或 $ 的目录不会成为命令。
    .env("SNAP_UPDATE_TARGET", &target)
    .env("SNAP_UPDATE_SOURCE", &source)
    .env("SNAP_UPDATE_PARENT", std::process::id().to_string())
    .env("SNAP_UPDATE_HASH", format!("{:x}", Sha256::digest(bytes)))
    .creation_flags(0x08000000)
    .spawn()
    .map_err(|e| e.to_string())?;
    let started = Instant::now();
    while !directory.path().join("ready").is_file() {
        if helper.try_wait().map_err(|e| e.to_string())?.is_some()
            || started.elapsed() > Duration::from_secs(5)
        {
            let _ = helper.kill();
            let _ = helper.wait();
            return Err("无法启动更新助手，程序保持运行，请重试或手动更新。".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 只有助手就绪才转移暂存目录所有权；助手仅清理其中的已知文件。
    let _ = directory.keep();
    app.exit(0);
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn install(
    _: &tauri::AppHandle,
    _: &tauri_plugin_updater::Update,
    _: &[u8],
) -> Result<(), String> {
    Err("独立 EXE 更新仅适用于 Windows。".into())
}
