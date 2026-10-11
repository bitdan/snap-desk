//! 截图和贴图共用原生导出；保存路径只能来自系统对话框。
use image::{ImageFormat, RgbaImage};
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;
#[cfg(not(target_os = "windows"))]
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

#[cfg(target_os = "windows")]
#[path = "windows_clipboard.rs"]
mod windows_clipboard;

/// 保留原始 RGBA 像素，不把窗口缩放或透明度混入导出图片。
pub fn copy(app: &AppHandle, image: &RgbaImage) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return windows_clipboard::copy(app, image);

    #[cfg(not(target_os = "windows"))]
    app.clipboard()
        .write_image(&tauri::image::Image::new(
            image.as_raw(),
            image.width(),
            image.height(),
        ))
        .map_err(|e| format!("复制失败，请重试或保存图片：{e}"))
}

/// 必须在阻塞工作线程调用；调用方负责隐藏置顶窗口，避免挡住对话框。
/// 取消返回 false；写入失败保留旧文件和当前图片，允许用户重试。
pub fn save(app: &AppHandle, image: &RgbaImage) -> Result<bool, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let path = app
        .dialog()
        .file()
        .set_title("保存图片")
        .add_filter("PNG 图片", &["png"])
        .set_file_name(format!("SnapDesk-{timestamp}.png"))
        .blocking_save_file();
    let Some(path) = path else { return Ok(false) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        return Err("请使用 .png 扩展名保存图片。".into());
    }
    let parent = path.parent().ok_or("保存路径无效。")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    image
        .write_to(temporary.as_file_mut(), ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    temporary.flush().map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(&path).map_err(|e| e.to_string())?;
    Ok(true)
}
