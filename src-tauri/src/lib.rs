mod annotations;
mod capture;
mod image_io;
mod pin_content;
mod pins;
mod platform;
mod settings;
mod stitch;

use capture::{require_window, show_main, CaptureState};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewWindow,
};
use tauri_plugin_global_shortcut::ShortcutState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    settings: settings::Settings,
    shortcut_error: Option<String>,
    config_warning: Option<String>,
    delay_remaining: u8,
    platform: &'static str,
}

#[tauri::command]
async fn app_status(app: AppHandle, window: WebviewWindow) -> Result<AppStatus, String> {
    require_window(&window, "main")?;
    let state = app.state::<settings::SettingsState>();
    let current = state.0.lock().map_err(|e| e.to_string())?;
    Ok(AppStatus {
        settings: current.value.clone(),
        shortcut_error: current.shortcut_error.clone(),
        config_warning: current.config_warning.clone(),
        delay_remaining: app
            .state::<CaptureState>()
            .delay_remaining
            .load(Ordering::SeqCst),
        platform: std::env::consts::OS,
    })
}

#[tauri::command]
fn hide_main(window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    window.hide().map_err(|e| e.to_string())
}

fn trigger(app: &AppHandle) {
    // 倒计时内再次按截图键代表取消；真正截图开始后不重复创建会话。
    if capture::cancel_delay(app) {
        return;
    }
    if capture::scroll_capture::stop(app) {
        return;
    }
    if capture::recording::request_stop(app, false) {
        return;
    }
    if app.state::<CaptureState>().active.load(Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = capture::begin(app.clone()).await {
            capture::report_error(&app, error);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if !app.state::<CaptureState>().active.load(Ordering::SeqCst) {
                show_main(app);
            }
        }))
        .manage(CaptureState::default())
        .manage(capture::recording::RecordingState::default())
        .manage(pins::PinState::default())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let capture = app.state::<CaptureState>();
                    if event.state() == ShortcutState::Released {
                        capture.hotkey_pressed.store(false, Ordering::SeqCst);
                        return;
                    }
                    // 保存设置时可能短暂注册两个键，只响应当前生效的那个。
                    // try_lock 不阻塞主线程，避免与后台注册快捷键的任务互相等待。
                    let matches = app
                        .try_state::<settings::SettingsState>()
                        .is_some_and(|state| {
                            state.0.try_lock().ok().is_some_and(|current| {
                                current.registered
                                    && current.value.validate().ok().as_ref() == Some(shortcut)
                            })
                        });
                    if matches && !capture.hotkey_pressed.swap(true, Ordering::SeqCst) {
                        trigger(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            settings::initialize(app.handle())?;
            let capture = MenuItem::with_id(app, "capture", "开始截图", true, None::<&str>)?;
            let cancel = MenuItem::with_id(app, "cancel", "取消延时截图", true, None::<&str>)?;
            let scroll_stop = MenuItem::with_id(
                app,
                "scroll-stop",
                "结束长截图并自动保存",
                true,
                None::<&str>,
            )?;
            let scroll_cancel =
                MenuItem::with_id(app, "scroll-cancel", "取消当前截图", true, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "打开 Snap Desk", true, None::<&str>)?;
            let record_pause = MenuItem::with_id(
                app,
                "record-pause",
                "暂停录屏 / 打开控制",
                true,
                None::<&str>,
            )?;
            let record_resume =
                MenuItem::with_id(app, "record-resume", "恢复录屏", true, None::<&str>)?;
            let record_stop =
                MenuItem::with_id(app, "record-stop", "停止录屏并导出 MP4", true, None::<&str>)?;
            let record_cancel =
                MenuItem::with_id(app, "record-cancel", "取消录屏并清理", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &capture,
                    &cancel,
                    &scroll_stop,
                    &scroll_cancel,
                    &record_pause,
                    &record_resume,
                    &record_stop,
                    &record_cancel,
                    &open,
                    &quit,
                ],
            )?;
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .tooltip("Snap Desk · 截图工具")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "capture" => trigger(app),
                    "record-pause" | "record-resume" => {
                        let app = app.clone();
                        let resume = event.id.as_ref() == "record-resume";
                        tauri::async_runtime::spawn_blocking(move || {
                            use capture::recording::RecordingAction;
                            if let Err(error) = capture::recording::control(
                                &app,
                                None,
                                if resume {
                                    RecordingAction::Resume
                                } else {
                                    RecordingAction::Pause
                                },
                            ) {
                                capture::report_error(&app, error);
                            }
                        });
                    }
                    "record-stop" => {
                        capture::recording::request_stop(app, false);
                    }
                    "record-cancel" => {
                        capture::recording::request_stop(app, true);
                    }
                    "scroll-cancel" => {
                        if let Err(error) = capture::cancel_active(app) {
                            capture::report_error(app, error);
                        }
                    }
                    "scroll-stop" => {
                        capture::scroll_capture::stop(app);
                    }
                    "cancel" => {
                        capture::cancel_delay(app);
                    }
                    "open" => {
                        if !app.state::<CaptureState>().active.load(Ordering::SeqCst) {
                            show_main(app);
                        }
                    }
                    // 先完成正在录制的视频；导出完成后可再次从托盘退出。
                    "quit" => {
                        if !capture::recording::request_stop(app, false) {
                            app.exit(0);
                        }
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Resized(_)) && window.label().contains("-edge-")
            {
                if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
                    if let Err(error) = capture::scroll_capture::chrome::restore_edge(&webview) {
                        let _ = webview.hide();
                        log::error!("选区边框裁剪恢复失败：{error}");
                    }
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "recording-controls"
                    || window.label().starts_with("recording-edge-")
                {
                    // 录屏范围与控制条不能被 Alt+F4 关闭，只在停止/取消完成后统一收起。
                    api.prevent_close();
                } else if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                } else if matches!(window.label(), "scroll-preview" | "scroll-controls") {
                    api.prevent_close();
                    capture::scroll_capture::stop(window.app_handle());
                } else if window.label() == "capture" {
                    api.prevent_close();
                    let _ = capture::cancel_active(window.app_handle());
                } else if window.label().starts_with("pin-")
                    && window
                        .app_handle()
                        .state::<pins::PinState>()
                        .operation
                        .load(Ordering::SeqCst)
                {
                    api.prevent_close();
                }
            }
            if matches!(event, tauri::WindowEvent::Destroyed) && window.label().starts_with("pin-")
            {
                pins::destroyed(window.app_handle(), window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            hide_main,
            settings::save_settings,
            capture::cancel_pending_capture,
            capture::start_capture,
            capture::current_capture,
            capture::show_capture,
            capture::cancel_capture,
            capture::copy_capture,
            capture::copy_capture_color,
            capture::save_capture,
            capture::pin_capture,
            capture::scroll_capture::start_scroll,
            capture::scroll_capture::current_scroll,
            capture::scroll_capture::control_scroll,
            capture::recording::start_recording,
            capture::recording::current_recording,
            capture::recording::recording_capabilities,
            capture::recording::control_recording,
            pins::pin_clipboard,
            pins::pin_file,
            pin_content::pin_color,
            pin_content::pin_text,
            pins::set_pin_group,
            pins::current_pin,
            pins::set_pin_annotations,
            pins::list_pins,
            pins::manage_pins,
            pins::set_pin_view,
            pins::drag_pin,
            pins::transform_pin,
            pins::pin_action,
        ])
        .run(tauri::generate_context!())
        .expect("无法启动 Snap Desk");
}
