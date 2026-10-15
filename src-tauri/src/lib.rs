mod annotations;
mod autostart;
mod capture;
mod clipboard;
mod image_io;
mod pin_content;
mod pins;
mod platform;
mod qr;
mod settings;
mod storage;

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
    version: String,
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
            version: app.package_info().version.to_string(),
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

/// 主窗口的录屏按钮直接进入录屏框选，复用全局录屏动作的采集入口。
#[tauri::command]
async fn select_recording_area(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_window(&window, "main")?;
    capture::begin_recording(app).await
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

/// 托盘和全局贴图键优先使用当前截图选区；没有截图会话时读取剪贴板。
fn trigger_pin(app: &AppHandle) {
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
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            // 登录自启与已有实例相遇时保持安静；手动再次启动仍唤起主窗口。
            if !autostart::is_autostart(args)
                && !app.state::<CaptureState>().active.load(Ordering::SeqCst)
            {
                show_main(app);
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![autostart::STARTUP_ARG]),
        ))
        .manage(autostart::AutostartState::default())
        .manage(CaptureState::default())
        .manage(settings::ShortcutPressState::default())
        .manage(qr::QrState::default())
        .manage(capture::recording::RecordingState::default())
        .manage(capture::recording::result::ResultState::default())
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
                    let pressed = app.state::<settings::ShortcutPressState>();
                    if event.state() == ShortcutState::Released {
                        if let Ok(mut keys) = pressed.0.lock() {
                            keys.retain(|key| key != shortcut);
                        }
                        return;
                    }
                    // 按已提交映射响应事件；try_lock 避免阻塞系统事件线程。
                    let action = app
                        .try_state::<settings::SettingsState>()
                        .and_then(|state| {
                            let current = state.0.try_lock().ok()?;
                            if current.editing
                                || !current.value.shortcuts_enabled
                                || !current.registered.contains(shortcut)
                            {
                                return None;
                            }
                            let action = current.value.action_for(shortcut)?;
                            let mut keys = pressed.0.lock().ok()?;
                            if keys.contains(shortcut) {
                                return None;
                            }
                            keys.push(*shortcut);
                            Some(action)
                        });
                    match action {
                        Some(settings::ShortcutAction::Capture) => trigger(app),
                        Some(settings::ShortcutAction::Pin) => trigger_pin(app),
                        Some(settings::ShortcutAction::Record) => trigger_recording(app),
                        Some(settings::ShortcutAction::History) => clipboard::open_history(app),
                        None => {}
                    }
                })
                .build(),
        )
        .setup(|app| {
            settings::initialize(app.handle())?;
            clipboard::initialize(app.handle());
            let capture = MenuItem::with_id(app, "capture", "开始截图", true, None::<&str>)?;
            let pin = MenuItem::with_id(app, "pin", "贴图（选区 / 剪贴板）", true, None::<&str>)?;
            let recording = MenuItem::with_id(app, "recording", "录屏", true, None::<&str>)?;
            let history = MenuItem::with_id(app, "history", "剪贴板历史", true, None::<&str>)?;
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
            let menu = Menu::with_items(
                app,
                &[
                    &capture, &pin, &recording, &history, &open, &shortcuts, &quit,
                ],
            )?;
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
                    "history" => clipboard::open_history(app),
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
            // 创建时即隐藏，避免先显示再 hide 导致登录时主窗口闪烁。
            let automatic = autostart::is_autostart(std::env::args_os().skip(1));
            WebviewWindowBuilder::from_config(app.handle(), main_config)?
                .visible(!automatic)
                .focused(!automatic)
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 按已提交映射响应事件；try_lock 避免阻塞系统事件线程。??
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::Focused(false)
                        | tauri::WindowEvent::CloseRequested { .. }
                        | tauri::WindowEvent::Destroyed
                )
            {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = settings::set_editing(&app, false) {
                        capture::report_error(&app, error);
                    }
                });
            }
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
            clipboard::clipboard_status,
            clipboard::clipboard_list,
            clipboard::clipboard_get_text,
            clipboard::clipboard_get_thumbnail,
            clipboard::clipboard_restore,
            clipboard::clipboard_pin_item,
            clipboard::clipboard_set_favorite,
            clipboard::clipboard_delete,
            clipboard::clipboard_clear,
            clipboard::clipboard_configure,
            clipboard::export::clipboard_export,
            autostart::autostart_enabled,
            autostart::set_autostart_enabled,
            select_recording_area,
            storage::app_directories,
            storage::open_app_directory,
            storage::copy_app_directory,
            storage::export_settings,
            storage::copy_shortcut_example,
            settings::complete_onboarding,
            hide_main,
            open_project_homepage,
            settings::save_settings,
            settings::set_shortcuts_enabled,
            settings::set_shortcut_editing,
            capture::cancel_pending_capture,
            capture::start_capture,
            capture::current_capture,
            capture::show_capture,
            capture::cancel_capture,
            capture::recognize_capture_qr,
            qr::qr_action,
            capture::copy_capture,
            capture::copy_capture_color,
            capture::save_capture,
            capture::pin_capture,
            capture::scroll::session::start_scroll,
            capture::scroll::session::current_scroll,
            capture::scroll::session::control_scroll,
            capture::recording::start_recording,
            capture::recording::current_recording,
            capture::recording::result::latest_recording_result,
            capture::recording::result::recording_result_action,
            capture::recording::recovery::recoverable_recordings,
            capture::recording::recovery::retry_recording_export,
            capture::recording::recording_capabilities,
            capture::recording::output::current_recording_directory,
            capture::recording::output::choose_recording_directory,
            capture::recording::control_recording,
            pins::pin_clipboard,
            pins::pin_file,
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
        .build(tauri::generate_context!())
        .expect("无法启动 Snap Desk")
        .run(|app, event| {
            // 录屏的退出流程完成后才收到 Exit；等待历史在途写入释放。
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<clipboard::Service>().shutdown();
            }
        });
}
