//! 行特征粗匹配、区域投票和像素复核；只返回可靠重叠，不修改成图。
use image::RgbaImage;
pub const MIN_OVERLAP: u32 = 32;
const COLUMNS: usize = 48;

pub(super) struct Signature {
    pixels: Vec<[u8; 3]>,
    edges: Vec<bool>,
    height: u32,
}

impl Signature {
    pub(super) fn new(image: &RgbaImage) -> Self {
        let mut pixels = Vec::with_capacity(image.height() as usize * COLUMNS);
        let mut edges = Vec::with_capacity(pixels.capacity());
        // 均匀抽取列但保留每一行，位移仍精确到原图像素，不按 DPI 粗略取整。
        for y in 0..image.height() {
            for column in 0..COLUMNS {
                let x =
                    ((column + 1) as u64 * u64::from(image.width()) / (COLUMNS + 1) as u64) as u32;
                let p = image.get_pixel(x, y);
                let right = image.get_pixel((x + 1).min(image.width() - 1), y);
                let below = image.get_pixel(x, (y + 1).min(image.height() - 1));
                pixels.push([p[0], p[1], p[2]]);
                edges.push(
                    (0..3).any(|c| p[c].abs_diff(right[c]) > 16 || p[c].abs_diff(below[c]) > 16),
                );
            }
        }
        Self {
            pixels,
            edges,
            height: image.height(),
        }
    }
}

fn unchanged(a: &Signature, b: &Signature) -> bool {
    let mut total = 0u64;
    let mut changed = 0usize;
    for (a, b) in a.pixels.iter().zip(&b.pixels) {
        let diff = (0..3).map(|c| u32::from(a[c].abs_diff(b[c]))).sum::<u32>();
        total += u64::from(diff);
        changed += usize::from(diff > 36);
    }
    total as f64 / (a.pixels.len() * 3) as f64 <= 0.4
        && changed as f64 / a.pixels.len() as f64 <= 0.005
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Match {
    Unchanged,
    Overlap(u32),
    Uncertain,
}

/// 在纵向位移范围内匹配内容边缘。白色背景不计作匹配证据，多个近似解拒绝自动拼接。
fn match_region(a: &Signature, b: &Signature) -> Match {
    if unchanged(a, b) {
        return Match::Unchanged;
    }
    let mut scores = Vec::new();
    for shift in 1..=a.height - MIN_OVERLAP {
        let overlap = a.height - shift;
        let rows = overlap.min(64);
        let mut total = 0u64;
        let mut count = 0u32;
        let mut mismatched = 0u32;
        let mut row_votes = 0u32;
        let mut textured_rows = 0u32;
        // NCC 校验亮度结构，行投票避免仅靠少数边缘碰巧匹配就接受接缝。
        let (mut sa, mut sb, mut aa, mut bb, mut ab) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for row in 0..rows {
            let y = row * (overlap - 1) / (rows - 1);
            let mut row_count = 0u32;
            let mut row_good = 0u32;
            for x in 0..COLUMNS {
                let ai = (y + shift) as usize * COLUMNS + x;
                let bi = y as usize * COLUMNS + x;
                if !a.edges[ai] && !b.edges[bi] {
                    continue;
                }
                let diff = (0..3)
                    .map(|c| u32::from(a.pixels[ai][c].abs_diff(b.pixels[bi][c])))
                    .sum::<u32>();
                total += u64::from(diff);
                count += 1;
                mismatched += u32::from(diff > 48);
                row_count += 1;
                row_good += u32::from(diff <= 48);
                let av = a.pixels[ai].iter().map(|v| f64::from(*v)).sum::<f64>() / 3.0;
                let bv = b.pixels[bi].iter().map(|v| f64::from(*v)).sum::<f64>() / 3.0;
                sa += av;
                sb += bv;
                aa += av * av;
                bb += bv * bv;
                ab += av * bv;
            }
            if row_count >= 4 {
                textured_rows += 1;
                row_votes += u32::from(row_good * 5 >= row_count * 4);
            }
        }
        let n = f64::from(count.max(1));
        let variance = ((aa - sa * sa / n).max(0.0) * (bb - sb * sb / n).max(0.0)).sqrt();
        let correlation = if variance > 1.0 {
            (ab - sa * sb / n) / variance
        } else {
            0.0
        };
        if count >= 24
            && textured_rows >= 3
            && row_votes * 5 >= textured_rows * 4
            && (correlation >= 0.90 || total as f64 / (n * 3.0) <= 1.0)
            && f64::from(mismatched) / f64::from(count) <= 0.12
        {
            scores.push((total as f64 / (f64::from(count) * 3.0), shift));
        }
    }
    scores.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(&(score, shift)) = scores.first() else {
        if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
            log::debug!("event=scroll.match_retry module=scroll code=MATCH_NO_CANDIDATE");
        }
        return Match::Uncertain;
    };
    if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
        log::debug!(
            "event=scroll.match_candidate module=scroll frames={}",
            scores.len()
        );
    }
    if score > 8.0
        || scores
            .iter()
            .skip(1)
            .any(|&(other, delta)| delta.abs_diff(shift) > 2 && other <= score + 1.5)
    {
        if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
            log::debug!("event=scroll.match_retry module=scroll code=MATCH_AMBIGUOUS");
        }
        return Match::Uncertain;
    }
    Match::Overlap(a.height - shift)
}

/// 固定页头/页脚只在两帧同屏位置连续一致、且边界内确实开始变化时排除。
/// 大块静止背景和侧栏不能据此推断滚动距离；最终仍需内容区匹配确认。
fn fixed_bands(a: &RgbaImage, b: &RgbaImage) -> (u32, u32) {
    let same_row = |y| {
        let equal = (0..COLUMNS)
            .filter(|column| {
                let x = ((*column + 1) as u64 * u64::from(a.width()) / (COLUMNS + 1) as u64) as u32;
                (0..3).all(|c| a.get_pixel(x, y)[c].abs_diff(b.get_pixel(x, y)[c]) <= 2)
            })
            .count();
        equal * 100 >= COLUMNS * 95
    };
    let limit = a.height() / 4;
    let band = |bottom: bool| {
        let count = (0..limit)
            .take_while(|y| same_row(if bottom { a.height() - 1 - y } else { *y }))
            .count() as u32;
        if count >= 4 && count < limit {
            count
        } else {
            0
        }
    };
    (band(false), band(true))
}

/// 行特征筛选后用更密的原始像素复核，避免采样列恰好落在重复背景上。
fn verified_region(a: &RgbaImage, b: &RgbaImage, sa: &Signature, sb: &Signature) -> Match {
    if a.width() < 32 || a.height() < 64 {
        return Match::Uncertain;
    }
    let result = match_region(sa, sb);
    if let Match::Overlap(overlap) = result {
        let shift = a.height() - overlap;
        let mut good = 0u64;
        let mut count = 0u64;
        for y in (0..overlap).step_by(2) {
            for x in (0..a.width()).step_by(3) {
                let difference: u32 = (0..3)
                    .map(|c| u32::from(a.get_pixel(x, y + shift)[c].abs_diff(b.get_pixel(x, y)[c])))
                    .sum();
                count += 1;
                good += u64::from(difference <= 48);
            }
        }
        if good * 100 < count * 90 {
            if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
                log::debug!("event=scroll.match_retry module=scroll code=MATCH_PIXEL_MISMATCH");
            }
            return Match::Uncertain;
        }
    }
    result
}

/// 返回可靠重叠与固定底栏高度；多个区域必须给出同一像素位移。
/// 局部动画/浮层可使一个区域失败，但两个区域冲突时绝不猜测接缝。
pub(super) fn alignment(
    previous: &RgbaImage,
    next: &RgbaImage,
    previous_signature: &Signature,
    next_signature: &Signature,
) -> (Match, u32) {
    if previous.dimensions() != next.dimensions() || previous.width() < 32 || previous.height() < 64
    {
        return (Match::Uncertain, 0);
    }
    if unchanged(previous_signature, next_signature) {
        return (Match::Unchanged, 0);
    }
    let (top, bottom) = fixed_bands(previous, next);
    let height = previous.height() - top - bottom;
    // 没有固定栏时直接借用原图和缓存特征，省去两次整帧复制和重复计算。
    let cropped;
    let signatures;
    let (a, b, sa, sb) = if top == 0 && bottom == 0 {
        (previous, next, previous_signature, next_signature)
    } else {
        cropped = (
            image::imageops::crop_imm(previous, 0, top, previous.width(), height).to_image(),
            image::imageops::crop_imm(next, 0, top, next.width(), height).to_image(),
        );
        signatures = (Signature::new(&cropped.0), Signature::new(&cropped.1));
        (&cropped.0, &cropped.1, &signatures.0, &signatures.1)
    };
    let full = verified_region(a, b, sa, sb);
    let matched = if matches!(full, Match::Overlap(_)) || a.width() < 96 {
        full
    } else {
        let mut votes = Vec::new();
        for region in 0..3 {
            let left = region * a.width() / 3;
            let width = (region + 1) * a.width() / 3 - left;
            let ra = image::imageops::crop_imm(a, left, 0, width, height).to_image();
            let rb = image::imageops::crop_imm(b, left, 0, width, height).to_image();
            if let Match::Overlap(overlap) =
                verified_region(&ra, &rb, &Signature::new(&ra), &Signature::new(&rb))
            {
                votes.push(overlap);
            }
        }
        if votes.len() >= 2 && votes.iter().all(|v| *v == votes[0]) {
            Match::Overlap(votes[0])
        } else {
            Match::Uncertain
        }
    };
    match matched {
        Match::Overlap(overlap) => (Match::Overlap(overlap + top + bottom), bottom),
        _ => (Match::Uncertain, 0),
    }
}

#[cfg(test)]
pub fn find_overlap(previous: &RgbaImage, next: &RgbaImage) -> Match {
    if previous.dimensions() != next.dimensions() || previous.width() < 32 || previous.height() < 64
    {
        return Match::Uncertain;
    }
    alignment(
        previous,
        next,
        &Signature::new(previous),
        &Signature::new(next),
    )
    .0
}

#[cfg(test)]
mod tests {
    use super::super::stability::stable;
    use super::*;
    use image::Rgba;

    fn document() -> RgbaImage {
        RgbaImage::from_fn(96, 600, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        })
    }
    fn frame(source: &RgbaImage, y: u32) -> RgbaImage {
        image::imageops::crop_imm(source, 0, y, 96, 160).to_image()
    }

    #[test]
    fn ignores_duplicates_but_rejects_reverse_scroll_and_missing_overlap() {
        let doc = document();
        assert_eq!(
            find_overlap(&frame(&doc, 80), &frame(&doc, 80)),
            Match::Unchanged
        );
        assert_eq!(
            find_overlap(&frame(&doc, 80), &frame(&doc, 0)),
            Match::Uncertain
        );
        assert_eq!(
            find_overlap(&frame(&doc, 0), &frame(&doc, 300)),
            Match::Uncertain
        );
        assert!(stable(&frame(&doc, 80), &frame(&doc, 80)));
        assert!(!stable(&frame(&doc, 80), &frame(&doc, 81)));
    }
    #[test]
    fn blank_frames_are_not_evidence_of_a_scroll() {
        let a = RgbaImage::from_pixel(96, 160, Rgba([255, 255, 255, 255]));
        let b = RgbaImage::from_pixel(96, 160, Rgba([240, 240, 240, 255]));
        assert_eq!(find_overlap(&a, &b), Match::Uncertain);
    }

    #[test]
    fn accepts_one_pixel_scroll_and_the_minimum_overlap() {
        let doc = document();
        assert_eq!(
            find_overlap(&frame(&doc, 0), &frame(&doc, 1)),
            Match::Overlap(159)
        );
        assert_eq!(
            find_overlap(&frame(&doc, 0), &frame(&doc, 128)),
            Match::Overlap(32)
        );
        assert_eq!(
            find_overlap(&frame(&doc, 0), &frame(&doc, 129)),
            Match::Uncertain
        );
    }

    #[test]
    fn two_content_regions_can_match_around_local_animation() {
        let doc = document();
        let a = frame(&doc, 0);
        let mut b = frame(&doc, 60);
        for y in 0..160 {
            for x in 0..30 {
                b.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        assert_eq!(find_overlap(&a, &b), Match::Overlap(100));
    }
}
