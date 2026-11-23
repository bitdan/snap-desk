#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Snap Desk 的 GPUI 版入口（迁移中，正式发布仍是 `src-tauri`）。
//!
//! 当前能力：单实例、日志、托盘菜单、读取与 Tauri 版相同的 `settings.json` 并注册全局快捷键，
//! 截图浮层（框选、吸附、复制、保存、贴图）、贴图窗口与录屏控制条；动作入口统一经 `shell::run_action`。
//!
//! 命令行与 Tauri 版一致：`--action <名称>`、`--action=<名称>` 或 `--<名称>`（如 `--capture`）。
//! 已有实例运行时，参数经单实例通道转交给它，本进程随即退出。
mod annotate;
mod capture;
mod clipboard;
mod clipboard_export;
mod clipboard_preview;
mod countdown;
mod drag_out;
mod logging;
mod magnifier;
mod native;
mod pins;
mod recognition;
mod live_annotate;
mod local_translate;
mod recording;
mod scroll;
mod settings_window;
mod selection;
mod shell;
mod shelf;
mod theme;
mod translation_window;
mod single_instance;
mod updater;

use futures::channel::mpsc;
use gpui_kit::*;

fn main() {
    let log_path = logging::init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // 托盘“重启”拉起的新进程：等旧进程退出、释放单实例锁后再继续。
    if let Some(index) = args.iter().position(|a| a.starts_with(shell::RELAUNCH_ARG)) {
        let arg = args.remove(index);
        if let Ok(pid) = arg[shell::RELAUNCH_ARG.len()..].parse::<u32>() {
            wait_for_exit(pid);
        }
    }
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
        updater::initialize(cx);
        // Linux 桌面能力探测含 D-Bus 往返，启动时后台预热，设置页读取时不必等待。
        #[cfg(target_os = "linux")]
        snap_core::desktop_support::warm_up();
        // 首次启动带的动作参数与转交参数走同一入口。
        if let Some(action) = snap_core::settings::action_from_args(&args) {
            shell::run_action(cx, action, "cli");
        }
    });
    log::info!("event=gpui.exit");
}

/// 等指定进程退出（最多 15 秒）；进程不存在时立即返回。
#[cfg(windows)]
fn wait_for_exit(pid: u32) {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            WaitForSingleObject(handle, 15_000);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
fn wait_for_exit(pid: u32) {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(15)
        && std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
