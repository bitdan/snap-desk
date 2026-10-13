//! BGRA 直接转换为 BT.601 有限范围 NV12；每个 2×2 块只读一次源像素。
use super::BgraImage;

/// 编码线程复用 data；奇数宽高补黑，忽略屏幕帧中可能未填写的 alpha。
#[cfg(any(target_os = "windows", test))]
pub fn nv12_into(image: &BgraImage, data: &mut Vec<u8>) {
    let width = ((image.width + 1) & !1) as usize;
    let height = ((image.height + 1) & !1) as usize;
    let plane = width * height;
    // 同尺寸不重新分配；循环覆盖全部 Y/UV，包括补齐的边缘。
    data.resize(plane + plane / 2, 0);
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let (mut red, mut green, mut blue) = (0, 0, 0);
            for dy in 0..2 {
                for dx in 0..2 {
                    let (px, py) = (x + dx, y + dy);
                    let (r, g, b) = if px < image.width as usize && py < image.height as usize {
                        let offset = (py * image.width as usize + px) * 4;
                        (
                            i32::from(image.data[offset + 2]),
                            i32::from(image.data[offset + 1]),
                            i32::from(image.data[offset]),
                        )
                    } else {
                        (0, 0, 0)
                    };
                    data[py * width + px] =
                        (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(16, 235) as u8;
                    red += r;
                    green += g;
                    blue += b;
                }
            }
            let (r, g, b) = ((red + 2) / 4, (green + 2) / 4, (blue + 2) / 4);
            let offset = plane + (y / 2 * width + x);
            data[offset] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
            data[offset + 1] =
                (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> BgraImage {
        let mut image = BgraImage::new(width, height);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&bgra);
        }
        image
    }

    #[test]
    fn primary_colors_and_alpha_are_correct() {
        let mut data = Vec::new();
        for (bgra, expected) in [
            ([0, 0, 0, 255], [16, 16, 16, 16, 128, 128]),
            ([255, 255, 255, 0], [235, 235, 235, 235, 128, 128]),
            ([0, 0, 255, 0], [82, 82, 82, 82, 90, 240]),
            ([255, 0, 0, 255], [41, 41, 41, 41, 240, 110]),
        ] {
            nv12_into(&solid(2, 2, bgra), &mut data);
            assert_eq!(data, expected);
        }
    }

    #[test]
    fn mixed_block_averages_chroma_once() {
        let image = BgraImage {
            width: 2,
            height: 2,
            data: vec![0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        };
        let mut data = Vec::new();
        nv12_into(&image, &mut data);
        assert_eq!(data, [82, 144, 41, 235, 128, 128]);
    }

    #[test]
    fn odd_padding_and_reused_buffer_are_fully_overwritten() {
        let mut data = Vec::new();
        nv12_into(&solid(3, 3, [255; 4]), &mut data);
        assert_eq!(data.len(), 24);
        assert_eq!(&data[..8], &[235, 235, 235, 16, 235, 235, 235, 16]);
        assert_eq!(&data[12..16], &[16; 4]);
        let pointer = data.as_ptr();
        nv12_into(&solid(3, 3, [0; 4]), &mut data);
        assert_eq!(pointer, data.as_ptr());
        assert_eq!(&data[..16], &[16; 16]);
        assert_eq!(&data[16..], &[128; 8]);
        nv12_into(&solid(2, 2, [0, 0, 255, 255]), &mut data);
        assert_eq!(data, [82, 82, 82, 82, 90, 240]);
    }
}
