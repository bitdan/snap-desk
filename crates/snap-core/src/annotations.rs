//! 将前端标注合成到裁剪后的原图。只接受结构化图形，不接收任意 SVG 或图片路径。
//! 线段按像素中心到线段的距离绘制，边缘使用覆盖率混色，支持任意方向的箭头。
//! 虚线、点线把路径切成圆头短段后走同一套线段绘制；矩形、椭圆的填充与描边写进同一张覆盖率掩码，
//! 只混色一次。预览、导出、贴图与录屏都调用这里，样式在各处像素一致。
use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub mod edit;
pub mod raster;

pub use raster::{rasterize, Fonts};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Rectangle,
    Arrow,
    Ellipse,
    Line,
    Polyline,
    Pen,
    Highlighter,
    /// 按原图像素直径局部擦除之前的标注，保留底图。
    Erase,
    Text,
    Number,
    Spotlight,
    Magnify,
    Mosaic,
    Blur,
}

impl Kind {
    /// 可选虚线 / 点线的描边图形；荧光笔、橡皮擦等笔刷保持实线。
    pub fn has_line_style(self) -> bool {
        matches!(
            self,
            Self::Rectangle | Self::Ellipse | Self::Line | Self::Polyline | Self::Arrow | Self::Pen
        )
    }

    /// 可填充内部的封闭图形。
    pub fn has_fill(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse)
    }
}

/// 描边线型。划与点的长度按线宽换算；端点为圆头，封闭图形按周长微调间距，使首尾衔接。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// 点划线：长划与圆点交替。
    #[serde(rename = "dashdot")]
    DashDot,
}

/// 箭头头部：只画在终点（旧数据的唯一形态）、起点终点都画、只画在起点（反向），
/// 或尺寸标注线（两端箭头加垂直的界线）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrowHeads {
    #[default]
    End,
    Both,
    Start,
    Dimension,
}

/// 矩形、椭圆的内部填充，颜色与描边相同；半透明按 [`FILL_TRANSLUCENT`] 覆盖，再乘标注不透明度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fill {
    #[default]
    None,
    Translucent,
    Solid,
}

/// 半透明填充的覆盖率：底图仍清晰可读，又能看出框住的范围。
pub const FILL_TRANSLUCENT: f64 = 0.3;
/// 圆角半径上限（原图像素）；实际半径还会限制在短边的一半以内。
pub const MAX_CORNER_RADIUS: f64 = 1024.0;

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

/// 标注的结构化描述，截图历史、贴图历史与录屏标注轨道都直接序列化它。
///
/// 样式字段（线型、箭头、填充、圆角、文字背景与描边）后加，全部带 `serde(default)`：
/// 旧历史缺这些字段时按实线、单向箭头、无填充、直角、无装饰读取；取缺省值时也不写出，
/// 只用旧样式的文件与旧版本保持一致。
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
    /// 线型，只用于 [`Kind::has_line_style`] 的图形；箭头只作用于主线，箭头头部保持实线。
    #[serde(default, skip_serializing_if = "is_default")]
    pub line_style: LineStyle,
    /// 只用于箭头。
    #[serde(default, skip_serializing_if = "is_default")]
    pub arrow_heads: ArrowHeads,
    /// 只用于 [`Kind::has_fill`] 的图形。
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill: Fill,
    /// 矩形圆角半径（原图像素），0 为直角。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub corner_radius: f64,
    /// 文字背景框颜色 `#RRGGBB`；内边距与圆角按字号推导（见 `raster::text_inset`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// 文字描边颜色 `#RRGGBB`；描边宽度取 `width`，即字形边缘向外扩展的像素数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
}
fn opaque() -> f64 {
    1.0
}

/// 黑白中与 `color` 对比更强的一种，用于有底色的文字和序号的字色、文字描边色。
/// 阈值与序号一直使用的亮度判断一致；颜色无效时按深色处理。
pub fn contrast(color_text: &str) -> String {
    let ink = contrast_ink(color(color_text).unwrap_or([0, 0, 0]));
    format!("#{:02x}{:02x}{:02x}", ink[0], ink[1], ink[2])
}

pub(crate) fn contrast_ink(rgb: [u8; 3]) -> [u8; 3] {
    let luminance =
        f64::from(rgb[0]) * 0.299 + f64::from(rgb[1]) * 0.587 + f64::from(rgb[2]) * 0.114;
    if luminance > 160.0 {
        [0x11, 0x18, 0x27]
    } else {
        [0xff, 0xff, 0xff]
    }
}

/// 字形/滤镜只允许受限 PNG；先检查解码尺寸再分配 RGBA，防止压缩图片导致巨量内存分配。
pub fn raster(s: &Annotation) -> Result<RgbaImage, String> {
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
    validate_impl(shapes, width, height, false)
}

/// 录制滤镜只接受几何和样式，由编码帧实时取样；文字/序号仍必须提供受限字形 PNG。
pub fn validate_live(shapes: &[Annotation], width: u32, height: u32) -> Result<(), String> {
    validate_impl(shapes, width, height, true)
}
fn validate_impl(shapes: &[Annotation], width: u32, height: u32, live: bool) -> Result<(), String> {
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
        if matches!(
            s.kind,
            Kind::Pen | Kind::Highlighter | Kind::Polyline | Kind::Erase
        ) && s.points.len() < if matches!(s.kind, Kind::Erase) { 1 } else { 2 }
        {
            return Err("路径至少需要两个点。".into());
        }
        if !matches!(
            s.kind,
            Kind::Pen | Kind::Highlighter | Kind::Polyline | Kind::Erase
        ) && !s.points.is_empty()
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
            if !(live && dynamic_filter(s)) {
                raster(s)?;
            }
        } else if s.raster.is_some() {
            return Err("此标注类型不接受像素数据。".into());
        }
        if matches!(s.kind, Kind::Mosaic | Kind::Blur | Kind::Erase) && s.opacity != 1.0 {
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
        // 样式只接受对该类型有意义的取值，避免历史里留下界面无法显示或编辑的状态。
        if (s.line_style != LineStyle::Solid && !s.kind.has_line_style())
            || (s.arrow_heads != ArrowHeads::End && s.kind != Kind::Arrow)
            || (s.fill != Fill::None && !s.kind.has_fill())
            || ((s.background.is_some() || s.outline.is_some()) && s.kind != Kind::Text)
        {
            return Err("此标注类型不支持所选样式。".into());
        }
        if !s.corner_radius.is_finite()
            || !(0.0..=MAX_CORNER_RADIUS).contains(&s.corner_radius)
            || (s.corner_radius > 0.0 && s.kind != Kind::Rectangle)
        {
            return Err("圆角半径无效。".into());
        }
        color(&s.color)?;
        for extra in [&s.background, &s.outline].into_iter().flatten() {
            color(extra)?;
        }
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

/// 笔画实际绘制的线段：主路径按线型切成虚线 / 点线，箭头头部始终为实线。
fn segments(s: &Annotation) -> Vec<(Point, Point)> {
    let (path, closed) = stroke_path(s);
    let mut out = match dash_pattern(s.line_style, s.width) {
        Some((marks, period)) if s.kind.has_line_style() => dash(&path, &marks, period, closed),
        _ => path,
    };
    if matches!(s.kind, Kind::Arrow) {
        for (tip, wings) in edit::arrow_heads(s) {
            out.extend(wings.map(|wing| (wing, tip)));
        }
        out.extend(edit::dimension_ticks(s));
    }
    out
}

/// 虚线 / 点线的“画、空”长度（原图像素）。线段两端是圆头，各多出半个线宽，
/// 所以看到的划比 `on` 长一个线宽、空隙比 `off` 短一个线宽：虚线约为 3 倍线宽的划、2 倍线宽的空，
/// 点线为直径等于线宽、间隔约一个线宽的圆点；点划线为长划、圆点交替，间隔约 1.5 倍线宽。
/// 细线按 2 像素计，避免划和空细碎到看不出线型。返回一个周期内各划的（起点, 长度）与周期长度。
fn dash_pattern(style: LineStyle, width: f64) -> Option<(Vec<(f64, f64)>, f64)> {
    let unit = width.max(2.0);
    match style {
        LineStyle::Solid => None,
        LineStyle::Dashed => Some((vec![(0.0, unit * 2.0)], unit * 5.0)),
        LineStyle::Dotted => Some((vec![(0.0, 0.0)], unit * 2.0)),
        LineStyle::DashDot => Some((vec![(0.0, unit * 3.0), (unit * 5.5, 0.0)], unit * 8.0)),
    }
}

/// 沿路径累计长度切出划段（划长为 0 的画成圆点）；一段划可以跨过拐角，连续经过多条线段。
/// `marks` 为一个周期内各划的（起点, 长度），按起点升序。开放路径缩放间距使两端都落在第一种划上，
/// 封闭路径缩放为整数个周期，起点处不会出现半截划或多出的空隙。
fn dash(path: &[(Point, Point)], marks: &[(f64, f64)], period: f64, closed: bool) -> Vec<(Point, Point)> {
    let lengths: Vec<f64> = path.iter().map(|(a, b)| (b.x - a.x).hypot(b.y - a.y)).collect();
    let total: f64 = lengths.iter().sum();
    if path.is_empty() || marks.is_empty() || total <= 0.0 || period <= 0.0 {
        return path.to_vec();
    }
    let first = marks[0].1;
    let scale = if closed {
        total / ((total / period).round().max(1.0) * period)
    } else {
        let count = ((total + period - first) / period).round().max(1.0);
        let span = (count - 1.0) * period + first;
        if span > 0.0 {
            total / span
        } else {
            1.0
        }
    };
    let period = period * scale;
    // 封闭路径的最后一个周期回到起点，不再重复画起点处的划；开放路径容许浮点误差，保住终点的划。
    let limit = if closed { total - 1e-6 } else { total + 1e-6 };
    let lerp = |a: Point, b: Point, t: f64| Point {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    };
    let mut out = Vec::new();
    // `index` 是第一条结束位置不早于当前划起点的线段，`offset` 为它之前的累计长度；划起点单调增加，只需向前扫描。
    let (mut index, mut offset) = (0, 0.0);
    let starts = (0..).map(|cycle| f64::from(cycle) * period).take_while(|c| *c <= limit).flat_map(|c| {
        marks.iter().map(move |&(at, on)| (c + at * scale, on * scale))
    });
    for (start, on) in starts {
        if start > limit {
            break;
        }
        let end = (start + on).min(total);
        while index + 1 < path.len() && offset + lengths[index] < start {
            offset += lengths[index];
            index += 1;
        }
        let (mut i, mut o) = (index, offset);
        loop {
            let (a, b) = path[i];
            let length = lengths[i];
            let (t0, t1) = if length > 0.0 {
                (
                    ((start - o) / length).clamp(0.0, 1.0),
                    ((end - o) / length).clamp(0.0, 1.0),
                )
            } else {
                (0.0, 0.0)
            };
            out.push((lerp(a, b, t0), lerp(a, b, t1)));
            if o + length >= end || i + 1 >= path.len() {
                break;
            }
            o += length;
            i += 1;
        }
    }
    out
}

/// 主路径与是否首尾相接（矩形、椭圆）；箭头只含主线，头部由 [`edit::arrow_heads`] 给出。
fn stroke_path(s: &Annotation) -> (Vec<(Point, Point)>, bool) {
    let a = s.start;
    let b = s.end;
    let closed = matches!(s.kind, Kind::Rectangle | Kind::Ellipse);
    let path = match s.kind {
        Kind::Erase if s.points.len() == 1 => vec![(a, a)],
        Kind::Pen | Kind::Highlighter | Kind::Polyline | Kind::Erase => {
            s.points.windows(2).map(|p| (p[0], p[1])).collect()
        }
        Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify | Kind::Spotlight => {
            vec![]
        }
        Kind::Line | Kind::Arrow => vec![(a, b)],
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
        Kind::Rectangle if s.corner_radius > 0.0 => rounded_rect(a, b, s.corner_radius),
        Kind::Rectangle => {
            let c = Point { x: b.x, y: a.y };
            let d = Point { x: a.x, y: b.y };
            vec![(a, c), (c, b), (b, d), (d, a)]
        }
    };
    (path, closed)
}

/// 圆角半径限制在短边一半以内，宽高为 0 时退化为直角。
fn corner(x0: f64, y0: f64, x1: f64, y1: f64, radius: f64) -> f64 {
    radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0)
}

/// 圆角矩形轮廓：从上边右端起顺时针，依次是右上弧、右边、右下弧……最后一段回到起点即上边。
/// 每段四分之一圆弧的段数与椭圆的估算一致，弧线误差约小于 0.5 像素。
fn rounded_rect(a: Point, b: Point, radius: f64) -> Vec<(Point, Point)> {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    let r = corner(x0, y0, x1, y1, radius);
    let steps = ((TAU * r.sqrt()).ceil() as usize / 4).clamp(2, 512);
    let mut points = Vec::with_capacity((steps + 1) * 4);
    // 屏幕坐标 y 向下：-90° 指向上方，角度增大即顺时针。
    for (cx, cy, from) in [
        (x1 - r, y0 + r, -FRAC_PI_2),
        (x1 - r, y1 - r, 0.0),
        (x0 + r, y1 - r, FRAC_PI_2),
        (x0 + r, y0 + r, PI),
    ] {
        for i in 0..=steps {
            let angle = from + FRAC_PI_2 * i as f64 / steps as f64;
            points.push(Point {
                x: cx + r * angle.cos(),
                y: cy + r * angle.sin(),
            });
        }
    }
    let n = points.len();
    (0..n).map(|i| (points[i], points[(i + 1) % n])).collect()
}

/// 圆角矩形（半径 0 即普通矩形）在像素中心的覆盖率：有符号距离减到 0.5 像素宽的过渡带。
/// 文字背景框也用它，保证与矩形填充的边缘一致。
fn rounded_rect_coverage(px: f64, py: f64, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> f64 {
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let qx = (px - (x0 + hx)).abs() - (hx - r);
    let qy = (py - (y0 + hy)).abs() - (hy - r);
    let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// 椭圆在像素中心的覆盖率：隐函数值除以梯度长度近似到边缘的距离，边缘附近误差远小于 1 像素。
fn ellipse_coverage(px: f64, py: f64, cx: f64, cy: f64, rx: f64, ry: f64) -> f64 {
    let (dx, dy) = (px - cx, py - cy);
    let value = (dx / rx).powi(2) + (dy / ry).powi(2) - 1.0;
    let gradient = 2.0 * (dx / (rx * rx)).hypot(dy / (ry * ry));
    if gradient == 0.0 {
        return 1.0;
    }
    (0.5 - value / gradient).clamp(0.0, 1.0)
}

/// 把矩形 / 椭圆内部的覆盖率（乘填充强度）以取大方式写入笔画掩码，和描边合成一次，
/// 描边内侧与填充重叠处不会因两次混色而变深。返回需要逐像素混色的范围 `(left, top, right, bottom)`：
/// 大面积填充不进 `touched` 列表，避免整屏填充时分配上千万个下标。
fn fill_mask(
    image: &RgbaImage,
    s: &Annotation,
    origin: Point,
    mask: &mut [u8],
) -> Option<(u32, u32, u32, u32)> {
    let strength = match s.fill {
        Fill::None => return None,
        Fill::Translucent => FILL_TRANSLUCENT,
        Fill::Solid => 1.0,
    };
    if !s.kind.has_fill() {
        return None;
    }
    let x0 = s.start.x.min(s.end.x) - origin.x;
    let y0 = s.start.y.min(s.end.y) - origin.y;
    let x1 = s.start.x.max(s.end.x) - origin.x;
    let y1 = s.start.y.max(s.end.y) - origin.y;
    // 外扩半个线宽，让描边外缘的抗锯齿像素也在混色范围内。
    let grow = s.width / 2.0 + 2.0;
    let left = (x0 - grow).floor().max(0.0) as u32;
    let top = (y0 - grow).floor().max(0.0) as u32;
    let right = (x1 + grow).ceil().min(f64::from(image.width())).max(0.0) as u32;
    let bottom = (y1 + grow).ceil().min(f64::from(image.height())).max(0.0) as u32;
    if left >= right || top >= bottom {
        return None;
    }
    let radius = if matches!(s.kind, Kind::Rectangle) {
        corner(x0, y0, x1, y1, s.corner_radius)
    } else {
        0.0
    };
    let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    for y in top..bottom {
        let py = f64::from(y) + 0.5;
        for x in left..right {
            let px = f64::from(x) + 0.5;
            let coverage = match s.kind {
                Kind::Ellipse if rx > 0.0 && ry > 0.0 => {
                    ellipse_coverage(px, py, x0 + rx, y0 + ry, rx, ry)
                }
                Kind::Ellipse => 0.0,
                _ => rounded_rect_coverage(px, py, x0, y0, x1, y1, radius),
            };
            let value = (coverage * strength * 255.0).round() as u8;
            let index = y as usize * image.width() as usize + x as usize;
            mask[index] = mask[index].max(value);
        }
    }
    Some((left, top, right, bottom))
}

/// 原图坐标减去裁剪原点后才是输出图片坐标；超出选区的笔画自然裁掉。
pub fn render(image: &mut RgbaImage, shapes: &[Annotation], origin: Point) -> Result<(), String> {
    // 橡皮擦只修改透明标注层，最后叠回底图，保留透明文件贴图。
    if shapes.iter().any(|s| matches!(s.kind, Kind::Erase)) {
        let mut layer = RgbaImage::new(image.width(), image.height());
        render_layer(&mut layer, shapes, origin, None)?;
        image::imageops::overlay(image, &layer, 0, 0);
        return Ok(());
    }
    render_layer(image, shapes, origin, None)
}

pub fn dynamic_filter(shape: &Annotation) -> bool {
    matches!(shape.kind, Kind::Mosaic | Kind::Blur | Kind::Magnify)
}
/// 以当前干净视频帧为滤镜来源，保持图层顺序和橡皮擦语义，不把某一帧截图冻结进视频。
pub fn render_live(image: &mut RgbaImage, shapes: &[Annotation]) -> Result<(), String> {
    let mut layer = RgbaImage::new(image.width(), image.height());
    render_layer(&mut layer, shapes, Point { x: 0.0, y: 0.0 }, Some(image))?;
    image::imageops::overlay(image, &layer, 0, 0);
    Ok(())
}

/// 滤镜临时图最长边限制为 1024；只在实际使用滤镜时分配，不影响普通线条标注的快速路径。
pub fn live_filter(frame: &RgbaImage, s: &Annotation) -> Result<RgbaImage, String> {
    use image::imageops::{self, FilterType};
    let x = (s.start.x.floor().max(0.0) as u32).min(frame.width() - 1);
    let y = (s.start.y.floor().max(0.0) as u32).min(frame.height() - 1);
    let width = (s.end.x.ceil() as u32)
        .min(frame.width())
        .saturating_sub(x)
        .max(1);
    let height = (s.end.y.ceil() as u32)
        .min(frame.height())
        .saturating_sub(y)
        .max(1);
    let scale = (1024.0 / f64::from(width.max(height))).min(1.0);
    let w = (f64::from(width) * scale).ceil().max(1.0) as u32;
    let h = (f64::from(height) * scale).ceil().max(1.0) as u32;
    let (sx, sy, sw, sh) = if matches!(s.kind, Kind::Magnify) {
        let zoom = s.magnification.unwrap_or(2.0);
        let sw = (f64::from(width) / zoom).round().max(1.0) as u32;
        let sh = (f64::from(height) / zoom).round().max(1.0) as u32;
        (x + (width - sw) / 2, y + (height - sh) / 2, sw, sh)
    } else {
        (x, y, width, height)
    };
    let crop = imageops::crop_imm(frame, sx, sy, sw, sh).to_image();
    let mut result = imageops::resize(&crop, w, h, FilterType::Triangle);
    match s.kind {
        Kind::Mosaic => {
            let block = s.width.max(4.0);
            let small = imageops::resize(
                &crop,
                (f64::from(width) / block).ceil().max(1.0) as u32,
                (f64::from(height) / block).ceil().max(1.0) as u32,
                FilterType::Triangle,
            );
            result = imageops::resize(&small, w, h, FilterType::Nearest);
        }
        Kind::Blur => result = imageops::blur(&result, (s.width * scale).max(2.0) as f32),
        Kind::Magnify => {
            let c = color(&s.color)?;
            let border = (s.width * scale).round().max(1.0) as u32;
            for (x, y, pixel) in result.enumerate_pixels_mut() {
                if x < border
                    || y < border
                    || x >= w.saturating_sub(border)
                    || y >= h.saturating_sub(border)
                {
                    *pixel = image::Rgba([c[0], c[1], c[2], 255]);
                }
            }
        }
        _ => {}
    }
    Ok(result)
}

fn render_layer(
    image: &mut RgbaImage,
    shapes: &[Annotation],
    origin: Point,
    live_source: Option<&RgbaImage>,
) -> Result<(), String> {
    let mut spotlight_drawn = false;
    // 笔画覆盖掩码整图大小（4K 约 8 MB）：全部笔画共用一份，首次用到时才分配，
    // 每个笔画混色后只把自己触碰过的位置归零，不再逐笔画重新分配并清零整图。
    let mut mask: Vec<u8> = Vec::new();
    let mut touched = Vec::new();
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
            // 遮罩覆盖整个选区，拖动时每次移动都要重画：透明像素的混色结果只取决于覆盖率，
            // 先算好 256 种结果直接查表，完全亮区跳过，只有已画过其他标注的像素才逐个混色。
            let fresh: Vec<image::Rgba<u8>> = (0..=255u8)
                .map(|coverage| {
                    let mut pixel = image::Rgba([0, 0, 0, 0]);
                    blend(&mut pixel, rgb, (1.0 - f64::from(coverage) / 255.0) * s.opacity);
                    pixel
                })
                .collect();
            for (pixel, coverage) in image.pixels_mut().zip(bright) {
                if coverage == 255 {
                    continue;
                }
                if pixel[3] == 0 {
                    *pixel = fresh[usize::from(coverage)];
                } else {
                    blend(pixel, rgb, (1.0 - f64::from(coverage) / 255.0) * s.opacity);
                }
            }
            continue;
        }
        if matches!(
            s.kind,
            Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify
        ) {
            let source = if let Some(frame) = live_source.filter(|_| dynamic_filter(s)) {
                live_filter(frame, s)?
            } else {
                raster(s)?
            };
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
        if mask.is_empty() {
            mask = vec![0u8; image.width() as usize * image.height() as usize];
        }
        let filled = fill_mask(image, s, origin, &mut mask);
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
        // 有填充时按范围逐像素混色并归零；描边下标都在范围内，随后的 touched 只剩已归零的空操作。
        if let Some((left, top, right, bottom)) = filled {
            for y in top..bottom {
                for x in left..right {
                    let index = y as usize * image.width() as usize + x as usize;
                    let coverage = f64::from(std::mem::take(&mut mask[index])) / 255.0 * s.opacity;
                    blend(image.get_pixel_mut(x, y), rgb, coverage);
                }
            }
        }
        for index in touched.drain(..) {
            // 取值同时归零，下一个笔画拿到的仍是全零掩码。
            let coverage = f64::from(std::mem::take(&mut mask[index])) / 255.0 * s.opacity;
            if matches!(s.kind, Kind::Erase) {
                let pixel =
                    image.get_pixel_mut(index as u32 % image.width(), index as u32 / image.width());
                pixel[3] = (f64::from(pixel[3]) * (1.0 - coverage)).round() as u8;
                continue;
            }
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
    fn eraser_restores_transparent_base_and_keeps_later_strokes() {
        let mut pen = shape();
        pen.kind = Kind::Line;
        pen.start = Point { x: 12.0, y: 20.0 };
        pen.end = Point { x: 38.0, y: 20.0 };
        pen.width = 6.0;
        let mut erase = pen.clone();
        erase.kind = Kind::Erase;
        erase.start = Point { x: 25.0, y: 20.0 };
        erase.end = erase.start;
        erase.points = vec![erase.start];
        erase.width = 10.0;
        validate(&[pen.clone(), erase.clone()], 40, 40).unwrap();
        let base = RgbaImage::from_pixel(30, 30, image::Rgba([40, 80, 120, 100]));
        let origin = Point { x: 10.0, y: 10.0 };
        let mut output = base.clone();
        render(&mut output, &[pen.clone(), erase.clone()], origin).unwrap();
        assert_eq!(output.get_pixel(15, 10), base.get_pixel(15, 10));
        assert_eq!(output.get_pixel(4, 10).0, [255, 0, 0, 255]);
        pen.color = "#00ff00".into();
        let mut later = base;
        render(&mut later, &[erase.clone(), pen], origin).unwrap();
        assert_eq!(later.get_pixel(15, 10).0, [0, 255, 0, 255]);
        erase.opacity = 0.5;
        assert!(validate(&[erase.clone()], 40, 40).is_err());
        erase.opacity = 1.0;
        erase.points.clear();
        assert!(validate(&[erase], 40, 40).is_err());
    }

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
            line_style: Default::default(),
            arrow_heads: Default::default(),
            fill: Default::default(),
            corner_radius: 0.0,
            background: None,
            outline: None,
        }
    }
    #[test]
    fn live_filters_sample_each_new_frame_without_requiring_frozen_png() {
        for kind in [Kind::Mosaic, Kind::Blur, Kind::Magnify] {
            let mut s = shape();
            s.kind = kind;
            s.magnification = if matches!(kind, Kind::Magnify) { Some(2.0) } else { None };
            assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
            validate_live(std::slice::from_ref(&s), 32, 32).unwrap();
            for rgb in [[20, 80, 160, 255], [180, 50, 40, 255]] {
                let mut frame = RgbaImage::from_pixel(32, 32, image::Rgba(rgb));
                render_live(&mut frame, std::slice::from_ref(&s)).unwrap();
                assert_eq!(frame.get_pixel(17, 17).0, rgb);
                assert_eq!(frame.get_pixel(0, 0).0, rgb);
            }
        }
        let mut text = shape();
        text.kind = Kind::Text;
        assert!(validate_live(&[text], 32, 32).is_err());
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

    #[test]
    fn old_annotations_load_with_default_styles_and_serialize_without_new_keys() {
        let old: Annotation = serde_json::from_str(r##"{"kind":"rectangle","start":{"x":1,"y":1},"end":{"x":9,"y":9},"color":"#ff0000","width":2}"##).unwrap();
        assert_eq!(old.line_style, LineStyle::Solid);
        assert_eq!(old.arrow_heads, ArrowHeads::End);
        assert_eq!(old.fill, Fill::None);
        assert_eq!(old.corner_radius, 0.0);
        assert!(old.background.is_none() && old.outline.is_none());
        let json = serde_json::to_value(&old).unwrap();
        for key in [
            "lineStyle",
            "arrowHeads",
            "fill",
            "cornerRadius",
            "background",
            "outline",
        ] {
            assert!(json.get(key).is_none(), "{key}");
        }
        let styled: Annotation = serde_json::from_str(r##"{"kind":"rectangle","start":{"x":1,"y":1},"end":{"x":9,"y":9},"color":"#ff0000","width":2,"lineStyle":"dashed","fill":"translucent","cornerRadius":3}"##).unwrap();
        validate(std::slice::from_ref(&styled), 10, 10).unwrap();
        let json = serde_json::to_value(&styled).unwrap();
        assert_eq!(json["lineStyle"], "dashed");
        assert_eq!(json["fill"], "translucent");
        assert_eq!(json["cornerRadius"], 3.0);
        let arrow: Annotation = serde_json::from_str(r##"{"kind":"arrow","start":{"x":1,"y":1},"end":{"x":9,"y":9},"color":"#ff0000","width":2,"arrowHeads":"both","lineStyle":"dotted"}"##).unwrap();
        assert_eq!(arrow.arrow_heads, ArrowHeads::Both);
        assert_eq!(arrow.line_style, LineStyle::Dotted);
        assert!(serde_json::from_str::<Annotation>(r##"{"kind":"line","start":{"x":1,"y":1},"end":{"x":9,"y":9},"color":"#ff0000","width":2,"lineStyle":"wavy"}"##).is_err());
    }

    #[test]
    fn styles_are_rejected_on_kinds_that_cannot_show_them() {
        let mut s = shape();
        s.kind = Kind::Highlighter;
        s.points = vec![s.start, s.end];
        s.line_style = LineStyle::Dashed;
        assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        let mut s = shape();
        s.kind = Kind::Line;
        s.arrow_heads = ArrowHeads::Both;
        assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        s.arrow_heads = ArrowHeads::End;
        s.fill = Fill::Solid;
        assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        let mut s = shape();
        s.kind = Kind::Ellipse;
        s.corner_radius = 4.0;
        assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        let mut s = shape();
        s.background = Some("#ffffff".into());
        assert!(validate(std::slice::from_ref(&s), 32, 32).is_err());
        for radius in [-1.0, f64::NAN, MAX_CORNER_RADIUS + 1.0] {
            let mut s = shape();
            s.corner_radius = radius;
            assert!(validate(&[s], 32, 32).is_err());
        }
        let mut s = shape();
        s.line_style = LineStyle::Dotted;
        s.fill = Fill::Translucent;
        s.corner_radius = 4.0;
        validate(&[s], 32, 32).unwrap();
    }

    /// 第 `y` 行中连续有像素的段数。
    fn runs(image: &RgbaImage, y: u32) -> usize {
        (0..image.width())
            .filter(|&x| image.get_pixel(x, y)[3] > 0 && (x == 0 || image.get_pixel(x - 1, y)[3] == 0))
            .count()
    }

    #[test]
    fn dashed_and_dotted_lines_split_into_evenly_fitted_pieces() {
        let mut s = shape();
        s.kind = Kind::Line;
        s.start = Point { x: 2.0, y: 10.0 };
        s.end = Point { x: 62.0, y: 10.0 };
        let draw = |s: &Annotation| {
            let mut image = RgbaImage::new(64, 20);
            render(&mut image, std::slice::from_ref(s), Point { x: 0.0, y: 0.0 }).unwrap();
            image
        };
        assert_eq!(runs(&draw(&s), 10), 1);
        // 线宽 2：虚线划 4、空 6，60 像素长的线缩放为 7 段划，两端都落在划上。
        s.line_style = LineStyle::Dashed;
        let dashed = draw(&s);
        assert_eq!(runs(&dashed, 10), 7);
        assert!(dashed.get_pixel(2, 10)[3] > 0 && dashed.get_pixel(61, 10)[3] > 0);
        assert_eq!(dashed.get_pixel(8, 10)[3], 0);
        // 点线：间隔 4 的圆点，0～60 共 16 个。
        s.line_style = LineStyle::Dotted;
        let dotted = draw(&s);
        assert_eq!(runs(&dotted, 10), 16);
        assert_eq!(dotted.get_pixel(4, 10)[3], 0);
        validate(std::slice::from_ref(&s), 64, 20).unwrap();
    }

    #[test]
    fn dash_fits_open_ends_and_whole_periods_on_closed_paths() {
        let p = |x: f64, y: f64| Point { x, y };
        let length = |pieces: &[(Point, Point)]| -> f64 {
            pieces.iter().map(|(a, b)| (b.x - a.x).hypot(b.y - a.y)).sum()
        };
        let square = [
            (p(0.0, 0.0), p(40.0, 0.0)),
            (p(40.0, 0.0), p(40.0, 40.0)),
            (p(40.0, 40.0), p(0.0, 40.0)),
            (p(0.0, 40.0), p(0.0, 0.0)),
        ];
        // 周长 160、周期 10：整 16 段划，转角处的划拆成两段但总长不变，最后一段不回到起点重画。
        let closed = dash(&square, &[(0.0, 4.0)], 10.0, true);
        assert!((length(&closed) - 64.0).abs() < 1e-9);
        assert_eq!((closed[0].0.x, closed[0].0.y), (0.0, 0.0));
        let line = [(p(0.0, 0.0), p(60.0, 0.0))];
        let open = dash(&line, &[(0.0, 4.0)], 10.0, false);
        assert_eq!(open.len(), 7);
        assert!((length(&open) - 7.0 * 3.75).abs() < 1e-9);
        assert_eq!(open.last().unwrap().1.x, 60.0);
        let dots = dash(&line, &[(0.0, 0.0)], 4.0, false);
        assert_eq!(dots.len(), 16);
        assert!(dots.iter().all(|(a, b)| a.x == b.x));
        // 零长度路径原样返回。
        assert_eq!(dash(&[(p(5.0, 5.0), p(5.0, 5.0))], &[(0.0, 4.0)], 10.0, false).len(), 1);
    }

    #[test]
    fn dash_dot_alternates_long_dashes_and_dots() {
        let p = |x: f64, y: f64| Point { x, y };
        let line = [(p(0.0, 0.0), p(160.0, 0.0))];
        let (marks, period) = dash_pattern(LineStyle::DashDot, 2.0).unwrap();
        let pieces = dash(&line, &marks, period, false);
        // 交替出现：长划（长度 > 0）与圆点（长度 0），从长划开始、以长划结束。
        let kinds: Vec<bool> = pieces.iter().map(|(a, b)| b.x > a.x).collect();
        assert!(kinds.len() >= 5);
        assert!(kinds.windows(2).all(|w| w[0] != w[1]));
        assert!(kinds[0] && *kinds.last().unwrap());
        assert!((pieces.last().unwrap().1.x - 160.0).abs() < 1e-6);
        let styled: Annotation = serde_json::from_str(
            r##"{"kind":"line","start":{"x":0,"y":0},"end":{"x":9,"y":9},"color":"#ff0000","width":2,"lineStyle":"dashdot"}"##,
        )
        .unwrap();
        assert_eq!(styled.line_style, LineStyle::DashDot);
    }

    #[test]
    fn reverse_arrow_and_dimension_line_place_heads_and_ticks() {
        let mut s = shape();
        s.kind = Kind::Arrow;
        s.start = Point { x: 10.0, y: 20.0 };
        s.end = Point { x: 60.0, y: 20.0 };
        s.arrow_heads = ArrowHeads::Start;
        let heads = edit::arrow_heads(&s);
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].0.x, 10.0);
        assert!(edit::dimension_ticks(&s).is_empty());
        s.arrow_heads = ArrowHeads::Dimension;
        assert_eq!(edit::arrow_heads(&s).len(), 2);
        let ticks = edit::dimension_ticks(&s);
        assert_eq!(ticks.len(), 2);
        // 界线垂直于主线、以端点为中点。
        assert!(ticks.iter().all(|(a, b)| a.x == b.x && (a.y + b.y) / 2.0 == 20.0));
        validate(std::slice::from_ref(&s), 80, 40).unwrap();
        let mut image = RgbaImage::new(80, 40);
        render(&mut image, std::slice::from_ref(&s), Point { x: 0.0, y: 0.0 }).unwrap();
        assert!(image.get_pixel(10, 8)[3] > 0 && image.get_pixel(60, 32)[3] > 0);
    }

    #[test]
    fn double_headed_arrow_draws_wings_at_both_ends_and_keeps_heads_solid() {
        let mut s = shape();
        s.kind = Kind::Arrow;
        s.start = Point { x: 10.0, y: 20.0 };
        s.end = Point { x: 50.0, y: 20.0 };
        let draw = |s: &Annotation| {
            let mut image = RgbaImage::new(60, 40);
            render(&mut image, std::slice::from_ref(s), Point { x: 0.0, y: 0.0 }).unwrap();
            image
        };
        let single = draw(&s);
        assert!(single.get_pixel(45, 17)[3] > 0);
        assert_eq!(single.get_pixel(15, 17)[3], 0);
        s.arrow_heads = ArrowHeads::Both;
        validate(std::slice::from_ref(&s), 60, 40).unwrap();
        let double = draw(&s);
        assert!(double.get_pixel(45, 17)[3] > 0 && double.get_pixel(15, 17)[3] > 0);
        assert!(edit::hit_test(std::slice::from_ref(&s), Point { x: 15.0, y: 17.5 }, 1.0, true, false).is_some());
        // 虚线只切主线，两端的箭头头部仍是实线。
        s.line_style = LineStyle::Dashed;
        let dashed = draw(&s);
        assert!(dashed.get_pixel(45, 17)[3] > 0 && dashed.get_pixel(15, 17)[3] > 0);
        assert!(runs(&dashed, 20) > 1);
    }

    #[test]
    fn fills_cover_interiors_once_and_rounded_corners_skip_the_box_corner() {
        let mut s = shape();
        s.start = Point { x: 10.0, y: 10.0 };
        s.end = Point { x: 40.0, y: 30.0 };
        let draw = |s: &Annotation| {
            let mut image = RgbaImage::new(50, 40);
            render(&mut image, std::slice::from_ref(s), Point { x: 0.0, y: 0.0 }).unwrap();
            image
        };
        assert_eq!(draw(&s).get_pixel(25, 20)[3], 0);
        s.fill = Fill::Translucent;
        let translucent = draw(&s);
        assert_eq!(translucent.get_pixel(25, 20).0, [255, 0, 0, 77]);
        assert_eq!(translucent.get_pixel(25, 10).0, [255, 0, 0, 255]);
        assert_eq!(translucent.get_pixel(5, 5)[3], 0);
        // 半透明时描边内侧与外侧一样深：填充与描边取大后只混色一次。
        s.opacity = 0.5;
        let faded = draw(&s);
        assert_eq!(faded.get_pixel(25, 10), faded.get_pixel(25, 9));
        s.opacity = 1.0;
        s.fill = Fill::Solid;
        assert_eq!(draw(&s).get_pixel(25, 20).0, [255, 0, 0, 255]);
        // 填充后的内部也算笔迹，整项删除点在内部即可命中。
        assert!(edit::hit_test(std::slice::from_ref(&s), Point { x: 25.0, y: 20.0 }, 1.0, true, false).is_some());
        s.kind = Kind::Ellipse;
        let ellipse = draw(&s);
        assert_eq!(ellipse.get_pixel(25, 20)[3], 255);
        assert_eq!(ellipse.get_pixel(10, 10)[3], 0);
        s.kind = Kind::Rectangle;
        s.fill = Fill::None;
        assert!(draw(&s).get_pixel(10, 10)[3] > 0);
        s.corner_radius = 8.0;
        let rounded = draw(&s);
        assert_eq!(rounded.get_pixel(10, 10)[3], 0);
        assert!(rounded.get_pixel(25, 10)[3] > 0 && rounded.get_pixel(10, 20)[3] > 0);
        assert_eq!(rounded.get_pixel(25, 20)[3], 0);
        s.fill = Fill::Solid;
        let filled = draw(&s);
        assert_eq!(filled.get_pixel(10, 10)[3], 0);
        assert_eq!(filled.get_pixel(25, 20)[3], 255);
        // 虚线圆角矩形：上边出现空隙，内部仍为空。
        s.fill = Fill::None;
        s.line_style = LineStyle::Dashed;
        let dashed = draw(&s);
        assert!(runs(&dashed, 10) > 1);
        assert_eq!(dashed.get_pixel(25, 20)[3], 0);
    }

    #[test]
    fn contrast_picks_dark_ink_on_light_colors() {
        assert_eq!(contrast("#ef4444"), "#ffffff");
        assert_eq!(contrast("#000000"), "#ffffff");
        assert_eq!(contrast("#facc15"), "#111827");
        assert_eq!(contrast("#ffffff"), "#111827");
    }
}
