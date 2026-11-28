//! 长截图结果的自动裁剪：找出两端连续的纯色空白（页面上下留白、横向长图左右留白），
//! 作为结果窗口裁剪线的初始位置。只是建议值，用户可以拖回或一键重置。
use image::RgbaImage;

/// 一行 / 一列内各像素与首像素的最大通道差不超过它，视为纯色。
const TOLERANCE: u8 = 8;
/// 裁到内容边缘后保留的留白（原图像素），避免文字贴边。
pub const MARGIN: u32 = 8;

/// 从起始端、末端可以裁掉的像素数：两端各自连续、颜色与该端第一行（列）一致的纯色行（列），
/// 减去 [`MARGIN`]；至少保留 `min_keep` 像素长度，整张都是空白时不裁。
pub fn blank_ends(image: &RgbaImage, horizontal: bool, min_keep: u32) -> (u32, u32) {
    let (w, h) = image.dimensions();
    let long = if horizontal { w } else { h };
    let cross = if horizontal { h } else { w };
    if long == 0 || cross == 0 {
        return (0, 0);
    }
    let pixel = |along: u32, across: u32| {
        if horizontal { image.get_pixel(along, across).0 } else { image.get_pixel(across, along).0 }
    };
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= TOLERANCE);
    // 第 `along` 行（列）是与 `color` 相同的纯色。
    let blank = |along: u32, color: [u8; 4]| (0..cross).all(|across| near(pixel(along, across), color));
    let run = |lines: &mut dyn Iterator<Item = u32>, first: u32| {
        let color = pixel(first, 0);
        lines.take_while(|&along| blank(along, color)).count() as u32
    };
    let head = run(&mut (0..long), 0);
    if head >= long {
        return (0, 0);
    }
    let tail = run(&mut (0..long).rev(), long - 1);
    let start = head.saturating_sub(MARGIN);
    let end = tail.saturating_sub(MARGIN);
    let room = long.saturating_sub(min_keep);
    if start + end > room {
        return (0, 0);
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn trims_blank_bands_at_both_ends_and_keeps_a_margin() {
        // 100 行：上 30 行白、中间 40 行有内容、下 30 行浅灰。
        let image = RgbaImage::from_fn(20, 100, |x, y| match y {
            0..30 => Rgba([255, 255, 255, 255]),
            30..70 => Rgba([(x * 10) as u8, 0, 0, 255]),
            _ => Rgba([240, 240, 240, 255]),
        });
        assert_eq!(blank_ends(&image, false, 16), (30 - MARGIN, 30 - MARGIN));
        let wide = RgbaImage::from_fn(100, 10, |x, y| if (40..60).contains(&x) { Rgba([0, (y * 20) as u8, 0, 255]) } else { Rgba([9, 9, 9, 255]) });
        assert_eq!(blank_ends(&wide, true, 16), (40 - MARGIN, 40 - MARGIN));
    }

    #[test]
    fn leaves_all_blank_or_too_short_results_alone() {
        let blank = RgbaImage::from_pixel(10, 50, Rgba([255, 255, 255, 255]));
        assert_eq!(blank_ends(&blank, false, 16), (0, 0));
        let content = RgbaImage::from_fn(10, 50, |x, y| Rgba([(x + y) as u8, 0, 0, 255]));
        assert_eq!(blank_ends(&content, false, 16), (0, 0));
        // 内容只有 4 行：裁完不足 min_keep 时不裁。
        let thin = RgbaImage::from_fn(10, 50, |x, y| if (23..27).contains(&y) { Rgba([x as u8 * 20, 0, 0, 255]) } else { Rgba([255, 255, 255, 255]) });
        assert_eq!(blank_ends(&thin, false, 40), (0, 0));
    }
}
