//! CPU 色彩转换，供持续编码使用。
#[cfg(any(target_os = "windows", test))]
pub fn nv12(image: &image::RgbaImage) -> (u32, u32, Vec<u8>) {
    let width = (image.width() + 1) & !1;
    let height = (image.height() + 1) & !1;
    let plane = (width * height) as usize;
    let mut data = vec![128; plane + plane / 2];
    data[..plane].fill(16);
    let rgb = |x: u32, y: u32| -> (i32, i32, i32) {
        if x >= image.width() || y >= image.height() {
            return (0, 0, 0);
        }
        let p = image.get_pixel(x, y).0;
        // 屏幕帧通常不透明；忽略 alpha，避免平台未填写 alpha 导致全黑。
        (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]))
    };
    for y in 0..height {
        for x in 0..width {
            let (r, g, b) = rgb(x, y);
            data[(y * width + x) as usize] =
                (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(16, 235) as u8;
        }
    }
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let (mut r, mut g, mut b) = (0, 0, 0);
            for dy in 0..2 {
                for dx in 0..2 {
                    let pixel = rgb(x + dx, y + dy);
                    r += pixel.0;
                    g += pixel.1;
                    b += pixel.2;
                }
            }
            r = (r + 2) / 4;
            g = (g + 2) / 4;
            b = (b + 2) / 4;
            let offset = plane + (y / 2 * width + x) as usize;
            data[offset] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
            data[offset + 1] =
                (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
        }
    }
    (width, height, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn black_white_and_odd_padding() {
        use image::{Rgba, RgbaImage};
        assert_eq!(
            nv12(&RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))).2,
            vec![16, 16, 16, 16, 128, 128]
        );
        let (w, h, data) = nv12(&RgbaImage::from_pixel(3, 3, Rgba([255, 255, 255, 255])));
        assert_eq!((w, h, data.len()), (4, 4, 24));
        assert_eq!(&data[..8], &[235, 235, 235, 16, 235, 235, 235, 16]);
    }
}
