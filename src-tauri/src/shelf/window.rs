//! 单个透明窗口按鼠标所在显示器的物理坐标定位；宽高先按该显示器 DPI 换算。
use super::ShelfState;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

pub(super) fn show(app: &AppHandle) -> Result<(), String> {
    if app
        .state::<crate::capture::CaptureState>()
        .active
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
    }
    let state = app.state::<ShelfState>();
    let _gate = state.window_gate.lock().map_err(|_| "暂存架窗口忙碌。")?;
    // 取得窗口锁期间也可能刚好开始截图，必须再次检查，避免晚到的展开污染抓屏。
    if app
        .state::<crate::capture::CaptureState>()
        .active
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
    }
    let snapshot = state.snapshot(app)?;
    if !snapshot.settings.enabled {
        return Err("请先开启顶部暂存架。".into());
    }
    let point = app.cursor_position().map_err(|e| e.to_string())?;
    let monitor = app
        .monitor_from_point(point.x, point.y)
        .map_err(|e| e.to_string())?
        .ok_or("显示器不可用。")?;
    let scale = monitor.scale_factor();
    let width = (((snapshot.items.len().clamp(1, 5) * 174 + 96) as f64 * scale) as u32)
        .min(monitor.size().width);
    // 底部留出翻页控件空间，避免它与最高卡片的悬停操作相互覆盖。
    let height = (248.0 * scale) as u32;
    let window = if let Some(window) = app.get_webview_window("shelf") {
        window
    } else {
        WebviewWindowBuilder::new(
            app,
            "shelf",
            WebviewUrl::App("index.html?window=shelf".into()),
        )
        .title("Snap Desk Shelf")
        .transparent(true)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .visible(false)
        .focused(false)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| e.to_string())?
    };
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(
            monitor.position().x + ((monitor.size().width - width) / 2) as i32,
            monitor.position().y,
        ))
        .map_err(|e| e.to_string())?;
    let _ = app.emit_to("shelf", "shelf-changed", ());
    window.show().map_err(|e| e.to_string())?;
    // 常驻 WebView 再次显示时不重新挂载；显式通知前端重播轻量入场并返回最近一组。
    let _ = app.emit_to("shelf", "shelf-revealed", ());
    Ok(())
}

pub(super) fn hide(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<ShelfState>();
    let _gate = state.window_gate.lock().map_err(|_| "暂存架窗口忙碌。")?;
    // 截图期间到达的前端自动收起请求不能覆盖“采集后恢复”的状态。
    if state
        .capture_visibility
        .lock()
        .map_err(|_| "暂存架状态不可用。")?
        .is_some()
    {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("shelf") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 抓屏前临时隐藏；返回是否真正隐藏了窗口，供采集端决定是否等待桌面合成。
pub(crate) fn suspend_for_capture(app: &AppHandle) -> Result<bool, String> {
    let Some(state) = app.try_state::<ShelfState>() else {
        return Ok(false);
    };
    let _gate = state.window_gate.lock().map_err(|_| "暂存架窗口忙碌。")?;
    let mut saved = state
        .capture_visibility
        .lock()
        .map_err(|_| "暂存架状态不可用。")?;
    if saved.is_some() {
        return Ok(false);
    }
    let visible = app
        .get_webview_window("shelf")
        .map(|window| {
            let visible = window.is_visible().map_err(|e| e.to_string())?;
            if visible {
                window.hide().map_err(|e| e.to_string())?;
            }
            Ok::<bool, String>(visible)
        })
        .transpose()?
        .unwrap_or(false);
    *saved = Some(visible);
    let _ = app.emit_to("shelf", "shelf-capture-state", true);
    Ok(visible)
}

/// 完成、取消或失败统一恢复原先展开的窗口，保留原显示器和位置，不抢焦点。
/// 恢复失败只报告错误，不能把已成功复制/保存的截图改报为失败。
pub(crate) fn resume_after_capture(app: &AppHandle) {
    let result = (|| {
        let Some(state) = app.try_state::<ShelfState>() else {
            return Ok(());
        };
        let _gate = state.window_gate.lock().map_err(|_| "暂存架窗口忙碌。")?;
        // 连续截图已经开始时保留恢复标记，交给新会话结束后恢复。
        if app
            .state::<crate::capture::CaptureState>()
            .active
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        let previous = state
            .capture_visibility
            .lock()
            .map_err(|_| "暂存架状态不可用。")?
            .take();
        let Some(visible) = previous else {
            return Ok(());
        };
        let result = (|| {
            if visible && state.preferences()?.settings.enabled {
                if let Some(window) = app.get_webview_window("shelf") {
                    window.show().map_err(|e| e.to_string())?;
                }
            }
            Ok::<(), String>(())
        })();
        let _ = app.emit_to("shelf", "shelf-capture-state", false);
        let _ = app.emit_to("shelf", "shelf-changed", ());
        result
    })();
    if let Err(error) = result {
        crate::capture::report_error(app, error);
    }
}
