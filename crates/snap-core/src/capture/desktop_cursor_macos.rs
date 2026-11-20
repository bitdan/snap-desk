//! macOS 截图光标：系统截图接口不包含鼠标指针，这里读取当前系统光标的图像与热点，
//! 按截图所在屏的像素缩放渲染为位图，再由调用方叠加到截图上。
use crate::macos::{ns_string, Id};
use image::RgbaImage;
use objc2::{
    class,
    encode::{Encode, Encoding},
    msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, Bool},
};

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}
unsafe impl Encode for NSPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}
unsafe impl Encode for NSSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}
unsafe impl Encode for NSRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
}

/// NSCompositingOperationCopy：位图初始为全透明，直接写入光标像素。
const COMPOSITE_COPY: usize = 1;
/// 光标位图边长上限（像素），防止异常尺寸占用过多内存。
const MAX_SIDE: f64 = 1024.0;

/// 光标在截图中的位图：`left`/`top` 为位图左上角相对截图原点的像素位置（已减去热点）。
pub struct Cursor {
    pub image: RgbaImage,
    pub left: i64,
    pub top: i64,
}

/// `origin` 为截图左上角的全局点坐标，`scale` 为截图像素/点之比。
/// 读不到鼠标位置或光标图像（例如光标被隐藏）时返回 None，截图照常进行、不含光标。
pub fn current(origin: (f64, f64), scale: f64) -> Option<Cursor> {
    let (cx, cy) = crate::macos::cursor_point()?;
    let (image, hot_x, hot_y) = bitmap(scale)?;
    Some(Cursor {
        image,
        left: ((cx - origin.0) * scale - hot_x).round() as i64,
        top: ((cy - origin.1) * scale - hot_y).round() as i64,
    })
}

/// 渲染当前系统光标（不限于本应用的光标）；返回位图与像素热点。
fn bitmap(scale: f64) -> Option<(RgbaImage, f64, f64)> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let mut cursor: Id = msg_send![class!(NSCursor), currentSystemCursor];
        if cursor.is_null() {
            cursor = msg_send![class!(NSCursor), arrowCursor];
        }
        if cursor.is_null() {
            return None;
        }
        let image: Id = msg_send![cursor, image];
        if image.is_null() {
            return None;
        }
        // NSCursor 的热点以图像左上角为原点、单位为点。
        let size: NSSize = msg_send![image, size];
        let hot: NSPoint = msg_send![cursor, hotSpot];
        let (w, h) = ((size.width * scale).round(), (size.height * scale).round());
        if !(1.0..=MAX_SIDE).contains(&w) || !(1.0..=MAX_SIDE).contains(&h) {
            return None;
        }
        let (w, h) = (w as isize, h as isize);
        let allocated: Allocated<AnyObject> = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
            pixelsWide: w,
            pixelsHigh: h,
            bitsPerSample: 8isize,
            samplesPerPixel: 4isize,
            hasAlpha: Bool::YES,
            isPlanar: Bool::NO,
            colorSpaceName: ns_string("NSDeviceRGBColorSpace"),
            bytesPerRow: 0isize,
            bitsPerPixel: 0isize
        ];
        let rep = rep?;
        let context: Id =
            msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: &*rep];
        if context.is_null() {
            return None;
        }
        let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: context];
        let target = NSRect {
            origin: NSPoint { x: 0.0, y: 0.0 },
            size: NSSize {
                width: w as f64,
                height: h as f64,
            },
        };
        let whole = NSRect {
            origin: NSPoint { x: 0.0, y: 0.0 },
            size: NSSize {
                width: 0.0,
                height: 0.0,
            },
        };
        let _: () = msg_send![
            image,
            drawInRect: target,
            fromRect: whole,
            operation: COMPOSITE_COPY,
            fraction: 1.0f64
        ];
        let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
        let data: *const u8 = msg_send![&*rep, bitmapData];
        let row: isize = msg_send![&*rep, bytesPerRow];
        if data.is_null() || row < w * 4 {
            return None;
        }
        let (w, h, row) = (w as usize, h as usize, row as usize);
        let bytes = std::slice::from_raw_parts(data, row * h);
        let mut pixels = Vec::with_capacity(w * h * 4);
        for line in bytes.chunks_exact(row) {
            pixels.extend_from_slice(&line[..w * 4]);
        }
        unpremultiply(&mut pixels);
        let image = RgbaImage::from_raw(w as u32, h as u32, pixels)?;
        Some((image, hot.x * scale, hot.y * scale))
    })
}

/// NSBitmapImageRep 默认存储预乘 alpha，叠加前还原为直通 alpha。
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::unpremultiply;
    #[test]
    fn restores_straight_alpha_and_keeps_opaque_and_clear_pixels() {
        let mut pixels = [64, 32, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }
}
