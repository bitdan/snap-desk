//! 应用组装入口：插件、托盘、窗口生命周期与 IPC 命令注册。
mod actions;
mod commands;

use actions::{trigger, trigger_pin, trigger_recording};
use crate::{capture, clipboard, diagnostics, pins, resources, settings};
use crate::recognition::{ocr, qr, translation, local_translation, translation_models};
use crate::settings::{autostart, shortcut_probe, storage};
use crate::resources::lifecycle as resource_lifecycle;
use crate::shared::{auxiliary_windows, native_i18n};
use capture::{show_main, CaptureState};
use std::sync::atomic::Ordering;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, WebviewWindowBuilder,
};
use tauri_plugin_global_shortcut::ShortcutState;

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
        .manage(auxiliary_windows::AuxiliaryWindows::default())
        .manage(capture::MainWindowState::default())
        .manage(settings::ShortcutPressState::default())
        .manage(shortcut_probe::ProbeState::default())
        .manage(qr::QrState::default())
        .manage(ocr::OcrState::default())
        .manage(resource_lifecycle::ResourceLifecycle::default())
        .manage(translation::TranslationState::default())
        .manage(local_translation::LocalTranslationState::default())
        .manage(capture::recording::RecordingState::default())
        .manage(capture::recording::live_annotations::LiveAnnotationState::default())
        .manage(capture::recording::result::ResultState::default())
        .manage(capture::recording::ExitState::default())
        .manage(pins::PinState::default())
        // 截图图片仅允许截图 WebView 读取；异步协议避免大响应复制占用 UI 线程。
        .register_asynchronous_uri_scheme_protocol("capture-image", |context, request, responder| {
            let app = context.app_handle().clone();
            let allowed = context.webview_label() == "capture";
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(capture::preview::serve(&app.state::<CaptureState>(), allowed, request));
            });
        })
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Debug)
                .filter(diagnostics::runtime::enabled)
                // 默认追加写入；保留 stdout 与 LogDir 两个目标，约 30 MB 总预算。
                .max_file_size(5_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                // 依赖库可能输出模型路径或网络正文，仅持久化已审计的应用事件。
                .filter(|metadata| metadata.target().starts_with("snap_desk_lib")
                    || metadata.target() == tauri_plugin_log::WEBVIEW_TARGET
                    || metadata.target().starts_with("webview::"))
                .format(|out, message, record| {
                    // JS 绑定自动将调用位置附在 target；不能把 URL/源码路径带进日志。
                    let target = if record.target().starts_with("webview") {
                        "frontend"
                    } else { record.target() };
                    out.finish(format_args!("{} [{}] [{}] {}",
                        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
                        record.level(), target, message));
                })
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
                            // 八秒测试复用此注册回调，只记录配置内的快捷键，不执行功能动作。
                            if shortcut_probe::receive(app, shortcut.to_string()) {
                                return None;
                            }
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
            diagnostics::runtime::install_panic_hook();
            log::info!("event=app.starting module=app version={} platform={}", app.package_info().version, std::env::consts::OS);
            settings::initialize(app.handle()).map_err(|error| {
                log::error!("event=app.initialize_failed module=app code=SETTINGS_INIT_FAILED");
                error
            })?;
            clipboard::initialize(app.handle());
            resource_lifecycle::initialize(app.handle());
            let capture = MenuItem::with_id(
                app,
                "capture",
                native_i18n::text(app.handle(), "开始截图"),
                true,
                None::<&str>,
            )?;
            let pin = MenuItem::with_id(
                app,
                "pin",
                native_i18n::text(app.handle(), "贴图（选区 / 剪贴板）"),
                true,
                None::<&str>,
            )?;
            let recording = MenuItem::with_id(
                app,
                "recording",
                native_i18n::text(app.handle(), "录屏"),
                true,
                None::<&str>,
            )?;
            let history = MenuItem::with_id(
                app,
                "history",
                native_i18n::text(app.handle(), "剪贴板历史"),
                true,
                None::<&str>,
            )?;
            let open = MenuItem::with_id(
                app,
                "open",
                native_i18n::text(app.handle(), "打开 Snap Desk"),
                true,
                None::<&str>,
            )?;
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
                native_i18n::text(app.handle(), "启用全局快捷键"),
                true,
                enabled,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(
                app,
                "quit",
                native_i18n::text(app.handle(), "退出"),
                true,
                None::<&str>,
            )?;
            let menu = Menu::with_items(
                app,
                &[
                    &capture, &pin, &recording, &history, &open, &shortcuts, &quit,
                ],
            )?;
            app.manage(settings::ShortcutMenu(shortcuts));
            let tray = TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../../icons/32x32.png"
                ))?)
                .tooltip(native_i18n::text(app.handle(), "Snap Desk · 截图工具"))
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
            app.manage(native_i18n::TrayLabels {
                items: vec![
                    ("开始截图", capture),
                    ("贴图（选区 / 剪贴板）", pin),
                    ("录屏", recording),
                    ("剪贴板历史", history),
                    ("打开 Snap Desk", open),
                    ("退出", quit),
                ],
                icon: tray,
            });
            // 配置中的主窗口禁用自动创建：Tauri 自动创建窗口早于本 setup，
            // 页面可能提前调用 app_status，读取尚未 manage 的 SettingsState 而 panic。
            // 等设置、快捷键和托盘全部就绪后，再沿用原窗口配置加载主界面。
            auxiliary_windows::initialize(app.handle())?;
            let main_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|config| config.label == "main")
                .ok_or("缺少主窗口配置。")?;
            // 首次加载也保持隐藏，收到前端就绪通知后再显示，避免冷启动白屏。
            let automatic = autostart::is_autostart(std::env::args_os().skip(1));
            app.state::<capture::MainWindowState>()
                .show_requested.store(!automatic, Ordering::SeqCst);
            WebviewWindowBuilder::from_config(app.handle(), main_config)?
                .visible(false)
                .focused(false)
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭截图/贴图时仅取消本窗口的文字任务；退出应用仍由统一出口取消全部任务。
            if (window.label() == "capture" || window.label().starts_with("pin-"))
                && matches!(event, tauri::WindowEvent::Destroyed | tauri::WindowEvent::CloseRequested { .. })
            {
                translation::cancel_window(window.app_handle(), window.label());
            }
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
                    if let Err(_error) = capture::scroll::chrome::restore_edge(&webview) {
                        let _ = webview.hide();
                        log::error!("event=app.edge_restore_failed module=app code=EDGE_RESTORE_FAILED");
                    }
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "recording-controls"
                    || window.label().starts_with("recording-edge-")
                {
                    // 录屏范围与控制条不能被 Alt+F4 关闭，只在停止/取消完成后统一收起。
                    api.prevent_close();
                    if window.label() == "recording-controls" {
                        let app = window.app_handle().clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            capture::recording::close_completed_controls(&app);
                        });
                    }
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
            capture::main_window_ready,
            resources::resource_status,
            resources::download_resources,
            resources::choose_export_font,
            capture::recognize_capture_text,
            capture::recognize_capture_v2,
            translation_models::translation_model_status,
            translation_models::translation_model_action,
            translation::translate_text,
            translation::cancel_ai_job,
            translation::ai_credential_set,
            translation::ai_provider_status,
            translation::ai_translation_config,
            translation::ai_provider_test,
            ocr::copy_ocr_text,
            commands::app_status,
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
            commands::select_recording_area,
            diagnostics::diagnostics_status,
            diagnostics::diagnostics_recent_logs,
            diagnostics::diagnostics_export,
            diagnostics::diagnostics_clear_archives,
            diagnostics::diagnostics_set_debug,
            storage::app_directories,
            storage::open_app_directory,
            storage::copy_app_directory,
            storage::export_settings,
            storage::copy_shortcut_example,
            settings::complete_onboarding,
            commands::hide_main,
            commands::open_project_homepage,
            settings::save_settings,
            settings::set_shortcuts_enabled,
            settings::set_shortcut_editing,
            shortcut_probe::probe_shortcuts,
            capture::cancel_pending_capture,
            capture::start_capture,
            capture::current_capture,
            capture::capture_native_window,
            capture::show_capture,
            capture::latency::capture_painted,
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
            capture::recording::recovery::discard_recording_recovery,
            capture::recording::recording_capabilities,
            capture::recording::output::current_recording_directory,
            capture::recording::output::choose_recording_directory,
            capture::recording::control_recording,
            capture::recording::live_annotations::current_recording_annotations,
            capture::recording::live_annotations::set_recording_annotation_mode,
            capture::recording::live_annotations::set_recording_annotation_regions,
            capture::recording::live_annotations::set_recording_annotation_tools,
            capture::recording::live_annotations::recording_annotation_editor_action,
            capture::recording::live_annotations::finish_recording_annotation_editor_action,
            capture::recording::live_annotations::publish_recording_annotation_editor,
            capture::recording::live_annotations::update_recording_annotations,
            pins::pin_clipboard,
            pins::pin_file,
            pins::set_pin_group,
            pins::current_pin,
            pins::recognize_pin_text,
            pins::recognize_pin_qr,
            pins::pin_qr_action,
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
                log::info!("event=app.exiting module=app");
                resource_lifecycle::shutdown(app);
                translation::cancel_all(app);
                local_translation::shutdown(app);
                app.state::<clipboard::Service>().shutdown();
            }
        });
}
