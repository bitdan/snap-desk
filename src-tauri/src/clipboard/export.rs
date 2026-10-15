//! 从历史原始素材导出，不读取贴图当前缩放/标注。快照与输出临时文件均由 RAII 清理。
use super::{
    model::{validate_ids, Kind},
    service::Service,
};
use ab_glyph::{point, Font, FontRef, ScaleFont};
use image::{ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Cursor, Seek, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

const FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSansSC.ttf");
const MAX_PIXELS: u64 = 32_000_000;
struct ExportGuard<'a>(&'a std::sync::atomic::AtomicBool);
impl Drop for ExportGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}
#[derive(Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Pdf,
    LongPng,
    Separate,
}
#[derive(Clone, Copy, PartialEq, Deserialize)]
pub enum PageSize {
    A4,
    #[serde(rename = "original")]
    Original,
}
#[derive(Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    White,
    Transparent,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub ids: Vec<i64>,
    pub mode: Mode,
    pub page_size: PageSize,
    pub gap_px: u32,
    pub margin_mm: f32,
    pub background: Background,
}
impl Options {
    fn validate(&self) -> Result<(), String> {
        validate_ids(&self.ids)?;
        if self.gap_px > 200
            || !self.margin_mm.is_finite()
            || !(0.0..=30.0).contains(&self.margin_mm)
        {
            return Err("间距须为 0～200px，边距为 0～30mm。".into());
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    status: &'static str,
    output_paths: Vec<String>,
}
/// 编码文件拷贝到临时快照，不把一百张图片解码后同时放进内存。
enum Snapshot {
    Text(String),
    Image(PathBuf, u32, u32),
}
fn io_error(_: impl std::fmt::Display) -> String {
    "导出读写失败，请检查目标目录权限和剩余空间。".into()
}
fn font() -> Result<FontRef<'static>, String> {
    FontRef::try_from_slice(FONT).map_err(|_| "内置中文字体不可用。".into())
}

/// CJK 按字换行，保留空行；不支持的字形明确报错，不能静默导出方框。
fn lines(text: &str, width: f32, size: f32) -> Result<Vec<String>, String> {
    let font = font()?;
    let units = font.units_per_em().ok_or("字体度量不可用。")?;
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0.0;
    for c in text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
        .chars()
    {
        if c == '\n' {
            lines.push(std::mem::take(&mut line));
            used = 0.0;
            continue;
        }
        let glyph = font.glyph_id(c);
        if glyph.0 == 0 {
            return Err(format!(
                "内置字体缺少 U+{:04X} 字形，请改用分别导出 TXT。",
                c as u32
            ));
        }
        let advance = font.h_advance_unscaled(glyph) / units * size;
        if used + advance > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            used = 0.0;
        }
        line.push(c);
        used += advance;
    }
    lines.push(line);
    Ok(lines)
}
fn snapshot(service: &Service, ids: &[i64], dir: &Path) -> Result<Vec<Snapshot>, String> {
    let data = service.data()?;
    let _guard = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
    let mut snapshots = Vec::new();
    let mut total = 0u64;
    for (index, id) in ids.iter().enumerate() {
        let row = data.repo.get(*id)?;
        total = total.saturating_add(row.item.byte_size);
        if total > 256 * 1024 * 1024 {
            return Err("所选素材超过 256 MiB，请分批导出。".into());
        }
        match row.item.kind {
            Kind::Text => snapshots.push(Snapshot::Text(row.text.ok_or("文本记录损坏。")?)),
            Kind::Image => {
                let target = dir.join(format!("{index}.png"));
                fs::copy(
                    data.blobs.path(&row.blob.ok_or("图片引用损坏。")?)?,
                    &target,
                )
                .map_err(io_error)?;
                snapshots.push(Snapshot::Image(
                    target,
                    row.item.width.ok_or("图片尺寸损坏。")?,
                    row.item.height.ok_or("图片尺寸损坏。")?,
                ));
            }
        }
    }
    Ok(snapshots)
}
fn text_image(text: &str) -> Result<RgbaImage, String> {
    let font = font()?;
    let size = 24.0;
    let scaled = font.as_scaled(size);
    // ab_glyph 的 scale 表示字形高度，换行宽度换算为同一 em 比例。
    let em = size * font.units_per_em().ok_or("字体度量无效。")? / font.height_unscaled();
    let lines = lines(text, 1152.0, em)?;
    let height = 48u64 + lines.len() as u64 * 34;
    if height * 1200 > MAX_PIXELS {
        return Err("文本长图超过 3200 万像素，请改用 PDF 或 TXT。".into());
    }
    let mut image = RgbaImage::from_pixel(1200, height as u32, Rgba([255, 255, 255, 255]));
    for (index, line) in lines.iter().enumerate() {
        let mut x = 24.0;
        for c in line.chars() {
            let id = font.glyph_id(c);
            let glyph = id.with_scale_and_position(
                size,
                point(x, 24.0 + scaled.ascent() + index as f32 * 34.0),
            );
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if px >= 0 && py >= 0 && px < image.width() as i32 && py < image.height() as i32
                    {
                        let v = (255.0 * (1.0 - coverage)) as u8;
                        let pixel = image.get_pixel_mut(px as u32, py as u32);
                        for channel in &mut pixel.0[..3] {
                            *channel = (*channel).min(v);
                        }
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    Ok(image)
}
fn dimensions(item: &Snapshot) -> Result<(u32, u32), String> {
    match item {
        Snapshot::Image(_, w, h) => Ok((*w, *h)),
        Snapshot::Text(t) => {
            let f = font()?;
            let em = 24.0 * f.units_per_em().ok_or("字体度量无效。")? / f.height_unscaled();
            Ok((
                1200,
                48 + u32::try_from(lines(t, 1152.0, em)?.len()).map_err(io_error)? * 34,
            ))
        }
    }
}
fn long_png<W: Write + Seek>(
    items: &[Snapshot],
    options: &Options,
    file: &mut W,
) -> Result<(), String> {
    let dims = items
        .iter()
        .map(dimensions)
        .collect::<Result<Vec<_>, _>>()?;
    let width = dims.iter().map(|(w, _)| *w).max().unwrap_or(1).min(2400);
    let heights = dims
        .iter()
        .map(|(w, h)| ((u64::from(*h) * u64::from(width)) / u64::from((*w).max(1))).max(1))
        .collect::<Vec<_>>();
    let height = heights.iter().sum::<u64>()
        + u64::from(options.gap_px) * items.len().saturating_sub(1) as u64;
    if height * u64::from(width) > MAX_PIXELS {
        return Err("长图超过 3200 万像素，请减少素材或改用 PDF。".into());
    }
    let bg = if options.background == Background::White {
        [255, 255, 255, 255]
    } else {
        [0, 0, 0, 0]
    };
    let mut output = RgbaImage::from_pixel(width, height as u32, Rgba(bg));
    let mut y = 0;
    for (item, h) in items.iter().zip(heights) {
        let image = match item {
            Snapshot::Text(t) => text_image(t)?,
            Snapshot::Image(p, _, _) => crate::pin_content::read_image(p)?,
        };
        let scaled = image::imageops::resize(
            &image,
            width,
            h as u32,
            image::imageops::FilterType::Triangle,
        );
        image::imageops::overlay(&mut output, &scaled, 0, y);
        y += h as i64 + i64::from(options.gap_px);
    }
    output.write_to(file, ImageFormat::Png).map_err(io_error)
}
fn pdf<W: Write>(items: &[Snapshot], options: &Options, file: &mut W) -> Result<(), String> {
    // 先检查整批预算，避免处理到第八张 4K 图片才发现超限。
    let pixels = items.iter().try_fold(0u64, |total, item| {
        let count = match item {
            Snapshot::Image(_, w, h) => u64::from(*w) * u64::from(*h),
            Snapshot::Text(_) => 0,
        };
        total.checked_add(count).filter(|sum| *sum <= 64_000_000)
    });
    if pixels.is_none() {
        return Err("PDF 图片总量超过 6400 万像素，请分批导出。".into());
    }
    use printpdf::*;
    let doc = PdfDocument::empty("Snap Desk 剪贴板素材");
    let pdf_font = doc
        .add_external_font(Cursor::new(FONT))
        .map_err(|_| "PDF 字体嵌入失败。")?;
    let mut pages = 0;
    for item in items {
        match item {
            Snapshot::Text(text) => {
                let margin = options.margin_mm;
                let font_size = 11.0;
                let wrapped = lines(text, (210.0 - 2.0 * margin) * 72.0 / 25.4, font_size)?;
                let per_page = ((297.0 - 2.0 * margin - 6.0) / 5.5).floor() as usize;
                for chunk in wrapped.chunks(per_page.max(1)) {
                    pages += 1;
                    if pages > 1000 {
                        return Err("PDF 超过 1000 页，请分批导出。".into());
                    }
                    let (page, layer) = doc.add_page(Mm(210.0), Mm(297.0), "文本");
                    let layer = doc.get_page(page).get_layer(layer);
                    for (i, line) in chunk.iter().enumerate() {
                        layer.use_text(
                            line,
                            font_size,
                            Mm(margin),
                            Mm(297.0 - margin - 5.0 - i as f32 * 5.5),
                            &pdf_font,
                        );
                    }
                }
            }
            Snapshot::Image(path, w, h) => {
                let image = crate::pin_content::read_image(path)?;
                // PDF 采用白色纸面，先正确合成透明像素，避免透明处变黑。
                let rgb = crate::image_io::white_background(&image);
                let margin = options.margin_mm;
                let (pw, ph) = if options.page_size == PageSize::Original {
                    (
                        *w as f32 * 25.4 / 96.0 + margin * 2.0,
                        *h as f32 * 25.4 / 96.0 + margin * 2.0,
                    )
                } else {
                    (210.0, 297.0)
                };
                let scale = ((pw - margin * 2.0) / (*w as f32 * 25.4 / 96.0))
                    .min((ph - margin * 2.0) / (*h as f32 * 25.4 / 96.0));
                let (page, layer) = doc.add_page(Mm(pw), Mm(ph), "图片");
                let image = Image::from(ImageXObject {
                    width: Px(*w as usize),
                    height: Px(*h as usize),
                    color_space: ColorSpace::Rgb,
                    bits_per_component: ColorBits::Bit8,
                    interpolate: true,
                    image_data: rgb.into_raw(),
                    image_filter: None,
                    smask: None,
                    clipping_bbox: None,
                });
                image.add_to_layer(
                    doc.get_page(page).get_layer(layer),
                    ImageTransform {
                        translate_x: Some(Mm((pw - *w as f32 * 25.4 / 96.0 * scale) / 2.0)),
                        translate_y: Some(Mm((ph - *h as f32 * 25.4 / 96.0 * scale) / 2.0)),
                        scale_x: Some(scale),
                        scale_y: Some(scale),
                        dpi: Some(96.0),
                        ..Default::default()
                    },
                );
                pages += 1;
            }
        }
    }
    let bytes = doc.save_to_bytes().map_err(|_| "PDF 编码失败。")?;
    file.write_all(&bytes).map_err(io_error)
}

#[tauri::command]
pub async fn clipboard_export(
    app: AppHandle,
    window: WebviewWindow,
    options: Options,
) -> Result<ExportResult, String> {
    crate::capture::require_window(&window, "main")?;
    options.validate()?;
    tauri::async_runtime::spawn_blocking(move || {
        let service = app.state::<Service>();
        service
            .exporting
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| "已有素材正在导出，请完成或取消当前保存对话框。")?;
        let _export = ExportGuard(&service.exporting);
        let temp = tempfile::tempdir().map_err(io_error)?;
        let items = snapshot(&app.state::<Service>(), &options.ids, temp.path())?;
        let cancelled = || ExportResult {
            status: "cancelled",
            output_paths: vec![],
        };
        let remember = app
            .state::<crate::settings::SettingsState>()
            .0
            .lock()
            .map_err(|_| "导出设置不可用。")?
            .value
            .export
            .remember_directory;
        let mut dialog = app.dialog().file().set_title("导出剪贴板素材");
        if remember {
            if let Some(dir) = crate::image_io::current_directory(&app)? {
                dialog = dialog.set_directory(dir);
            }
        }
        let mut outputs = Vec::new();
        let directory;
        if options.mode == Mode::Separate {
            let Some(path) = dialog.blocking_pick_folder() else {
                return Ok(cancelled());
            };
            let parent = path.into_path().map_err(io_error)?;
            directory = parent.clone();
            // 在用户选择的目录内创建独立批次文件夹，不静默覆盖任何已有文件。
            let staging = tempfile::Builder::new()
                .prefix("SnapDesk-export-")
                .tempdir_in(&parent)
                .map_err(io_error)?;
            for (index, item) in items.iter().enumerate() {
                let path = staging.path().join(format!(
                    "{:03}.{}",
                    index + 1,
                    if matches!(item, Snapshot::Text(_)) {
                        "txt"
                    } else {
                        "png"
                    }
                ));
                match item {
                    Snapshot::Text(t) => fs::write(&path, t.as_bytes()).map_err(io_error)?,
                    Snapshot::Image(p, _, _) => {
                        fs::copy(p, &path).map_err(io_error)?;
                    }
                }
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .and_then(|f| f.sync_all())
                    .map_err(io_error)?;
                outputs.push(path.to_string_lossy().into_owned());
            }
            // 全部成功才保留目录；任意失败由 TempDir 删除本批次，不留下半套输出。
            let _saved = staging.keep();
        } else {
            let ext = if options.mode == Mode::Pdf {
                "pdf"
            } else {
                "png"
            };
            let Some(path) = dialog
                .add_filter("导出文件", &[ext])
                .set_file_name(format!("SnapDesk-{}.{}", super::service::now(), ext))
                .blocking_save_file()
            else {
                return Ok(cancelled());
            };
            let path = path.into_path().map_err(io_error)?;
            if !path
                .extension()
                .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case(ext))
            {
                return Err(format!("请选择 .{ext} 扩展名。"));
            }
            directory = path.parent().ok_or("导出路径无效。")?.to_path_buf();
            let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(io_error)?;
            if options.mode == Mode::Pdf {
                pdf(&items, &options, file.as_file_mut())?;
            } else {
                long_png(&items, &options, file.as_file_mut())?;
            }
            file.flush().map_err(io_error)?;
            file.as_file().sync_all().map_err(io_error)?;
            file.persist(&path).map_err(io_error)?;
            outputs.push(path.to_string_lossy().into_owned());
        }
        if remember {
            let _ = crate::image_io::remember_export_directory(&app, &directory);
        }
        Ok(ExportResult {
            status: "saved",
            output_paths: outputs,
        })
    })
    .await
    .map_err(|_| "导出任务执行失败。".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Options {
        Options {
            ids: vec![1],
            mode: Mode::Pdf,
            page_size: PageSize::A4,
            gap_px: 16,
            margin_mm: 12.0,
            background: Background::White,
        }
    }

    #[test]
    fn rejects_4k_batches_before_reading_any_image() {
        // 路径故意不存在；必须先返回预算错误，不能先解码前几张图。
        let items: Vec<_> = (0..8)
            .map(|_| Snapshot::Image("missing.png".into(), 3840, 2160))
            .collect();
        assert!(pdf(&items, &options(), &mut Vec::new())
            .unwrap_err()
            .contains("6400"));
        let items: Vec<_> = (0..10)
            .map(|_| Snapshot::Image("missing.png".into(), 3840, 2160))
            .collect();
        assert!(long_png(&items, &options(), &mut Cursor::new(Vec::new()))
            .unwrap_err()
            .contains("3200"));
    }

    #[test]
    fn transparent_png_and_mixed_text_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alpha.png");
        RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 0]))
            .save(&path)
            .unwrap();
        let items = vec![Snapshot::Image(path, 8, 8)];
        let mut settings = options();
        for (background, expected) in [
            (Background::Transparent, [0, 0, 0, 0]),
            (Background::White, [255, 255, 255, 255]),
        ] {
            settings.background = background;
            let mut output = Cursor::new(Vec::new());
            long_png(&items, &settings, &mut output).unwrap();
            let image = image::load_from_memory(output.get_ref())
                .unwrap()
                .to_rgba8();
            assert_eq!(image.get_pixel(0, 0).0, expected);
        }
        let mut items = items;
        items.push(Snapshot::Text("中文 English 123\n\n透明图片".into()));
        let mut output = Vec::new();
        pdf(&items, &settings, &mut output).unwrap();
        assert!(output.starts_with(b"%PDF-"));
    }

    /// 用可定位的故障写入器模拟磁盘写满，不实际填满开发者磁盘。
    struct FullDisk(Cursor<Vec<u8>>);
    impl Write for FullDisk {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Seek for FullDisk {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(position)
        }
    }
    #[test]
    fn disk_full_is_reported_for_both_encoders() {
        let items = vec![Snapshot::Text("中文 English".into())];
        assert!(
            pdf(&items, &options(), &mut FullDisk(Cursor::new(Vec::new())))
                .unwrap_err()
                .contains("剩余空间")
        );
        assert!(
            long_png(&items, &options(), &mut FullDisk(Cursor::new(Vec::new())))
                .unwrap_err()
                .contains("剩余空间")
        );
    }

    #[test]
    fn very_long_text_is_rejected_before_canvas_allocation() {
        let items = vec![Snapshot::Text("中文 English\n".repeat(20_000))];
        assert!(long_png(&items, &options(), &mut Cursor::new(Vec::new()))
            .unwrap_err()
            .contains("3200"));
    }

    /// 显式运行的压力用例：避免日常测试自动分配数百 MiB。
    #[test]
    #[ignore = "4K 批量导出压力测试，需显式运行并观测进程峰值内存"]
    fn stress_4k_batch_exports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("4k.png");
        // 不同位置使用不同颜色和透明度，覆盖白底合成与透明长图路径。
        RgbaImage::from_fn(3840, 2160, |x, y| {
            Rgba([x as u8, y as u8, (x ^ y) as u8, (x.wrapping_mul(y)) as u8])
        })
        .save(&path)
        .unwrap();
        let items: Vec<_> = (0..7)
            .map(|_| Snapshot::Image(path.clone(), 3840, 2160))
            .collect();
        let mut file = fs::File::create(dir.path().join("batch.pdf")).unwrap();
        pdf(&items, &options(), &mut file).unwrap();
        assert!(file.metadata().unwrap().len() > 0);
        let mut settings = options();
        settings.background = Background::Transparent;
        let mut file = fs::File::create(dir.path().join("batch.png")).unwrap();
        long_png(&items, &settings, &mut file).unwrap();
        assert_eq!(
            image::image_dimensions(dir.path().join("batch.png")).unwrap(),
            (2400, 9546)
        );
        let mut file = fs::File::create(dir.path().join("long-text.pdf")).unwrap();
        pdf(
            &[Snapshot::Text("中文 English 123\n".repeat(20_000))],
            &settings,
            &mut file,
        )
        .unwrap();
    }
    #[test]
    fn cjk_wrap_and_missing_glyphs_are_explicit() {
        let wrapped = lines("中文测试\n\nhello", 25.0, 11.0).unwrap();
        assert_eq!(wrapped[0], "中文");
        assert_eq!(wrapped[1], "测试");
        assert_eq!(wrapped[2], "");
        assert!(lines("\u{10ffff}", 100.0, 11.0)
            .unwrap_err()
            .contains("TXT"));
        assert!(text_image("中文 English").unwrap().height() > 0);
    }
    #[test]
    fn rejects_unbounded_and_duplicate_requests() {
        let mut value = Options {
            ids: vec![1, 1],
            mode: Mode::Pdf,
            page_size: PageSize::A4,
            gap_px: 16,
            margin_mm: 12.0,
            background: Background::White,
        };
        assert!(value.validate().is_err());
        value.ids = vec![1];
        value.margin_mm = f32::NAN;
        assert!(value.validate().is_err());
    }
}
