//! 剪贴板组合导出的编码部分：参数校验、素材快照、长图与 PDF 排版。
//! 保存对话框、暂存与恢复登记由外壳负责（Tauri `clipboard/export.rs`、GPUI 历史窗口）。
use super::{
    model::{validate_ids, Kind},
    service::Service,
};
use crate::text_image::{font, lines_with_font};
use ab_glyph::Font;
use image::{ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Cursor, Seek, Write},
    path::{Path, PathBuf},
};

pub const MAX_PIXELS: u64 = 32_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Pdf,
    LongPng,
    Separate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSize {
    A4,
    #[serde(rename = "original")]
    Original,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    White,
    Transparent,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub ids: Vec<i64>,
    /// 组合编辑器里改写过的文本，与 ids 一一对应；None 沿用历史原文，不回写历史。
    #[serde(default)]
    pub texts: Vec<Option<String>>,
    pub mode: Mode,
    pub page_size: PageSize,
    pub gap_px: u32,
    pub margin_mm: f32,
    pub background: Background,
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        validate_ids(&self.ids)?;
        if !self.texts.is_empty() && self.texts.len() != self.ids.len() {
            return Err("改写文本与素材数量不一致。".into());
        }
        if self
            .texts
            .iter()
            .flatten()
            .any(|text| text.trim().is_empty() || text.len() > 1024 * 1024)
        {
            return Err("改写后的文本不能为空，且不超过 1 MiB。".into());
        }
        if self.gap_px > 200
            || !self.margin_mm.is_finite()
            || !(0.0..=30.0).contains(&self.margin_mm)
        {
            return Err("间距须为 0～200px，边距为 0～30mm。".into());
        }
        Ok(())
    }
}
/// 编码文件拷贝到临时快照，不把一百张图片解码后同时放进内存。
pub enum Snapshot {
    Text(String),
    Image(PathBuf, u32, u32),
}
pub fn io_error(_: impl std::fmt::Display) -> String {
    "导出读写失败，请检查目标目录权限和剩余空间。".into()
}
pub fn snapshot(
    service: &Service,
    ids: &[i64],
    texts: &[Option<String>],
    dir: &Path,
) -> Result<Vec<Snapshot>, String> {
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
        let edited = texts.get(index).cloned().flatten();
        if edited.is_some() && row.item.kind != Kind::Text {
            return Err("只有文本素材可以改写。".into());
        }
        match row.item.kind {
            Kind::File => {
                return Err("文件条目仅保存引用，不支持组合导出。请复制后在资源管理器粘贴。".into())
            }
            Kind::Text => snapshots.push(Snapshot::Text(match edited {
                Some(text) => text,
                None => row.text.ok_or("文本记录损坏。")?,
            })),
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
pub fn text_image_with_font(text: &str, font_bytes: &[u8]) -> Result<RgbaImage, String> {
    crate::text_image::render(text, font_bytes, 1200, MAX_PIXELS)
}
fn dimensions(item: &Snapshot, font_bytes: &[u8]) -> Result<(u32, u32), String> {
    match item {
        Snapshot::Image(_, w, h) => Ok((*w, *h)),
        Snapshot::Text(t) => {
            let f = font(font_bytes)?;
            let em = 24.0 * f.units_per_em().ok_or("字体度量无效。")? / f.height_unscaled();
            Ok((
                1200,
                48 + u32::try_from(lines_with_font(t, 1152.0, em, font_bytes)?.len())
                    .map_err(io_error)?
                    * 34,
            ))
        }
    }
}
pub fn long_png_with_font<W: Write + Seek>(
    items: &[Snapshot],
    options: &Options,
    file: &mut W,
    font_bytes: &[u8],
) -> Result<(), String> {
    let dims = items
        .iter()
        .map(|item| dimensions(item, font_bytes))
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
            Snapshot::Text(t) => text_image_with_font(t, font_bytes)?,
            Snapshot::Image(p, _, _) => crate::pins::content::read_image(p)?,
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
pub fn pdf_with_font<W: Write>(
    items: &[Snapshot],
    options: &Options,
    file: &mut W,
    font_bytes: &[u8],
) -> Result<(), String> {
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
    // 纯图片 PDF 无需字体；文本排版与嵌入始终使用同一份资源快照。
    let pdf_font = if items.iter().any(|item| matches!(item, Snapshot::Text(_))) {
        Some(
            doc.add_external_font(Cursor::new(font_bytes))
                .map_err(|_| "PDF 字体嵌入失败。")?,
        )
    } else {
        None
    };
    let mut pages = 0;
    for item in items {
        match item {
            Snapshot::Text(text) => {
                let margin = options.margin_mm;
                let font_size = 11.0;
                let wrapped = lines_with_font(
                    text,
                    (210.0 - 2.0 * margin) * 72.0 / 25.4,
                    font_size,
                    font_bytes,
                )?;
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
                            pdf_font.as_ref().ok_or("导出字体未准备。")?,
                        );
                    }
                }
            }
            Snapshot::Image(path, w, h) => {
                let image = crate::pins::content::read_image(path)?;
                // PDF 采用白色纸面，先正确合成透明像素，避免透明处变黑。
                let rgb = crate::export::white_background(&image);
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

/// “分别保存”时批次内的文件名：001.txt / 002.png …（与前端 separateNames 一致）。
pub fn separate_names(items: &[Snapshot]) -> Vec<String> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            format!(
                "{:03}.{}",
                index + 1,
                if matches!(item, Snapshot::Text(_)) {
                    "txt"
                } else {
                    "png"
                }
            )
        })
        .collect()
}
/// 是否需要排版字体：分别保存与纯图片不需要。
pub fn needs_font(items: &[Snapshot], mode: Mode) -> bool {
    mode != Mode::Separate && items.iter().any(|item| matches!(item, Snapshot::Text(_)))
}

/// 校验参数并把所选历史复制成快照；临时目录随返回值释放（没有恢复中心的外壳使用，例如 GPUI 版）。
pub fn stage(
    service: &Service,
    options: &Options,
) -> Result<(tempfile::TempDir, Vec<Snapshot>), String> {
    options.validate()?;
    let dir = tempfile::Builder::new()
        .prefix("snapdesk-export-")
        .tempdir()
        .map_err(io_error)?;
    let items = snapshot(service, &options.ids, &options.texts, dir.path())?;
    Ok((dir, items))
}

/// 写出 PDF 或长图：先写入目标目录中的临时文件并同步到磁盘，完整成功后才改名为目标路径，
/// 失败时不留半个文件。目标已存在时覆盖（由保存对话框向用户确认）。
pub fn write_combined(
    items: &[Snapshot],
    options: &Options,
    target: &Path,
    font_bytes: &[u8],
) -> Result<(), String> {
    let ext = if options.mode == Mode::Pdf {
        "pdf"
    } else {
        "png"
    };
    if options.mode == Mode::Separate {
        return Err("分别保存请选择目录。".into());
    }
    if !target
        .extension()
        .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case(ext))
    {
        return Err(format!("请选择 .{ext} 扩展名。"));
    }
    let directory = target.parent().ok_or("导出路径无效。")?;
    let mut file = tempfile::Builder::new()
        .prefix(".snapdesk-")
        .suffix(&format!(".{ext}.tmp"))
        .tempfile_in(directory)
        .map_err(io_error)?;
    if options.mode == Mode::Pdf {
        pdf_with_font(items, options, file.as_file_mut(), font_bytes)?;
    } else {
        long_png_with_font(items, options, file.as_file_mut(), font_bytes)?;
    }
    file.as_file().sync_all().map_err(io_error)?;
    file.persist(target).map_err(|e| io_error(e.error))?;
    Ok(())
}

/// 分别保存：在 `parent` 下新建批次目录（`SnapDesk-<stamp>`，重名时追加序号），逐个写入 001.txt / 002.png …，
/// 不覆盖任何已有文件；中途失败时删除这个新建的批次目录。返回写出的文件路径。
pub fn write_separate(
    items: &[Snapshot],
    parent: &Path,
    stamp: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut directory = parent.join(format!("SnapDesk-{stamp}"));
    let mut suffix = 1;
    loop {
        match fs::create_dir(&directory) {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && suffix < 100 => {
                suffix += 1;
                directory = parent.join(format!("SnapDesk-{stamp}-{suffix}"));
            }
            Err(error) => return Err(io_error(error)),
        }
    }
    let result = (|| {
        let mut outputs = Vec::new();
        for (item, name) in items.iter().zip(separate_names(items)) {
            let path = directory.join(name);
            match item {
                Snapshot::Text(text) => fs::write(&path, text.as_bytes()).map_err(io_error)?,
                Snapshot::Image(source, _, _) => {
                    fs::copy(source, &path).map_err(io_error)?;
                }
            }
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|f| f.sync_all())
                .map_err(io_error)?;
            outputs.push(path);
        }
        Ok(outputs)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Noto Sans SC 静态字体的测试子集：ASCII 与下方用例的八个汉字，保留原始度量。
    // 来源 https://github.com/notofonts/noto-cjk；许可证在 tests/fixtures/OFL.txt。
    // 由 fontTools.subset 生成，仅编译进测试程序，生产导出使用用户下载/选择的字体。
    const FONT: &[u8] = include_bytes!("../../../../src-tauri/tests/fixtures/export-test.ttf");
    fn pdf<W: Write>(items: &[Snapshot], options: &Options, file: &mut W) -> Result<(), String> {
        pdf_with_font(items, options, file, FONT)
    }
    fn long_png<W: Write + Seek>(
        items: &[Snapshot],
        options: &Options,
        file: &mut W,
    ) -> Result<(), String> {
        long_png_with_font(items, options, file, FONT)
    }
    fn lines(text: &str, width: f32, size: f32) -> Result<Vec<String>, String> {
        lines_with_font(text, width, size, FONT)
    }
    fn text_image(text: &str) -> Result<RgbaImage, String> {
        text_image_with_font(text, FONT)
    }

    #[test]
    fn image_only_exports_do_not_need_a_font() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        RgbaImage::new(2, 2).save(&path).unwrap();
        let items = vec![Snapshot::Image(path, 2, 2)];
        pdf_with_font(&items, &options(), &mut Vec::new(), &[]).unwrap();
        long_png_with_font(&items, &options(), &mut Cursor::new(Vec::new()), &[]).unwrap();
        assert!(pdf_with_font(
            &[Snapshot::Text("text".into())],
            &options(),
            &mut Vec::new(),
            &[]
        )
        .is_err());
    }
    fn options() -> Options {
        Options {
            ids: vec![1],
            texts: vec![],
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
    fn combined_and_separate_outputs_land_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("source.png");
        RgbaImage::from_pixel(4, 3, Rgba([1, 2, 3, 255]))
            .save(&image)
            .unwrap();
        let items = vec![
            Snapshot::Text("中文 English".into()),
            Snapshot::Image(image, 4, 3),
        ];
        let mut settings = options();
        settings.mode = Mode::LongPng;
        let target = dir.path().join("out.png");
        assert!(
            write_combined(&items, &settings, &dir.path().join("out.pdf"), FONT)
                .unwrap_err()
                .contains(".png")
        );
        write_combined(&items, &settings, &target, FONT).unwrap();
        assert!(image::open(&target).unwrap().width() > 0);
        // 只留下目标文件与原素材，没有残留的临时文件。
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);

        let out = dir.path().join("batch");
        fs::create_dir(&out).unwrap();
        fs::create_dir(out.join("SnapDesk-1")).unwrap();
        let written = write_separate(&items, &out, "1").unwrap();
        assert_eq!(written[0], out.join("SnapDesk-1-2").join("001.txt"));
        assert_eq!(fs::read_to_string(&written[0]).unwrap(), "中文 English");
        assert_eq!(written[1].file_name().unwrap(), "002.png");
        // 素材缺失时整个新批次目录被删除。
        let missing = vec![Snapshot::Image(dir.path().join("missing.png"), 1, 1)];
        assert!(write_separate(&missing, &out, "2").is_err());
        assert!(!out.join("SnapDesk-2").exists());
    }

    #[test]
    fn rejects_unbounded_and_duplicate_requests() {
        let mut value = Options {
            ids: vec![1, 1],
            texts: vec![],
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
        value.margin_mm = 12.0;
        value.texts = vec![Some("  ".into())];
        assert!(value.validate().is_err());
        value.texts = vec![None, None];
        assert!(value.validate().is_err());
        value.texts = vec![Some("改写".into())];
        assert!(value.validate().is_ok());
    }
}
