//! 截图和贴图共用原生导出；保存路径只能来自系统对话框。
use crate::settings::{ExportFormat, ExportSettings, SettingsState};
use image::{codecs::jpeg::JpegEncoder, ImageFormat, Rgb, RgbImage, RgbaImage};
use std::{
    fs,
    io::{Seek, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
#[cfg(not(target_os = "windows"))]
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

#[cfg(target_os = "windows")]
#[path = "windows_clipboard.rs"]
mod windows_clipboard;

/// 专项基准只构造系统载荷，不替换开发者当前剪贴板。
#[cfg(all(test, windows))]
pub(crate) fn clipboard_bitmap_for_benchmark(image: &RgbaImage) -> Result<Vec<u8>, String> {
    windows_clipboard::benchmark_dib(image)
}

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

impl ExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Png => ("PNG 图片（无损）", &["png"]),
            Self::Jpeg => ("JPEG 图片（白底）", &["jpg", "jpeg"]),
            Self::Webp => ("WebP 图片（无损）", &["webp"]),
        }
    }
}

/// 不自动重写用户指定的扩展名，避免绕过系统对话框对实际目标文件的覆盖确认。
fn validate_extension(path: &Path, format: ExportFormat) -> Result<(), String> {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            format
                .filter()
                .1
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
    {
        Ok(())
    } else {
        Err(format!(
            "当前导出格式需要 .{} 扩展名；可在设置中更换格式。",
            format.extension()
        ))
    }
}

/// 文件名只由已验证的前缀、毫秒时间戳和可选像素尺寸组成，不接收任意路径模板。
fn filename(
    settings: &ExportSettings,
    timestamp: u128,
    width: u32,
    height: u32,
) -> Result<String, String> {
    settings.validate()?;
    let dimensions = if settings.include_dimensions {
        format!("-{width}x{height}")
    } else {
        String::new()
    };
    Ok(format!(
        "{}-{timestamp}{dimensions}.{}",
        settings.filename_prefix,
        settings.format.extension()
    ))
}

/// JPEG 没有 alpha：先与白底合成，不能直接丢掉 alpha，避免透明处变黑。
pub(crate) fn white_background(image: &RgbaImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y).0;
        let alpha = u32::from(pixel[3]);
        Rgb([0, 1, 2]
            .map(|i| ((u32::from(pixel[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8))
    })
}

/// 直接编码到临时文件，PNG / WebP 保留 alpha；质量仅作用于 JPEG。
fn encode(
    writer: &mut (impl Write + Seek),
    image: &RgbaImage,
    settings: &ExportSettings,
) -> Result<(), String> {
    settings.validate()?;
    match settings.format {
        ExportFormat::Png => image.write_to(writer, ImageFormat::Png),
        ExportFormat::Webp => image.write_to(writer, ImageFormat::WebP),
        ExportFormat::Jpeg => JpegEncoder::new_with_quality(writer, settings.jpeg_quality)
            .encode_image(&white_background(image)),
    }
    .map_err(|e| format!("图片编码失败：{e}"))
}

/// 保存目录与用户设置分文件存放，不把系统路径暴露给前端，也不覆盖并发保存的偏好。
fn directory_record(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("export-directory.json"))
}

// 兼容旧版只记录上次保存路径；显式设置的目录不受“记住上次位置”开关影响。
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
enum DirectoryPreference {
    Remembered(PathBuf),
    Selected { directory: PathBuf },
}
static DIRECTORY_RECORD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read_preference(path: &Path) -> Option<DirectoryPreference> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn preferred_directory(path: &Path, remember: bool) -> Option<PathBuf> {
    let directory = match read_preference(path)? {
        DirectoryPreference::Selected { directory } => directory,
        DirectoryPreference::Remembered(directory) if remember => directory,
        _ => return None,
    };
    // 移动磁盘拔出、目录被删除或记录损坏时，让系统选择默认位置。
    (directory.is_absolute() && directory.is_dir()).then_some(directory)
}
fn read_directory(path: &Path) -> Option<PathBuf> {
    preferred_directory(path, true)
}

/// 通用设置展示显式默认目录或已有保存位置；尚未设置时使用系统默认位置。
pub(crate) fn current_directory(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    Ok(read_directory(&directory_record(app)?))
}

fn write_directory(record: &Path, preference: &DirectoryPreference) -> Result<(), String> {
    let parent = record.parent().ok_or("目录记录路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(temporary.as_file_mut(), preference).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(record).map_err(|e| e.to_string())?;
    Ok(())
}

fn remember_directory(record: &Path, directory: &Path) -> Result<(), String> {
    let _guard = DIRECTORY_RECORD_LOCK.lock().map_err(|e| e.to_string())?;
    // 保存到临时的其他位置不能覆盖用户在设置页显式选定的默认目录。
    if matches!(
        read_preference(record),
        Some(DirectoryPreference::Selected { .. })
    ) {
        return Ok(());
    }
    write_directory(
        record,
        &DirectoryPreference::Remembered(directory.to_owned()),
    )
}

/// 目录只能来自设置页的系统选择器；重置仅移除偏好文件，不触碰图片。
pub(crate) fn configure_directory(app: &AppHandle, directory: Option<&Path>) -> Result<(), String> {
    configure_directory_record(&directory_record(app)?, directory)
}
fn configure_directory_record(record: &Path, directory: Option<&Path>) -> Result<(), String> {
    let _guard = DIRECTORY_RECORD_LOCK.lock().map_err(|e| e.to_string())?;
    if let Some(directory) = directory {
        validate_directory(directory)?;
        write_directory(
            record,
            &DirectoryPreference::Selected {
                directory: directory.to_owned(),
            },
        )
    } else {
        match fs::remove_file(record) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// 配置导入与目录选择共用可用性校验，校验失败不改写偏好。
pub(crate) fn validate_directory(directory: &Path) -> Result<(), String> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("截图保存目录不可用，请重新选择。".into());
    }
    tempfile::NamedTempFile::new_in(directory)
        .and_then(|file| file.close())
        .map_err(|_| "截图保存目录无法写入，请选择其他目录。".to_string())
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
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let (label, extensions) = settings.format.filter();
    let mut dialog = app
        .dialog()
        .file()
        .set_title(crate::shared::native_i18n::text(&app, "保存图片"))
        .add_filter(crate::shared::native_i18n::text(app, label), extensions)
        .set_file_name(filename(
            &settings,
            timestamp,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn lossless_formats_preserve_visible_pixels_and_alpha() {
        let image = RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([80, (x * 40) as u8, 120, (100 + y * 100) as u8])
        });
        for format in [ExportFormat::Png, ExportFormat::Webp] {
            let mut output = Cursor::new(Vec::new());
            encode(
                &mut output,
                &image,
                &ExportSettings {
                    format,
                    ..ExportSettings::default()
                },
            )
            .unwrap();
            let decoded = image::load_from_memory(output.get_ref())
                .unwrap()
                .to_rgba8();
            assert_eq!(decoded, image);
        }
    }

    #[test]
    fn jpeg_uses_white_background_and_decodes_at_both_quality_limits() {
        let image = RgbaImage::from_fn(3, 1, |x, _| {
            image::Rgba([255, 0, 0, [0, 128, 255][x as usize]])
        });
        let rgb = white_background(&image);
        assert_eq!(rgb.get_pixel(0, 0).0, [255, 255, 255]);
        assert_eq!(rgb.get_pixel(1, 0).0, [255, 127, 127]);
        assert_eq!(rgb.get_pixel(2, 0).0, [255, 0, 0]);
        for quality in [1, 100] {
            let mut output = Cursor::new(Vec::new());
            encode(
                &mut output,
                &image,
                &ExportSettings {
                    format: ExportFormat::Jpeg,
                    jpeg_quality: quality,
                    ..ExportSettings::default()
                },
            )
            .unwrap();
            assert_eq!(
                image::guess_format(output.get_ref()).unwrap(),
                ImageFormat::Jpeg
            );
            let decoded = image::load_from_memory(output.get_ref()).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (3, 1));
        }
    }

    #[test]
    fn extension_must_match_encoder_and_accepts_jpeg_alias() {
        for path in ["screen.JPG", "screen.jpeg"] {
            assert!(validate_extension(Path::new(path), ExportFormat::Jpeg).is_ok());
        }
        for path in ["screen", "screen.jpg", "screen.png.exe"] {
            assert!(validate_extension(Path::new(path), ExportFormat::Png).is_err());
        }
        assert!(validate_extension(Path::new("screen.PNG"), ExportFormat::Png).is_ok());
        assert!(validate_extension(Path::new("screen.webp"), ExportFormat::Webp).is_ok());
    }

    #[test]
    fn names_use_prefix_timestamp_dimensions_and_correct_extension() {
        let mut settings = ExportSettings::default();
        assert_eq!(
            filename(&settings, 1234, 640, 480).unwrap(),
            "SnapDesk-1234.png"
        );
        settings.filename_prefix = "截图".into();
        settings.include_dimensions = true;
        settings.format = ExportFormat::Jpeg;
        assert_eq!(
            filename(&settings, 1234, 640, 480).unwrap(),
            "截图-1234-640x480.jpg"
        );
        settings.filename_prefix = "../escape".into();
        assert!(filename(&settings, 1234, 640, 480).is_err());
    }

    #[test]
    fn directory_record_survives_restart_and_ignores_missing_or_corrupt_paths() {
        let directory = tempfile::tempdir().unwrap();
        let record = directory.path().join("config/export-directory.json");
        assert!(read_directory(&record).is_none());
        remember_directory(&record, directory.path()).unwrap();
        assert_eq!(read_directory(&record).as_deref(), Some(directory.path()));
        remember_directory(&record, &directory.path().join("missing")).unwrap();
        assert!(read_directory(&record).is_none());
        fs::write(&record, "broken").unwrap();
        assert!(read_directory(&record).is_none());
        fs::write(&record, r#""relative""#).unwrap();
        assert!(read_directory(&record).is_none());
    }

    #[test]
    fn configured_capture_directory_survives_memory_toggle_and_other_save_locations() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("config/export-directory.json");
        let selected = root.path().join("selected");
        let other = root.path().join("other");
        fs::create_dir_all(&selected).unwrap();
        fs::create_dir_all(&other).unwrap();
        let existing = selected.join("existing.png");
        fs::write(&existing, b"keep").unwrap();
        // 旧版字符串记录仍可读取，关闭自动记忆时不作为默认目录。
        remember_directory(&record, &other).unwrap();
        assert_eq!(preferred_directory(&record, true), Some(other.clone()));
        assert!(preferred_directory(&record, false).is_none());
        configure_directory_record(&record, Some(&selected)).unwrap();
        assert_eq!(preferred_directory(&record, false), Some(selected.clone()));
        remember_directory(&record, &other).unwrap();
        assert_eq!(read_directory(&record), Some(selected.clone()));
        assert!(configure_directory_record(&record, Some(&root.path().join("missing"))).is_err());
        assert_eq!(read_directory(&record), Some(selected));
        configure_directory_record(&record, None).unwrap();
        configure_directory_record(&record, None).unwrap();
        assert!(read_directory(&record).is_none());
        assert_eq!(fs::read(existing).unwrap(), b"keep");
    }
}
