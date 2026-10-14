//! 按区域判断静止：少量局部动画不阻塞采集，大范围滚动仍须等待稳定。
use image::RgbaImage;

pub fn stable(a: &RgbaImage, b: &RgbaImage) -> bool {
    if a.dimensions() != b.dimensions() || a.width() < 4 || a.height() < 4 {
        return false;
    }
    let mut quiet = 0;
    let mut quiet_rows = [0; 4];
    let mut quiet_columns = [0; 4];
    for row in 0..4 {
        for column in 0..4 {
            let mut changed = 0;
            let mut total = 0;
            let mut samples = 0;
            for y in (row * a.height() / 4..(row + 1) * a.height() / 4).step_by(2) {
                for x in (column * a.width() / 4..(column + 1) * a.width() / 4).step_by(2) {
                    let diff: u32 = (0..3)
                        .map(|c| u32::from(a.get_pixel(x, y)[c].abs_diff(b.get_pixel(x, y)[c])))
                        .sum();
                    total += u64::from(diff);
                    changed += u32::from(diff > 36);
                    samples += 1;
                }
            }
            if changed * 100 <= samples && total as f64 <= f64::from(samples) * 3.0 {
                quiet += 1;
                quiet_rows[row as usize] += 1;
                quiet_columns[column as usize] += 1;
            }
        }
    }
    // 至少 14/16 格静止，并覆盖每一行/列；不允许固定侧栏掩盖主体滚动。
    quiet >= 14 && quiet_rows.iter().all(|n| *n >= 2) && quiet_columns.iter().all(|n| *n >= 2)
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
