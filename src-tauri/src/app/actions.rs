//! 托盘、全局快捷键与命令行共用的功能入口，负责处理会话冲突并异步启动任务。
use crate::{capture::{self, CaptureState}, clipboard, pins, recognition, settings::ShortcutAction, shelf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

/// 按动作分发；全局快捷键与命令行触发走同一入口。
pub(super) fn run(app: &AppHandle, action: ShortcutAction) {
    match action {
        ShortcutAction::Capture => trigger(app),
        ShortcutAction::Pin => trigger_pin(app),
        ShortcutAction::Record => trigger_recording(app),
        ShortcutAction::History => clipboard::open_history(app),
        ShortcutAction::Shelf => shelf::toggle(app),
        ShortcutAction::Ocr => trigger_ocr(app),
        ShortcutAction::Translate => recognition::text_input::open(app),
    }
}

/// 命令行动作解析在 core，GPUI 版共用同一套参数格式。
pub(super) use snap_core::settings::action_from_args as from_args;

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

/// OCR 全局键直接进入文字框选；与普通截图共用会话占用与错误处理。
/// 截图已在进行时交给截图窗口识别当前选区，不再静默忽略。
pub(super) fn trigger_ocr(app: &AppHandle) {
    match capture::request_ocr(app) {
        Ok(false) => {}
        Ok(true) => return,
        Err(error) => return capture::report_error(app, error),
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = capture::begin_ocr(app.clone()).await {
            capture::report_error(&app, error);
        }
    });
}

/// 托盘和全局贴图键优先使用当前截图选区；没有截图会话时读取剪贴板。
pub(super) fn trigger_pin(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let worker_app = app.clone();
        // 必须接收 JoinError：剪贴板解码或窗口创建 panic 后提示失败，不能悄悄丢失任务。
        let result = tauri::async_runtime::spawn_blocking(move || {
            pins::restore_click_through(&worker_app).and_then(|restored| {
                if restored { return Ok(()); }
                capture::request_pin(&worker_app).and_then(|handled| {
                    if handled { Ok(()) } else { pins::clipboard_pin(&worker_app) }
                })
            })
        }).await.unwrap_or_else(|_| Err("贴图处理失败，请重新复制内容后重试。".into()));
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
