//! 系统录屏编码边界。Windows 使用 Media Foundation；其余平台暂不降级到外部程序。
#[cfg(not(target_os = "windows"))]
use std::path::Path;

#[cfg(target_os = "windows")]
#[path = "windows_recording.rs"]
mod native;
#[cfg(target_os = "windows")]
pub use native::{encode, probe};

#[cfg(not(target_os = "windows"))]
pub fn probe(_root: &Path, _width: u32, _height: u32) -> Result<(), String> {
    Err("当前仅 Windows 已接入系统原生录屏编码；macOS / Linux 原生适配尚未实现。".into())
}

#[cfg(not(target_os = "windows"))]
pub fn encode(
    _output: &Path,
    _frames: &Path,
    _samples: &[(i64, i64)],
    _cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    Err("当前平台尚未实现系统原生录屏编码。".into())
}

/// Media Foundation 使用 100 ns 为时间单位。起点归零，慢帧保留真实间隔。
pub fn sample_times(times: &[f64], end: f64) -> Result<Vec<(i64, i64)>, String> {
    if times.is_empty()
        || !end.is_finite()
        || times.iter().any(|t| !t.is_finite() || *t < 0.0)
        || times.windows(2).any(|pair| pair[1] <= pair[0])
        || end < *times.last().unwrap()
    {
        return Err("录屏帧时间无效。".into());
    }
    let ticks = |time: f64| ((time - times[0]) * 10_000_000.0).round() as i64;
    Ok(times
        .iter()
        .enumerate()
        .map(|(index, &time)| {
            let start = ticks(time);
            let stop = ticks(times.get(index + 1).copied().unwrap_or(end));
            (start, (stop - start).max(1))
        })
        .collect())
}

/// RGBA 转为系统 H.264 编码器直接接受的 NV12（BT.601 有限范围）。
/// 偶数宽高，Y 平面在前，2×2 像素平均后的交错 UV 平面在后；奇数边缘补黑。
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
    fn time_origin_and_slow_frames_are_preserved() {
        assert_eq!(
            sample_times(&[0.02, 0.12, 0.47], 0.72).unwrap(),
            vec![
                (0, 1_000_000),
                (1_000_000, 3_500_000),
                (4_500_000, 2_500_000)
            ]
        );
        assert!(sample_times(&[], 1.0).is_err());
        assert!(sample_times(&[1.0, 0.5], 2.0).is_err());
        assert!(sample_times(&[f64::NAN], 1.0).is_err());
    }
    #[test]
    fn nv12_black_white_red_and_odd_padding() {
        use image::{Rgba, RgbaImage};
        let (_, _, black) = nv12(&RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255])));
        assert_eq!(black, vec![16, 16, 16, 16, 128, 128]);
        let (_, _, white) = nv12(&RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 255])));
        assert_eq!(white, vec![235, 235, 235, 235, 128, 128]);
        let (_, _, red) = nv12(&RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255])));
        assert_eq!(red, vec![82, 82, 82, 82, 90, 240]);
        let (w, h, odd) = nv12(&RgbaImage::from_pixel(3, 3, Rgba([255, 255, 255, 255])));
        assert_eq!((w, h, odd.len()), (4, 4, 24));
        assert_eq!(&odd[..8], &[235, 235, 235, 16, 235, 235, 235, 16]);
        assert_eq!(&odd[12..16], &[16; 4]);
    }
}
