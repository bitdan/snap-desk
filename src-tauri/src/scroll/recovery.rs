//! 匹配失败只暂停追加：保留可信锚点和短期候选帧，找到重叠后自动接续。
use super::{capture::FrameCache, matcher::Match, stitcher::Stitch};
use image::RgbaImage;
#[cfg(test)]
use std::time::Duration;
use std::{sync::Arc, time::Instant};

#[derive(Clone, Default)]
pub struct Recovery {
    failures: u32,
    // 最多三帧、合计 800 万像素、五秒有效；不因无限重试积累内存。
    recent: FrameCache,
}

impl Recovery {
    pub fn consider(
        &mut self,
        stitch: &mut Stitch,
        image: impl Into<Arc<RgbaImage>>,
    ) -> Result<Match, String> {
        self.consider_at(stitch, image, Instant::now())
    }

    /// 显式时钟让缓存有效期可以定向测试，不依赖机器上匹配算法的运行速度。
    pub(super) fn consider_at(
        &mut self,
        stitch: &mut Stitch,
        image: impl Into<Arc<RgbaImage>>,
        now: Instant,
    ) -> Result<Match, String> {
        let image = image.into();
        self.recent.expire(now);
        let result = stitch.consider(image.clone())?;
        match result {
            Match::Uncertain => {
                self.failures = self.failures.saturating_add(1);
                self.recent.remember(image, now);
            }
            Match::Overlap(_) => {
                self.failures = 0;
                // 新画面补上缺口后，可依次接回最近已采到的后续帧；每一帧重新验证，
                // 不能仅凭采集时间推断顺序，更不能跳过仍无重叠的内容。
                loop {
                    let mut advanced = false;
                    for index in 0..self.recent.len() {
                        let candidate = self.recent.candidate(index);
                        if matches!(stitch.consider(candidate)?, Match::Overlap(_)) {
                            self.recent.remove(index);
                            advanced = true;
                            break;
                        }
                    }
                    if !advanced {
                        break;
                    }
                }
            }
            Match::Unchanged => {
                self.failures = 0;
            }
        }
        Ok(result)
    }

    pub fn message(&self, result: Option<Match>) -> &'static str {
        match result {
            Some(Match::Uncertain) | None if self.failures >= 3 => {
                "未找到可靠重叠，可能滚动过快；请稍微向上滚动，匹配后自动继续。"
            }
            Some(Match::Uncertain) => "正在重新匹配，请稍作停顿；已拼接内容保留。",
            None => "等待页面稳定；局部动画会自动过滤。",
            _ => "自动拼接中，请正常滚动页面，完成后点击完成。",
        }
    }

    pub fn delay(&self) -> u64 {
        // 重试短暂退避，仍持续采集，绝不按失败次数结束会话。
        80 + u64::from(self.failures.min(4)) * 80
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn repeated_failures_preserve_anchor_and_bridge_recent_frames() {
        let doc = RgbaImage::from_fn(96, 600, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let frame = |y| image::imageops::crop_imm(&doc, 0, y, 96, 160).to_image();
        let mut stitch = Stitch::new(frame(0)).unwrap();
        let mut recovery = Recovery::default();
        let now = Instant::now();
        for _ in 0..10 {
            assert_eq!(
                recovery.consider_at(&mut stitch, frame(300), now).unwrap(),
                Match::Uncertain
            );
        }
        assert_eq!(stitch.compose(), frame(0));
        assert_eq!(recovery.recent.len(), 1);
        assert!(recovery
            .message(Some(Match::Uncertain))
            .contains("向上滚动"));
        recovery.consider_at(&mut stitch, frame(240), now).unwrap();
        recovery.consider_at(&mut stitch, frame(120), now).unwrap();
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&doc, 0, 0, 96, 460).to_image()
        );
        assert_eq!(recovery.failures, 0);
        assert_eq!(recovery.delay(), 80);
    }

    #[test]
    fn old_candidates_expire_and_do_not_accumulate() {
        let a = RgbaImage::from_pixel(96, 160, Rgba([255, 255, 255, 255]));
        let mut stitch = Stitch::new(a).unwrap();
        let mut recovery = Recovery::default();
        let now = Instant::now();
        for value in 0..8 {
            recovery
                .consider_at(
                    &mut stitch,
                    RgbaImage::from_pixel(96, 160, Rgba([value, value, value, 255])),
                    now,
                )
                .unwrap();
            assert!(recovery.recent.len() <= 3);
        }
        recovery
            .consider_at(
                &mut stitch,
                RgbaImage::from_pixel(96, 160, Rgba([80, 80, 80, 255])),
                now + Duration::from_secs(6),
            )
            .unwrap();
        assert_eq!(recovery.recent.len(), 1);
        assert_eq!(stitch.count(), 1);
    }
}
