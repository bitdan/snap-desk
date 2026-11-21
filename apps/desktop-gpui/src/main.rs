#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Snap Desk 的 GPUI 版入口（迁移中，正式发布仍是 `src-tauri`）。
//!
//! 当前能力：单实例、日志、托盘菜单、读取与 Tauri 版相同的 `settings.json` 并注册全局快捷键，
//! 截图浮层（框选、吸附、复制、保存、贴图）与贴图窗口。录屏等窗口在后续步骤接入 `shell::run_action`。
//!
//! 命令行与 Tauri 版一致：`--action <名称>`、`--action=<名称>` 或 `--<名称>`（如 `--capture`）。
//! 已有实例运行时，参数经单实例通道转交给它，本进程随即退出。
mod annotate;
mod capture;
mod clipboard;
mod clipboard_export;
mod countdown;
mod logging;
mod magnifier;
mod native;
mod pin_manager;
mod pins;
mod recognition;
mod selection;
mod shell;
mod single_instance;

use futures::channel::mpsc;
use gpui_kit::*;

fn main() {
    let log_path = logging::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let listener = match single_instance::acquire(&args) {
        single_instance::Instance::Primary(listener) => listener,
        single_instance::Instance::Secondary => {
            log::info!("event=gpui.forwarded_to_primary");
            return;
        }
    };
    log::info!(
        "event=gpui.start version={} log={}",
        env!("CARGO_PKG_VERSION"),
        log_path.map(|p| p.display().to_string()).unwrap_or_default()
    );

    native::consume_startup_show_state();
    let (sender, receiver) = mpsc::unbounded();
    let forward = sender.clone();
    listener.start(move |args| {
        let _ = forward.unbounded_send(shell::ShellEvent::Forwarded(args));
    });

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        // 只有托盘、没有窗口时也不退出；退出只走托盘菜单。
        cx.set_quit_mode(QuitMode::Explicit);
        if let Err(error) = shell::start(cx, sender, receiver) {
            log::error!("event=gpui.shell_failed message={error}");
            cx.quit();
            return;
        }
        // 首次启动带的动作参数与转交参数走同一入口。
        if let Some(action) = snap_core::settings::action_from_args(&args) {
            shell::run_action(cx, action, "cli");
        }
    });
    log::info!("event=gpui.exit");
}
