//! 纵向长截图拼接：保留原帧与接缝，支持撤销和人工重叠修正。
//! 不猜测横向移动；低纹理、重复图案或无可靠重叠时暂停，由用户决定接缝。
use image::RgbaImage;

pub const MAX_PIXELS: u64 = 32_000_000;
pub const MAX_HEIGHT: u32 = 60_000;
const MAX_FRAMES: usize = 64;
pub const MIN_OVERLAP: u32 = 32;
const COLUMNS: usize = 48;

struct Signature {
    pixels: Vec<[u8; 3]>,
    edges: Vec<bool>,
    height: u32,
}

impl Signature {
    fn new(image: &RgbaImage) -> Self {
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

/// 连续两次取样稳定后才交给拼接，过滤滚动动画中间帧。忽略极少量光标闪烁。
pub fn stable(a: &RgbaImage, b: &RgbaImage) -> bool {
    a.dimensions() == b.dimensions()
        && a.width() > 0
        && a.height() > 0
        && unchanged(&Signature::new(a), &Signature::new(b))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Match {
    Unchanged,
    Overlap(u32),
    Uncertain,
}

/// 在纵向位移范围内匹配内容边缘。白色背景不计作匹配证据，多个近似解拒绝自动拼接。
pub fn find_overlap(previous: &RgbaImage, next: &RgbaImage) -> Match {
    if previous.dimensions() != next.dimensions() || previous.width() < 32 || previous.height() < 64
    {
        return Match::Uncertain;
    }
    let a = Signature::new(previous);
    let b = Signature::new(next);
    if unchanged(&a, &b) {
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
        return Match::Uncertain;
    };
    if score > 8.0
        || scores
            .iter()
            .skip(1)
            .any(|&(other, delta)| delta.abs_diff(shift) > 2 && other <= score + 1.5)
    {
        return Match::Uncertain;
    }
    Match::Overlap(a.height - shift)
}

struct Segment {
    image: RgbaImage,
    overlap: u32,
}

pub struct Stitch {
    segments: Vec<Segment>,
    pub pending: Option<RgbaImage>,
    pub width: u32,
    pub frame_height: u32,
    pub height: u32,
    pub suggested_overlap: u32,
}

impl Stitch {
    pub fn new(image: RgbaImage) -> Result<Self, String> {
        let (width, height) = image.dimensions();
        if width < 32
            || height < 64
            || width > 8192
            || height > 8192
            || u64::from(width) * u64::from(height) > 8_000_000
        {
            return Err(
                "长截图选区须至少 32 × 64 像素，宽高不超过 8192，单帧不超过 800 万像素。".into(),
            );
        }
        Ok(Self {
            segments: vec![Segment { image, overlap: 0 }],
            pending: None,
            width,
            frame_height: height,
            height,
            suggested_overlap: height / 2,
        })
    }
    pub fn count(&self) -> usize {
        self.segments.len()
    }
    pub fn last(&self) -> &RgbaImage {
        &self.segments[self.segments.len() - 1].image
    }

    pub fn consider(&mut self, image: RgbaImage) -> Result<Match, String> {
        if image.dimensions() != (self.width, self.frame_height) {
            return Err("采集区域尺寸发生变化，请结束本次长截图。".into());
        }
        let result = find_overlap(self.last(), &image);
        // 重试时画面可能回到上一帧，不能继续保留先前不可信的候选帧。
        if result == Match::Unchanged {
            self.pending = None;
        }
        if result != Match::Unchanged {
            self.pending = Some(image);
            self.suggested_overlap = if let Match::Overlap(overlap) = result {
                overlap
            } else {
                self.frame_height / 2
            };
            if let Match::Overlap(overlap) = result {
                self.accept(overlap)?;
            }
        }
        Ok(result)
    }

    /// 自动/手动共用边界校验；先完成校验再取走待处理帧，失败不会损坏已拼接结果。
    pub fn accept(&mut self, overlap: u32) -> Result<(), String> {
        if self.pending.is_none() {
            return Err("没有待处理画面。".into());
        }
        if !(MIN_OVERLAP..self.frame_height).contains(&overlap) {
            return Err("重叠行数超出允许范围。".into());
        }
        let height = self
            .height
            .checked_add(self.frame_height - overlap)
            .ok_or("长截图高度溢出。")?;
        if self.count() >= MAX_FRAMES
            || height > MAX_HEIGHT
            || u64::from(self.width) * u64::from(height) > MAX_PIXELS
            || (self.count() as u64 + 1) * u64::from(self.width) * u64::from(self.frame_height)
                > MAX_PIXELS
        {
            return Err("已达到长截图容量限制（64 帧 / 60000 行 / 3200 万像素或源帧缓存上限），已自动结束，仅输出已确认部分。".into());
        }
        let image = self.pending.take().ok_or("没有待处理画面。")?;
        self.segments.push(Segment { image, overlap });
        self.height = height;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("请先处理或丢弃待拼接画面。".into());
        }
        if self.count() <= 1 {
            return Err("首帧不能撤销。".into());
        }
        let segment = self.segments.pop().ok_or("没有可撤销的画面。")?;
        self.height -= self.frame_height - segment.overlap;
        self.suggested_overlap = segment.overlap;
        self.pending = Some(segment.image);
        Ok(())
    }

    /// 仅在预览/导出时组装整图，采集过程中不反复复制逐渐变长的图片。
    pub fn compose(&self) -> RgbaImage {
        let mut output = RgbaImage::new(self.width, self.height);
        let mut y = 0u32;
        for segment in &self.segments {
            let rows = self.frame_height - segment.overlap;
            let start = segment.overlap as usize * self.width as usize * 4;
            let target = y as usize * self.width as usize * 4;
            let bytes: &mut [u8] = output.as_mut();
            bytes[target..target + rows as usize * self.width as usize * 4]
                .copy_from_slice(&segment.image.as_raw()[start..]);
            y += rows;
        }
        output
    }

    /// 直接从分段取样缩略图，避免预览时先分配整张超长 RGBA 图片。
    pub fn preview(&self, max_width: u32, max_height: u32) -> RgbaImage {
        let scale = (f64::from(max_width) / f64::from(self.width))
            .min(f64::from(max_height) / f64::from(self.height))
            .min(1.0);
        let width = (f64::from(self.width) * scale).floor().max(1.0) as u32;
        let height = (f64::from(self.height) * scale).floor().max(1.0) as u32;
        let mut output = RgbaImage::new(width, height);
        let mut index = 0;
        let mut start = 0;
        for y in 0..height {
            let source_y = (u64::from(y) * u64::from(self.height) / u64::from(height)) as u32;
            while source_y >= start + self.frame_height - self.segments[index].overlap {
                start += self.frame_height - self.segments[index].overlap;
                index += 1;
            }
            let segment = &self.segments[index];
            for x in 0..width {
                let source_x = (u64::from(x) * u64::from(self.width) / u64::from(width)) as u32;
                output.put_pixel(
                    x,
                    y,
                    *segment
                        .image
                        .get_pixel(source_x, source_y - start + segment.overlap),
                );
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
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
    fn joins_multiple_scrolls_without_duplicate_or_missing_rows_and_can_undo() {
        let doc = document();
        let mut stitch = Stitch::new(frame(&doc, 0)).unwrap();
        assert_eq!(
            stitch.consider(frame(&doc, 60)).unwrap(),
            Match::Overlap(100)
        );
        assert_eq!(
            stitch.consider(frame(&doc, 135)).unwrap(),
            Match::Overlap(85)
        );
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&doc, 0, 0, 96, 295).to_image()
        );
        stitch.undo().unwrap();
        assert_eq!(stitch.height, 220);
        assert_eq!(stitch.last(), &frame(&doc, 60));
        assert_eq!(stitch.suggested_overlap, 85);
        assert!(stitch.pending.is_some());
        assert!(stitch.undo().is_err());
        stitch.accept(85).unwrap();
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&doc, 0, 0, 96, 295).to_image()
        );
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
    fn ambiguous_patterns_require_manual_correction_and_failed_accept_keeps_pending() {
        let pattern = RgbaImage::from_fn(96, 600, |x, y| {
            Rgba([((y % 20) * 12) as u8, (x * 2) as u8, 0, 255])
        });
        let mut stitch = Stitch::new(frame(&pattern, 0)).unwrap();
        assert_eq!(
            stitch.consider(frame(&pattern, 55)).unwrap(),
            Match::Uncertain
        );
        assert!(stitch.accept(0).is_err());
        assert!(stitch.pending.is_some());
        stitch.accept(105).unwrap();
        assert_eq!(stitch.height, 215);
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&pattern, 0, 0, 96, 215).to_image()
        );
    }
    #[test]
    fn blank_frames_are_not_evidence_of_a_scroll() {
        let a = RgbaImage::from_pixel(96, 160, Rgba([255, 255, 255, 255]));
        let b = RgbaImage::from_pixel(96, 160, Rgba([240, 240, 240, 255]));
        assert_eq!(find_overlap(&a, &b), Match::Uncertain);
    }

    #[test]
    fn retry_returning_to_last_frame_clears_stale_candidate() {
        let doc = document();
        let mut stitch = Stitch::new(frame(&doc, 0)).unwrap();
        stitch.consider(frame(&doc, 300)).unwrap();
        assert!(stitch.pending.is_some());
        assert_eq!(stitch.consider(frame(&doc, 0)).unwrap(), Match::Unchanged);
        assert!(stitch.pending.is_none());
    }

    #[test]
    fn preview_reads_correct_segment_rows_without_composing() {
        let doc = document();
        let mut stitch = Stitch::new(frame(&doc, 0)).unwrap();
        stitch.consider(frame(&doc, 60)).unwrap();
        assert_eq!(stitch.preview(96, 220), stitch.compose());
        let small = stitch.preview(48, 110);
        assert_eq!(small.dimensions(), (48, 110));
        for y in 0..110 {
            for x in 0..48 {
                assert_eq!(small.get_pixel(x, y), doc.get_pixel(x * 2, y * 2));
            }
        }
    }
    #[test]
    fn capacity_failure_preserves_previous_output() {
        let mut stitch = Stitch::new(frame(&document(), 0)).unwrap();
        stitch.height = MAX_HEIGHT;
        stitch.pending = Some(frame(&document(), 60));
        assert!(stitch.accept(100).is_err());
        assert_eq!(stitch.count(), 1);
        assert!(stitch.pending.is_some());
        assert!(Stitch::new(RgbaImage::new(31, 64)).is_err());
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
    fn frame_limit_and_invalid_overlap_do_not_consume_the_candidate() {
        let first = frame(&document(), 0);
        let mut stitch = Stitch::new(first.clone()).unwrap();
        for _ in 1..MAX_FRAMES {
            stitch.pending = Some(first.clone());
            stitch.accept(100).unwrap();
        }
        stitch.pending = Some(first);
        let height = stitch.height;
        assert!(stitch.accept(100).is_err());
        assert!(stitch.accept(160).is_err());
        assert_eq!(stitch.height, height);
        assert_eq!(stitch.count(), MAX_FRAMES);
        assert!(stitch.pending.is_some());
    }
}
