//! 滚动截图：采集、稳定检测、匹配、拼接和锚点恢复，由 session 协调生命周期。
//! 窗口（预览、控制条、选区边框）与平台集成由外壳通过 [`session::Host`] 提供。
mod archive;
mod capture;
mod input;
mod matcher;
mod recovery;
pub mod session;
mod stability;
mod stitcher;

pub use matcher::Bounds;
pub use session::{
    abort, cancel, current, finish, install, launch, new_session, stop, write_png,
    ExportBackground, Host, Screen, ScrollSession, ScrollState, ScrollView,
};

// 原一维算法仅作为测试回归对照，不进入发布路径。
#[cfg(test)]
mod legacy_matcher;
#[cfg(test)]
mod legacy_stitcher;

use std::sync::atomic::{AtomicU64, Ordering};

/// 按物理桌面原点查找来源显示器（xcap，物理像素）；不因主屏切换而换到另一块屏幕。
/// 不依赖 UI 框架的外壳（GPUI）用它实现 [`Host::screen`]。
pub fn screen_at(x: i32, y: i32) -> Result<Screen, String> {
    #[cfg(windows)]
    let _dpi = crate::capture::PhysicalDpi::enter()?;
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let describe = |m: &xcap::Monitor| -> Result<(i32, i32, u32, u32, f32), String> {
        let e = |e: xcap::XCapError| e.to_string();
        Ok((m.x().map_err(e)?, m.y().map_err(e)?, m.width().map_err(e)?, m.height().map_err(e)?, m.scale_factor().map_err(e)?))
    };
    let mut layout = Vec::new();
    let mut found = None;
    for monitor in &monitors {
        let (mx, my, width, height, scale) = describe(monitor)?;
        layout.push((mx, my, width, height, u64::from(scale.to_bits())));
        if (mx, my) == (x, y) {
            found = Some((monitor.name().ok(), width, height, scale));
        }
    }
    layout.sort_unstable();
    let (name, width, height, scale) = found.ok_or("来源显示器已移除或布局改变。")?;
    Ok(Screen {
        name,
        x,
        y,
        width,
        height,
        scale: f64::from(scale).to_bits(),
        layout,
    })
}

static LAST_SAMPLE: AtomicU64 = AtomicU64::new(0);

/// 高频匹配细节每 500 ms 最多一条；只限制 Debug，不丢弃故障与生命周期事件。
pub(crate) fn sample_debug() -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let previous = LAST_SAMPLE.load(Ordering::Relaxed);
    now.saturating_sub(previous) >= 500
        && LAST_SAMPLE
            .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}
