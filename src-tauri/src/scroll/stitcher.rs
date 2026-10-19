//! 分段存储可信图像，自动去重并处理固定底栏；负责容量、预览和组图。
use super::matcher::{alignment, Match, Signature, MIN_OVERLAP};
use image::RgbaImage;
use std::sync::Arc;
pub const MAX_PIXELS: u64 = 32_000_000;
pub const MAX_HEIGHT: u32 = 60_000;
const MAX_FRAMES: usize = 64;

#[cfg(test)]
#[path = "regression_tests.rs"]
mod regression_tests;

#[derive(Clone)]
struct Segment {
    image: Arc<RgbaImage>,
    overlap: u32,
    end: u32,
}

#[derive(Clone)]
pub struct Stitch {
    // 快照仅复制分段边界，图片和特征共享；匹配失败可直接丢弃快照。
    segments: Vec<Segment>,
    // 仅随可信锚点更新；不确定候选不能污染下一轮匹配。
    signature: Arc<Signature>,
    pub width: u32,
    pub frame_height: u32,
    pub height: u32,
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
            signature: Arc::new(Signature::new(&image)),
            segments: vec![Segment {
                image: Arc::new(image),
                overlap: 0,
                end: height,
            }],
            width,
            frame_height: height,
            height,
        })
    }
    pub fn count(&self) -> usize {
        self.segments.len()
    }
    pub fn last(&self) -> &RgbaImage {
        &self.segments[self.segments.len() - 1].image
    }
    /// 存档与恢复共享不可变帧，不复制整张 RGBA 图片。
    pub fn last_shared(&self) -> Arc<RgbaImage> {
        self.segments[self.segments.len() - 1].image.clone()
    }

    pub fn consider(&mut self, image: impl Into<Arc<RgbaImage>>) -> Result<Match, String> {
        let image = image.into();
        if image.dimensions() != (self.width, self.frame_height) {
            return Err("采集区域尺寸发生变化，请结束本次长截图。".into());
        }
        let started = std::time::Instant::now();
        let signature = Signature::new(&image);
        let (result, bottom) = alignment(self.last(), &image, &self.signature, &signature);
        if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
            log::debug!(
                "event=scroll.match module=scroll duration_ms={:.2}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Match::Overlap(overlap) = result {
            let last = &self.segments[self.segments.len() - 1];
            // 底栏布局刚变化时，不能用新推断裁掉已提交内容；按不确定匹配重试。
            if bottom >= last.end - last.overlap || bottom > overlap {
                return Ok(Match::Uncertain);
            }
            self.append(image, overlap, bottom)?;
            self.signature = Arc::new(signature);
        }
        Ok(result)
    }

    /// 仅供自动匹配成功后提交；先校验所有容量，失败时锚点和成图完全不变。
    fn append(&mut self, image: Arc<RgbaImage>, overlap: u32, bottom: u32) -> Result<(), String> {
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
        // 旧帧底栏不进入中间接缝；新帧从同一内容坐标接续，底栏仅在最终末尾保留。
        let previous = self.segments.last_mut().ok_or("缺少可信锚点。")?;
        if bottom >= previous.end - previous.overlap || bottom > overlap {
            return Err("固定底栏超出有效内容区域。".into());
        }
        previous.end -= bottom;
        self.segments.push(Segment {
            image,
            overlap: overlap - bottom,
            end: self.frame_height,
        });
        self.height = height;
        Ok(())
    }

    /// 仅在预览/导出时组装整图，采集过程中不反复复制逐渐变长的图片。
    pub fn compose(&self) -> RgbaImage {
        let mut output = RgbaImage::new(self.width, self.height);
        let mut y = 0u32;
        for segment in &self.segments {
            let rows = segment.end - segment.overlap;
            let start = segment.overlap as usize * self.width as usize * 4;
            let target = y as usize * self.width as usize * 4;
            let bytes: &mut [u8] = output.as_mut();
            bytes[target..target + rows as usize * self.width as usize * 4].copy_from_slice(
                &segment.image.as_raw()[start..start + rows as usize * self.width as usize * 4],
            );
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
            while source_y >= start + self.segments[index].end - self.segments[index].overlap {
                start += self.segments[index].end - self.segments[index].overlap;
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
    fn joins_multiple_scrolls_without_duplicate_or_missing_rows() {
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
    }
    #[test]
    fn ambiguous_patterns_never_change_the_anchor() {
        let pattern = RgbaImage::from_fn(96, 600, |x, y| {
            Rgba([((y % 20) * 12) as u8, (x * 2) as u8, 0, 255])
        });
        let mut stitch = Stitch::new(frame(&pattern, 0)).unwrap();
        for _ in 0..8 {
            assert_eq!(
                stitch.consider(frame(&pattern, 55)).unwrap(),
                Match::Uncertain
            );
            assert_eq!(stitch.compose(), frame(&pattern, 0));
        }
    }
    #[test]
    fn retry_returning_to_anchor_preserves_output() {
        let doc = document();
        let mut stitch = Stitch::new(frame(&doc, 0)).unwrap();
        stitch.consider(frame(&doc, 300)).unwrap();
        assert_eq!(stitch.last(), &frame(&doc, 0));
        assert_eq!(stitch.consider(frame(&doc, 0)).unwrap(), Match::Unchanged);
        assert_eq!(stitch.compose(), frame(&doc, 0));
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
        assert!(stitch.consider(frame(&document(), 60)).is_err());
        assert_eq!(stitch.count(), 1);
        assert_eq!(stitch.last(), &frame(&document(), 0));
        assert!(Stitch::new(RgbaImage::new(31, 64)).is_err());
    }

    #[test]
    fn frame_limit_and_invalid_overlap_preserve_output() {
        let first = frame(&document(), 0);
        let mut stitch = Stitch::new(first.clone()).unwrap();
        for _ in 1..MAX_FRAMES {
            stitch.append(Arc::new(first.clone()), 100, 0).unwrap();
        }
        let before = stitch.compose();
        assert!(stitch.append(Arc::new(first.clone()), 100, 0).is_err());
        assert!(stitch.append(Arc::new(first), 160, 0).is_err());
        assert_eq!(stitch.compose(), before);
        assert_eq!(stitch.count(), MAX_FRAMES);
    }

    #[test]
    fn fixed_header_and_footer_appear_only_once() {
        let doc = document();
        let fixed = |offset| {
            let mut image = frame(&doc, offset);
            for y in (0..12).chain(148..160) {
                for x in 0..96 {
                    image.put_pixel(x, y, *doc.get_pixel(x, y + 400));
                }
            }
            image
        };
        let mut stitch = Stitch::new(fixed(0)).unwrap();
        assert_eq!(stitch.consider(fixed(60)).unwrap(), Match::Overlap(100));
        assert_eq!(stitch.consider(fixed(120)).unwrap(), Match::Overlap(100));
        let mut expected = image::imageops::crop_imm(&doc, 0, 0, 96, 280).to_image();
        for y in (0..12).chain(268..280) {
            for x in 0..96 {
                let source_y = if y < 12 { y } else { y - 120 };
                expected.put_pixel(x, y, *doc.get_pixel(x, source_y + 400));
            }
        }
        assert_eq!(stitch.compose(), expected);
        assert_eq!(stitch.preview(96, 280), expected);
    }
}
