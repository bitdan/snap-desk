//! 截图翻译“覆盖原文”的纯逻辑：识别行合并为段落、原图取色与译文排版。
//!
//! 坐标单位均为识别输入图片的像素（`OcrResult::image_width/height` 所在坐标系）；
//! 界面层按显示缩放比换算后绘制。任一前提不满足（缺坐标、明显倾斜、段落过多）时返回 `None`，
//! 调用方退回普通的分栏翻译显示。
use super::ocr::{OcrLine, merge_lines};
use image::RgbaImage;

/// 批量翻译最多 50 段；超出时退回普通翻译，不截断原文。
const MAX_PARAGRAPHS: usize = 50;
/// 倾斜超过约 15° 的行（竖排、旋转文字）无法用水平文本覆盖。
const MAX_TILT: f32 = 0.268;
/// 译文行距（字号的倍数）。
pub const LINE_SPACING: f32 = 1.2;

/// 轴对齐矩形（识别像素）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 一个覆盖段落：合并后的原文、外接框与中位行高。
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub text: String,
    pub rect: Rect,
    pub line_height: f32,
}

/// 覆盖块的背景色与文字色（RGB）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub background: [u8; 3],
    pub foreground: [u8; 3],
}

fn line_rect(line: &OcrLine) -> Option<(Rect, f32)> {
    let points = line.polygon.as_ref()?;
    if points.len() < 3 || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    // 前两个顶点是文字基线方向，用它估计倾斜。
    let (dx, dy) = (points[1].x - points[0].x, points[1].y - points[0].y);
    let tilt = if dx > 0.0 { (dy / dx).abs() } else { f32::INFINITY };
    Some((Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }, tilt))
}

/// 按阅读顺序把相邻行合成段落：行高相近、上下间距小且水平方向重叠。
/// 逐行翻译会把句子拆碎，按段落翻译语义更完整。
pub fn paragraphs(lines: &[OcrLine]) -> Option<Vec<Paragraph>> {
    let mut sorted: Vec<&OcrLine> = lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by_key(|l| l.order);
    let mut groups: Vec<Vec<(&OcrLine, Rect)>> = Vec::new();
    for line in sorted {
        let (rect, tilt) = line_rect(line)?;
        if rect.width <= 0.0 || rect.height <= 0.0 || tilt > MAX_TILT {
            return None;
        }
        if let Some(group) = groups.last_mut() {
            let last = group.last().map(|(_, r)| *r).unwrap_or(rect);
            let small = last.height.min(rect.height);
            let large = last.height.max(rect.height);
            let gap = rect.y - (last.y + last.height);
            let overlap = (last.x + last.width).min(rect.x + rect.width) - last.x.max(rect.x);
            if large / small <= 1.6 && gap <= small * 0.8 && gap >= -small * 0.5 && overlap > -small {
                group.push((line, rect));
                continue;
            }
        }
        groups.push(vec![(line, rect)]);
    }
    if groups.len() > MAX_PARAGRAPHS {
        return None;
    }
    Some(
        groups
            .into_iter()
            .map(|group| {
                let x = group.iter().map(|(_, r)| r.x).fold(f32::MAX, f32::min);
                let y = group.iter().map(|(_, r)| r.y).fold(f32::MAX, f32::min);
                let right = group.iter().map(|(_, r)| r.x + r.width).fold(f32::MIN, f32::max);
                let bottom = group.iter().map(|(_, r)| r.y + r.height).fold(f32::MIN, f32::max);
                let mut heights: Vec<f32> = group.iter().map(|(_, r)| r.height).collect();
                heights.sort_by(f32::total_cmp);
                let text = group.iter().map(|(l, _)| l.text.trim()).collect::<Vec<_>>().join("\n");
                Paragraph {
                    text: merge_lines(&text),
                    rect: Rect { x, y, width: right - x, height: bottom - y },
                    line_height: heights[heights.len() / 2],
                }
            })
            .collect(),
    )
}

/// 用户改过原文后，段落译文不再对应原图；比较时忽略空白和连字符（合并行会去掉断词连字符）。
pub fn matches(text: &str, paragraphs: &[Paragraph]) -> bool {
    let compact = |value: &str| -> String {
        value.chars().filter(|c| !c.is_whitespace() && !matches!(c, '-' | '‐' | '‑')).collect()
    };
    !paragraphs.is_empty()
        && compact(text) == compact(&paragraphs.iter().map(|p| p.text.as_str()).collect::<String>())
}

/// 取色：背景取段落外沿一圈最常见的颜色（量化后取众数再求均值），可抵抗相邻文字和抗锯齿；
/// 文字色取框内与背景差异最大那部分像素的均值。`rect` 与 `image` 同一像素坐标系。
pub fn sample_colors(image: &RgbaImage, rect: Rect) -> Colors {
    let (width, height) = (image.width() as i64, image.height() as i64);
    if width == 0 || height == 0 {
        return Colors { background: [255; 3], foreground: [17; 3] };
    }
    let clamp_x = |v: i64| v.clamp(0, width - 1);
    let clamp_y = |v: i64| v.clamp(0, height - 1);
    let pad = ((rect.width.min(rect.height) * 0.15).round() as i64).max(2);
    let left = clamp_x(rect.x.floor() as i64 - pad);
    let top = clamp_y(rect.y.floor() as i64 - pad);
    let right = clamp_x((rect.x + rect.width).ceil() as i64 + pad - 1);
    let bottom = clamp_y((rect.y + rect.height).ceil() as i64 + pad - 1);
    let pixel = |x: i64, y: i64| image.get_pixel(x as u32, y as u32).0;

    let mut buckets: std::collections::HashMap<u16, [u64; 4]> = std::collections::HashMap::new();
    let mut add = |x: i64, y: i64| {
        let [r, g, b, _] = pixel(x, y);
        let key = (u16::from(r >> 4) << 8) | (u16::from(g >> 4) << 4) | u16::from(b >> 4);
        let bucket = buckets.entry(key).or_default();
        bucket[0] += u64::from(r);
        bucket[1] += u64::from(g);
        bucket[2] += u64::from(b);
        bucket[3] += 1;
    };
    for x in left..=right {
        add(x, top);
        if bottom != top {
            add(x, bottom);
        }
    }
    for y in (top + 1)..bottom {
        add(left, y);
        if right != left {
            add(right, y);
        }
    }
    let best = buckets.values().max_by_key(|b| b[3]).copied().unwrap_or([255, 255, 255, 1]);
    let bg = [0, 1, 2].map(|i| best[i] as f32 / best[3] as f32);
    let background = bg.map(|v| v.round() as u8);

    let x0 = clamp_x(rect.x.floor() as i64);
    let y0 = clamp_y(rect.y.floor() as i64);
    let x1 = clamp_x((rect.x + rect.width).ceil() as i64 - 1);
    let y1 = clamp_y((rect.y + rect.height).ceil() as i64 - 1);
    let distance = |p: [u8; 4]| {
        let d = [0, 1, 2].map(|i| f32::from(p[i]) - bg[i]);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    // 两遍扫描：先求最大色差，再对色差较大的“墨迹”像素求均值。
    let mut max_distance = 0.0f32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            max_distance = max_distance.max(distance(pixel(x, y)));
        }
    }
    // 框内几乎没有对比（纯色块）时按背景亮度给出可读的黑或白。
    if max_distance < 48.0 {
        let luma = 0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2];
        return Colors { background, foreground: if luma > 140.0 { [17; 3] } else { [245; 3] } };
    }
    let (mut sum, mut count) = ([0u64; 3], 0u64);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = pixel(x, y);
            if distance(p) >= max_distance * 0.6 {
                for i in 0..3 {
                    sum[i] += u64::from(p[i]);
                }
                count += 1;
            }
        }
    }
    let count = count.max(1);
    Colors { background, foreground: sum.map(|v| (v / count) as u8) }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xff00..=0xffef | 0x3000..=0x303f)
}

/// 中日韩逐字断行，其他文字按单词（含尾随空格）断行。
fn tokens(text: &str) -> Vec<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in normalized.chars() {
        if is_cjk(c) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            out.push(c.to_string());
        } else if c == ' ' {
            word.push(' ');
            out.push(std::mem::take(&mut word));
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn wrap(text: &str, max_width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for token in tokens(text) {
        let next = format!("{current}{token}");
        if current.is_empty() || measure(next.trim_end()) <= max_width {
            if !current.is_empty() || measure(token.trim_end()) <= max_width {
                current = next;
                continue;
            }
        } else {
            lines.push(current.trim_end().to_owned());
            current.clear();
            if measure(token.trim_end()) <= max_width {
                current = token.trim_start().to_owned();
                continue;
            }
        }
        // 单个单词比框还宽：逐字放入，避免整段溢出。
        for c in token.chars() {
            let candidate = format!("{current}{c}");
            if !current.is_empty() && measure(candidate.trim_end()) > max_width {
                lines.push(current.trim_end().to_owned());
                current.clear();
            }
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_owned());
    }
    lines
}

/// 粗略文字宽度（字号 1 时）：中日韩全角约 1em，其余约 0.55em。界面没有逐字测量时使用。
pub fn estimate_width(text: &str) -> f32 {
    text.chars()
        .map(|c| if is_cjk(c) { 1.0 } else if c == ' ' { 0.3 } else { 0.56 })
        .sum()
}

/// 译文放进原文所在的框：从原文行高起步逐步缩小字号直到全部行放得下；
/// 最小字号仍放不下时保留最小字号（允许少量溢出）。`measure(text)` 返回字号 1 时的宽度。
pub fn fit_text(
    text: &str,
    width: f32,
    height: f32,
    line_height: f32,
    min_font: f32,
    measure: &dyn Fn(&str) -> f32,
) -> (f32, Vec<String>) {
    let mut size = (line_height * 0.85).min(height / LINE_SPACING).max(min_font);
    loop {
        let lines = wrap(text, width, &|value| measure(value) * size);
        if lines.len() as f32 * size * LINE_SPACING <= height * 1.05 || size <= min_font {
            return (size, lines);
        }
        size = (size * 0.92).max(min_font);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognition::ocr::PolygonPoint;

    fn line(order: usize, text: &str, x: f32, y: f32, w: f32, h: f32) -> OcrLine {
        OcrLine {
            id: order.to_string(),
            text: text.into(),
            confidence: None,
            polygon: Some(vec![
                PolygonPoint { x, y },
                PolygonPoint { x: x + w, y },
                PolygonPoint { x: x + w, y: y + h },
                PolygonPoint { x, y: y + h },
            ]),
            language: None,
            order,
            paragraph_id: None,
        }
    }

    #[test]
    fn adjacent_lines_merge_and_distant_lines_split() {
        let lines = [
            line(0, "Hello", 10.0, 10.0, 100.0, 20.0),
            line(1, "world", 10.0, 34.0, 90.0, 20.0),
            line(2, "Footer", 10.0, 200.0, 80.0, 20.0),
        ];
        let result = paragraphs(&lines).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "Hello world");
        assert_eq!(result[0].rect, Rect { x: 10.0, y: 10.0, width: 100.0, height: 44.0 });
        assert!(matches("Hello\nworld\n\nFooter", &result));
        assert!(!matches("Hello there", &result));
    }

    #[test]
    fn missing_polygon_disables_overlay() {
        let mut bad = line(0, "x", 0.0, 0.0, 10.0, 10.0);
        bad.polygon = None;
        assert!(paragraphs(&[bad]).is_none());
    }

    #[test]
    fn samples_background_and_ink() {
        let mut image = RgbaImage::from_pixel(40, 20, image::Rgba([250, 250, 250, 255]));
        for x in 12..28 {
            for y in 8..12 {
                image.put_pixel(x, y, image::Rgba([10, 20, 30, 255]));
            }
        }
        let colors = sample_colors(&image, Rect { x: 10.0, y: 6.0, width: 20.0, height: 8.0 });
        assert_eq!(colors.background, [250, 250, 250]);
        assert_eq!(colors.foreground, [10, 20, 30]);
    }

    #[test]
    fn fit_shrinks_until_lines_fit() {
        let (size, lines) = fit_text("你好世界你好世界", 40.0, 24.0, 20.0, 8.0, &estimate_width);
        assert!(size < 17.0);
        assert!(lines.len() as f32 * size * LINE_SPACING <= 24.0 * 1.05 || size <= 8.0);
        assert_eq!(lines.concat(), "你好世界你好世界");
    }
}
