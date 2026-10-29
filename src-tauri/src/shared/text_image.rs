//! 文字导出与桌面贴图共用的字体排版：保留空行、制表符和中英文换行。
use ab_glyph::{point, Font, FontRef, ScaleFont};
use image::{Rgba, RgbaImage};

/// 排版前校验字体，导出和贴图使用相同字形度量。
pub(crate) fn font(bytes: &[u8]) -> Result<FontRef<'_>, String> {
    FontRef::try_from_slice(bytes).map_err(|_| "导出字体不可用，请在通用设置重新选择。".into())
}

/// CJK 按字换行，保留空行；不支持的字形明确报错，不能静默导出方框。
pub(crate) fn lines_with_font(
    text: &str,
    width: f32,
    size: f32,
    font_bytes: &[u8],
) -> Result<Vec<String>, String> {
    let font = font(font_bytes)?;
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
                "所选字体缺少 U+{:04X} 字形，请更换字体或分别导出 TXT。",
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
/// 按目标宽度生成白底文字图片；分配像素缓冲前检查调用方的容量限制。
pub(crate) fn render(
    text: &str,
    font_bytes: &[u8],
    width: u32,
    max_pixels: u64,
) -> Result<RgbaImage, String> {
    if width < 64 {
        return Err("文字图片宽度无效。".into());
    }
    let font = font(font_bytes)?;
    let size = 24.0;
    let scaled = font.as_scaled(size);
    // ab_glyph 的 scale 表示字形高度，换行宽度换算为同一 em 比例。
    let em = size * font.units_per_em().ok_or("字体度量无效。")? / font.height_unscaled();
    let lines = lines_with_font(text, (width - 48) as f32, em, font_bytes)?;
    let height = 48u64 + lines.len() as u64 * 34;
    if height * u64::from(width) > max_pixels {
        return Err("文字图片超过像素上限，请减少文字后重试。".into());
    }
    let mut image = RgbaImage::from_pixel(width, height as u32, Rgba([255, 255, 255, 255]));
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

#[cfg(test)]
mod tests {
    use super::*;
    // 复用导出测试字体，覆盖中英文与排版；不将测试字体嵌入发布程序。
    const FONT: &[u8] = include_bytes!("../../tests/fixtures/export-test.ttf");

    #[test]
    fn text_pins_preserve_lines_wrap_and_draw_visible_pixels() {
        let image = render("中文\n\nEnglish", FONT, 640, 16_000_000).unwrap();
        assert_eq!(image.dimensions(), (640, 48 + 3 * 34));
        assert!(image.pixels().any(|pixel| pixel.0[0] < 255));
        let wrapped = render(&"text ".repeat(100), FONT, 640, 16_000_000).unwrap();
        assert!(wrapped.height() > 82);
        assert_eq!(
            lines_with_font("a\r\n\tb", 500.0, 24.0, FONT).unwrap(),
            vec!["a", "    b"]
        );
    }

    #[test]
    fn rejects_invalid_fonts_missing_glyphs_and_oversized_text_images() {
        assert!(render("hello", b"invalid", 640, 16_000_000).is_err());
        assert!(render("\u{10ffff}", FONT, 640, 16_000_000).is_err());
        assert!(render("hello", FONT, 640, 1).is_err());
        assert!(render("hello", FONT, 0, 16_000_000).is_err());
    }
}
