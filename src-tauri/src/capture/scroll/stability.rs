//! 按区域判断静止：少量局部动画或实时读数不阻塞采集，大范围滚动仍须等待稳定。
use image::RgbaImage;

/// 8×8 分格；只统计非纯色或发生变化的格子，空白格不能为运动“凑票”。
/// 原地刷新的读数只影响少数格子；滚动会让几乎所有有纹理的格子同时变化。
pub fn stable(a: &RgbaImage, b: &RgbaImage) -> bool {
    if a.dimensions() != b.dimensions() || a.width() < 4 || a.height() < 4 {
        return false;
    }
    let grid = if a.width() >= 64 && a.height() >= 64 {
        8
    } else {
        4
    };
    let mut active = 0u32;
    let mut noisy = 0u32;
    let mut changed_samples = 0u64;
    let mut all_samples = 0u64;
    let mut noisy_rows = [0u32; 8];
    let mut active_rows = [0u32; 8];
    let mut noisy_columns = [0u32; 8];
    let mut active_columns = [0u32; 8];
    for row in 0..grid {
        for column in 0..grid {
            let mut changed = 0u32;
            let mut total = 0u64;
            let mut samples = 0u32;
            // 格内亮度范围；纯色背景格不算有效格，渐变和文字都算。
            let (mut low, mut high) = (u32::MAX, 0u32);
            for y in (row * a.height() / grid..(row + 1) * a.height() / grid).step_by(2) {
                for x in (column * a.width() / grid..(column + 1) * a.width() / grid).step_by(2) {
                    let diff: u32 = (0..3)
                        .map(|c| u32::from(a.get_pixel(x, y)[c].abs_diff(b.get_pixel(x, y)[c])))
                        .sum();
                    total += u64::from(diff);
                    changed += u32::from(diff > 36);
                    samples += 1;
                    let p = a.get_pixel(x, y);
                    let level = u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
                    low = low.min(level);
                    high = high.max(level);
                }
            }
            changed_samples += u64::from(changed);
            all_samples += u64::from(samples);
            let quiet = changed * 100 <= samples && total as f64 <= f64::from(samples) * 3.0;
            if !quiet || high.saturating_sub(low) > 24 {
                active += 1;
                active_rows[row as usize] += 1;
                active_columns[column as usize] += 1;
            }
            if !quiet {
                noisy += 1;
                noisy_rows[row as usize] += 1;
                noisy_columns[column as usize] += 1;
            }
        }
    }
    if noisy == 0 {
        return true;
    }
    // 局部变化：变化格不超过有效格的 1/5，且整体变化像素不超过 6%。
    // 任一行或列的有效格过半变化时，可能是固定侧栏或表头掩盖下的正文滚动，仍视为运动。
    let local = noisy * 5 <= active && changed_samples * 100 <= all_samples * 6;
    let banded = (0..grid as usize).any(|i| {
        (active_rows[i] >= 2 && noisy_rows[i] * 2 > active_rows[i])
            || (active_columns[i] >= 2 && noisy_columns[i] * 2 > active_columns[i])
    });
    local && !banded
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn local_animation_is_allowed_but_content_motion_is_not() {
        let a = RgbaImage::from_fn(96, 160, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        let mut b = a.clone();
        for y in 40..60 {
            for x in 24..40 {
                b.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        assert!(stable(&a, &b));
        assert!(!stable(
            &a,
            &RgbaImage::from_pixel(96, 160, Rgba([0, 0, 0, 255]))
        ));
        assert!(!stable(&a, &RgbaImage::new(95, 160)));
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn fixed_header_does_not_hide_fast_scroll_or_continuous_animation() {
        let frame = |offset: u32| {
            RgbaImage::from_fn(160, 160, |x, y| {
                let y = if y < 32 { y } else { y + offset };
                let v =
                    (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
                Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
            })
        };
        let first = frame(0);
        for offset in [1, 10, 80, 200] {
            assert!(!stable(&first, &frame(offset)));
        }
        assert!(stable(&first, &first));
        let animated = RgbaImage::from_pixel(160, 160, Rgba([255, 0, 255, 255]));
        assert!(!stable(&first, &animated));
    }
}
