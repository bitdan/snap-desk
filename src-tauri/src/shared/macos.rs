//! macOS 原生设施的 Tauri 包装。系统函数与 NSWindow 操作在 `snap_core::macos`（GPUI 版共用），
//! 这里只负责从 WebviewWindow 取得 NSWindow 指针并切到主线程执行。
pub use snap_core::macos::*;

use std::{sync::mpsc, time::Duration};
use tauri::WebviewWindow;

/// 把截图蒙层提升到菜单栏之上；ns_window 指针只在主线程有效，因此在主线程闭包内读取并发送消息。
pub fn raise_overlay(window: &WebviewWindow) -> Result<(), String> {
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let Ok(pointer) = target.ns_window() else {
                log::warn!("event=capture.overlay_level_failed module=capture");
                return;
            };
            // SAFETY: 指针来自 Tauri 管理的存活 NSWindow，且当前位于主线程。
            unsafe { raise_ns_window(pointer) }
        })
        .map_err(|e| e.to_string())
}

/// 在主线程同步设置窗口外框并读回实际 frame（物理像素，左上角原点）；非主线程调用时等待最多 3 秒。
pub(crate) fn set_window_frame_sync(
    window: &WebviewWindow,
    requested: (i32, i32, u32, u32),
    scale: f64,
) -> Result<(i32, i32, u32, u32), String> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("显示器缩放比例无效：{scale}。"));
    }
    let target = window.clone();
    let apply = move || -> Result<(i32, i32, u32, u32), String> {
        let pointer = target.ns_window().map_err(|e| e.to_string())?;
        if pointer.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: 指针来自 Tauri 管理的存活 NSWindow，且当前位于主线程。
        Ok(unsafe { set_ns_window_frame(pointer, requested, scale) })
    };
    if is_main_thread() {
        return apply();
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let _ = sender.send(apply());
        })
        .map_err(|e| e.to_string())?;
    // 主线程卡住时给出明确失败，而不是无限等待；正常情况下同一轮事件循环内完成。
    receiver
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "等待窗口尺寸生效超时：主线程未响应。".to_string())?
}
