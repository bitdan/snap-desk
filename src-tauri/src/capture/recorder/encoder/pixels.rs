//! BGRA 直接转换为 BT.601 有限范围 NV12；每个 2×2 块只读一次源像素。
use super::BgraImage;
use std::mem::MaybeUninit;

/// NV12 使用偶数尺寸；返回精确字节数，拒绝空画面和长度溢出。
#[cfg(any(target_os = "windows", test))]
pub fn nv12_len(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 {
        return Err("编码画面尺寸不能为空。".into());
    }
    let width = (width as usize).checked_add(1).map(|v| v & !1);
    let height = (height as usize).checked_add(1).map(|v| v & !1);
    width
        .zip(height)
        .and_then(|(w, h)| w.checked_mul(h))
        .and_then(|plane| plane.checked_add(plane / 2))
        .ok_or_else(|| "NV12 缓冲尺寸溢出。".into())
}

/// 直接写入调用者独占的缓冲，允许传入已锁定的 MF 内存，避免 Vec → MF 的整帧复制。
/// 奇数宽高补黑，忽略屏幕帧中可能未填写的 alpha；任何校验失败均不修改目标。
#[cfg(any(target_os = "windows", test))]
pub fn nv12_into(image: &BgraImage, data: &mut [MaybeUninit<u8>]) -> Result<(), String> {
    let expected = nv12_len(image.width, image.height)?;
    let source_len = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if source_len != Some(image.data.len()) || data.len() != expected {
        return Err("BGRA / NV12 缓冲长度与画面尺寸不一致。".into());
    }
    let width = (image.width as usize + 1) & !1;
    let height = (image.height as usize + 1) & !1;
    let plane = width * height;
    // 不读取目标原内容；循环覆盖全部 Y/UV，包括补齐的边缘，无需预先清零。
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
                    data[py * width + px].write(
                        (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(16, 235) as u8,
                    );
                    red += r;
                    green += g;
                    blue += b;
                }
            }
            let (r, g, b) = ((red + 2) / 4, (green + 2) / 4, (blue + 2) / 4);
            let offset = plane + (y / 2 * width + x);
            data[offset]
                .write((((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8);
            data[offset + 1]
                .write((((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(16, 240) as u8);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 已初始化的 u8 可以安全视为 MaybeUninit<u8>；保留哨兵检查写入边界与失败原子性。
    fn nv12_into(image: &BgraImage, data: &mut [u8]) -> Result<(), String> {
        let destination = unsafe {
            std::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<MaybeUninit<u8>>(), data.len())
        };
        super::nv12_into(image, destination)
    }

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> BgraImage {
        let mut image = BgraImage::new(width, height);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&bgra);
        }
        image
    }

    #[test]
    fn primary_colors_and_alpha_are_correct() {
        let mut data = vec![0; 6];
        for (bgra, expected) in [
            ([0, 0, 0, 255], [16, 16, 16, 16, 128, 128]),
            ([255, 255, 255, 0], [235, 235, 235, 235, 128, 128]),
            ([0, 0, 255, 0], [82, 82, 82, 82, 90, 240]),
            ([255, 0, 0, 255], [41, 41, 41, 41, 240, 110]),
        ] {
            nv12_into(&solid(2, 2, bgra), &mut data).unwrap();
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
        let mut data = vec![0; 6];
        nv12_into(&image, &mut data).unwrap();
        assert_eq!(data, [82, 144, 41, 235, 128, 128]);
    }

    #[test]
    fn odd_padding_and_reused_buffer_are_fully_overwritten() {
        let mut data = vec![0xCD; nv12_len(3, 3).unwrap()];
        nv12_into(&solid(3, 3, [255; 4]), &mut data).unwrap();
        assert_eq!(data.len(), 24);
        assert_eq!(&data[..8], &[235, 235, 235, 16, 235, 235, 235, 16]);
        assert_eq!(&data[12..16], &[16; 4]);
        let pointer = data.as_ptr();
        nv12_into(&solid(3, 3, [0; 4]), &mut data).unwrap();
        assert_eq!(pointer, data.as_ptr());
        assert_eq!(&data[..16], &[16; 16]);
        assert_eq!(&data[16..], &[128; 8]);
        data.resize(6, 0xCD);
        nv12_into(&solid(2, 2, [0, 0, 255, 255]), &mut data).unwrap();
        assert_eq!(data, [82, 82, 82, 82, 90, 240]);
    }

    #[test]
    fn invalid_lengths_do_not_touch_destination_and_slices_keep_guards() {
        let mut guarded = [0xCD; 8];
        let image = solid(2, 2, [0, 0, 255, 255]);
        nv12_into(&image, &mut guarded[1..7]).unwrap();
        assert_eq!(guarded, [0xCD, 82, 82, 82, 82, 90, 240, 0xCD]);
        for len in [0, 5, 7] {
            let mut data = vec![0xCD; len];
            assert!(nv12_into(&image, &mut data).is_err());
            assert_eq!(data, vec![0xCD; len]);
        }
        let mut broken = image;
        broken.data.pop();
        let mut data = [0xCD; 6];
        assert!(nv12_into(&broken, &mut data).is_err());
        assert_eq!(data, [0xCD; 6]);
        assert!(nv12_len(0, 2).is_err());
        assert!(nv12_len(u32::MAX, u32::MAX).is_err());
    }
}
