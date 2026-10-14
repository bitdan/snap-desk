//! 连续屏幕取样和短期帧缓存；等待稳定画面，不选择接缝或改变会话状态。
use super::super::{CaptureState, Rect};
use super::stability::stable;
use image::RgbaImage;
use std::{
    collections::VecDeque,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

/// 有界短期帧缓存；可信锚点由 stitcher 独立持有，过期候选不参与恢复。
#[derive(Default)]
pub(super) struct FrameCache {
    frames: VecDeque<(Instant, RgbaImage)>,
}
impl FrameCache {
    pub(super) fn expire(&mut self, now: Instant) {
        self.frames
            .retain(|(time, _)| now.saturating_duration_since(*time) < Duration::from_secs(5));
    }
    /// 最多三帧、合计 800 万像素，同一静止失败画面不重复存储。
    pub(super) fn remember(&mut self, image: RgbaImage, now: Instant) {
        if !self
            .frames
            .back()
            .is_some_and(|(_, previous)| previous == &image)
        {
            self.frames.push_back((now, image));
        }
        while self.frames.len() > 3
            || self
                .frames
                .iter()
                .map(|(_, frame)| u64::from(frame.width()) * u64::from(frame.height()))
                .sum::<u64>()
                > 8_000_000
        {
            self.frames.pop_front();
        }
    }
    pub(super) fn len(&self) -> usize {
        self.frames.len()
    }
    pub(super) fn candidate(&self, index: usize) -> RgbaImage {
        self.frames[index].1.clone()
    }
    pub(super) fn remove(&mut self, index: usize) {
        self.frames.remove(index);
    }
}

pub(super) fn wait(state: &CaptureState, millis: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        if state.scroll_stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn sample(region: Rect, source_size: (u32, u32)) -> Result<RgbaImage, String> {
    let started = Instant::now();
    let image = crate::platform::capture_primary()?;
    if image.dimensions() != source_size {
        return Err("显示器像素尺寸发生变化，已结束采集，仅输出已确认部分。".into());
    }
    region.validate(image.width(), image.height())?;
    let cropped =
        image::imageops::crop_imm(&image, region.x, region.y, region.width, region.height)
            .to_image();
    log::debug!(
        "长截图采集：source={source_size:?}, roi={}x{}, millis={:.2}",
        region.width,
        region.height,
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(cropped)
}

/// 连续取样并等待区域稳定；结束前允许最后一次取样，取消仍立即生效。
pub(super) fn sample_pair(
    state: &CaptureState,
    rect: Rect,
    source_size: (u32, u32),
    finishing: bool,
) -> Result<Option<RgbaImage>, String> {
    let mut reference = sample(rect, source_size)?;
    let mut quiet_samples = 0;
    let mut settled = None;
    // 每 80 ms 检查，连续 240 ms 稳定即可继续，不再每轮固定停顿近一秒。
    // 始终与本段静止期的首帧比较，防止缓慢动画的逐帧小变化累积成错位。
    // 最多等待约 1.3 秒；持续运动交由外层重试，期间绝不再发送滚轮。
    for _ in 0..16 {
        if finishing {
            if state.scroll_cancel.load(Ordering::SeqCst) {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(80));
        } else if !wait(state, 80) {
            return Ok(None);
        }
        let candidate = sample(rect, source_size)?;
        if !finishing && state.scroll_stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if stable(&reference, &candidate) {
            quiet_samples += 1;
            if quiet_samples >= 3 {
                settled = Some(candidate);
                break;
            }
        } else {
            reference = candidate;
            quiet_samples = 0;
        }
    }
    Ok(settled)
}
