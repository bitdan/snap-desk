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
    abort, cancel, current, finish, install, launch, new_session, stop, ExportBackground, Host,
    Screen, ScrollSession, ScrollState, ScrollView,
};

// 原一维算法仅作为测试回归对照，不进入发布路径。
#[cfg(test)]
mod legacy_matcher;
#[cfg(test)]
mod legacy_stitcher;

use std::sync::atomic::{AtomicU64, Ordering};

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
