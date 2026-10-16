//! 显式运行的性能测试共享素材；不截图、不访问用户历史、不自动下载资源。
use ab_glyph::{point, Font, FontRef, ScaleFont};
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

/// 基准使用的真实模型和完整字体需按 assets/downloads.json 预先下载并校验。
pub fn assets() -> PathBuf {
    std::env::var_os("SNAP_DESK_PERF_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/perf-assets"))
}

/// 确定性的中英文截图样本，使用测试字体；固定内容使性能对比同时检查识别质量。
pub fn screenshot(width: u32, height: u32) -> RgbaImage {
    let font =
        FontRef::try_from_slice(include_bytes!("../tests/fixtures/export-test.ttf")).unwrap();
    let scaled = font.as_scaled(42.0);
    let mut image = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    for row in 0..((height - 60) / 48) {
        let mut x = 30.0;
        for ch in "中文测试 English 123 Snap Desk".chars() {
            let id = font.glyph_id(ch);
            let glyph = id.with_scale_and_position(
                42.0,
                point(x, 30.0 + scaled.ascent() + row as f32 * 48.0),
            );
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if px >= 0 && py >= 0 && px < width as i32 && py < height as i32 {
                        let color = (255.0 * (1.0 - coverage)) as u8;
                        image.put_pixel(px as u32, py as u32, Rgba([color, color, color, 255]));
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    image
}
