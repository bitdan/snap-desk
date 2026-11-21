//! 截图和贴图共用原生导出的 Tauri 外壳；文件名、编码与目录记忆在 `snap_core::export`。
//! 保存路径只能来自系统对话框。
use crate::settings::SettingsState;
use image::RgbaImage;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
#[cfg(not(target_os = "windows"))]
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

pub(crate) use snap_core::export::validate_directory;
/// 组合导出移入 core 后只剩微信 OCR（Windows x64）使用。
#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) use snap_core::export::white_background;
use snap_core::export::{
    configure_directory_record, encode, filename, preferred_directory, read_directory,
    remember_directory, validate_extension,
};

/// Windows 剪贴板需要一个所有者窗口；沿用常驻的主窗口。
#[cfg(target_os = "windows")]
fn clipboard_owner(app: &AppHandle) -> Result<isize, String> {
    let window = app
        .get_webview_window("main")
        .ok_or("主窗口不可用，无法复制图片。")?;
    Ok(window.hwnd().map_err(|e| e.to_string())?.0 as isize)
}

/// 保留原始 RGBA 像素，不把窗口缩放或透明度混入导出图片。
pub fn copy(app: &AppHandle, image: &RgbaImage) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return snap_core::clipboard::windows_image::copy(clipboard_owner(app)?, image);

    // X11 由本程序持有选区，额外提供 BMP 目标（VirtualBox 等只认 BMP）；失败时交回剪贴板插件。
    #[cfg(target_os = "linux")]
    if !crate::shared::x11::wayland() {
        match crate::clipboard::write_x11_image(image) {
            Ok(()) => return Ok(()),
            Err(error) => {
                log::warn!("event=clipboard.x11_image_failed module=clipboard error={error}")
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    app.clipboard()
        .write_image(&tauri::image::Image::new(
            image.as_raw(),
            image.width(),
            image.height(),
        ))
        .map_err(|e| format!("复制失败，请重试或保存图片：{e}"))
}

/// 内部导出流程复用刚编码的 PNG；非 Windows 仍由平台插件提交原始像素。
pub(crate) fn copy_png(app: &AppHandle, image: &RgbaImage, png: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return snap_core::clipboard::windows_image::copy_png(clipboard_owner(app)?, image, png);

    #[cfg(not(target_os = "windows"))]
    {
        let _ = png;
        copy(app, image)
    }
}

/// 保存目录记录文件；与 Tauri `app_config_dir()` 同一位置。
fn directory_record(_app: &AppHandle) -> Result<PathBuf, String> {
    snap_core::export::directory_record().ok_or_else(|| "无法定位配置目录。".to_string())
}

/// 通用设置展示显式默认目录或已有保存位置；尚未设置时使用系统默认位置。
pub(crate) fn current_directory(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    Ok(read_directory(&directory_record(app)?))
}

/// 目录只能来自设置页的系统选择器；重置仅移除偏好文件，不触碰图片。
pub(crate) fn configure_directory(app: &AppHandle, directory: Option<&Path>) -> Result<(), String> {
    configure_directory_record(&directory_record(app)?, directory)
}

/// 历史组合导出复用既有目录记忆，只有成功保存后调用。
pub(crate) fn remember_export_directory(app: &AppHandle, directory: &Path) -> Result<(), String> {
    remember_directory(&directory_record(app)?, directory)
}

/// 必须在阻塞工作线程调用；调用方负责隐藏置顶窗口，避免挡住对话框。
/// 设置只取快照，不持锁等待对话框。取消返回 false；失败保留旧文件和当前图片。
pub fn save(app: &AppHandle, image: &RgbaImage) -> Result<bool, String> {
    let settings = app
        .state::<SettingsState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .value
        .export
        .clone();
    let stamp = snap_core::export::timestamp();
    let (label, extensions) = settings.format.filter();
    let mut dialog = app
        .dialog()
        .file()
        .set_title(crate::shared::native_i18n::text(&app, "保存图片"))
        .add_filter(crate::shared::native_i18n::text(app, label), extensions)
        .set_file_name(filename(
            &settings,
            &stamp,
            image.width(),
            image.height(),
        )?);
    let record = directory_record(app).ok();
    if let Some(directory) = record
        .as_deref()
        .and_then(|path| preferred_directory(path, settings.remember_directory))
    {
        dialog = dialog.set_directory(directory);
    }
    let Some(path) = dialog.blocking_save_file() else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    validate_extension(&path, settings.format)?;
    let parent = path.parent().ok_or("保存路径无效。")?;
    let mut temporary = crate::shared::export_staging::TrackedFile::new(
        app,
        parent,
        settings.format.extension(),
        "截图 / 贴图导出",
    )?;
    encode(temporary.file_mut(), image, &settings)?;
    temporary.finalize()?;
    temporary.publish(&path, true)?;
    if settings.remember_directory {
        if let Some(record) = record {
            if let Err(_error) = remember_directory(&record, parent) {
                // 图片已经成功保存；记忆失败不能将导出报告为失败。
                log::warn!("event=capture.directory_memory_failed module=capture code=DIRECTORY_MEMORY_FAILED");
            }
        }
    }
    Ok(true)
}
