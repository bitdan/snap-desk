//! 截图取色放大镜：光标处 15×15 原图像素放大 8 倍，显示坐标与色值。
//!
//! 放大镜与取色：
//! Shift 切换 HEX / RGB，C 复制色值（保留截图会话，可继续取色或框选），P 切换全局/相对坐标。
//! 像素格直接用 225 个纯色方块绘制，不生成纹理，光标高频移动时没有 GPU 上传与释放。
use gpui_kit::*;
use image::RgbaImage;

pub const PIXELS: i64 = 15;
pub const ZOOM: f32 = 8.0;
const PANEL_WIDTH: f32 = 167.0;
const PANEL_HEIGHT: f32 = 212.0;

/// 取色状态，由截图会话持有。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Picker {
    /// 光标所在浮层序号与原图像素。
    pub at: Option<(usize, u32, u32)>,
    pub hex: bool,
    pub relative: bool,
    pub copied: bool,
}

/// 色值文本。
pub fn color_text(image: &RgbaImage, x: u32, y: u32, hex: bool) -> Option<String> {
    if x >= image.width() || y >= image.height() {
        return None;
    }
    let p = image.get_pixel(x, y);
    Some(if hex {
        format!("#{:02X}{:02X}{:02X}", p[0], p[1], p[2])
    } else {
        format!("rgb({}, {}, {})", p[0], p[1], p[2])
    })
}

/// 优先放在光标右下方，空间不足时翻到左/上方；不产生负坐标。
pub fn panel_position(pointer: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    let place = |p: f32, size: f32, limit: f32| {
        let v = if p + 24.0 + size <= limit - 8.0 { p + 24.0 } else { p - 24.0 - size };
        v.clamp(0.0, (limit - size).max(0.0))
    };
    (place(pointer.0, PANEL_WIDTH, viewport.0), place(pointer.1, PANEL_HEIGHT, viewport.1))
}

/// 放大镜面板。`pointer` 为窗口内逻辑坐标，`origin` 为相对坐标的原点（选区左上角）。
pub fn element(
    image: &RgbaImage,
    picker: &Picker,
    pixel: (u32, u32),
    pointer: (f32, f32),
    viewport: (f32, f32),
    origin: (u32, u32),
    text: &dyn Fn(&'static str) -> String,
) -> Div {
    let (left, top) = panel_position(pointer, viewport);
    let radius = PIXELS / 2;
    let mut grid = div().relative().size(px(PIXELS as f32 * ZOOM)).bg(rgb(0x000000));
    for row in 0..PIXELS {
        for column in 0..PIXELS {
            let x = i64::from(pixel.0) + column - radius;
            let y = i64::from(pixel.1) + row - radius;
            // 靠近边缘时留空，准星始终指向当前像素。
            if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height()) {
                continue;
            }
            let p = image.get_pixel(x as u32, y as u32);
            grid = grid.child(
                div()
                    .absolute()
                    .left(px(column as f32 * ZOOM))
                    .top(px(row as f32 * ZOOM))
                    .size(px(ZOOM))
                    .bg(rgb(u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))),
            );
        }
    }
    let center = radius as f32 * ZOOM;
    grid = grid
        .child(div().absolute().left(px(0.0)).top(px(center + ZOOM / 2.0 - 0.5)).w_full().h(px(1.0)).bg(rgba(0x2f80ff80)))
        .child(div().absolute().top(px(0.0)).left(px(center + ZOOM / 2.0 - 0.5)).h_full().w(px(1.0)).bg(rgba(0x2f80ff80)))
        .child(div().absolute().left(px(center - 1.0)).top(px(center - 1.0)).size(px(ZOOM + 2.0)).border_1().border_color(rgb(0xffffff)));
    let (cx, cy) = if picker.relative {
        (i64::from(pixel.0) - i64::from(origin.0), i64::from(pixel.1) - i64::from(origin.1))
    } else {
        (i64::from(pixel.0), i64::from(pixel.1))
    };
    let color = color_text(image, pixel.0, pixel.1, picker.hex).unwrap_or_default();
    let swatch = color_text(image, pixel.0, pixel.1, true)
        .and_then(|hex| u32::from_str_radix(&hex[1..], 16).ok())
        .unwrap_or(0);
    let hint = |message: String, highlight: bool| {
        div().text_xs().text_color(rgb(if highlight { 0x86efac } else { 0xa1a1aa })).child(message)
    };
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(PANEL_WIDTH))
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .rounded(px(6.0))
        .bg(rgba(0x18181be8))
        .border_1()
        .border_color(rgb(0x3f3f46))
        .shadow_lg()
        .text_color(rgb(0xf4f4f5))
        .child(grid)
        .child(
            div()
                .text_xs()
                .child(format!("{}({cx}, {cy})", if picker.relative { text("相对 ") } else { String::new() })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_xs()
                .child(div().size(px(12.0)).border_1().border_color(rgb(0xffffff)).bg(rgb(swatch)))
                .child(color),
        )
        .child(hint(text("Shift: 切换颜色格式"), false))
        .child(hint(text(if picker.copied { "已复制，可继续截图" } else { "C: 复制颜色值" }), picker.copied))
        .child(hint(text("P: 切换全局/相对坐标"), false))
}

#[cfg(test)]
mod tests {
    use super::{color_text, panel_position};
    use image::{Rgba, RgbaImage};

    #[test]
    fn color_text_formats_and_rejects_out_of_bounds() {
        let image = RgbaImage::from_pixel(2, 2, Rgba([255, 8, 160, 255]));
        assert_eq!(color_text(&image, 1, 1, true).as_deref(), Some("#FF08A0"));
        assert_eq!(color_text(&image, 0, 0, false).as_deref(), Some("rgb(255, 8, 160)"));
        assert!(color_text(&image, 2, 0, true).is_none());
    }

    #[test]
    fn panel_flips_near_edges_and_stays_on_screen() {
        assert_eq!(panel_position((100.0, 100.0), (1920.0, 1080.0)), (124.0, 124.0));
        let (x, y) = panel_position((1900.0, 1070.0), (1920.0, 1080.0));
        assert!(x + 167.0 <= 1900.0 && y + 212.0 <= 1070.0);
        assert_eq!(panel_position((10.0, 10.0), (100.0, 100.0)), (0.0, 0.0));
    }
}
