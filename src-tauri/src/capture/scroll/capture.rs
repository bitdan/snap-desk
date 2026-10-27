//! 连续屏幕取样和短期帧缓存；等待稳定画面，不选择接缝或改变会话状态。
use super::super::{CaptureState, Rect};
use super::stability::stable;
use image::RgbaImage;
use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

/// 有界短期帧缓存；可信锚点由 stitcher 独立持有，过期候选不参与恢复。
#[derive(Clone, Default)]
pub(super) struct FrameCache {
    frames: VecDeque<(Instant, Arc<RgbaImage>)>,
}
impl FrameCache {
    pub(super) fn expire(&mut self, now: Instant) {
        self.frames
            .retain(|(time, _)| now.saturating_duration_since(*time) < Duration::from_secs(5));
    }
    /// 最多三帧、合计 800 万像素，同一静止失败画面不重复存储。
    pub(super) fn remember(&mut self, image: Arc<RgbaImage>, now: Instant) {
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
    /// 只复用完全相同的像素，不能用“基本稳定”代替失败帧身份判断。
    pub(super) fn latest_matches(&self, image: &RgbaImage) -> bool {
        self.frames
            .back()
            .is_some_and(|(_, previous)| previous.as_ref() == image)
    }
    pub(super) fn candidate(&self, index: usize) -> Arc<RgbaImage> {
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

fn sample(
    monitor: &xcap::Monitor,
    region: Rect,
    source_size: (u32, u32),
) -> Result<RgbaImage, String> {
    let started = Instant::now();
    let image = monitor
        .capture_image()
        .map_err(|e| format!("滚动截图采集失败：{e}"))?;
    if image.dimensions() != source_size {
        return Err("显示器像素尺寸发生变化，已结束采集，仅输出已确认部分。".into());
    }
    region.validate(image.width(), image.height())?;
    let cropped =
        image::imageops::crop_imm(&image, region.x, region.y, region.width, region.height)
            .to_image();
    if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
        log::debug!(
            "event=scroll.sample module=scroll width={} height={} duration_ms={:.2}",
            region.width,
            region.height,
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(cropped)
}

/// 快速路径也核对来源显示器的当前物理尺寸，避免切换分辨率后继续使用旧选区。
fn sample_region(
    monitor: &xcap::Monitor,
    region: Rect,
    source_size: (u32, u32),
) -> Result<RgbaImage, String> {
    region.validate(source_size.0, source_size.1)?;
    let image = monitor
        .capture_region(region.x, region.y, region.width, region.height)
        .map_err(|e| e.to_string())?;
    // 捕获后再查询，切换分辨率若发生在抓取过程中也会回退到整屏尺寸校验。
    let size = (
        monitor.width().map_err(|e| e.to_string())?,
        monitor.height().map_err(|e| e.to_string())?,
    );
    if size != source_size {
        return Err("显示器像素尺寸发生变化。".into());
    }
    Ok(image)
}

/// 区域采集是一条可回退的快速路径；失败时仍由整屏采集验证来源尺寸并返回可信像素。
fn region_or_full(
    enabled: &mut bool,
    size: (u32, u32),
    region: impl FnOnce() -> Result<RgbaImage, String>,
    full: impl FnOnce() -> Result<RgbaImage, String>,
) -> Result<RgbaImage, String> {
    match region() {
        Ok(image) if image.dimensions() == size => Ok(image),
        _ => {
            *enabled = false;
            full()
        }
    }
}

/// 连续取样并等待区域稳定；结束前允许最后一次取样，取消仍立即生效。
pub(super) fn sample_pair(
    state: &CaptureState,
    rect: Rect,
    source_size: (u32, u32),
    screen: &super::Screen,
    finishing: bool,
) -> Result<Option<RgbaImage>, String> {
    #[cfg(windows)]
    let _dpi = crate::capture::desktop::PhysicalDpi::enter()?;
    // 一轮稳定性检测绑定选区所在屏幕，全部坐标均为物理像素。
    // 区域采集与整屏回退共用同一来源监视器，尺寸校验不依赖整屏首帧。
    let monitor = xcap::Monitor::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| {
            m.x().ok() == Some(screen.x)
                && m.y().ok() == Some(screen.y)
                && m.width().ok() == Some(screen.width)
                && m.height().ok() == Some(screen.height)
        })
        .ok_or("来源显示器已变化，已结束采集，仅输出已确认部分。")?;
    // Windows 已进入物理 DPI 上下文，优先直接采选区；区域失败时回退整屏校验。
    // 不再每轮先抓整屏，再做一次区域性能/像素对照，避免重复采集和分配。
    let mut native_region = cfg!(windows);
    let mut reference = if native_region {
        region_or_full(
            &mut native_region,
            (rect.width, rect.height),
            || sample_region(&monitor, rect, source_size),
            || sample(&monitor, rect, source_size),
        )?
    } else {
        sample(&monitor, rect, source_size)?
    };
    let mut quiet_samples = 0;
    let mut settled = None;
    // 每 40 ms 检查，连续 80 ms 稳定即可匹配，降低滚动到拼接的等待。
    // 始终与本段静止期的首帧比较，防止缓慢动画的逐帧小变化累积成错位。
    // 最多等待约 0.64 秒；持续运动交由外层重试，期间绝不再发送滚轮。
    for _ in 0..16 {
        if finishing {
            if state.scroll_cancel.load(Ordering::SeqCst) {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(40));
        } else if !wait(state, 40) {
            return Ok(None);
        }
        let candidate = if native_region {
            // 区域失败或返回尺寸异常时立即回退，本轮不再尝试，避免不断重试不支持的后端。
            region_or_full(
                &mut native_region,
                (rect.width, rect.height),
                || sample_region(&monitor, rect, source_size),
                || sample(&monitor, rect, source_size),
            )?
        } else {
            sample(&monitor, rect, source_size)?
        };
        if !finishing && state.scroll_stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if stable(&reference, &candidate) {
            quiet_samples += 1;
            if quiet_samples >= 2 {
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

#[cfg(test)]
mod benchmarks {
    use super::*;

    /// 区域采集失败或尺寸异常时，整屏路径必须仍能返回可信结果或明确错误。
    #[test]
    fn region_failures_fall_back_without_losing_full_capture_errors() {
        let mut enabled = true;
        let full = RgbaImage::from_pixel(2, 3, image::Rgba([7, 8, 9, 255]));
        for region in [Err("unsupported".into()), Ok(RgbaImage::new(3, 2))] {
            let image =
                region_or_full(&mut enabled, (2, 3), || region, || Ok(full.clone())).unwrap();
            assert_eq!(image, full);
            assert!(!enabled);
        }
        enabled = true;
        let image = region_or_full(
            &mut enabled,
            (2, 3),
            || Ok(full.clone()),
            || panic!("不应重复整屏采集"),
        )
        .unwrap();
        assert_eq!(image, full);
        assert!(enabled);
        assert!(region_or_full(
            &mut enabled,
            (2, 3),
            || Err("region".into()),
            || Err("display changed".into())
        )
        .is_err());
    }

    #[test]
    #[ignore = "实机区域采集基准，会采集当前主屏；需手动运行"]
    fn compare_full_capture_and_native_region() {
        let monitor = xcap::Monitor::all()
            .unwrap()
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .unwrap();
        let size = (monitor.width().unwrap(), monitor.height().unwrap());
        for (width, height) in [
            (320.min(size.0), 240.min(size.1)),
            (size.0 / 2, size.1 / 2),
            size,
        ] {
            let rect = Rect {
                x: (size.0 - width) / 2,
                y: (size.1 - height) / 2,
                width,
                height,
            };
            let mut full_times = Vec::new();
            let mut roi_times = Vec::new();
            let mut different = 0u64;
            for index in 0..110 {
                let started = Instant::now();
                let full = sample(&monitor, rect, size).unwrap();
                let full_ms = started.elapsed().as_secs_f64() * 1000.0;
                let started = Instant::now();
                let region = monitor
                    .capture_region(rect.x, rect.y, width, height)
                    .unwrap();
                let roi_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(region.dimensions(), full.dimensions());
                if index >= 10 {
                    full_times.push(full_ms);
                    roi_times.push(roi_ms);
                    different += full
                        .pixels()
                        .zip(region.pixels())
                        .filter(|(a, b)| a != b)
                        .count() as u64;
                }
            }
            full_times.sort_by(f64::total_cmp);
            roi_times.sort_by(f64::total_cmp);
            eprintln!("ROI {width}x{height}: full P50/P95={:.2}/{:.2}ms, region={:.2}/{:.2}ms, different_pixels={different}", full_times[49],full_times[94],roi_times[49],roi_times[94]);
            assert_eq!(
                different, 0,
                "静态桌面下区域采集与整屏裁剪必须一致；动态内容需要重新采样"
            );
        }
    }
}
