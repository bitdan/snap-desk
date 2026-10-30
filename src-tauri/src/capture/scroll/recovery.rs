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
    failed_revision: Option<u64>,
    rejection: &'static str,
    recoveries: u32,
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
        let before = stitch.covered_pixels();
        let viewport = stitch.viewport();
        let recovering = self.is_recovering();
        let started = Instant::now();
        self.recent.expire(now);
        // 图像、可信画布版本和五秒有效期都不变时，不再反复搜索同一失败帧。
        let cache_hit = self.failed_revision == Some(stitch.match_revision())
            && self.recent.latest_matches(&image);
        let (result, cacheable) = if cache_hit {
            (Match::Uncertain, true)
        } else {
            stitch.consider_recoverable(image.clone())?
        };
        match result {
            Match::Uncertain => {
                if !cache_hit {
                    self.rejection = stitch.rejection();
                }
                if self.failures == 0 {
                    stitch.log_rejection();
                }
                self.failures = self.failures.saturating_add(1);
                self.failed_revision = Some(stitch.match_revision());
                if cacheable {
                    self.recent.remember(image, now);
                } else {
                    // 已发现布局、遮挡或画布冲突时清空旧候选，避免问题帧绕过新检查。
                    self.recent = FrameCache::default();
                }
            }
            Match::Aligned(_) | Match::Unchanged => {
                if recovering {
                    self.recoveries = self.recoveries.saturating_add(1);
                }
                self.rejection = "none";
                self.failures = 0;
                self.failed_revision = None;
                let observed = stitch.clone();
                // 新画面补上缺口后，可依次接回最近已采到的后续帧；每一帧重新验证，
                // 不能仅凭采集时间推断顺序，更不能跳过仍无重叠的内容。
                while recovering || matches!(result, Match::Aligned(_)) {
                    let mut advanced = false;
                    for index in 0..self.recent.len() {
                        let candidate = self.recent.candidate(index);
                        let (replayed, cacheable) = stitch.consider_recoverable(candidate)?;
                        if !matches!(replayed, Match::Uncertain) || !cacheable {
                            self.recent.remove(index);
                            advanced = true;
                            break;
                        }
                    }
                    if !advanced {
                        break;
                    }
                }
                // 缓存只补齐已观察内容，视口最终回到本次真正采到的位置。
                if recovering || matches!(result, Match::Aligned(_)) {
                    stitch.restore_viewport(&observed);
                }
            }
        }
        let (dx, dy, confidence) = match result {
            Match::Aligned(a) => (a.dx, a.dy, a.confidence),
            _ => (0, 0, if result == Match::Unchanged { 1.0 } else { 0.0 }),
        };
        log::debug!("event=scroll.match module=scroll result={:?} reason={} dx={} dy={} confidence={:.4} growth_pixels={} viewport_changed={} failures={} recoveries={} cache_frames={} duration_ms={:.2}",
            result, self.rejection, dx, dy, confidence,
            stitch.covered_pixels().saturating_sub(before), stitch.viewport() != viewport,
            self.failures, self.recoveries, self.recent.len(), started.elapsed().as_secs_f64() * 1000.0);
        Ok(result)
    }

    /// 只有可信匹配（包括回到原画面）才解除恢复状态，页面运动不能清除失败记录。
    pub fn is_recovering(&self) -> bool {
        self.failures > 0
    }

    pub fn message(&self, result: Option<Match>) -> &'static str {
        if self.is_recovering() {
            match self.rejection {
                "fixed_bounds_changed" | "anchor_fixed_bounds_changed" => {
                    return "暂时没接上，请往回滚一点再停一下。"
                }
                "stationary_overlay" => return "请移开悬浮内容，再往回滚一点。",
                "seam_conflict" | "canvas_conflict" => {
                    return "画面有变化，请暂停动画后往回滚一点。"
                }
                "anchor_position_conflict" => return "内容太相似，请往回滚到有明显区别的位置。",
                _ => {}
            }
        }
        match result {
            Some(Match::Uncertain) | None if self.failures >= 3 => {
                "还没接上，请往回滚到上一段内容并停一下。"
            }
            Some(Match::Uncertain) | None if self.is_recovering() => "往回滚一点，停一下即可继续。",
            None => "请停一下，等待页面稳定。",
            _ => "慢慢滚动，停一下再继续。",
        }
    }

    pub fn delay(&self) -> u64 {
        // 重试短暂退避，仍持续采集，绝不按失败次数结束会话。
        40 + u64::from(self.failures.min(4)) * 40
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 缓存补到远端后恢复真实视口，下一帧只与补采终点重叠也应继续。
    /// 四向都用正常采样顺序构造，不人工注入锚点；输出必须逐像素等于真值。
    #[test]
    fn replay_endpoint_remains_available_after_restoring_observed_viewport() {
        let doc = RgbaImage::from_fn(900, 900, |x, y| {
            let mut v =
                (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            v ^= v >> 13;
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        for (dx, dy) in [(0i64, 1i64), (0, -1), (1, 0), (-1, 0)] {
            let seed_x = if dx < 0 { 500i64 } else { 0 };
            let seed_y = if dy < 0 { 500i64 } else { 0 };
            let frame = |offset: i64| {
                image::imageops::crop_imm(
                    &doc,
                    (seed_x + dx * offset) as u32,
                    (seed_y + dy * offset) as u32,
                    160,
                    160,
                )
                .to_image()
            };
            let mut stitch = Stitch::new(frame(0)).unwrap();
            let mut recovery = Recovery::default();
            let now = Instant::now();
            for offset in [300, 240] {
                assert_eq!(
                    recovery
                        .consider_at(&mut stitch, frame(offset), now)
                        .unwrap(),
                    Match::Uncertain
                );
            }
            assert!(matches!(
                recovery.consider_at(&mut stitch, frame(120), now).unwrap(),
                Match::Aligned(_)
            ));
            assert_eq!(recovery.recent.len(), 0);
            assert!(!recovery.is_recovering());
            assert_eq!(
                (stitch.viewport().x, stitch.viewport().y),
                (dx * 120, dy * 120)
            );
            let bounds = stitch.bounds;
            assert_eq!(
                stitch.compose(),
                image::imageops::crop_imm(
                    &doc,
                    (seed_x + bounds.x) as u32,
                    (seed_y + bounds.y) as u32,
                    bounds.width,
                    bounds.height,
                )
                .to_image()
            );
            // 距离真实视口 300、其他历史锚点至少 180，只有补采终点留下 40 像素重叠。
            assert!(matches!(
                recovery.consider_at(&mut stitch, frame(420), now).unwrap(),
                Match::Aligned(_)
            ));
            assert_eq!(
                (stitch.viewport().x, stitch.viewport().y),
                (dx * 420, dy * 420)
            );
            assert_eq!(recovery.recent.len(), 0);
            assert!(!stitch.has_holes());
            let bounds = stitch.bounds;
            assert_eq!(bounds.width, if dx == 0 { 160 } else { 580 });
            assert_eq!(bounds.height, if dy == 0 { 160 } else { 580 });
            assert_eq!(
                stitch.compose(),
                image::imageops::crop_imm(
                    &doc,
                    (seed_x + bounds.x) as u32,
                    (seed_y + bounds.y) as u32,
                    bounds.width,
                    bounds.height,
                )
                .to_image()
            );
        }
    }

    #[test]
    fn new_fixed_boundaries_clear_candidates_and_resume_on_clean_frames() {
        let doc = RgbaImage::from_fn(700, 700, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let frame = |x, y| image::imageops::crop_imm(&doc, x, y, 160, 160).to_image();
        // 四边分别验证；冻结列沿横向暴露，页头/页脚沿纵向暴露。
        for side in 0..4 {
            let columns = side < 2;
            // 图像裁剪使用无符号物理像素坐标；显式标注，避免整数默认推断为 i32。
            let position = |offset: u32| -> (u32, u32) {
                if columns {
                    (200 + offset, 200)
                } else {
                    (200, 200 + offset)
                }
            };
            let mut stitch = Stitch::new(frame(200, 200)).unwrap();
            let mut recovery = Recovery::default();
            let now = Instant::now();
            let (x, y) = position(40);
            assert!(matches!(
                recovery.consider_at(&mut stitch, frame(x, y), now).unwrap(),
                Match::Aligned(_)
            ));
            let before = stitch.compose();
            let viewport = stitch.viewport();
            // 先放入没有重叠的正常候选，布局冲突必须连同它一起清除。
            assert_eq!(
                recovery
                    .consider_at(&mut stitch, frame(500, 500), now)
                    .unwrap(),
                Match::Uncertain
            );
            assert_eq!(recovery.recent.len(), 1);
            let (x, y) = position(80);
            let mut dirty = frame(x, y);
            for py in 0..160 {
                for px in 0..160 {
                    let fixed = match side {
                        0 => px < 12,
                        1 => px >= 148,
                        2 => py < 12,
                        _ => py >= 148,
                    };
                    if fixed {
                        dirty.put_pixel(px, py, *stitch.last().get_pixel(px, py));
                    }
                }
            }
            assert_eq!(
                recovery.consider_at(&mut stitch, dirty, now).unwrap(),
                Match::Uncertain
            );
            assert!(recovery.is_recovering());
            assert_eq!(recovery.recent.len(), 0);
            assert_eq!(stitch.compose(), before);
            assert_eq!(stitch.viewport(), viewport);
            let (x, y) = position(60);
            assert!(matches!(
                recovery.consider_at(&mut stitch, frame(x, y), now).unwrap(),
                Match::Aligned(_)
            ));
            assert!(!recovery.is_recovering());
            assert_eq!(
                stitch.compose(),
                image::imageops::crop_imm(
                    &doc,
                    200,
                    200,
                    if columns { 220 } else { 160 },
                    if columns { 160 } else { 220 }
                )
                .to_image()
            );
        }
    }

    #[test]
    fn animated_seam_is_not_cached_for_later_replay() {
        let doc = RgbaImage::from_fn(400, 400, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let frame = |y| image::imageops::crop_imm(&doc, 0, y, 160, 160).to_image();
        let mut stitch = Stitch::new(frame(0)).unwrap();
        let before = stitch.compose();
        let mut dirty = frame(40);
        for y in 112..128 {
            for x in 80..96 {
                let p = dirty.get_pixel_mut(x, y);
                *p = Rgba([255 - p[0], 255 - p[1], 255 - p[2], 255]);
            }
        }
        let mut recovery = Recovery::default();
        assert_eq!(
            recovery.consider(&mut stitch, dirty).unwrap(),
            Match::Uncertain
        );
        assert!(recovery.is_recovering());
        assert_eq!(recovery.recent.len(), 0);
        assert_eq!(stitch.compose(), before);
        assert!(matches!(
            recovery.consider(&mut stitch, frame(20)).unwrap(),
            Match::Aligned(_)
        ));
        assert_eq!(stitch.height, 180);
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&doc, 0, 0, 160, 180).to_image()
        );
        assert!(!recovery.is_recovering());
    }

    #[test]
    fn detected_overlay_is_not_replayed_after_it_disappears() {
        let doc = RgbaImage::from_fn(400, 400, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let frame = |y| image::imageops::crop_imm(&doc, 0, y, 160, 160).to_image();
        let overlay = |mut image: RgbaImage| {
            for y in 72..80 {
                for x in 72..80 {
                    image.put_pixel(x, y, *doc.get_pixel(x + 200, y + 200));
                }
            }
            image
        };
        let mut stitch = Stitch::new(overlay(frame(0))).unwrap();
        let before = stitch.compose();
        let mut recovery = Recovery::default();
        assert_eq!(
            recovery.consider(&mut stitch, overlay(frame(40))).unwrap(),
            Match::Uncertain
        );
        assert!(recovery.is_recovering());
        assert_eq!(recovery.recent.len(), 0);
        assert_eq!(stitch.compose(), before);
        assert!(matches!(
            recovery.consider(&mut stitch, frame(20)).unwrap(),
            Match::Aligned(_)
        ));
        // 若被拒绝的 y=40 帧仍在缓存，会再次匹配并把成图错误扩到 200 高。
        assert_eq!(stitch.height, 180);
        assert_eq!(stitch.viewport().y, 20);
        assert!(!recovery.is_recovering());
        assert_eq!(recovery.recent.len(), 0);
    }

    #[test]
    fn returning_to_last_confirmed_frame_resumes_without_growing_canvas() {
        let first = RgbaImage::from_fn(96, 160, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let mut stitch = Stitch::new(first.clone()).unwrap();
        let mut recovery = Recovery::default();
        assert_eq!(
            recovery
                .consider(
                    &mut stitch,
                    RgbaImage::from_pixel(96, 160, Rgba([0, 0, 0, 255]))
                )
                .unwrap(),
            Match::Uncertain
        );
        assert!(recovery.is_recovering());
        assert!(recovery.message(None).contains("往回滚"));
        assert_eq!(stitch.compose(), first);
        assert_eq!(
            recovery.consider(&mut stitch, first.clone()).unwrap(),
            Match::Unchanged
        );
        assert!(!recovery.is_recovering());
        assert_eq!(stitch.compose(), first);
        assert_eq!(stitch.count(), 1);
    }

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
        assert!(recovery.is_recovering());
        // 不稳定帧不参与匹配；其提示仍必须保留恢复操作，不能闪回“自动拼接”。
        assert!(recovery.message(None).contains("往回滚"));
        assert_eq!(recovery.recent.len(), 1);
        assert!(recovery.message(Some(Match::Uncertain)).contains("往回滚"));
        recovery.consider_at(&mut stitch, frame(240), now).unwrap();
        recovery.consider_at(&mut stitch, frame(120), now).unwrap();
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&doc, 0, 0, 96, 460).to_image()
        );
        assert_eq!(recovery.failures, 0);
        assert!(!recovery.is_recovering());
        assert_eq!(recovery.delay(), 40);
        assert_eq!(stitch.viewport().y, 120);
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

    #[test]
    fn failed_frame_cache_is_invalidated_when_trusted_canvas_advances() {
        let doc = RgbaImage::from_fn(96, 600, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let frame = |y| image::imageops::crop_imm(&doc, 0, y, 96, 160).to_image();
        let mut stitch = Stitch::new(frame(0)).unwrap();
        let mut recovery = Recovery::default();
        let now = Instant::now();
        assert_eq!(
            recovery.consider_at(&mut stitch, frame(240), now).unwrap(),
            Match::Uncertain
        );
        let failed_revision = recovery.failed_revision.unwrap();
        assert!(matches!(
            stitch.consider(frame(120)).unwrap(),
            Match::Aligned(_)
        ));
        assert_ne!(stitch.match_revision(), failed_revision);
        assert!(matches!(
            recovery.consider_at(&mut stitch, frame(240), now).unwrap(),
            Match::Aligned(_)
        ));
        assert_eq!(
            stitch.compose(),
            image::imageops::crop_imm(&doc, 0, 0, 96, 400).to_image()
        );
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn returning_to_anchor_with_minor_redraw_resumes_without_canvas_growth() {
        let frame = |offset: u32| {
            RgbaImage::from_fn(96, 160, |x, y| {
                let v = (x.wrapping_mul(73856093) ^ (y + offset).wrapping_mul(19349663))
                    .wrapping_mul(83492791);
                Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
            })
        };
        let first = frame(0);
        let mut stitch = Stitch::new(first.clone()).unwrap();
        let mut recovery = Recovery::default();
        assert_eq!(
            recovery.consider(&mut stitch, frame(300)).unwrap(),
            Match::Uncertain
        );
        let mut redrawn = first.clone();
        // 少量文字像素重绘，整帧仍稳定，但无法满足完全相等的 Unchanged 路径。
        for y in 70..74 {
            for x in 40..44 {
                redrawn.put_pixel(x, y, Rgba([255, 0, 255, 255]));
            }
        }
        let pixels = stitch.covered_pixels();
        assert!(
            matches!(recovery.consider(&mut stitch, redrawn).unwrap(), Match::Aligned(a) if a.dx == 0 && a.dy == 0)
        );
        assert!(!recovery.is_recovering());
        assert_eq!(stitch.covered_pixels(), pixels);
        assert_eq!(stitch.count(), 1);
        assert_eq!(stitch.compose(), first);
        assert!(matches!(
            recovery.consider(&mut stitch, frame(100)).unwrap(),
            Match::Aligned(_)
        ));
        assert_eq!(stitch.height, 260);
    }

    #[test]
    fn returning_to_trusted_overlap_replays_cache_even_when_observation_is_unchanged() {
        let frame = |offset: u32| {
            RgbaImage::from_fn(96, 160, |x, y| {
                let v = (x.wrapping_mul(73856093) ^ (y + offset).wrapping_mul(19349663))
                    .wrapping_mul(83492791);
                Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
            })
        };
        let mut stitch = Stitch::new(frame(0)).unwrap();
        let mut recovery = Recovery::default();
        let now = Instant::now();
        assert_eq!(
            recovery.consider_at(&mut stitch, frame(240), now).unwrap(),
            Match::Uncertain
        );
        // 模拟可信画布已推进，但本次观察仍停在该可信位置；不能只清除失败计数。
        assert!(matches!(
            stitch.consider(frame(120)).unwrap(),
            Match::Aligned(_)
        ));
        assert_eq!(
            recovery.consider_at(&mut stitch, frame(120), now).unwrap(),
            Match::Unchanged
        );
        assert_eq!(stitch.height, 400);
        assert_eq!(stitch.viewport().y, 120);
        assert_eq!(recovery.recent.len(), 0);
        assert_eq!(recovery.recoveries, 1);
        assert!(!stitch.has_holes());
        assert!(!recovery.is_recovering());
    }
}
