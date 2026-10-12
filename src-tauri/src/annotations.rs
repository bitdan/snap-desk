//! 将前端标注合成到裁剪后的原图。只接受结构化图形，不接收任意 SVG 或图片路径。
//! 线段按像素中心到线段的距离绘制，边缘使用覆盖率混色，支持任意方向的箭头。
use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Rectangle,
    Arrow,
    Ellipse,
    Line,
    Polyline,
    Pen,
    Highlighter,
    Text,
    Number,
    Spotlight,
    Magnify,
    Mosaic,
    Blur,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Annotation {
    pub kind: Kind,
    pub start: Point,
    pub end: Point,
    pub color: String,
    pub width: f64,
    #[serde(default = "opaque")]
    pub opacity: f64,
    /// 仅放大镜使用；前端在该倍率下采样底图，并提交受限 PNG 供预览和导出复用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnification: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster: Option<String>,
}
fn opaque() -> f64 {
    1.0
}

/// 字形/滤镜只允许受限 PNG；先检查解码尺寸再分配 RGBA，防止压缩图片导致巨量内存分配。
pub(crate) fn raster(s: &Annotation) -> Result<RgbaImage, String> {
    let value = s.raster.as_deref().ok_or("标注缺少像素数据。")?;
    if value.len() > 2_000_000 {
        return Err("单个标注像素数据过大。".into());
    }
    let bytes = STANDARD.decode(value).map_err(|_| "标注 PNG 编码无效。")?;
    let reader = image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png);
    let (w, h) = reader.into_dimensions().map_err(|_| "标注 PNG 无效。")?;
    if w == 0 || h == 0 || w > 2048 || h > 2048 || u64::from(w) * u64::from(h) > 1_048_576 {
        return Err("标注 PNG 尺寸超限。".into());
    }
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map(|image| image.to_rgba8())
        .map_err(|e| e.to_string())
}

/// 限制数量、坐标、颜色和线宽，防止无效 IPC 数据造成越界或过长绘制循环。
pub fn validate(shapes: &[Annotation], width: u32, height: u32) -> Result<(), String> {
    if shapes.len() > 100 {
        return Err("单张截图最多支持 100 个标注。".into());
    }
    if shapes
        .iter()
        .map(|s| s.raster.as_ref().map_or(0, String::len))
        .sum::<usize>()
        > 8_000_000
    {
        return Err("标注像素数据总量过大。".into());
    }
    for s in shapes {
        if !s.width.is_finite() || !(0.5..=64.0).contains(&s.width) {
            return Err("标注线宽无效。".into());
        }
        if !s.opacity.is_finite() || !(0.05..=1.0).contains(&s.opacity) || s.points.len() > 2048 {
            return Err("标注透明度或路径点数量无效。".into());
        }
        for p in [s.start, s.end].iter().chain(s.points.iter()) {
            if !p.x.is_finite()
                || !p.y.is_finite()
                || p.x < 0.0
                || p.y < 0.0
                || p.x > f64::from(width)
                || p.y > f64::from(height)
            {
                return Err("标注坐标超出截图边界。".into());
            }
        }
        if matches!(s.kind, Kind::Pen | Kind::Highlighter | Kind::Polyline) && s.points.len() < 2 {
            return Err("路径至少需要两个点。".into());
        }
        if !matches!(s.kind, Kind::Pen | Kind::Highlighter | Kind::Polyline) && !s.points.is_empty()
        {
            return Err("此标注类型不接受路径点。".into());
        }
        if matches!(
            s.kind,
            Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify
        ) {
            if s.start.x >= s.end.x || s.start.y >= s.end.y {
                return Err("文字或滤镜范围为空。".into());
            }
            raster(s)?;
        } else if s.raster.is_some() {
            return Err("此标注类型不接受像素数据。".into());
        }
        if matches!(s.kind, Kind::Mosaic | Kind::Blur) && s.opacity != 1.0 {
            return Err("遮挡滤镜必须完全不透明。".into());
        }
        if matches!(s.kind, Kind::Spotlight)
            && ((s.start.x - s.end.x).abs() < 1.0 || (s.start.y - s.end.y).abs() < 1.0)
        {
            return Err("聚光灯亮区为空。".into());
        }
        if matches!(s.kind, Kind::Magnify) {
            if s.magnification
                .is_none_or(|zoom| !zoom.is_finite() || !(2.0..=4.0).contains(&zoom))
            {
                return Err("放大倍数须为 2～4。".into());
            }
        } else if s.magnification.is_some() {
            return Err("此标注类型不接受放大倍数。".into());
        }
        // 序号沿用 text 字段传递数字，但只接受 1～999 的规范十进制表示。
        if matches!(s.kind, Kind::Number)
            && !s.text.as_deref().is_some_and(|text| {
                text.parse::<u16>()
                    .is_ok_and(|value| (1..=999).contains(&value) && value.to_string() == text)
            })
        {
            return Err("序号须为 1～999。".into());
        }
        if matches!(s.kind, Kind::Text)
            && (s
                .text
                .as_ref()
                .is_none_or(|text| text.is_empty() || text.chars().count() > 500)
                || s.font_size
                    .is_none_or(|size| !size.is_finite() || !(8.0..=128.0).contains(&size))
                || !matches!(s.font_family.as_deref(), Some("sans" | "serif" | "mono")))
        {
            return Err("文字或字体参数无效。".into());
        }
        color(&s.color)?;
    }
    Ok(())
}

fn color(text: &str) -> Result<[u8; 3], String> {
    if text.len() != 7
        || !text.starts_with('#')
        || !text[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("标注颜色必须为 #RRGGBB。".into());
    }
    let value = u32::from_str_radix(&text[1..], 16).map_err(|e| e.to_string())?;
    Ok([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

fn segments(s: &Annotation) -> Vec<(Point, Point)> {
    let a = s.start;
    let b = s.end;
    match s.kind {
        Kind::Pen | Kind::Highlighter | Kind::Polyline => {
            s.points.windows(2).map(|p| (p[0], p[1])).collect()
        }
        Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify | Kind::Spotlight => {
            vec![]
        }
        Kind::Line => vec![(a, b)],
        Kind::Ellipse => {
            let rx = (b.x - a.x).abs() / 2.0;
            let ry = (b.y - a.y).abs() / 2.0;
            let cx = (a.x + b.x) / 2.0;
            let cy = (a.y + b.y) / 2.0;
            // 用短线段逼近椭圆，复用抗锯齿绘制。未触及段数上限时弧线误差约小于 0.5 像素。
            // 最大段数防止异常的大图让一次导出占用过多时间。
            let count =
                ((std::f64::consts::TAU * rx.max(ry).sqrt()).ceil() as usize).clamp(24, 2048);
            let point = |i: usize| {
                let angle = std::f64::consts::TAU * i as f64 / count as f64;
                Point {
                    x: cx + rx * angle.cos(),
                    y: cy + ry * angle.sin(),
                }
            };
            (0..count)
                .map(|i| (point(i), point((i + 1) % count)))
                .collect()
        }
        Kind::Rectangle => {
            let c = Point { x: b.x, y: a.y };
            let d = Point { x: a.x, y: b.y };
            vec![(a, c), (c, b), (b, d), (d, a)]
        }
        Kind::Arrow => {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let length = dx.hypot(dy);
            if length == 0.0 {
                return vec![(a, b)];
            }
            // 与 TypeScript arrowWings 共用约定：短箭头的翼不能超过主线长度的 45%。
            let size = (s.width * 4.0).max(10.0).min(length * 0.45);
            let ux = dx / length;
            let uy = dy / length;
            vec![
                (a, b),
                (
                    Point {
                        x: b.x - ux * size + uy * size * 0.5,
                        y: b.y - uy * size - ux * size * 0.5,
                    },
                    b,
                ),
                (
                    Point {
                        x: b.x - ux * size - uy * size * 0.5,
                        y: b.y - uy * size + ux * size * 0.5,
                    },
                    b,
                ),
            ]
        }
    }
}

/// 原图坐标减去裁剪原点后才是输出图片坐标；超出选区的笔画自然裁掉。
pub fn render(image: &mut RgbaImage, shapes: &[Annotation], origin: Point) -> Result<(), String> {
    let mut spotlight_drawn = false;
    for s in shapes {
        if matches!(s.kind, Kind::Spotlight) {
            if spotlight_drawn {
                continue;
            }
            spotlight_drawn = true;
            // 全部亮区共用一个覆盖掩码，在首个聚光灯所在图层只混色一次。
            // 取最大覆盖率，重复或重叠亮区不会加深背景，也不会重新遮住已有亮区。
            let mut bright = vec![0u8; image.width() as usize * image.height() as usize];
            for spot in shapes
                .iter()
                .filter(|shape| matches!(shape.kind, Kind::Spotlight))
            {
                let left = spot.start.x.min(spot.end.x) - origin.x;
                let top = spot.start.y.min(spot.end.y) - origin.y;
                let right = spot.start.x.max(spot.end.x) - origin.x;
                let bottom = spot.start.y.max(spot.end.y) - origin.y;
                for y in
                    top.floor().max(0.0) as u32..bottom.ceil().min(f64::from(image.height())) as u32
                {
                    for x in left.floor().max(0.0) as u32
                        ..right.ceil().min(f64::from(image.width())) as u32
                    {
                        let fx = f64::from(x);
                        let fy = f64::from(y);
                        let inside_x = ((fx + 1.0).min(right) - fx.max(left)).clamp(0.0, 1.0);
                        let inside_y = ((fy + 1.0).min(bottom) - fy.max(top)).clamp(0.0, 1.0);
                        let index = y as usize * image.width() as usize + x as usize;
                        bright[index] =
                            bright[index].max((inside_x * inside_y * 255.0).round() as u8);
                    }
                }
            }
            let rgb = color(&s.color)?;
            for (pixel, coverage) in image.pixels_mut().zip(bright) {
                blend(pixel, rgb, (1.0 - f64::from(coverage) / 255.0) * s.opacity);
            }
            continue;
        }
        if matches!(
            s.kind,
            Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify
        ) {
            let source = raster(s)?;
            let left = s.start.x - origin.x;
            let top = s.start.y - origin.y;
            let width = s.end.x - s.start.x;
            let height = s.end.y - s.start.y;
            for y in top.floor().max(0.0) as u32
                ..(top + height).ceil().min(f64::from(image.height())) as u32
            {
                for x in left.floor().max(0.0) as u32
                    ..(left + width).ceil().min(f64::from(image.width())) as u32
                {
                    let sx = (((f64::from(x) + 0.5 - left) / width).clamp(0.0, 1.0)
                        * f64::from(source.width())) as u32;
                    let sy = (((f64::from(y) + 0.5 - top) / height).clamp(0.0, 1.0)
                        * f64::from(source.height())) as u32;
                    let pixel =
                        source.get_pixel(sx.min(source.width() - 1), sy.min(source.height() - 1));
                    blend(
                        image.get_pixel_mut(x, y),
                        [pixel[0], pixel[1], pixel[2]],
                        f64::from(pixel[3]) / 255.0 * s.opacity,
                    );
                }
            }
            continue;
        }
        let rgb = color(&s.color)?;
        // 同一笔画先取最大覆盖率再混色，荧光笔拐角不会因重复线段叠加而变深。
        let mut mask = vec![0u8; image.width() as usize * image.height() as usize];
        let mut touched = Vec::new();
        for (a, b) in segments(s) {
            line(
                image,
                Point {
                    x: a.x - origin.x,
                    y: a.y - origin.y,
                },
                Point {
                    x: b.x - origin.x,
                    y: b.y - origin.y,
                },
                s.width,
                &mut mask,
                &mut touched,
            );
        }
        for index in touched {
            let coverage = f64::from(mask[index]) / 255.0 * s.opacity;
            blend(
                image.get_pixel_mut(index as u32 % image.width(), index as u32 / image.width()),
                rgb,
                coverage,
            );
        }
    }
    Ok(())
}

fn blend(pixel: &mut image::Rgba<u8>, rgb: [u8; 3], coverage: f64) {
    if coverage <= 0.0 {
        return;
    }
    let old_alpha = f64::from(pixel[3]) / 255.0;
    let alpha = coverage + old_alpha * (1.0 - coverage);
    for channel in 0..3 {
        pixel[channel] = ((f64::from(rgb[channel]) * coverage
            + f64::from(pixel[channel]) * old_alpha * (1.0 - coverage))
            / alpha)
            .round() as u8;
    }
    pixel[3] = (alpha * 255.0).round() as u8;
}

fn line(
    image: &RgbaImage,
    a: Point,
    b: Point,
    width: f64,
    mask: &mut [u8],
    touched: &mut Vec<usize>,
) {
    let radius = width / 2.0;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let squared = dx * dx + dy * dy;
    // 逐行计算线段覆盖的窄区间，避免斜线在 4K 图上遍历整个包围矩形。
    let top = (a.y.min(b.y) - radius - 1.0).floor().max(0.0) as u32;
    let bottom = (a.y.max(b.y) + radius + 1.0)
        .ceil()
        .min(f64::from(image.height())) as u32;
    for y in top..bottom {
        let py = f64::from(y) + 0.5;
        let (t0, t1) = if dy.abs() < f64::EPSILON {
            (0.0, 1.0)
        } else {
            let t0 = (py - radius - 1.0 - a.y) / dy;
            let t1 = (py + radius + 1.0 - a.y) / dy;
            (t0.min(t1).clamp(0.0, 1.0), t0.max(t1).clamp(0.0, 1.0))
        };
        let x0 = a.x + dx * t0;
        let x1 = a.x + dx * t1;
        let left = (x0.min(x1) - radius - 1.0).floor().max(0.0) as u32;
        let right = (x0.max(x1) + radius + 1.0)
            .ceil()
            .min(f64::from(image.width())) as u32;
        for x in left..right {
            let px = f64::from(x) + 0.5;
            let t = if squared == 0.0 {
                0.0
            } else {
                ((px - a.x) * dx + (py - a.y) * dy) / squared
            }
            .clamp(0.0, 1.0);
            let distance = (px - a.x - t * dx).hypot(py - a.y - t * dy);
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            if coverage == 0.0 {
                continue;
            }
            let index = y as usize * image.width() as usize + x as usize;
            let value = (coverage * 255.0).round() as u8;
            if value > 0 && mask[index] == 0 {
                touched.push(index);
            }
            mask[index] = mask[index].max(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_and_overlapping_spotlights_do_not_darken_background() {
        let mut a = shape();
        a.kind = Kind::Spotlight;
        a.color = "#000000".into();
        a.opacity = 0.5;
        a.start = Point { x: 2.0, y: 2.0 };
        a.end = Point { x: 8.0, y: 8.0 };
        let mut b = a.clone();
        b.start = Point { x: 6.0, y: 6.0 };
        b.end = Point { x: 12.0, y: 12.0 };
        let base = RgbaImage::from_pixel(16, 16, image::Rgba([200, 200, 200, 255]));
        let mut single = base.clone();
        render(&mut single, &[a.clone()], Point { x: 0.0, y: 0.0 }).unwrap();
        let mut duplicate = base.clone();
        render(
            &mut duplicate,
            &[a.clone(), a.clone()],
            Point { x: 0.0, y: 0.0 },
        )
        .unwrap();
        assert_eq!(single, duplicate);
        let mut combined = base;
        render(&mut combined, &[a, b], Point { x: 0.0, y: 0.0 }).unwrap();
        assert_eq!(combined.get_pixel(0, 0).0, [100, 100, 100, 255]);
        for (x, y) in [(3, 3), (7, 7), (10, 10)] {
            assert_eq!(combined.get_pixel(x, y).0, [200, 200, 200, 255]);
        }
    }

    #[test]
    fn spotlight_keeps_bright_area_and_shades_crop_with_reverse_coordinates() {
        let mut s = shape();
        s.kind = Kind::Spotlight;
        s.color = "#000000".into();
        s.opacity = 0.5;
        s.start = Point { x: 18.0, y: 18.0 };
        s.end = Point { x: 12.0, y: 12.0 };
        validate(std::slice::from_ref(&s), 32, 32).unwrap();
        let mut image = RgbaImage::from_pixel(10, 10, image::Rgba([200, 200, 200, 255]));
        render(
            &mut image,
            std::slice::from_ref(&s),
            Point { x: 10.0, y: 10.0 },
        )
        .unwrap();
        assert_eq!(image.get_pixel(4, 4).0, [200, 200, 200, 255]);
        assert_eq!(image.get_pixel(0, 0).0, [100, 100, 100, 255]);
        assert_eq!(image.get_pixel(9, 9).0, [100, 100, 100, 255]);
        s.end = s.start;
        assert!(validate(&[s], 32, 32).is_err());
    }

    #[test]
    fn magnifier_requires_bounded_zoom_and_exports_pixels() {
        let mut png = Cursor::new(Vec::new());
        RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut s = shape();
        s.kind = Kind::Magnify;
        s.raster = Some(STANDARD.encode(png.into_inner()));
        for zoom in [None, Some(1.0), Some(5.0), Some(f64::NAN)] {
            s.magnification = zoom;
            assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        }
        s.magnification = Some(3.0);
        validate(std::slice::from_ref(&s), 32, 32).unwrap();
        let mut image = RgbaImage::new(32, 32);
        render(
            &mut image,
            std::slice::from_ref(&s),
            Point { x: 0.0, y: 0.0 },
        )
        .unwrap();
        assert_eq!(image.get_pixel(15, 15).0, [20, 40, 60, 255]);
        s.raster = None;
        assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        s.kind = Kind::Rectangle;
        assert!(validate(&[s], 32, 32).is_err());
    }

    #[test]
    fn number_annotations_validate_labels_and_export_raster() {
        let mut png = Cursor::new(Vec::new());
        RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut s = shape();
        s.kind = Kind::Number;
        s.raster = Some(STANDARD.encode(png.into_inner()));
        for text in ["1", "99", "999"] {
            s.text = Some(text.into());
            validate(std::slice::from_ref(&s), 32, 32).unwrap();
        }
        let mut image = RgbaImage::new(32, 32);
        render(
            &mut image,
            std::slice::from_ref(&s),
            Point { x: 0.0, y: 0.0 },
        )
        .unwrap();
        assert_eq!(image.get_pixel(15, 15).0, [255, 0, 0, 255]);
        for text in ["", "0", "01", "-1", "1000", "1.5", "中文"] {
            s.text = Some(text.into());
            assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        }
        s.text = Some("1".into());
        s.raster = None;
        assert!(validate(&[s], 32, 32).is_err());
    }

    #[test]
    fn highlighter_does_not_darken_its_own_overlapping_segments() {
        let mut s = shape();
        s.kind = Kind::Highlighter;
        s.start = Point { x: 2.0, y: 10.0 };
        s.end = s.start;
        s.points = vec![s.start, Point { x: 20.0, y: 10.0 }, s.start];
        s.width = 6.0;
        s.opacity = 0.35;
        validate(std::slice::from_ref(&s), 24, 24).unwrap();
        let mut image = RgbaImage::new(24, 24);
        render(&mut image, &[s], Point { x: 0.0, y: 0.0 }).unwrap();
        assert_eq!(image.get_pixel(10, 10)[3], 89);
    }
    #[test]
    fn raster_annotations_validate_and_export_with_alpha_and_crop_origin() {
        let source = RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 128]));
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let mut s = shape();
        s.kind = Kind::Text;
        s.opacity = 0.5;
        s.text = Some("中文".into());
        s.font_size = Some(24.0);
        s.font_family = Some("sans".into());
        s.raster = Some(STANDARD.encode(png.into_inner()));
        validate(std::slice::from_ref(&s), 32, 32).unwrap();
        let mut image = RgbaImage::new(20, 20);
        render(
            &mut image,
            std::slice::from_ref(&s),
            Point { x: 10.0, y: 10.0 },
        )
        .unwrap();
        assert_eq!(image.get_pixel(5, 5).0, [0, 255, 0, 64]);
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        s.raster = Some("not a png".into());
        assert!(validate(&[s], 32, 32).is_err());
    }
    #[test]
    fn rejects_invalid_opacity_and_path_limits() {
        let mut s = shape();
        s.opacity = f64::NAN;
        assert!(validate(&[s], 32, 32).is_err());
        let mut s = shape();
        s.kind = Kind::Pen;
        s.points = vec![s.start; 2049];
        assert!(validate(&[s], 32, 32).is_err());
        let mut s = shape();
        s.kind = Kind::Polyline;
        assert!(validate(&[s], 32, 32).is_err());
    }
    #[test]
    fn ellipse_keeps_center_empty_and_handles_reverse_drag_and_crop() {
        let mut s = shape();
        s.kind = Kind::Ellipse;
        s.start = Point { x: 42.0, y: 32.0 };
        s.end = Point { x: 12.0, y: 12.0 };
        let mut image = RgbaImage::new(40, 30);
        render(&mut image, &[s], Point { x: 10.0, y: 10.0 }).unwrap();
        for (x, y) in [(2, 12), (31, 12), (17, 2), (17, 21)] {
            assert!(image.get_pixel(x, y)[3] > 0);
        }
        assert_eq!(image.get_pixel(17, 12)[3], 0);
        assert_eq!(image.get_pixel(2, 2)[3], 0);
    }
    #[test]
    fn line_has_no_arrow_wings_and_accepts_ipc_kind() {
        let s: Annotation = serde_json::from_str(r##"{"kind":"line","start":{"x":2,"y":20},"end":{"x":30,"y":20},"width":2,"color":"#ff0000"}"##).unwrap();
        validate(std::slice::from_ref(&s), 32, 32).unwrap();
        let mut image = RgbaImage::new(32, 32);
        render(&mut image, &[s], Point { x: 0.0, y: 0.0 }).unwrap();
        assert!(image.get_pixel(15, 20)[3] > 0);
        assert_eq!(image.get_pixel(20, 15)[3], 0);
    }
    fn shape() -> Annotation {
        Annotation {
            kind: Kind::Rectangle,
            start: Point { x: 12.0, y: 12.0 },
            end: Point { x: 22.0, y: 22.0 },
            color: "#ff0000".into(),
            width: 2.0,
            opacity: 1.0,
            magnification: None,
            points: vec![],
            text: None,
            font_size: None,
            font_family: None,
            raster: None,
        }
    }
    #[test]
    fn rectangle_uses_crop_origin_and_keeps_interior() {
        let mut image = RgbaImage::from_pixel(20, 20, image::Rgba([255, 255, 255, 255]));
        render(&mut image, &[shape()], Point { x: 10.0, y: 10.0 }).unwrap();
        assert_eq!(image.get_pixel(2, 7).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(7, 7).0, [255, 255, 255, 255]);
    }
    #[test]
    fn invalid_coordinates_width_and_color_are_rejected() {
        let mut s = shape();
        s.start.x = f64::NAN;
        assert!(validate(&[s], 100, 100).is_err());
        let mut s = shape();
        s.width = 100.0;
        assert!(validate(&[s], 100, 100).is_err());
        let mut s = shape();
        s.color = "红色".into();
        assert!(validate(&[s], 100, 100).is_err());
        assert!(validate(&[shape()], 10, 10).is_err());
    }
    #[test]
    fn arrow_renders_shaft_and_two_wings_and_clips_at_boundary() {
        let mut s = shape();
        s.kind = Kind::Arrow;
        s.start = Point { x: 0.0, y: 20.0 };
        s.end = Point { x: 30.0, y: 20.0 };
        let mut image = RgbaImage::new(32, 32);
        render(&mut image, &[s], Point { x: 0.0, y: 0.0 }).unwrap();
        assert!(image.get_pixel(15, 20)[3] > 0);
        assert!(image.get_pixel(20, 15)[3] > 0);
        assert!(image.get_pixel(20, 24)[3] > 0);
        assert_eq!(image.get_pixel(15, 5)[3], 0);
    }
}
