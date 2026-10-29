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

/// 采样结果显式区分运动、停止、取消和系统错误；运动不会触发错误导出。
#[derive(Debug)]
pub(super) enum SampleOutcome {
    Stable(RgbaImage),
    StillMoving,
    CaptureFailure(String),
    Cancelled,
    Stopped,
}

/// 直接记录 xcap 调用耗时（不包含裁剪、尺寸查询和稳定性检测）。
fn timed_capture(
    backend: &str,
    capture: impl FnOnce() -> Result<RgbaImage, String>,
) -> Result<RgbaImage, String> {
    let started = Instant::now();
    let result = capture();
    log::debug!(
        "event=scroll.capture module=scroll backend={} duration_ms={:.2} success={}",
        backend,
        started.elapsed().as_secs_f64() * 1000.0,
        result.is_ok()
    );
    result
}

fn sample(
    monitor: &xcap::Monitor,
    region: Rect,
    source_size: (u32, u32),
) -> Result<RgbaImage, String> {
    let started = Instant::now();
    let image = timed_capture("capture_image", || {
        monitor
            .capture_image()
            .map_err(|e| format!("滚动截图采集失败：{e}"))
    })?;
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
    let image = timed_capture("capture_region", || {
        monitor
            .capture_region(region.x, region.y, region.width, region.height)
            .map_err(|e| e.to_string())
    })?;
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
        failed => {
            log::debug!(
                "event=scroll.capture_fallback module=scroll reason={}",
                if failed.is_err() {
                    "region_failed"
                } else {
                    "region_dimensions"
                }
            );
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
) -> SampleOutcome {
    let result = sample_pair_impl(state, rect, source_size, screen, finishing);
    let outcome = result.unwrap_or_else(SampleOutcome::CaptureFailure);
    log::debug!(
        "event=scroll.sample_result module=scroll result={}",
        match &outcome {
            SampleOutcome::Stable(_) => "Stable",
            SampleOutcome::StillMoving => "StillMoving",
            SampleOutcome::CaptureFailure(_) => "CaptureFailure",
            SampleOutcome::Cancelled => "Cancelled",
            SampleOutcome::Stopped => "Stopped",
        }
    );
    outcome
}

fn interrupted(state: &CaptureState, finishing: bool) -> Option<SampleOutcome> {
    if state.scroll_cancel.load(Ordering::SeqCst) {
        Some(SampleOutcome::Cancelled)
    } else if !finishing && state.scroll_stop.load(Ordering::SeqCst) {
        Some(SampleOutcome::Stopped)
    } else {
        None
    }
}

fn sample_pair_impl(
    state: &CaptureState,
    rect: Rect,
    source_size: (u32, u32),
    screen: &super::Screen,
    finishing: bool,
) -> Result<SampleOutcome, String> {
    if let Some(outcome) = interrupted(state, finishing) {
        return Ok(outcome);
    }
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
    let started = Instant::now();
    let mut capture_started = started;
    let mut quiet_samples = 0;
    let mut comparisons = 0;
    // 按真实时间限制稳定等待；40 ms 是两次采样的最小间隔，不在慢抓屏后叠加睡眠。
    // 至少做两次复核，避免慢抓屏始终在第二次复核前超时；单次系统抓屏无法中断。
    // 与静止期首帧比较，不能把缓慢运动逐帧放行。
    loop {
        if let Some(outcome) = interrupted(state, finishing) {
            return Ok(outcome);
        }
        if comparisons >= 2 && started.elapsed() >= Duration::from_millis(1200) {
            return Ok(SampleOutcome::StillMoving);
        }
        // finishing 忽略 stop，但每 10 ms 检查 cancel。
        while capture_started.elapsed() < Duration::from_millis(40) {
            if let Some(outcome) = interrupted(state, finishing) {
                return Ok(outcome);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        capture_started = Instant::now();
        let candidate = if native_region {
            region_or_full(
                &mut native_region,
                (rect.width, rect.height),
                || sample_region(&monitor, rect, source_size),
                || sample(&monitor, rect, source_size),
            )?
        } else {
            sample(&monitor, rect, source_size)?
        };
        if let Some(outcome) = interrupted(state, finishing) {
            return Ok(outcome);
        }
        comparisons += 1;
        if stable(&reference, &candidate) {
            quiet_samples += 1;
            if quiet_samples >= 2 {
                return Ok(SampleOutcome::Stable(candidate));
            }
        } else {
            reference = candidate;
            quiet_samples = 0;
        }
    }
}

#[cfg(test)]
mod tests {
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
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn finish_ignores_stop_but_cancel_always_wins() {
        let state = CaptureState::default();
        state.scroll_stop.store(true, Ordering::SeqCst);
        assert!(matches!(
            interrupted(&state, false),
            Some(SampleOutcome::Stopped)
        ));
        assert!(interrupted(&state, true).is_none());
        state.scroll_cancel.store(true, Ordering::SeqCst);
        for finishing in [false, true] {
            assert!(matches!(
                interrupted(&state, finishing),
                Some(SampleOutcome::Cancelled)
            ));
        }
    }

    /// 显式运行时只输出数值，不写原始帧；两种后端分别计时，包含冷启动样本。
    #[test]
    #[ignore = "需要真实桌面，测量 xcap 区域/整屏采集耗时"]
    fn native_capture_timings() {
        #[cfg(windows)]
        let _dpi = crate::capture::desktop::PhysicalDpi::enter().unwrap();
        let monitor = xcap::Monitor::all()
            .unwrap()
            .into_iter()
            .next()
            .expect("需要显示器");
        let width = monitor.width().unwrap().min(960);
        let height = monitor.height().unwrap().min(720);
        for iteration in 0..5 {
            for backend in ["capture_region", "capture_image"] {
                let started = Instant::now();
                let result = if backend == "capture_region" {
                    monitor.capture_region(0, 0, width, height)
                } else {
                    monitor.capture_image()
                };
                eprintln!("event=scroll.capture_probe backend={backend} iteration={iteration} duration_ms={:.2} success={} region={}x{}", started.elapsed().as_secs_f64() * 1000.0, result.is_ok(), width, height);
                assert!(result.is_ok(), "{backend}: {:?}", result.err());
            }
        }
    }
}
