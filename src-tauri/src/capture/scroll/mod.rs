//! 滚动截图：采集、稳定检测、匹配、拼接和锚点恢复，由 session 协调生命周期。
//! archive/chrome 分别封装存档和辅助窗口，录屏复用后者。
mod archive;
mod capture;
pub(crate) mod chrome;
mod input;
mod matcher;
mod recovery;
pub(crate) mod session;
mod stability;
mod stitcher;

pub use session::stop;
pub(super) use session::{Screen, ScrollSession};

// 原一维算法仅作为测试回归对照，不进入发布路径。
#[cfg(test)]
mod legacy_matcher;
#[cfg(test)]
mod legacy_stitcher;
