//! 顶部暂存架是剪贴板历史的轻量视图，只持久化设置和固定 ID，不复制素材。
pub mod commands;
mod window;
pub(crate) use window::{resume_after_capture, suspend_for_capture};

use crate::clipboard::Service as ClipboardService;
/// 设置模型、`shelf.json` 读写、条目操作与快照拼装在 core（GPUI 版共用）；这里保留窗口、悬停采样与命令。
pub use snap_core::shelf::{Preferences, ShelfSettings, ShelfSnapshot};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

/// 保存视图偏好与悬停采样生命周期；历史服务仍拥有全部内容。
pub struct ShelfState {
    preferences: Mutex<Result<Preferences, String>>,
    pub(crate) window_gate: Mutex<()>,
    /// Some 表示采集临时隐藏期间，值记录采集前是否展开；不持久化到设置。
    capture_visibility: Mutex<Option<bool>>,
    stop: AtomicBool,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ShelfState {
    fn preferences(&self) -> Result<Preferences, String> {
        self.preferences.lock().map_err(|_| "暂存架忙碌。")?.clone()
    }
    /// 临时文件与目标同目录，替换失败时保留原配置与内存状态。
    fn update(
        &self,
        app: &AppHandle,
        change: impl FnOnce(&mut Preferences) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut guard = self.preferences.lock().map_err(|_| "暂存架忙碌。")?;
        let mut next = guard.as_ref().map_err(Clone::clone)?.clone();
        change(&mut next)?;
        let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        next.save(&dir.join("shelf.json"))?;
        *guard = Ok(next);
        drop(guard);
        let _ = app.emit_to("shelf", "shelf-changed", ());
        Ok(())
    }
    /// 同一轮读取持历史操作锁，避免清理素材时拼出不一致的卡片列表。
    pub fn snapshot(&self, app: &AppHandle) -> Result<ShelfSnapshot, String> {
        let prefs = self.preferences()?;
        snap_core::shelf::snapshot(&prefs, &app.state::<ClipboardService>())
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

/// 位置采样仅在开启顶部悬停时运行；隐藏时没有拦截鼠标的透明窗口。
pub fn initialize(app: &AppHandle) {
    let preferences = (|| {
        let path = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("shelf.json");
        Preferences::load(&path)
    })();
    app.manage(ShelfState {
        preferences: Mutex::new(preferences),
        window_gate: Mutex::new(()),
        capture_visibility: Mutex::new(None),
        stop: AtomicBool::new(false),
        worker: Mutex::new(None),
    });
    let handle = app.clone();
    let worker = std::thread::Builder::new()
        .name("shelf-hover".into())
        .spawn(move || {
            let mut hovering = None;
            let mut armed = true;
            loop {
                std::thread::sleep(Duration::from_millis(100));
                let state = handle.state::<ShelfState>();
                if state.stop.load(Ordering::Acquire) {
                    break;
                }
                if !state
                    .preferences()
                    .is_ok_and(|p| p.settings.enabled && p.settings.top_hover)
                {
                    hovering = None;
                    continue;
                }
                if handle
                    .state::<crate::capture::CaptureState>()
                    .active
                    .load(Ordering::Acquire)
                {
                    hovering = None;
                    continue;
                }
                let Ok(point) = handle.cursor_position() else {
                    continue;
                };
                let Ok(Some(monitor)) = handle.monitor_from_point(point.x, point.y) else {
                    continue;
                };
                let scale = monitor.scale_factor();
                let pos = monitor.position();
                let size = monitor.size();
                let top = (point.y - f64::from(pos.y)).abs() <= 2.0 * scale;
                let center = (point.x - (f64::from(pos.x) + f64::from(size.width) / 2.0)).abs()
                    <= 80.0 * scale;
                if top && center {
                    let since = hovering.get_or_insert_with(std::time::Instant::now);
                    if armed && since.elapsed() >= Duration::from_millis(350) {
                        armed = false;
                        // 窗口创建可能等待 UI 线程；采样线程不能等待它，否则退出时 join 会卡住 UI。
                        let app = handle.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = window::show(&app) {
                                crate::capture::report_error(&app, error);
                            }
                        });
                    }
                } else {
                    hovering = None;
                    armed = true;
                }
            }
        });
    if let Ok(worker) = worker {
        *app.state::<ShelfState>().worker.lock().unwrap() = Some(worker);
    }
}

/// 由资源回收循环调用：收起较久的暂存架窗口释放 WebView。
pub fn release_idle_window(app: &AppHandle) {
    window::release_idle(app);
}

/// 快捷键沿用应用统一映射、冲突校验和注册生命周期；默认不占用新按键。
pub fn toggle(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !handle
            .state::<ShelfState>()
            .preferences()
            .is_ok_and(|p| p.settings.enabled)
        {
            return;
        }
        let result = if handle
            .get_webview_window("shelf")
            .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
            window::hide(&handle)
        } else {
            window::show(&handle)
        };
        if let Err(error) = result {
            crate::capture::report_error(&handle, error);
        }
    });
}
