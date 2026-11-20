//! 贴图的纯逻辑：外观配置、窗口布局（物理像素）、容量预算、缩放步进与像素变换。
//! 窗口、图片缓存与系统交互由各外壳实现（Tauri 版 `src-tauri/src/pins`，GPUI 版 `apps/desktop-gpui/src/pins.rs`）。
use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub mod content;
pub mod text;

/// 全局贴图外观只影响显示；阴影由 WebView 按尺寸和焦点计算，原图像素保持独立。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PinAppearance {
    pub shadow: PinShadow,
    pub corner_radius: u8,
    /// 只接受 #RRGGBB，旧配置缺少字段时沿用默认中性灰。
    pub border_color: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinShadow {
    #[default]
    Auto,
    Soft,
    None,
}
impl Default for PinAppearance {
    fn default() -> Self {
        Self {
            shadow: PinShadow::Auto,
            corner_radius: 8,
            border_color: "#a1a1aa".into(),
        }
    }
}
impl PinAppearance {
    pub fn validate(&self) -> Result<(), String> {
        let color = self.border_color.as_bytes();
        if color.len() != 7 || color[0] != b'#' || !color[1..].iter().all(u8::is_ascii_hexdigit) {
            return Err("边框颜色需为六位十六进制颜色。".into());
        }
        if self.corner_radius > 24 {
            return Err("贴图圆角需为 0～24 像素。".into());
        }
        Ok(())
    }
}

/// 物理像素坐标（可为负，副屏在主屏左侧/上方时）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}
impl Position {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}
/// 物理像素尺寸。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}
impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// 同时存在的贴图数上限与像素预算：单张 1600 万像素，原图与处理图的独立缓存合计 3200 万像素。
pub const MAX_PINS: usize = 12;
pub const MAX_IMAGE_PIXELS: u64 = 16_000_000;
pub const MAX_TOTAL_PIXELS: u64 = 32_000_000;
/// 缩放上限 400%；极细图片为满足最小可见边可超出。
pub const MAX_ZOOM: f64 = 4.0;

/// 缩放只改变查看比例，按 10% 步进并消除小数累计误差（与前端 stepZoom 一致）；导出始终使用原图像素。
pub fn step_zoom(zoom: f64, direction: i32, minimum: f64) -> f64 {
    let next = ((zoom + f64::from(direction.signum()) * 0.1) * 10.0).round() / 10.0;
    next.max(minimum).min(MAX_ZOOM.max(minimum))
}

/// 与 Vue 共用同一份布局尺寸，单位均为逻辑像素。
/// 窗口只包住图片；工具栏显示时才在图片上方或下方扩出一条，图片不让位、不被裁。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinLayout {
    pub padding: f64,
    pub gap: f64,
    pub border: f64,
    pub min_visible_side: f64,
    /// 工具栏单行加样式行的高度；显示时整体放在图片外侧。
    pub toolbar_height: f64,
    /// 工具栏完整显示所需宽度；仅在工具栏显示时撑宽窗口。
    pub toolbar_width: f64,
    /// 空闲窗口的透明下限，供右键菜单展开；多出的透明部分由 set_pin_region 裁掉。
    pub min_window_width: f64,
    pub min_window_height: f64,
}

/// 一次布局结果：窗口外框（物理像素）、图片首像素相对窗口的偏移、工具栏是否在图片上方。
#[derive(Debug, PartialEq)]
pub struct Placement {
    pub position: Position,
    pub size: Size,
    pub image: Position,
    pub toolbar_above: bool,
}

/// 显示器工作区，物理像素。
#[derive(Clone, Copy, Debug)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PinLayout {
    pub fn load() -> &'static Self {
        // 编译时嵌入仓库内常量，不读取用户文件；前端直接导入相同 JSON。
        // 内容在进程生命周期内不变，只解析一次；缩放、滚轮等高频命令直接复用。
        static LAYOUT: std::sync::OnceLock<PinLayout> = std::sync::OnceLock::new();
        LAYOUT.get_or_init(|| {
            serde_json::from_str(include_str!("../../../../src/features/pins/pin-layout.json"))
                .expect("内置贴图布局配置无效")
        })
    }
    /// 与前端使用相同短边下限，原生命令也不能把小图缩成不可见的小点。
    pub fn minimum_zoom(&self, width: u32, height: u32, scale: f64) -> f64 {
        (self.min_visible_side * scale / f64::from(width.min(height).max(1))).max(0.1)
    }
    /// 图片首像素到窗口原点的物理距离；与前端 padding 取整规则一致。
    pub fn image_inset(&self, scale: f64) -> i32 {
        ((self.padding + self.border) * scale).round() as i32
    }
    /// 已测得的工具栏宽度优先；未测量时按布局常量预留。
    pub fn toolbar_width_for(&self, measured: f64) -> f64 {
        if measured > 0.0 {
            measured
        } else {
            self.toolbar_width
        }
    }
    /// 工具栏占用的高度（含与图片的间隙），物理像素。
    pub fn toolbar_band(&self, scale: f64) -> i32 {
        ((self.gap + self.toolbar_height) * scale).ceil() as i32
    }
    /// 新贴图默认原尺寸；放不进工作区（含为工具栏预留的一条）时等比缩小到能完整显示。
    pub fn fit_zoom(&self, width: u32, height: u32, scale: f64, work: WorkArea) -> f64 {
        let inset = f64::from(self.image_inset(scale) * 2);
        let room_width = f64::from(work.width) - inset;
        let room_height = f64::from(work.height - self.toolbar_band(scale)) - inset;
        let fit = (room_width / f64::from(width.max(1)))
            .min(room_height / f64::from(height.max(1)))
            .min(1.0);
        // 保留两位小数，滚轮按 10% 步进时仍从可读的比例出发。
        ((fit * 100.0).floor() / 100.0).max(self.minimum_zoom(width, height, scale))
    }
    /// 图片按缩放比例显示的物理尺寸。
    pub fn displayed(&self, width: u32, height: u32, zoom: f64) -> (i32, i32) {
        (
            (f64::from(width) * zoom).round().max(1.0) as i32,
            (f64::from(height) * zoom).round().max(1.0) as i32,
        )
    }

    /// 原位贴图：图片首像素对准选区，不按工作区裁小窗口。
    pub fn source_bounds(
        &self,
        origin: Position,
        width: u32,
        height: u32,
        scale: f64,
        zoom: f64,
    ) -> (Position, Size) {
        let inset = self.image_inset(scale);
        let (width, height) = self.displayed(width, height, zoom);
        (
            Position::new(origin.x - inset, origin.y - inset),
            self.window_size(width, height, scale, None),
        )
    }

    /// `toolbar` 为显示中的工具栏宽度（逻辑像素），`None` 表示工具栏隐藏。
    pub fn window_size(
        &self,
        width: i32,
        height: i32,
        scale: f64,
        toolbar: Option<f64>,
    ) -> Size {
        let inset = self.image_inset(scale);
        let content = if let Some(toolbar) = toolbar {
            width.max((toolbar * scale).ceil() as i32)
        } else {
            width
        };
        let band = if toolbar.is_some() {
            self.toolbar_band(scale)
        } else {
            0
        };
        Size::new(
            (content + inset * 2).max((self.min_window_width * scale).ceil() as i32) as u32,
            (height + inset * 2 + band).max((self.min_window_height * scale).ceil() as i32) as u32,
        )
    }

    /// 以图片当前屏幕位置为锚点重新布局：图片保持在工作区内，工具栏优先放下方，
    /// 下方放不下放上方，两侧都放不下时才把图片上移；横向超出右缘时窗口向左展开。
    /// `image` 为图片首像素的屏幕坐标，`width`/`height` 为图片显示尺寸。
    pub fn place(
        &self,
        image: Position,
        width: i32,
        height: i32,
        scale: f64,
        work: WorkArea,
        toolbar: Option<f64>,
    ) -> Placement {
        let inset = self.image_inset(scale);
        let band = if toolbar.is_some() {
            self.toolbar_band(scale)
        } else {
            0
        };
        let (left, top) = (work.x, work.y);
        let (right, bottom) = (work.x + work.width, work.y + work.height);
        // 超出工作区的大图由前端在边框内滚动，显示区域按工作区收窄。
        let width = width.min(work.width - inset * 2).max(1);
        let height = height.min(work.height - inset * 2 - band).max(1);
        let x = image.x.clamp(left, left.max(right - width));
        let mut y = image.y.clamp(top, top.max(bottom - height));
        let size = self.window_size(width, height, scale, toolbar);
        let mut toolbar_above = false;
        if toolbar.is_some() && y + height + inset + band > bottom {
            if y - inset - band >= top {
                toolbar_above = true;
            } else {
                y = top.max(bottom - inset - band - height);
            }
        }
        let window_y = if toolbar_above {
            y - inset - band
        } else {
            y - inset
        };
        let mut window_x = x - inset;
        if window_x + size.width as i32 > right {
            window_x = window_x.min((left - inset).max(right - size.width as i32));
        }
        Placement {
            position: Position::new(window_x, window_y),
            size,
            image: Position::new(x - window_x, y - window_y),
            toolbar_above,
        }
    }
}

pub fn validate_capacity(width: u32, height: u32, count: usize, used: u64) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_IMAGE_PIXELS {
        return Err("图片为空或超过 1600 万像素限制。".into());
    }
    if count >= MAX_PINS || used.saturating_add(pixels) > MAX_TOTAL_PIXELS {
        return Err(
            "贴图最多 12 张，原图与处理图的独立缓存合计最多 3200 万像素；请先关闭部分贴图。".into(),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transform {
    RotateRight,
    RotateLeft,
    FlipHorizontal,
    FlipVertical,
    Grayscale,
    Invert,
    Reset,
}
/// 变换只作用于图片像素；反色保留 alpha，恢复原图会同时清除之前的全部变换。
pub fn transform(image: &RgbaImage, original: &RgbaImage, action: Transform) -> RgbaImage {
    match action {
        Transform::RotateRight => image::imageops::rotate90(image),
        Transform::RotateLeft => image::imageops::rotate270(image),
        Transform::FlipHorizontal => image::imageops::flip_horizontal(image),
        Transform::FlipVertical => image::imageops::flip_vertical(image),
        Transform::Reset => original.clone(),
        Transform::Grayscale | Transform::Invert => {
            let mut result = image.clone();
            for pixel in result.pixels_mut() {
                if matches!(action, Transform::Invert) {
                    for channel in &mut pixel.0[..3] {
                        *channel = 255 - *channel;
                    }
                } else {
                    let gray = ((u32::from(pixel[0]) * 299
                        + u32::from(pixel[1]) * 587
                        + u32::from(pixel[2]) * 114
                        + 500)
                        / 1000) as u8;
                    pixel.0[..3].fill(gray);
                }
            }
            result
        }
    }
}
/// 整图变换同步变换标注坐标及字形/滤镜像素，保留可编辑对象而不是合并图层。
pub fn transform_annotations(
    shapes: &[crate::annotations::Annotation],
    width: u32,
    height: u32,
    action: Transform,
) -> Result<Vec<crate::annotations::Annotation>, String> {
    use crate::annotations::{Kind, Point};
    let map = |p: Point| -> Point {
        match action {
            Transform::RotateRight => Point {
                x: f64::from(height) - p.y,
                y: p.x,
            },
            Transform::RotateLeft => Point {
                x: p.y,
                y: f64::from(width) - p.x,
            },
            Transform::FlipHorizontal => Point {
                x: f64::from(width) - p.x,
                y: p.y,
            },
            Transform::FlipVertical => Point {
                x: p.x,
                y: f64::from(height) - p.y,
            },
            _ => p,
        }
    };
    shapes
        .iter()
        .map(|source| {
            let mut s = source.clone();
            s.start = map(s.start);
            s.end = map(s.end);
            s.points = s.points.into_iter().map(map).collect();
            if matches!(
                s.kind,
                Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify
            ) {
                let a = s.start;
                let b = s.end;
                s.start = Point {
                    x: a.x.min(b.x),
                    y: a.y.min(b.y),
                };
                s.end = Point {
                    x: a.x.max(b.x),
                    y: a.y.max(b.y),
                };
                let raster = crate::annotations::raster(source)?;
                let mut png = Cursor::new(Vec::new());
                transform(&raster, &raster, action)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                s.raster = Some(STANDARD.encode(png.into_inner()));
            }
            if matches!(action, Transform::Grayscale | Transform::Invert) {
                let value = u32::from_str_radix(&s.color[1..], 16).map_err(|e| e.to_string())?;
                let pixel = RgbaImage::from_pixel(
                    1,
                    1,
                    image::Rgba([(value >> 16) as u8, (value >> 8) as u8, value as u8, 255]),
                );
                let converted = transform(&pixel, &pixel, action);
                let p = converted.get_pixel(0, 0);
                s.color = format!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2]);
            }
            Ok(s)
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    const WORK: WorkArea = WorkArea {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    #[test]
    fn capture_pin_keeps_image_origin_on_primary_negative_and_scaled_displays() {
        let layout = PinLayout::load();
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for origin in [
                Position::new(0, 0),
                Position::new(320, 180),
                Position::new(-1920, -600),
            ] {
                let inset = layout.image_inset(scale);
                let (position, size) = layout.source_bounds(origin, 1233, 626, scale, 1.0);
                assert_eq!(position.x + inset, origin.x);
                assert_eq!(position.y + inset, origin.y);
                // 空闲窗口只包住图片，不为工具栏预留宽度或高度。
                assert_eq!(size.width as i32, 1233 + inset * 2);
                assert_eq!(size.height as i32, 626 + inset * 2);
            }
        }
        // 小图在最小窗口内也从左上角开始，不因原生最小尺寸额外居中。
        let origin = Position::new(-10, 15);
        let (position, _) = layout.source_bounds(origin, 60, 40, 1.5, 1.0);
        assert_eq!(position.x + layout.image_inset(1.5), origin.x);
        assert_eq!(position.y + layout.image_inset(1.5), origin.y);
    }

    #[test]
    fn effects_survive_pin_rotation_and_flip() {
        let mut png = Cursor::new(Vec::new());
        let pixels = RgbaImage::from_fn(2, 3, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 60, 20, 255])
        });
        pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let lens: crate::annotations::Annotation =
            serde_json::from_value(serde_json::json!({
                "kind":"magnify", "start":{"x":10,"y":5}, "end":{"x":30,"y":25},
                "color":"#ff0000", "width":3, "magnification":3,
                "raster":STANDARD.encode(png.into_inner())
            }))
            .unwrap();
        let spotlight: crate::annotations::Annotation =
            serde_json::from_value(serde_json::json!({
                "kind":"spotlight", "start":{"x":10,"y":5}, "end":{"x":30,"y":25},
                "color":"#000000", "width":3, "opacity":0.7
            }))
            .unwrap();
        let source = vec![lens, spotlight];
        let rotated = transform_annotations(&source, 100, 50, Transform::RotateRight).unwrap();
        crate::annotations::validate(&rotated, 50, 100).unwrap();
        assert_eq!(rotated[0].magnification, Some(3.0));
        assert_eq!(
            crate::annotations::raster(&rotated[0]).unwrap(),
            image::imageops::rotate90(&pixels)
        );
        assert_eq!(rotated[1].start.x, 45.0);
        assert_eq!(rotated[1].end.x, 25.0);
        let flipped = transform_annotations(&source, 100, 50, Transform::FlipHorizontal).unwrap();
        crate::annotations::validate(&flipped, 100, 50).unwrap();
        assert_eq!(
            crate::annotations::raster(&flipped[0]).unwrap(),
            image::imageops::flip_horizontal(&pixels)
        );
        assert!(flipped[1].raster.is_none());
    }


    #[test]
    fn rotating_and_flipping_keep_annotations_editable_and_in_bounds() {
        let s: crate::annotations::Annotation = serde_json::from_value(serde_json::json!({
            "kind":"pen", "start":{"x":10,"y":5}, "end":{"x":30,"y":20},
            "color":"#ff0000", "width":3, "points":[{"x":10,"y":5},{"x":20,"y":25},{"x":30,"y":20}]
        }))
        .unwrap();
        let source = vec![s];
        let rotated = transform_annotations(&source, 100, 50, Transform::RotateRight).unwrap();
        crate::annotations::validate(&rotated, 50, 100).unwrap();
        assert_eq!(rotated[0].start.x, 45.0);
        assert_eq!(rotated[0].points[1].y, 20.0);
        let restored = transform_annotations(&rotated, 50, 100, Transform::RotateLeft).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(&source).unwrap()
        );
        let flipped = transform_annotations(&source, 100, 50, Transform::FlipHorizontal).unwrap();
        assert_eq!(flipped[0].start.x, 90.0);
        let inverted = transform_annotations(&source, 100, 50, Transform::Invert).unwrap();
        assert_eq!(inverted[0].color, "#00ffff");
    }

    #[test]
    fn toolbar_sits_outside_the_image_and_never_moves_it() {
        let layout = PinLayout::load();
        let inset = layout.image_inset(1.0);
        let band = layout.toolbar_band(1.0);
        let image = Position::new(300, 200);
        let hidden = layout.place(image, 800, 400, 1.0, WORK, None);
        assert_eq!(hidden.image, Position::new(inset, inset));
        assert_eq!(
            hidden.size,
            Size::new(800 + inset as u32 * 2, 400 + inset as u32 * 2)
        );
        // 下方放得下：窗口向下扩出一条，图片屏幕位置不变。
        let below = layout.place(image, 800, 400, 1.0, WORK, Some(layout.toolbar_width));
        assert!(!below.toolbar_above);
        assert_eq!(below.position.x + below.image.x, 300);
        assert_eq!(below.position.y + below.image.y, 200);
        assert_eq!(below.size.height as i32, 400 + inset * 2 + band);
        // 贴近底部：工具栏翻到上方，图片仍不动。
        let low = Position::new(300, 1040 - 400 - 4);
        let above = layout.place(low, 800, 400, 1.0, WORK, Some(layout.toolbar_width));
        assert!(above.toolbar_above);
        assert_eq!(above.position.y + above.image.y, low.y);
        assert_eq!(above.image.y, inset + band);
        // 小图靠右：窗口为工具栏向左展开，不越过工作区右缘。
        let right = Position::new(1800, 200);
        let narrow = layout.place(right, 100, 80, 1.0, WORK, Some(layout.toolbar_width));
        assert_eq!(narrow.position.x + narrow.image.x, 1800);
        assert!(narrow.position.x + narrow.size.width as i32 <= WORK.width);
        assert!(narrow.size.width as i32 >= layout.toolbar_width as i32);
    }

    #[test]
    fn tall_images_fit_the_work_area_with_room_for_the_toolbar() {
        let layout = PinLayout::load();
        for scale in [1.0, 1.25, 1.5] {
            let zoom = layout.fit_zoom(640, 1340, scale, WORK);
            assert!(zoom < 1.0);
            let (width, height) = layout.displayed(640, 1340, zoom);
            let inset = layout.image_inset(scale);
            assert!(height + inset * 2 + layout.toolbar_band(scale) <= WORK.height);
            // 显示工具栏时整张图仍完整可见，只把图片上移让出一条。
            let top = Position::new(100, 0);
            let shown = layout.place(top, width, height, scale, WORK, Some(layout.toolbar_width));
            // 透明内边距允许越过屏幕边缘，图片和工具栏本身留在工作区内。
            let image_top = shown.position.y + shown.image.y;
            assert!(image_top >= 0);
            assert!(image_top + height + layout.toolbar_band(scale) <= WORK.height);
        }
        assert_eq!(layout.fit_zoom(800, 600, 1.0, WORK), 1.0);
        // 极细图片仍满足 20px 的可见下限。
        assert!(layout.fit_zoom(2, 100_000, 1.0, WORK) >= layout.minimum_zoom(2, 100_000, 1.0));
    }

    #[test]
    fn idle_window_keeps_room_for_the_context_menu() {
        let layout = PinLayout::load();
        let tiny = layout.place(Position::new(50, 50), 20, 20, 1.0, WORK, None);
        assert_eq!(tiny.size.width, layout.min_window_width as u32);
        assert_eq!(tiny.size.height, layout.min_window_height as u32);
        assert_eq!(
            tiny.image,
            Position::new(layout.image_inset(1.0), layout.image_inset(1.0))
        );
    }

    #[test]
    fn rejects_empty_oversized_and_excess_pins() {
        assert!(validate_capacity(0, 1, 0, 0).is_err());
        assert!(validate_capacity(u32::MAX, u32::MAX, 0, 0).is_err());
        assert!(validate_capacity(1, 1, MAX_PINS, 0).is_err());
        assert!(validate_capacity(1, 1, 0, MAX_TOTAL_PIXELS).is_err());
        assert!(validate_capacity(4000, 4000, 1, 16_000_000).is_ok());
    }

    #[test]
    fn transformations_preserve_alpha_and_restore_original() {
        let image = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 7, 0, 100, 200, 99]).unwrap();
        let flipped = transform(&image, &image, Transform::FlipHorizontal);
        assert_eq!(flipped.get_pixel(0, 0), image.get_pixel(1, 0));
        let rotated = transform(&image, &image, Transform::RotateRight);
        assert_eq!(rotated.dimensions(), (1, 2));
        assert_eq!(transform(&rotated, &image, Transform::RotateLeft), image);
        assert_eq!(transform(&flipped, &image, Transform::Reset), image);
        assert_eq!(
            transform(&image, &image, Transform::Invert)
                .get_pixel(0, 0)
                .0,
            [0, 255, 255, 7]
        );
        assert_eq!(
            transform(&image, &image, Transform::Grayscale)
                .get_pixel(0, 0)
                .0,
            [76, 76, 76, 7]
        );
    }

    #[test]
    fn pin_appearance_defaults_and_validation_keep_old_settings_compatible() {
        let value: PinAppearance = serde_json::from_str("{}").unwrap();
        assert_eq!(value, PinAppearance::default());
        assert!(PinAppearance {
            corner_radius: 25,
            ..value
        }
        .validate()
        .is_err());
        assert!(serde_json::from_str::<PinAppearance>(r#"{"shadow":"unknown"}"#).is_err());
    }

    #[test]
    fn zoom_steps_by_ten_percent_within_limits() {
        assert_eq!(step_zoom(1.0, 1, 0.1), 1.1);
        assert_eq!(step_zoom(0.15, -1, 0.1), 0.1);
        assert_eq!(step_zoom(4.0, 1, 0.1), 4.0);
        // 极细图片的最小缩放超过 400% 时以它为准。
        assert_eq!(step_zoom(5.0, -1, 5.0), 5.0);
    }
}
