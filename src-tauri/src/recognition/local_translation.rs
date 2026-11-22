//! 本地翻译进程桥：协议、校验与 worker 生命周期在 core `snap_core::recognition::local_worker`；
//! 这里提供应用状态、资源目录、退出标记与设置中的闲置秒数。
use snap_core::recognition::local_worker as core;
use tauri::{AppHandle, Manager};

pub use core::{LocalError, LocalTranslationState};

/// 用目标语言明确选择方向；Future 被丢弃（取消/超时）即终止 worker。
pub async fn translate(
    app: &AppHandle,
    id: &str,
    target: &str,
    text: &str,
    timeout: u64,
) -> Result<String, LocalError> {
    let resources = crate::resources::directory(app)?;
    let state = app.state::<LocalTranslationState>().inner().clone();
    let handle = app.clone();
    let stopping = move || crate::resources::lifecycle::stopping(&handle);
    core::translate(&state, &resources, &stopping, id, target, text, timeout).await
}

/// 使用最新保存的闲置秒数；0 表示保持缓存。先释放设置锁，再尝试 Worker 锁。
pub async fn release_idle(app: &AppHandle) {
    let seconds = match app.state::<crate::settings::SettingsState>().0.try_lock() {
        Ok(settings) => settings.value.ai.local_idle_seconds,
        Err(_) => return,
    };
    let state = app.state::<LocalTranslationState>().inner().clone();
    core::release_idle(&state, seconds).await;
}

/// 正常退出显式终止缓存进程；活动任务由现有取消信号清理。
pub fn shutdown(app: &AppHandle) {
    core::shutdown(app.state::<LocalTranslationState>().inner());
}
