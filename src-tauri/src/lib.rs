mod annotations;
mod capture;
mod platform;
mod settings;

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
            let open = MenuItem::with_id(app, "open", "打开 Snap Desk", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&capture, &cancel, &open, &quit])?;
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .tooltip("Snap Desk · 截图工具")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "capture" => trigger(app),
                    "cancel" => {
                        capture::cancel_delay(app);
                    }
                    "open" => {
                        if !app.state::<CaptureState>().active.load(Ordering::SeqCst) {
                            show_main(app);
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "main" {
                    let _ = window.hide();
                } else if window.label() == "capture" {
                    let _ = capture::cancel_active(window.app_handle());
                }
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
            capture::save_capture,
        ])
        .run(tauri::generate_context!())
        .expect("无法启动 Snap Desk");
}
