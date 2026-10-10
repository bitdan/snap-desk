//! 将前端标注合成到裁剪后的原图。只接受结构化图形，不接收任意 SVG 或图片路径。
//! 线段按像素中心到线段的距离绘制，边缘使用覆盖率混色，支持任意方向的箭头。
use image::RgbaImage;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Rectangle,
    Arrow,
    Ellipse,
    Line,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub kind: Kind,
    pub start: Point,
    pub end: Point,
    pub color: String,
    pub width: f64,
}

/// 限制数量、坐标、颜色和线宽，防止无效 IPC 数据造成越界或过长绘制循环。
pub fn validate(shapes: &[Annotation], width: u32, height: u32) -> Result<(), String> {
    if shapes.len() > 100 {
        return Err("单张截图最多支持 100 个标注。".into());
    }
    for s in shapes {
        if !s.width.is_finite() || !(0.5..=64.0).contains(&s.width) {
            return Err("标注线宽无效。".into());
        }
        for p in [s.start, s.end] {
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
    for s in shapes {
        let rgb = color(&s.color)?;
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
                rgb,
            );
        }
    }
    Ok(())
}

fn line(image: &mut RgbaImage, a: Point, b: Point, width: f64, rgb: [u8; 3]) {
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
            let pixel = image.get_pixel_mut(x, y);
            // source-over 混合，保留截图本身的透明度语义。
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
