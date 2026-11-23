//! 快捷键兼容测试的 Tauri 外壳：托管状态、热键回调接入与设置页命令；测试与平台探测在
//! `snap_core::settings::shortcut_probe`。
use crate::{capture::require_window, settings::SettingsState};
use snap_core::settings::shortcut_probe::{ProbeReport, ProbeState as CoreProbe, Registration};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Default)]
pub struct ProbeState(CoreProbe);

fn capturing(app: &AppHandle) -> bool {
    app.state::<crate::capture::CaptureState>()
        .active
        .load(std::sync::atomic::Ordering::SeqCst)
}

fn registration(app: &AppHandle) -> Result<Registration, String> {
    let state = app.state::<SettingsState>();
    let current = state.0.lock().map_err(|e| e.to_string())?;
    Ok(Registration {
        enabled: current.value.shortcuts_enabled,
        editing: current.editing,
        error: current.shortcut_error.clone(),
        registered: current.registered.iter().map(ToString::to_string).collect(),
    })
}

/// 仅由已有全局快捷键的按下回调调用；松键仍走原有去重清理路径。
pub fn receive(app: &AppHandle, shortcut: String) -> bool {
    // 测试中若从托盘开始采集，立即让出热键，尤其不能吞掉停止录屏/滚动截图的动作。
    if capturing(app) {
        return false;
    }
    app.state::<ProbeState>().0.receive(shortcut)
}

/// 在后台等待用户切换到 Axure/游戏并按键，避免阻塞 Tauri 系统消息线程。
#[tauri::command]
pub async fn probe_shortcuts(app: AppHandle, window: WebviewWindow) -> Result<ProbeReport, String> {
    require_window(&window, "main")?;
    if !cfg!(any(windows, target_os = "macos")) {
        return Err("此环境测试仅支持 Windows 和 macOS。".into());
    }
    if capturing(&app)
        || crate::capture::recording::current_recording(app.clone(), window)?.is_some()
    {
        return Err("请先结束截图、滚动截图或录屏，再测试快捷键。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let interrupted = || capturing(&app);
        // macOS 输入法标识需在主线程读取。
        let input_source = || {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            app.run_on_main_thread(move || {
                let _ = sender.send(input_source_id());
            })
            .ok()
            .and_then(|_| receiver.recv_timeout(Duration::from_millis(200)).ok())
            .flatten()
        };
        let mut report =
            app.state::<ProbeState>()
                .0
                .run(registration(&app)?, &interrupted, &input_source)?;
        report.set_registration(registration(&app)?);
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "macos")]
fn input_source_id() -> Option<String> {
    snap_core::macos::input_source_id()
}
#[cfg(not(target_os = "macos"))]
fn input_source_id() -> Option<String> {
    None
}
