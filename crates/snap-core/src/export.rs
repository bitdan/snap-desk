//! 截图与贴图的图片导出：文件名、编码（PNG / JPEG / WebP / 单页 PDF）、扩展名校验和保存目录记忆。
//! 保存路径只能来自系统对话框，对话框本身与临时文件追踪由各外壳负责（GPUI 用 rfd）。
use crate::settings::{ExportFormat, ExportSettings};
use image::{codecs::jpeg::JpegEncoder, ImageFormat, RgbImage, RgbaImage};
use std::{
    fs,
    io::{Seek, Write},
    path::{Path, PathBuf},
};

/// PDF 页面尺寸换算：图片 1 像素按 96 DPI 计，与剪贴板组合导出的“原始尺寸”页面一致。
const PDF_MM_PER_PX: f32 = 25.4 / 96.0;

/// 保存目录与用户设置分文件存放（与 settings.json 同目录），不覆盖并发保存的偏好。
pub fn directory_record() -> Option<PathBuf> {
    crate::paths::config_dir().map(|dir| dir.join("export-directory.json"))
}

/// 快速保存与自动保存的目录：设置里选定的截图保存目录（开启“记住上次保存目录”时也可能是上次保存的位置），
/// 都没有时为“图片/Snap Desk”。目录不存在时由保存时创建。
pub fn save_directory(remember: bool) -> Option<PathBuf> {
    directory_record()
        .as_deref()
        .and_then(|record| preferred_directory(record, remember))
        .or_else(|| dirs::picture_dir().map(|dir| dir.join("Snap Desk")))
}

/// 目录里不与已有文件重名的路径：重名时在扩展名前加 -2、-3……，不覆盖已有文件。
pub fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let first = directory.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = name.rsplit_once('.').map_or((name, None), |(stem, ext)| (stem, Some(ext)));
    (2..)
        .map(|n| match extension {
            Some(ext) => directory.join(format!("{stem}-{n}.{ext}")),
            None => directory.join(format!("{stem}-{n}")),
        })
        .find(|path| !path.exists())
        .expect("编号足够多时总能找到未使用的文件名")
}

/// 不弹对话框直接保存：按设置的默认格式与文件名规则存进 `directory`（不存在时创建），重名时加编号。返回保存路径。
pub fn quick_save(image: &RgbaImage, settings: &ExportSettings, directory: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(directory).map_err(|_| "截图保存目录无法创建，请在设置中重新选择。".to_string())?;
    let name = filename(settings, &timestamp(), image.width(), image.height())?;
    let path = unique_path(directory, &name);
    write_file(&path, image, settings)?;
    Ok(path)
}

/// 文件名所用的本地时间戳（YYYYMMDD-HHMMSS）。
pub fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// 编码到同目录临时文件再替换目标：失败时保留旧文件。不做崩溃残留追踪。
/// 编码格式由对话框返回路径的扩展名决定（对话框里可临时改存其他格式），JPEG 质量等其余偏好沿用设置。
pub fn write_file(path: &Path, image: &RgbaImage, settings: &ExportSettings) -> Result<(), String> {
    let settings = ExportSettings {
        format: format_for_path(path)?,
        ..settings.clone()
    };
    let parent = path.parent().ok_or("保存路径无效。")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    encode(file.as_file_mut(), image, &settings)?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

/// 按扩展名确定编码格式。不自动补写或改写用户指定的扩展名，避免绕过系统对话框对实际目标文件的覆盖确认。
pub fn format_for_path(path: &Path) -> Result<ExportFormat, String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| {
            ExportFormat::ALL.into_iter().find(|format| {
                format
                    .filter()
                    .1
                    .iter()
                    .any(|allowed| ext.eq_ignore_ascii_case(allowed))
            })
        })
        .ok_or_else(|| "文件扩展名须为 .png、.jpg、.webp 或 .pdf。".into())
}

/// 文件名只由已验证的前缀、本地日期时间（YYYYMMDD-HHMMSS）和可选像素尺寸组成，不接收任意路径模板。
/// 同一秒内重名由保存对话框提示覆盖或改名。
pub fn filename(
    settings: &ExportSettings,
    stamp: &str,
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
        "{}-{stamp}{dimensions}.{}",
        settings.filename_prefix,
        settings.format.extension()
    ))
}

/// JPEG 没有 alpha：先与白底合成，不能直接丢掉 alpha，避免透明处变黑。
pub fn white_background(image: &RgbaImage) -> RgbImage {
    // 直接按 4 字节一组遍历原始缓冲，不逐像素做坐标换算和边界检查；
    // 截图几乎全是不透明像素，这类像素原样拷贝 RGB。
    let mut rgb = Vec::with_capacity(image.as_raw().len() / 4 * 3);
    for pixel in image.as_raw().chunks_exact(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            rgb.extend_from_slice(&pixel[..3]);
        } else {
            rgb.extend(pixel[..3].iter().map(|channel| {
                ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
            }));
        }
    }
    RgbImage::from_raw(image.width(), image.height(), rgb)
        .expect("RGB 缓冲长度由同一张图片的像素数得出")
}

/// 直接编码到临时文件，PNG / WebP 保留 alpha；质量仅作用于 JPEG。
pub fn encode(
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
        ExportFormat::Pdf => {
            let bytes = pdf(image)?;
            return writer.write_all(&bytes).map_err(|e| format!("图片编码失败：{e}"));
        }
    }
    .map_err(|e| format!("图片编码失败：{e}"))
}

/// 单页 PDF：页面与图片同尺寸（96 DPI）、无边距，图片铺满整页。滚动截图等超长图片按 96 DPI 会超过
/// 常见阅读器的页面上限（200 英寸），这时整页等比缩小到长边 200 英寸；像素数据不变，只是标称尺寸变小。
pub fn pdf(image: &RgbaImage) -> Result<Vec<u8>, String> {
    const MAX_PAGE_MM: f32 = 200.0 * 25.4;
    let (w, h) = (image.width() as f32 * PDF_MM_PER_PX, image.height() as f32 * PDF_MM_PER_PX);
    let k = (MAX_PAGE_MM / w.max(h)).min(1.0);
    let doc = printpdf::PdfDocument::empty("Snap Desk");
    add_pdf_image_page(&doc, image, (k < 1.0).then_some((w * k, h * k)), 0.0);
    doc.save_to_bytes().map_err(|_| "PDF 编码失败。".to_string())
}

/// 向 PDF 追加一页图片（剪贴板组合导出与截图保存共用）。`page_mm` 为 None 时页面取图片尺寸加边距，
/// 否则用给定纸张（毫米）；图片在边距内等比缩放并居中。PDF 是白色纸面，透明像素先与白色合成，避免变黑。
pub(crate) fn add_pdf_image_page(
    doc: &printpdf::PdfDocumentReference,
    image: &RgbaImage,
    page_mm: Option<(f32, f32)>,
    margin_mm: f32,
) {
    use printpdf::*;
    let (w, h) = image.dimensions();
    let (image_w, image_h) = (w as f32 * PDF_MM_PER_PX, h as f32 * PDF_MM_PER_PX);
    let (pw, ph) = page_mm.unwrap_or((image_w + margin_mm * 2.0, image_h + margin_mm * 2.0));
    let scale = ((pw - margin_mm * 2.0) / image_w).min((ph - margin_mm * 2.0) / image_h);
    let (page, layer) = doc.add_page(Mm(pw), Mm(ph), "图片");
    let object = Image::from(ImageXObject {
        width: Px(w as usize),
        height: Px(h as usize),
        color_space: ColorSpace::Rgb,
        bits_per_component: ColorBits::Bit8,
        interpolate: true,
        image_data: white_background(image).into_raw(),
        image_filter: None,
        smask: None,
        clipping_bbox: None,
    });
    object.add_to_layer(
        doc.get_page(page).get_layer(layer),
        ImageTransform {
            translate_x: Some(Mm((pw - image_w * scale) / 2.0)),
            translate_y: Some(Mm((ph - image_h * scale) / 2.0)),
            scale_x: Some(scale),
            scale_y: Some(scale),
            dpi: Some(96.0),
            ..Default::default()
        },
    );
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
pub fn preferred_directory(path: &Path, remember: bool) -> Option<PathBuf> {
    let directory = match read_preference(path)? {
        DirectoryPreference::Selected { directory } => directory,
        DirectoryPreference::Remembered(directory) if remember => directory,
        _ => return None,
    };
    // 移动磁盘拔出、目录被删除或记录损坏时，让系统选择默认位置。
    (directory.is_absolute() && directory.is_dir()).then_some(directory)
}
pub fn read_directory(path: &Path) -> Option<PathBuf> {
    preferred_directory(path, true)
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

pub fn remember_directory(record: &Path, directory: &Path) -> Result<(), String> {
    let _guard = DIRECTORY_RECORD_LOCK.lock().map_err(|e| e.to_string())?;
    // 保存到临时的其他位置不能覆盖用户在设置页显式选定的默认目录。
    match read_preference(record) {
        Some(DirectoryPreference::Selected { .. }) => return Ok(()),
        // 连续保存到同一目录时记录没有变化，不再重写文件并等待落盘。
        Some(DirectoryPreference::Remembered(current)) if current == directory => return Ok(()),
        _ => {}
    }
    write_directory(
        record,
        &DirectoryPreference::Remembered(directory.to_owned()),
    )
}

/// 目录只能来自设置页的系统选择器；重置仅移除偏好文件，不触碰图片。
pub fn configure_directory_record(record: &Path, directory: Option<&Path>) -> Result<(), String> {
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
pub fn validate_directory(directory: &Path) -> Result<(), String> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("截图保存目录不可用，请重新选择。".into());
    }
    tempfile::NamedTempFile::new_in(directory)
        .and_then(|file| file.close())
        .map_err(|_| "截图保存目录无法写入，请选择其他目录。".to_string())
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
    fn extension_selects_the_encoder_and_accepts_jpeg_alias() {
        for path in ["screen.JPG", "screen.jpeg"] {
            assert_eq!(format_for_path(Path::new(path)), Ok(ExportFormat::Jpeg));
        }
        for path in ["screen", "screen.png.exe", "screen.gif"] {
            assert!(format_for_path(Path::new(path)).is_err());
        }
        assert_eq!(format_for_path(Path::new("screen.PNG")), Ok(ExportFormat::Png));
        assert_eq!(format_for_path(Path::new("screen.webp")), Ok(ExportFormat::Webp));
        assert_eq!(format_for_path(Path::new("screen.Pdf")), Ok(ExportFormat::Pdf));
    }

    #[test]
    fn quick_save_numbers_duplicates_instead_of_overwriting() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("nested");
        let image = RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let settings = ExportSettings::default();
        let first = quick_save(&image, &settings, &target).unwrap();
        assert!(first.starts_with(&target) && first.extension().unwrap() == "png");
        // 同一秒内再存两次：依次加 -2、-3，原文件不被覆盖。
        let name = first.file_name().unwrap().to_str().unwrap().to_owned();
        let stem = name.trim_end_matches(".png");
        assert_eq!(unique_path(&target, &name), target.join(format!("{stem}-2.png")));
        fs::write(target.join(format!("{stem}-2.png")), b"x").unwrap();
        assert_eq!(unique_path(&target, &name), target.join(format!("{stem}-3.png")));
        assert_eq!(unique_path(&target, "free.png"), target.join("free.png"));
    }

    #[test]
    fn dialog_lists_the_default_format_first_then_the_rest_once() {
        let order: Vec<_> = ExportFormat::Webp.dialog_order().collect();
        assert_eq!(
            order,
            [ExportFormat::Webp, ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Pdf]
        );
    }

    #[test]
    fn pdf_page_matches_the_image_and_is_chosen_by_extension() {
        let image = RgbaImage::from_pixel(192, 96, image::Rgba([10, 20, 30, 0]));
        let bytes = pdf(&image).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        // 超过 200 英寸的长图缩小页面后照常输出。
        assert!(pdf(&RgbaImage::new(2, 20_000)).unwrap().starts_with(b"%PDF-"));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("screen.pdf");
        // 设置里的默认格式是 PNG；对话框里改存 .pdf 时按扩展名编码。
        write_file(&path, &image, &ExportSettings::default()).unwrap();
        assert!(fs::read(&path).unwrap().starts_with(b"%PDF-"));
        let png = directory.path().join("screen.png");
        write_file(&png, &image, &ExportSettings { format: ExportFormat::Pdf, ..ExportSettings::default() }).unwrap();
        assert_eq!(image::guess_format(&fs::read(&png).unwrap()).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn names_use_prefix_timestamp_dimensions_and_correct_extension() {
        let mut settings = ExportSettings::default();
        assert_eq!(
            filename(&settings, "20261113-143005", 640, 480).unwrap(),
            "SnapDesk-20261113-143005.png"
        );
        settings.filename_prefix = "截图".into();
        settings.include_dimensions = true;
        settings.format = ExportFormat::Jpeg;
        assert_eq!(
            filename(&settings, "20261113-143005", 640, 480).unwrap(),
            "截图-20261113-143005-640x480.jpg"
        );
        settings.filename_prefix = "../escape".into();
        assert!(filename(&settings, "20261113-143005", 640, 480).is_err());
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

    #[test]
    fn write_file_replaces_target_and_rejects_unknown_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        fs::write(&path, b"old").unwrap();
        let image = RgbaImage::from_pixel(4, 3, image::Rgba([1, 2, 3, 255]));
        write_file(&path, &image, &ExportSettings::default()).unwrap();
        let decoded = image::open(&path).unwrap().to_rgba8();
        assert_eq!(decoded, image);
        let wrong = dir.path().join("shot.gif");
        assert!(write_file(&wrong, &image, &ExportSettings::default()).is_err());
        assert!(!wrong.exists());
        // 失败的写入不留下临时文件。
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
