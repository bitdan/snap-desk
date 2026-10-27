//! 辅助 WebView 的创建和释放策略；截图窗口独立管理，首次冻结桌面后创建并常驻复用。
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder};

const CONFIGURED: [&str; 3] = ["recording-controls", "scroll-preview", "scroll-controls"];

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    Eager,
    LazyRetain,
    LazyRelease,
}

/// 每次启动固定一种策略，便于用同一产物比较启动、首次使用及释放后的成本。
/// SNAP_DESK_AUX_LIFECYCLE 只供资源测量使用，不影响主窗口和截图窗口。
pub(crate) struct AuxiliaryWindows {
    policy: Policy,
    operation: Mutex<()>,
}
impl Default for AuxiliaryWindows {
    fn default() -> Self {
        let policy = match std::env::var("SNAP_DESK_AUX_LIFECYCLE").as_deref() {
            Ok("eager") => Policy::Eager,
            Ok("lazy-retain") => Policy::LazyRetain,
            _ => Policy::LazyRelease,
        };
        Self { policy, operation: Mutex::new(()) }
    }
}

/// 配置保留完整样式，仅关闭自动创建；调用方均在异步/阻塞任务中准备业务窗口。
pub(crate) fn ensure(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    if !CONFIGURED.contains(&label) { return Err("未知辅助窗口。".into()); }
    let state = app.state::<AuxiliaryWindows>();
    let _operation = state.operation.lock().map_err(|e| e.to_string())?;
    if let Some(window) = app.get_webview_window(label) { return Ok(window); }
    let config = app.config().app.windows.iter().find(|config| config.label == label)
        .ok_or("辅助窗口配置缺失。")?;
    WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .visible(false)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())
}

pub(crate) fn initialize(app: &AppHandle) -> Result<(), String> {
    if app.state::<AuxiliaryWindows>().policy == Policy::Eager {
        for label in CONFIGURED { ensure(app, label)?; }
    }
    Ok(())
}

/// 只释放已存在的辅助窗口；不创建窗口，也不触及主窗口、截图和贴图。
/// 取样避让不调用这里，避免销毁后 SampleGuard 又试图恢复同一窗口。
pub(crate) fn release(app: &AppHandle, label: &str) {
    if !CONFIGURED.contains(&label)
        && !label.starts_with("scroll-edge-") && !label.starts_with("recording-edge-") { return; }
    let state = app.state::<AuxiliaryWindows>();
    let Ok(_operation) = state.operation.lock() else { return };
    if let Some(window) = app.get_webview_window(label) {
        if state.policy == Policy::LazyRelease {
            if label == "recording-controls" {
                crate::capture::recording::editing::release_window(app, label);
            }
            if window.destroy().is_err() {
                // 销毁失败也先收起，避免异常辅助窗口挡住桌面；记录固定事件，不写用户内容。
                let _ = window.hide();
                log::warn!("event=app.auxiliary_release_failed module=app code=AUXILIARY_RELEASE_FAILED");
            }
        } else { let _ = window.hide(); }
    }
}
