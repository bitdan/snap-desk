//! 在 Rust 侧生成文字、序号与局部滤镜（马赛克、模糊、放大镜）的受限 PNG。
//!
//! 几何、尺寸上限、透明度规则与界面绘制保持一致，
//! 结果写入 `Annotation::raster`，预览与导出都走 `annotations::render`，两者像素一致。
//! 滤镜直接复用录屏实时滤镜 `live_filter` 的取样，避免两套算法。
//! 字体：无衬线/衬线/等宽各取本机字体，缺字时回退到无衬线（中文）字体；字体文件只在栅格化时读取，
//! 用完即释放，不常驻内存。
use super::{
    edit::{bounds, clamp, is_raster, magnifier_source, with_bounds, Bounds},
    live_filter, Annotation, Kind,
};
use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{Rgba, RgbaImage};

/// 单个标注 PNG 的 base64 长度上限（与 `annotations::raster` 解码检查一致）。
const MAX_RASTER_CHARS: usize = 2_000_000;
/// 行高系数与内边距，与前端 `annotationTextLayout` 一致。
const LINE_HEIGHT: f64 = 1.3;
const PADDING: f64 = 2.0;
/// 文字背景框的水平 / 垂直内边距与圆角半径（单位：字号 em）。行高本身已在字形上下各留约 0.15 em，
/// 垂直内边距因此比水平小。
const BOX_PAD_X: f64 = 0.35;
const BOX_PAD_Y: f64 = 0.15;
const BOX_RADIUS: f64 = 0.25;
/// 描边宽度上限（em）：再粗字形内部的空隙会被填满，文字认不出来。
const MAX_OUTLINE: f64 = 0.3;

/// 有背景框或描边的文字：字形、描边与底色叠在同一张 PNG 里，改色时不能只替换颜色。
pub fn decorated(shape: &Annotation) -> bool {
    shape.background.is_some() || shape.outline.is_some()
}

/// 描边实际宽度（原图像素）：`width` 限制在字号的 [`MAX_OUTLINE`] 以内。
fn outline_width(shape: &Annotation) -> f64 {
    let size = shape.font_size.unwrap_or(24.0);
    shape.width.min(size * MAX_OUTLINE).max(0.5)
}

/// 文字图片四周留白（原图像素，水平、垂直）：基础 2 像素；描边向外扩出描边宽度；
/// 背景框按字号留内边距，内边距不小于描边宽度，描边不会画出框外。
pub fn text_inset(shape: &Annotation) -> (f64, f64) {
    let size = shape.font_size.unwrap_or(24.0);
    let outline = if shape.outline.is_some() {
        outline_width(shape)
    } else {
        0.0
    };
    let (x, y) = if shape.background.is_some() {
        (size * BOX_PAD_X, size * BOX_PAD_Y)
    } else {
        (0.0, 0.0)
    };
    (PADDING + outline.max(x), PADDING + outline.max(y))
}

/// 背景框与描边让字形比无装饰文字向右下多偏移的距离（原图像素）。输入框按无装饰文字摆放，
/// 编辑器提交、重新编辑和切换装饰时用它换算起点，字形停在输入时的位置，框和描边向四周展开。
pub fn decoration_offset(shape: &Annotation) -> (f64, f64) {
    let (x, y) = text_inset(shape);
    (x - PADDING, y - PADDING)
}

/// 一组可用字体（原始字节）。`sans` 必须可用，其余缺失时回退 `sans`。
pub struct Fonts {
    pub sans: Vec<u8>,
    pub serif: Option<Vec<u8>>,
    pub mono: Option<Vec<u8>>,
    /// 序号用的粗体；缺失时用 `sans`。
    pub bold: Option<Vec<u8>>,
}

fn read_font(paths: &[std::path::PathBuf]) -> Option<Vec<u8>> {
    paths.iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        FontRef::try_from_slice(&bytes).is_ok().then_some(bytes)
    })
}

impl Fonts {
    /// 读取本机字体。`sans` 优先用调用方提供的用户字体（“通用设置”里的导出字体），
    /// 否则用 `pins::text::font_bytes` 的系统中文字体；`family` 指定时只额外读取该字体，避免整套常驻。
    pub fn system(
        preferred: Option<Vec<u8>>,
        family: Option<&str>,
        bold: bool,
    ) -> Result<Self, String> {
        let sans = crate::pins::text::font_bytes(preferred)?;
        #[cfg(target_os = "windows")]
        let (serif, mono, heavy) = {
            let root = std::path::PathBuf::from(
                std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()),
            )
            .join("Fonts");
            (
                vec![root.join("simsun.ttc"), root.join("times.ttf")],
                vec![root.join("consola.ttf"), root.join("cour.ttf")],
                vec![root.join("msyhbd.ttc"), root.join("arialbd.ttf")],
            )
        };
        #[cfg(target_os = "macos")]
        let (serif, mono, heavy) = (
            vec![
                "/System/Library/Fonts/Supplemental/Songti.ttc".into(),
                "/System/Library/Fonts/Supplemental/Times New Roman.ttf".into(),
            ],
            vec![
                "/System/Library/Fonts/Menlo.ttc".into(),
                "/System/Library/Fonts/Monaco.ttf".into(),
            ],
            vec!["/System/Library/Fonts/Supplemental/Arial Bold.ttf".into()],
        );
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let (serif, mono, heavy): (
            Vec<std::path::PathBuf>,
            Vec<std::path::PathBuf>,
            Vec<std::path::PathBuf>,
        ) = (
            vec![
                "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc".into(),
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf".into(),
            ],
            vec!["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".into()],
            vec![
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc".into(),
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".into(),
            ],
        );
        Ok(Self {
            sans,
            serif: (family == Some("serif"))
                .then(|| read_font(&serif))
                .flatten(),
            mono: (family == Some("mono")).then(|| read_font(&mono)).flatten(),
            bold: bold.then(|| read_font(&heavy)).flatten(),
        })
    }

    /// 某一字体族的首选字体与中文回退字体。
    pub(crate) fn stack(&self, family: &str) -> Result<Vec<FontRef<'_>>, String> {
        let mut stack = Vec::new();
        let preferred = match family {
            "serif" => self.serif.as_deref(),
            "mono" => self.mono.as_deref(),
            "bold" => self.bold.as_deref(),
            _ => None,
        };
        if let Some(bytes) = preferred {
            stack.push(crate::text_image::font(bytes)?);
        }
        stack.push(crate::text_image::font(&self.sans)?);
        Ok(stack)
    }
}

/// 字体栈中第一个含该字形的字体；都没有时报错，不能静默导出方框。
pub(crate) fn glyph_font<'a>(stack: &'a [FontRef<'a>], c: char) -> Result<&'a FontRef<'a>, String> {
    stack
        .iter()
        .find(|font| font.glyph_id(c).0 != 0)
        .ok_or_else(|| format!("所选字体缺少 U+{:04X} 字形，请更换字体。", c as u32))
}

/// ab_glyph 的 PxScale 是字形高度（ascent - descent），换算成 CSS 字号（em）。
pub(crate) fn em_scale(font: &FontRef<'_>, size: f64) -> PxScale {
    let units = font.units_per_em().unwrap_or(1000.0);
    PxScale::from(size as f32 * font.height_unscaled() / units)
}

pub(crate) fn advance(stack: &[FontRef<'_>], c: char, size: f64) -> Result<f64, String> {
    if c == '\t' {
        return Ok(advance(stack, ' ', size)? * 4.0);
    }
    let font = glyph_font(stack, c)?;
    Ok(f64::from(
        font.as_scaled(em_scale(font, size))
            .h_advance(font.glyph_id(c)),
    ))
}

/// 文字排版结果：原图像素中的行、宽、高。
pub struct TextLayout {
    pub lines: Vec<String>,
    pub width: f64,
    pub height: f64,
}

/// 按字换行（中英文一致），保留空行；宽度上限 2048，规则与前端 `annotationTextLayout` 相同。
/// `inset` 为四周留白（见 [`text_inset`]），无装饰文字为 2 像素。
pub fn text_layout(
    text: &str,
    size: f64,
    available_width: f64,
    fonts: &Fonts,
    family: &str,
    inset: (f64, f64),
) -> Result<TextLayout, String> {
    let stack = fonts.stack(family)?;
    let max_width = available_width.floor().clamp(1.0, 2048.0);
    let mut lines = Vec::new();
    let mut widest = 0.0_f64;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    for paragraph in text.split('\n') {
        let (mut line, mut used) = (String::new(), 0.0);
        for c in paragraph.chars() {
            let width = advance(&stack, c, size)?;
            if !line.is_empty() && used + width > max_width - inset.0 * 2.0 {
                widest = widest.max(used);
                lines.push(std::mem::take(&mut line));
                used = 0.0;
            }
            line.push(c);
            used += width;
        }
        widest = widest.max(used);
        lines.push(line);
    }
    Ok(TextLayout {
        width: max_width.min((widest + inset.0 * 2.0).ceil().max(1.0)),
        height: (lines.len() as f64 * size * LINE_HEIGHT + inset.1 * 2.0)
            .ceil()
            .max(1.0),
        lines,
    })
}

fn parse_color(text: &str) -> Result<[u8; 3], String> {
    super::color(text)
}

/// 用覆盖率以 max 方式写入单色字形，重叠字形不会加深。
pub(crate) fn draw_text(
    image: &mut RgbaImage,
    stack: &[FontRef<'_>],
    text: &str,
    size: f64,
    origin: (f64, f64),
    rgb: [u8; 3],
) -> Result<f64, String> {
    let mut x = origin.0;
    for c in text.chars() {
        if c == '\t' {
            x += advance(stack, ' ', size)? * 4.0;
            continue;
        }
        let font = glyph_font(stack, c)?;
        let scale = em_scale(font, size);
        let scaled = font.as_scaled(scale);
        // 每个字体按自己的上下伸部在行内垂直居中，混排中英文基线一致。
        let baseline = origin.1 + f64::from(scaled.ascent());
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(x as f32, baseline as f32));
        if let Some(outline) = font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = gx as i64 + b.min.x as i64;
                let py = gy as i64 + b.min.y as i64;
                if px < 0
                    || py < 0
                    || px >= i64::from(image.width())
                    || py >= i64::from(image.height())
                {
                    return;
                }
                let pixel = image.get_pixel_mut(px as u32, py as u32);
                let alpha = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
                if alpha > pixel[3] {
                    *pixel = Rgba([rgb[0], rgb[1], rgb[2], alpha]);
                }
            });
        }
        x += f64::from(scaled.h_advance(id));
    }
    Ok(x)
}

/// 行高内垂直居中时字形顶部到行顶的偏移（按首选字体度量）。
fn line_offset(stack: &[FontRef<'_>], size: f64) -> f64 {
    let font = &stack[0];
    let scaled = font.as_scaled(em_scale(font, size));
    let glyph_height = f64::from(scaled.ascent() - scaled.descent());
    ((size * LINE_HEIGHT - glyph_height) / 2.0).max(0.0)
}

fn encode(image: &RgbaImage) -> Result<String, String> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let raster = STANDARD.encode(bytes.into_inner());
    if raster.len() > MAX_RASTER_CHARS {
        return Err("此标注图片过大，请缩小区域。".into());
    }
    Ok(raster)
}

/// 序号：144×144 实心圆 + 居中数字；浅色底配深色字，深色底配白字。
fn number_image(shape: &Annotation, fonts: &Fonts) -> Result<RgbaImage, String> {
    let text = shape.text.as_deref().unwrap_or_default();
    if !text
        .parse::<u16>()
        .is_ok_and(|value| (1..=999).contains(&value) && value.to_string() == text)
    {
        return Err("序号须为 1～999。".into());
    }
    let rgb = parse_color(&shape.color)?;
    let mut image = RgbaImage::new(144, 144);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let distance = (f64::from(x) + 0.5 - 72.0).hypot(f64::from(y) + 0.5 - 72.0);
        let coverage = (70.0 + 0.5 - distance).clamp(0.0, 1.0);
        if coverage > 0.0 {
            *pixel = Rgba([rgb[0], rgb[1], rgb[2], (coverage * 255.0).round() as u8]);
        }
    }
    let ink = super::contrast_ink(rgb);
    let size = if text.len() == 3 { 60.0 } else { 76.0 };
    let stack = fonts.stack("bold")?;
    let width: f64 = text
        .chars()
        .map(|c| advance(&stack, c, size))
        .sum::<Result<f64, _>>()?;
    let font = glyph_font(&stack, text.chars().next().unwrap_or('1'))?;
    let scaled = font.as_scaled(em_scale(font, size));
    // 数字没有下伸部：数字高度约 0.7 em，让其中线落在 y=76，与前端 middle 基线视觉一致。
    let glyph_top = 76.0 + size * 0.35 - f64::from(scaled.ascent());
    let mut layer = RgbaImage::new(144, 144);
    draw_text(
        &mut layer,
        &stack,
        text,
        size,
        (72.0 - width / 2.0, glyph_top),
        ink,
    )?;
    // 数字压在圆上：按覆盖率混入字色，圆的透明边缘保持不变。
    for (base, glyph) in image.pixels_mut().zip(layer.pixels()) {
        let a = f64::from(glyph[3]) / 255.0;
        for channel in 0..3 {
            base[channel] = (f64::from(glyph[channel]) * a + f64::from(base[channel]) * (1.0 - a))
                .round() as u8;
        }
    }
    Ok(image)
}

/// 文字：按 `crop` 宽度换行，高度超出时报错；超大字形只降低 PNG 采样率，原图上的尺寸不变。
fn text_image(
    shape: &Annotation,
    crop: Bounds,
    fonts: &Fonts,
) -> Result<(RgbaImage, Bounds), String> {
    let text = shape.text.as_deref().unwrap_or_default();
    if text.trim().is_empty() {
        return Err("文字不能为空。".into());
    }
    if text.chars().count() > 500 {
        return Err("文字最多 500 个字符。".into());
    }
    let size = shape.font_size.unwrap_or(24.0);
    if !size.is_finite() || !(8.0..=128.0).contains(&size) {
        return Err("文字或字体参数无效。".into());
    }
    let family = shape.font_family.as_deref().unwrap_or("sans");
    let inset = text_inset(shape);
    let layout = text_layout(text, size, crop.width, fonts, family, inset)?;
    if layout.height > crop.height + 0.5 {
        return Err("文字超出区域，请减少文字、缩小字号或扩大选区。".into());
    }
    let scale = 1.0_f64
        .min(2048.0 / layout.height)
        .min((1_048_576.0 / (layout.width * layout.height)).sqrt());
    let width = ((layout.width * scale).floor() as u32).max(1);
    let height = ((layout.height * scale).floor() as u32).max(1);
    let rgb = parse_color(&shape.color)?;
    let stack = fonts.stack(family)?;
    let mut image = RgbaImage::new(width, height);
    let offset = line_offset(&stack, size);
    for (index, line) in layout.lines.iter().enumerate() {
        let top = inset.1 + index as f64 * size * LINE_HEIGHT + offset;
        draw_text(
            &mut image,
            &stack,
            line,
            size * scale,
            (inset.0 * scale, top * scale),
            rgb,
        )?;
    }
    if decorated(shape) {
        image = decorate(&image, shape, scale)?;
    }
    let b = bounds(shape);
    let placed = Bounds {
        x: clamp(b.x, crop.x, crop.right() - layout.width),
        y: clamp(b.y, crop.y, crop.bottom() - layout.height),
        width: layout.width,
        height: layout.height,
    };
    Ok((image, placed))
}

/// 在单色字形层下垫背景框与描边，自下而上：圆角背景框 → 描边 → 字形，三层按覆盖率依次混色。
/// `scale` 为 PNG 相对原图的采样比例，描边宽度与圆角都按它换算到 PNG 像素。
fn decorate(glyphs: &RgbaImage, shape: &Annotation, scale: f64) -> Result<RgbaImage, String> {
    let size = shape.font_size.unwrap_or(24.0);
    let (w, h) = (f64::from(glyphs.width()), f64::from(glyphs.height()));
    let mut image = RgbaImage::new(glyphs.width(), glyphs.height());
    if let Some(background) = &shape.background {
        let rgb = parse_color(background)?;
        let radius = (size * BOX_RADIUS * scale).min(w / 2.0).min(h / 2.0);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let coverage = super::rounded_rect_coverage(
                f64::from(x) + 0.5,
                f64::from(y) + 0.5,
                0.0,
                0.0,
                w,
                h,
                radius,
            );
            super::blend(pixel, rgb, coverage);
        }
    }
    if let Some(outline) = &shape.outline {
        let rgb = parse_color(outline)?;
        let radius = outline_width(shape) * scale;
        let distance = glyph_distance(glyphs);
        // 距离按像素中心计；字形实际边缘约在半像素外，覆盖率在 radius～radius+1 之间过渡，外缘保持抗锯齿。
        for (pixel, d) in image.pixels_mut().zip(distance) {
            super::blend(pixel, rgb, (radius + 1.0 - f64::from(d)).clamp(0.0, 1.0));
        }
    }
    for (pixel, glyph) in image.pixels_mut().zip(glyphs.pixels()) {
        super::blend(
            pixel,
            [glyph[0], glyph[1], glyph[2]],
            f64::from(glyph[3]) / 255.0,
        );
    }
    Ok(image)
}

/// 每个像素中心到最近字形像素（覆盖率不低于一半）中心的欧氏距离。
/// 用 Felzenszwalb–Huttenlocher 平方距离变换：先逐列、再逐行求一维下包络，耗时只与像素数成正比，
/// 描边加粗也不会变慢（逐像素按半径搜索在 1024×1024 的大字上会慢到秒级）。
fn glyph_distance(glyphs: &RgbaImage) -> Vec<f32> {
    // 有限的“无穷远”：参与抛物线交点运算时不会出现 INF - INF。
    const FAR: f64 = 1e20;
    let (w, h) = (glyphs.width() as usize, glyphs.height() as usize);
    let mut grid: Vec<f64> = glyphs
        .pixels()
        .map(|p| if p[3] >= 128 { 0.0 } else { FAR })
        .collect();
    let n = w.max(h);
    let (mut f, mut d) = (vec![0.0; n], vec![0.0; n]);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h {
            f[y] = grid[y * w + x];
        }
        squared_distance_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..(y + 1) * w]);
        squared_distance_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        grid[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
    grid.into_iter().map(|value| value.sqrt() as f32).collect()
}

/// 一维平方距离变换：`d[q] = min_p (q - p)² + f[p]`。`v`、`z` 为下包络抛物线的顶点与分界，由调用方复用。
fn squared_distance_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    if f.is_empty() {
        return;
    }
    let parabola = |q: usize, p: usize| {
        ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64))
    };
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..f.len() {
        let mut s = parabola(q, v[k]);
        // z[0] 为负无穷，k 回退到 0 时循环必然结束。
        while s <= z[k] {
            k -= 1;
            s = parabola(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let offset = q as f64 - v[k] as f64;
        *out = offset * offset + f[v[k]];
    }
}

/// 已有字形改色：保留覆盖率（alpha），只替换颜色；旋转/翻转后的文字不会被重新排成水平行。
fn recolor(shape: &Annotation) -> Result<RgbaImage, String> {
    let rgb = parse_color(&shape.color)?;
    let mut image = super::raster(shape)?;
    for pixel in image.pixels_mut() {
        pixel.0[..3].copy_from_slice(&rgb);
    }
    Ok(image)
}

/// 为文字、序号与滤镜标注生成像素数据并返回新标注；其他类型原样返回。
///
/// - `source`：标注坐标所在的原图（截图为整张虚拟桌面截图，贴图为贴图当前图像）。
/// - `crop`：允许放置的范围；文字按其宽度换行并被平移回范围内。
/// - `preserve_geometry`：只改样式（颜色/透明度）时为真，文字复用已有字形只换色、不重新排版。
///   有背景框或描边的文字三层颜色叠在一张 PNG 里，无法只换色，总是按当前字号与装饰重新排版；
///   因此这类文字在贴图旋转 / 翻转后改样式会回到水平排列。调用方切换装饰前后都须要求重新排版。
pub fn rasterize(
    shape: &Annotation,
    source: &RgbaImage,
    crop: Bounds,
    fonts: &Fonts,
    preserve_geometry: bool,
) -> Result<Annotation, String> {
    if !is_raster(shape.kind) {
        return Ok(shape.clone());
    }
    let (image, placed) = match shape.kind {
        Kind::Number => (number_image(shape, fonts)?, bounds(shape)),
        Kind::Text if preserve_geometry && shape.raster.is_some() && !decorated(shape) => {
            (recolor(shape)?, bounds(shape))
        }
        Kind::Text => text_image(shape, crop, fonts)?,
        _ => {
            let b = bounds(shape);
            if b.width < 1.0 || b.height < 1.0 || source.width() == 0 || source.height() == 0 {
                return Err("文字或滤镜范围为空。".into());
            }
            if matches!(shape.kind, Kind::Magnify) {
                magnifier_source(shape)?;
            }
            let normalized = with_bounds(shape, b);
            (live_filter(source, &normalized)?, b)
        }
    };
    let opacity = if matches!(shape.kind, Kind::Text | Kind::Number | Kind::Magnify) {
        shape.opacity
    } else {
        // 遮挡滤镜固定不透明，避免敏感内容透出。
        1.0
    };
    Ok(Annotation {
        raster: Some(encode(&image)?),
        opacity,
        ..with_bounds(shape, placed)
    })
}

/// 判断某标注是否需要（重新）栅格化：新建的文字/序号/滤镜，以及移动或缩放后的滤镜
/// （滤镜要按新位置重新取样）。文字与序号的几何编辑直接缩放已有 PNG。
pub fn needs_raster(shape: &Annotation, editing: bool) -> bool {
    match shape.kind {
        Kind::Mosaic | Kind::Blur | Kind::Magnify => true,
        Kind::Text | Kind::Number => !editing || shape.raster.is_none(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{render, validate, Point};
    const FONT: &[u8] = include_bytes!("../../tests/fixtures/export-test.ttf");

    fn fonts() -> Fonts {
        Fonts {
            sans: FONT.to_vec(),
            serif: None,
            mono: None,
            bold: None,
        }
    }
    fn shape(kind: Kind, a: (f64, f64), b: (f64, f64)) -> Annotation {
        Annotation {
            kind,
            start: Point { x: a.0, y: a.1 },
            end: Point { x: b.0, y: b.1 },
            color: "#ff0000".into(),
            width: 8.0,
            opacity: 0.5,
            magnification: None,
            points: vec![],
            text: None,
            font_size: None,
            font_family: None,
            raster: None,
            line_style: Default::default(),
            arrow_heads: Default::default(),
            fill: Default::default(),
            corner_radius: 0.0,
            background: None,
            outline: None,
        }
    }
    const CROP: Bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 120.0,
    };

    #[test]
    fn text_is_laid_out_inside_crop_and_exports_visible_pixels() {
        let mut s = shape(Kind::Text, (190.0, 10.0), (190.0, 10.0));
        s.text = Some("中文 text\nEnglish".into());
        s.font_size = Some(20.0);
        s.font_family = Some("serif".into());
        let out = rasterize(&s, &RgbaImage::new(1, 1), CROP, &fonts(), false).unwrap();
        // 靠近右边缘放置的文字被平移回选区；高度为两行。
        assert!(out.end.x <= 200.0 && out.start.x < 190.0);
        assert_eq!(out.end.y - out.start.y, (2.0 * 20.0 * 1.3 + 4.0_f64).ceil());
        assert_eq!(out.opacity, 0.5);
        validate(std::slice::from_ref(&out), 200, 120).unwrap();
        let mut image = RgbaImage::new(200, 120);
        render(
            &mut image,
            std::slice::from_ref(&out),
            Point { x: 0.0, y: 0.0 },
        )
        .unwrap();
        assert!(image.pixels().any(|p| p[3] > 0 && p[0] > 0));
        // 改色复用字形：覆盖率不变，颜色替换。
        let mut blue = out.clone();
        blue.color = "#0000ff".into();
        let recolored = rasterize(&blue, &RgbaImage::new(1, 1), CROP, &fonts(), true).unwrap();
        let (a, b) = (
            crate::annotations::raster(&out).unwrap(),
            crate::annotations::raster(&recolored).unwrap(),
        );
        assert_eq!(a.dimensions(), b.dimensions());
        assert!(b
            .pixels()
            .zip(a.pixels())
            .all(|(b, a)| b[3] == a[3] && (b[3] == 0 || b.0[..3] == [0, 0, 255])));
    }

    #[test]
    fn text_background_and_outline_grow_the_box_and_bake_three_layers() {
        let fonts = fonts();
        let none = RgbaImage::new(1, 1);
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let mut plain = shape(Kind::Text, (20.0, 20.0), (20.0, 20.0));
        plain.text = Some("Ab 中".into());
        plain.font_size = Some(20.0);
        plain.font_family = Some("sans".into());
        plain.opacity = 1.0;
        let base = rasterize(&plain, &none, CROP, &fonts, false).unwrap();
        let size = |s: &Annotation| (s.end.x - s.start.x, s.end.y - s.start.y);
        assert_eq!(decoration_offset(&plain), (0.0, 0.0));
        // 背景框：左右各多 0.35 em、上下各多 0.15 em（向上取整最多差 1 像素）；
        // 框内是背景色，圆角外透明，字形为字色。
        let mut boxed = plain.clone();
        boxed.color = "#ffffff".into();
        boxed.background = Some("#ef4444".into());
        let (dx, dy) = decoration_offset(&boxed);
        assert!(near(dx, 7.0) && near(dy, 3.0));
        let out = rasterize(&boxed, &none, CROP, &fonts, false).unwrap();
        validate(std::slice::from_ref(&out), 200, 120).unwrap();
        assert!((size(&out).0 - size(&base).0 - 14.0).abs() <= 1.0);
        assert!((size(&out).1 - size(&base).1 - 6.0).abs() <= 1.0);
        assert_eq!((out.start.x, out.start.y), (20.0, 20.0));
        let image = crate::annotations::raster(&out).unwrap();
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert_eq!(image.get_pixel(1, image.height() / 2).0, [0xef, 0x44, 0x44, 255]);
        assert!(image.pixels().any(|p| p.0 == [255, 255, 255, 255]));
        // 有装饰时不能只换色（背景会被染成字色），即使要求保留几何也重新排版。
        let mut blue = out.clone();
        blue.background = Some("#3b82f6".into());
        let restyled = rasterize(&blue, &none, CROP, &fonts, true).unwrap();
        let image = crate::annotations::raster(&restyled).unwrap();
        assert_eq!(image.get_pixel(1, image.height() / 2).0, [0x3b, 0x82, 0xf6, 255]);
        assert!(image.pixels().any(|p| p.0 == [255, 255, 255, 255]));
        // 描边：四周各多出描边宽度；字形外圈为描边色，字形本身仍是字色。
        let mut outlined = plain.clone();
        outlined.width = 3.0;
        outlined.outline = Some("#ffffff".into());
        assert_eq!(decoration_offset(&outlined), (3.0, 3.0));
        let out = rasterize(&outlined, &none, CROP, &fonts, false).unwrap();
        validate(std::slice::from_ref(&out), 200, 120).unwrap();
        assert!((size(&out).0 - size(&base).0 - 6.0).abs() <= 1.0);
        let image = crate::annotations::raster(&out).unwrap();
        let white = image.pixels().filter(|p| p.0 == [255, 255, 255, 255]).count();
        let red = image.pixels().filter(|p| p.0 == [255, 0, 0, 255]).count();
        assert!(white > red && red > 0, "white={white} red={red}");
        // 描边宽度不超过 0.3 em，避免把字形内部的空隙填满。
        outlined.width = 64.0;
        let (dx, dy) = decoration_offset(&outlined);
        assert!(near(dx, 6.0) && near(dy, 6.0));
    }

    #[test]
    fn glyph_distance_is_euclidean_and_ignores_faint_fringe() {
        let mut glyphs = RgbaImage::new(12, 12);
        glyphs.put_pixel(2, 3, Rgba([0, 0, 0, 255]));
        // 覆盖率不足一半的抗锯齿边缘不算字形。
        glyphs.put_pixel(9, 9, Rgba([0, 0, 0, 100]));
        let distance = glyph_distance(&glyphs);
        assert_eq!(distance[3 * 12 + 2], 0.0);
        assert_eq!(distance[7 * 12 + 5], 5.0);
        assert!((distance[9 * 12 + 9] - 85f32.sqrt()).abs() < 1e-4);
    }

    #[test]
    fn text_wraps_and_rejects_overflow_empty_and_missing_glyphs() {
        let fonts = fonts();
        let layout = text_layout(&"word ".repeat(40), 16.0, 120.0, &fonts, "sans", (PADDING, PADDING)).unwrap();
        assert!(layout.lines.len() > 3 && layout.width <= 120.0);
        let mut s = shape(Kind::Text, (0.0, 0.0), (0.0, 0.0));
        s.text = Some("x\n".repeat(30));
        assert!(rasterize(&s, &RgbaImage::new(1, 1), CROP, &fonts, false).is_err());
        s.text = Some("  ".into());
        assert!(rasterize(&s, &RgbaImage::new(1, 1), CROP, &fonts, false).is_err());
        s.text = Some("\u{10ffff}".into());
        assert!(rasterize(&s, &RgbaImage::new(1, 1), CROP, &fonts, false).is_err());
    }

    #[test]
    fn numbers_keep_geometry_and_validate() {
        let mut s = shape(Kind::Number, (10.0, 10.0), (46.0, 46.0));
        s.text = Some("12".into());
        s.color = "#fde047".into();
        let out = rasterize(&s, &RgbaImage::new(1, 1), CROP, &fonts(), false).unwrap();
        assert_eq!((out.start.x, out.end.x), (10.0, 46.0));
        validate(std::slice::from_ref(&out), 200, 120).unwrap();
        let image = crate::annotations::raster(&out).unwrap();
        assert_eq!(image.dimensions(), (144, 144));
        // 浅色底用深色字：圆内出现明显暗于底色的像素。
        assert!(image.pixels().any(|p| p[3] == 255 && p[0] < 0x80));
        s.text = Some("0".into());
        assert!(rasterize(&s, &RgbaImage::new(1, 1), CROP, &fonts(), false).is_err());
    }

    #[test]
    fn filters_sample_source_are_opaque_and_match_export() {
        let mut source = RgbaImage::from_pixel(200, 120, Rgba([200, 30, 30, 255]));
        for x in 0..100 {
            for y in 0..120 {
                source.put_pixel(x, y, Rgba([10, 20, 250, 255]));
            }
        }
        for kind in [Kind::Mosaic, Kind::Blur, Kind::Magnify] {
            let mut s = shape(kind, (120.0, 60.0), (60.0, 20.0));
            if matches!(kind, Kind::Magnify) {
                s.magnification = Some(2.0);
            }
            let out = rasterize(&s, &source, CROP, &fonts(), false).unwrap();
            assert_eq!(
                (out.start.x, out.start.y, out.end.x, out.end.y),
                (60.0, 20.0, 120.0, 60.0)
            );
            validate(std::slice::from_ref(&out), 200, 120).unwrap();
            if !matches!(kind, Kind::Magnify) {
                assert_eq!(out.opacity, 1.0);
            }
            let mut composed = source.clone();
            render(
                &mut composed,
                std::slice::from_ref(&out),
                Point { x: 0.0, y: 0.0 },
            )
            .unwrap();
            assert_eq!(composed.get_pixel(5, 5), source.get_pixel(5, 5));
        }
        let empty = shape(Kind::Blur, (10.0, 10.0), (10.0, 40.0));
        assert!(rasterize(&empty, &source, CROP, &fonts(), false).is_err());
        let plain = shape(Kind::Rectangle, (10.0, 10.0), (40.0, 40.0));
        assert!(rasterize(&plain, &source, CROP, &fonts(), false)
            .unwrap()
            .raster
            .is_none());
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;
    use crate::annotations::Point;

    /// 依赖本机字体，CI 不跑：`cargo test -p snap-core system_fonts -- --ignored`。
    #[test]
    #[ignore]
    fn system_fonts_render_every_family_and_numbers() {
        for family in ["sans", "serif", "mono"] {
            let fonts = Fonts::system(None, Some(family), true).unwrap();
            let shape = Annotation {
                kind: Kind::Text,
                start: Point { x: 0.0, y: 0.0 },
                end: Point { x: 0.0, y: 0.0 },
                color: "#ef4444".into(),
                width: 3.0,
                opacity: 1.0,
                magnification: None,
                points: vec![],
                text: Some("中文标注 Text 123".into()),
                font_size: Some(24.0),
                font_family: Some(family.into()),
                raster: None,
                line_style: Default::default(),
                arrow_heads: Default::default(),
                fill: Default::default(),
                corner_radius: 0.0,
                background: None,
                outline: None,
            };
            let crop = Bounds {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 200.0,
            };
            let out = rasterize(&shape, &RgbaImage::new(1, 1), crop, &fonts, false).unwrap();
            let image = crate::annotations::raster(&out).unwrap();
            assert!(
                image.pixels().filter(|p| p[3] > 128).count() > 200,
                "{family}"
            );
            if let Ok(dir) = std::env::var("SNAP_RASTER_DUMP") {
                image.save(format!("{dir}/text-{family}.png")).unwrap();
            }
            let number = Annotation {
                kind: Kind::Number,
                text: Some("7".into()),
                start: Point { x: 0.0, y: 0.0 },
                end: Point { x: 36.0, y: 36.0 },
                font_size: None,
                font_family: None,
                ..shape
            };
            let out = rasterize(&number, &RgbaImage::new(1, 1), crop, &fonts, false).unwrap();
            if let Ok(dir) = std::env::var("SNAP_RASTER_DUMP") {
                crate::annotations::raster(&out)
                    .unwrap()
                    .save(format!("{dir}/number.png"))
                    .unwrap();
            }
        }
    }
}
