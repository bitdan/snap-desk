//! 托盘与全局快捷键共用的功能入口，负责处理会话冲突并异步启动任务。
use crate::{capture::{self, CaptureState}, pins};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub(super) fn trigger(app: &AppHandle) {
    // 倒计时内再次按截图键代表取消；真正截图开始后不重复创建会话。
    if capture::cancel_delay(app) {
        return;
    }
    if capture::scroll::stop(app) {
        return;
    }
    if app.state::<CaptureState>().active.load(Ordering::SeqCst) {
        return;
    }
    let requested_at = std::time::Instant::now();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = capture::begin_at(app.clone(), requested_at).await {
            capture::report_error(&app, error);
        }
    });
}

/// 托盘和全局贴图键优先使用当前截图选区；没有截图会话时读取剪贴板。
pub(super) fn trigger_pin(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = capture::request_pin(&app).and_then(|handled| {
            if handled {
                Ok(())
            } else {
                pins::clipboard_pin(&app)
            }
        });
        if let Err(error) = result {
            capture::report_error(&app, error);
        }
    });
}

/// F3 首次进入录屏选区，录制或暂停期间再次按下则结束并保存。
pub(super) fn trigger_recording(app: &AppHandle) {
    if capture::recording::request_stop(app, false)
        || app.state::<CaptureState>().active.load(Ordering::SeqCst)
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = capture::begin_recording(app.clone()).await {
            capture::report_error(&app, error);
        }
    });
}
