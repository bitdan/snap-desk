mod annotations;
mod capture;
mod image_io;
mod pin_content;
mod pins;
mod platform;
mod settings;

use capture::{require_window, show_main, CaptureState};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewWindow, WebviewWindowBuilder,
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
    // 设置保存会调用主线程注册快捷键/更新菜单，查询不能在主线程等待同一把锁。
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn hide_main(window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    window.hide().map_err(|e| e.to_string())
}

/// 关于页点击 GitHub 图标后交给系统浏览器；仅允许主窗口打开固定项目地址。
#[tauri::command]
async fn open_project_homepage(window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(platform::open_project_homepage)
        .await
        .map_err(|e| e.to_string())?
}

fn trigger(app: &AppHandle) {
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
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = capture::begin(app.clone()).await {
            capture::report_error(&app, error);
        }
    });
}

/// F2 直接读取剪贴板内存中的图片，复用托盘贴图入口，不需要先另存文件。
fn trigger_pin(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = pins::clipboard_pin(&app) {
            capture::report_error(&app, error);
        }
    });
}

/// F3 首次进入录屏选区，录制或暂停期间再次按下则结束并保存。
fn trigger_recording(app: &AppHandle) {
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if !app.state::<CaptureState>().active.load(Ordering::SeqCst) {
                show_main(app);
            }
        }))
        .manage(CaptureState::default())
        .manage(capture::recording::RecordingState::default())
        .manage(capture::recording::ExitState::default())
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
                    let bit = if *shortcut == settings::pin_shortcut() {
                        2
                    } else if *shortcut == settings::recording_shortcut() {
                        4
                    } else {
                        1
                    };
                    if event.state() == ShortcutState::Released {
                        capture.hotkey_pressed.fetch_and(!bit, Ordering::SeqCst);
                        return;
                    }
                    // 保存设置时可能短暂注册两个键，只响应当前生效的那个。
                    // try_lock 不阻塞主线程，避免与后台注册快捷键的任务互相等待。
                    let matches = app
                        .try_state::<settings::SettingsState>()
                        .is_some_and(|state| {
                            state.0.try_lock().ok().is_some_and(|current| {
                                current.registered.contains(shortcut)
                                    && current.value.shortcuts_enabled
                                    && (bit != 1
                                        || current.value.validate().ok().as_ref() == Some(shortcut))
                            })
                        });
                    if matches && capture.hotkey_pressed.fetch_or(bit, Ordering::SeqCst) & bit == 0
                    {
                        match bit {
                            2 => trigger_pin(app),
                            4 => trigger_recording(app),
                            _ => trigger(app),
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            settings::initialize(app.handle())?;
            let capture = MenuItem::with_id(app, "capture", "开始截图", true, None::<&str>)?;
            let pin = MenuItem::with_id(app, "pin", "剪贴板贴图 (F2)", true, None::<&str>)?;
            let recording = MenuItem::with_id(app, "recording", "录屏 (F3)", true, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "打开 Snap Desk", true, None::<&str>)?;
            let enabled = app
                .state::<settings::SettingsState>()
                .0
                .lock()
                .map_err(|e| e.to_string())?
                .value
                .shortcuts_enabled;
            let shortcuts = CheckMenuItem::with_id(
                app,
                "shortcuts",
                "启用全局快捷键",
                true,
                enabled,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu =
                Menu::with_items(app, &[&capture, &pin, &recording, &open, &shortcuts, &quit])?;
            app.manage(settings::ShortcutMenu(shortcuts));
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .tooltip("Snap Desk · 截图工具")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "capture" => {
                        // 菜单只启动截图；录屏期间由共享占用状态拒绝新截图。
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = capture::begin(app.clone()).await {
                                capture::report_error(&app, error);
                            }
                        });
                    }
                    "pin" => trigger_pin(app),
                    "recording" => trigger_recording(app),
                    "shortcuts" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = settings::set_shortcuts(&app, None) {
                                capture::report_error(&app, error);
                            }
                        });
                    }
                    "open" => show_main(app),
                    // 先完成正在录制的视频，避免退出截断输出。
                    "quit" => {
                        capture::recording::request_exit(app);
                    }
                    _ => {}
                })
                .build(app)?;
            // 配置中的主窗口禁用自动创建：Tauri 自动创建窗口早于本 setup，
            // 页面可能提前调用 app_status，读取尚未 manage 的 SettingsState 而 panic。
            // 等设置、快捷键和托盘全部就绪后，再沿用原窗口配置加载主界面。
            let main_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|config| config.label == "main")
                .ok_or("缺少主窗口配置。")?;
            WebviewWindowBuilder::from_config(app.handle(), main_config)?.build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Resized(_)) && window.label().contains("-edge-")
            {
                if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
                    if let Err(error) = capture::scroll::chrome::restore_edge(&webview) {
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
                    capture::scroll::stop(window.app_handle());
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
            open_project_homepage,
            settings::save_settings,
            settings::set_shortcuts_enabled,
            capture::cancel_pending_capture,
            capture::start_capture,
            capture::current_capture,
            capture::show_capture,
            capture::cancel_capture,
            capture::copy_capture,
            capture::copy_capture_color,
            capture::save_capture,
            capture::pin_capture,
            capture::scroll::session::start_scroll,
            capture::scroll::session::current_scroll,
            capture::scroll::session::control_scroll,
            capture::recording::start_recording,
            capture::recording::current_recording,
            capture::recording::recording_capabilities,
            capture::recording::output::current_recording_directory,
            capture::recording::output::choose_recording_directory,
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
