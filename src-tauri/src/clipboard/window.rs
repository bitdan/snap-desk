//! 剪贴板独立窗口：由快捷键、托盘或主窗口唤出，贴近光标显示，失焦后由前端决定是否收起。
//! 坐标均为物理像素；定位只使用鼠标所在显示器的工作区，不遮挡任务栏。
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use crate::shared::idle_release::{IdleSince, WINDOW_IDLE};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub(super) const LABEL: &str = "clipboard";
/// 快捷键连按时串行化创建与显示，避免两次请求同时构建同名窗口。
static GATE: Mutex<()> = Mutex::new(());

/// 收起后开始计时，到期由资源回收循环销毁 WebView；再次唤出按首次使用重新创建。
static IDLE: IdleSince = IdleSince::new();

/// 直接粘贴的目标：唤出前的前台窗口句柄数值，0 表示没有可粘贴的外部窗口。
static TARGET: AtomicUsize = AtomicUsize::new(0);

pub(super) fn target() -> usize {
    TARGET.load(Ordering::Acquire)
}

/// 固定窗口后用户会在别的应用与本窗口之间来回切换；失焦时记下切换到的窗口，
/// 再回来选择条目时粘贴到它。前台仍是本程序（预览、导出等子窗口）时保留原目标。
pub(super) fn remember_switched_target() {
    let hwnd = super::platform::foreground_window();
    if hwnd != 0 {
        TARGET.store(hwnd, Ordering::Release);
    }
}

/// 首次使用时创建，收起较久后由 release_idle 销毁；预览、导出和确认弹窗沿用固定空白页的开窗通道。
fn ensure(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(LABEL) {
        return Ok(window);
    }
    let builder = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?window=clipboard".into()),
    )
    .title("Snap Desk Clipboard")
    .inner_size(460.0, 680.0)
    .min_inner_size(420.0, 460.0)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .focused(false);
    crate::shared::dialog_windows::attach(builder, app, LABEL)
        .build()
        .map_err(|e| e.to_string())
}

/// 窗口左上角放在光标右下方，并夹紧到工作区内；读取失败时保留上次位置。
fn place(app: &AppHandle, window: &WebviewWindow) {
    let Ok(point) = app.cursor_position() else {
        return;
    };
    let Ok(Some(monitor)) = app.monitor_from_point(point.x, point.y) else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let area = monitor.work_area();
    let offset = (12.0 * monitor.scale_factor()) as i32;
    let max_x = area.position.x + area.size.width as i32 - size.width as i32;
    let max_y = area.position.y + area.size.height as i32 - size.height as i32;
    // 窗口比工作区大时 max 小于起点，此时贴齐工作区左上角。
    let x = (point.x as i32 + offset).min(max_x).max(area.position.x);
    let y = (point.y as i32 + offset).min(max_y).max(area.position.y);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// 隐藏状态下重新定位再显示；已显示时只取回焦点，不移动用户摆好的位置。
pub(super) fn show(app: &AppHandle) -> Result<(), String> {
    let _gate = GATE.lock().map_err(|_| "剪贴板窗口忙碌。")?;
    IDLE.clear();
    let window = ensure(app)?;
    // 必须在取得焦点之前记录；从主窗口按钮唤出时得到 0，随后的回车只复制不粘贴。
    if !window.is_focused().unwrap_or(false) {
        TARGET.store(super::platform::foreground_window(), Ordering::Release);
    }
    if !window.is_visible().unwrap_or(false) {
        place(app, &window);
    }
    let _ = window.unminimize();
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    // 复用的 WebView 再次显示时不重新挂载；通知前端聚焦搜索并刷新列表。新建窗口在挂载时自行加载。
    let _ = app.emit_to(LABEL, "clipboard-open", ());
    Ok(())
}

pub(super) fn hide(app: &AppHandle) -> Result<(), String> {
    let _gate = GATE.lock().map_err(|_| "剪贴板窗口忙碌。")?;
    if let Some(window) = app.get_webview_window(LABEL) {
        window.hide().map_err(|e| e.to_string())?;
        IDLE.start();
    }
    Ok(())
}

/// 仍隐藏且没有预览、导出等子窗口时才销毁；窗口忙碌时跳过，下一轮再检查。
pub(super) fn release_idle(app: &AppHandle) {
    let Ok(_gate) = GATE.try_lock() else {
        return;
    };
    let Some(window) = app.get_webview_window(LABEL) else {
        IDLE.clear();
        return;
    };
    if window.is_visible().unwrap_or(true)
        || crate::shared::dialog_windows::has_children(LABEL)
        || !IDLE.take_expired(WINDOW_IDLE)
    {
        return;
    }
    match window.destroy() {
        Ok(()) => log::info!("event=clipboard.window_released module=clipboard reason=idle"),
        Err(error) => {
            log::warn!("event=clipboard.window_release_failed module=clipboard error={error}")
        }
    }
}

/// 快捷键与托盘共用：正在使用时收起，隐藏或被其他窗口盖住时唤到前台。
pub(super) fn toggle(app: &AppHandle) -> Result<(), String> {
    let active = app.get_webview_window(LABEL).is_some_and(|window| {
        window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
    });
    if active {
        hide(app)
    } else {
        show(app)
    }
}
