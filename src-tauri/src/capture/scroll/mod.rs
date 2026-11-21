//! 滚动截图：采集、匹配、拼接与会话生命周期在 core（`snap_core::scroll`）；
//! 这里负责 Tauri 命令、辅助窗口（chrome，录屏复用）与输出集成。
pub(crate) mod chrome;
pub(crate) mod session;

pub use session::stop;
pub(super) use snap_core::scroll::{Screen, ScrollState};
