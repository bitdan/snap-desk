//! 全局翻译键先在原应用执行复制，确认剪贴板更新后再读取，避免使用历史文字。
use tauri::AppHandle;

/// HWND 转为纯数值跨线程传递；执行复制时重新核对前台，窗口切换即停止。
#[cfg(windows)]
pub(super) fn foreground() -> Result<usize, String> {
    let window = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if window.0.is_null() {
        return Err("没有可复制文字的前台窗口。".into());
    }
    Ok(window.0 as usize)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(super) fn foreground() -> Result<usize, String> {
    Err("当前平台暂不支持自动复制选中文字，请在翻译页读取剪贴板。".into())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(super) fn copy(_app: &AppHandle, _target: usize) -> Result<String, String> {
    Err("当前平台暂不支持自动复制选中文字，请在翻译页读取剪贴板。".into())
}

/// macOS 以前台应用进程号代替 HWND；全局快捷键不会激活本程序，前台仍是用户选中文字的应用。
#[cfg(target_os = "macos")]
pub(super) fn foreground() -> Result<usize, String> {
    match crate::shared::macos::frontmost_pid() {
        Some(pid) if pid as u32 != std::process::id() => Ok(pid as usize),
        _ => Err("没有可复制文字的前台窗口。".into()),
    }
}

/// 与 Windows 流程相同：等快捷键全部松开，注入 ⌘C，确认剪贴板计数变化后再读文字。
/// 注入按键需要“辅助功能”授权，未授权时直接提示而不是静默失败。
#[cfg(target_os = "macos")]
pub(super) fn copy(app: &AppHandle, target: usize) -> Result<String, String> {
    use crate::shared::macos;
    use std::time::{Duration, Instant};
    use tauri_plugin_clipboard_manager::ClipboardExt;
    if !macos::accessibility_trusted() {
        return Err("自动复制选中文字需要辅助功能权限，请在“设置 → 通用 → 系统权限”中授权。".into());
    }
    let same_app = || macos::frontmost_pid() == Some(target as i32);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if !same_app() {
            return Err("前台窗口已切换，请重新选中文字。".into());
        }
        // ⌥T 等快捷键仍按着时注入 ⌘C 会变成 ⌘⌥C，必须等修饰键与 C 全部松开。
        if macos::modifier_flags() == 0 && !macos::key_down(macos::KEY_C) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("请松开快捷键后重新尝试翻译。".into());
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let sequence = macos::pasteboard_change_count();
    macos::post_command_key(macos::KEY_C)
        .map_err(|_| "无法自动复制选中文字，请检查辅助功能权限。".to_string())?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if !same_app() {
            return Err("前台窗口已切换，请重新选中文字。".into());
        }
        if macos::pasteboard_change_count() != sequence {
            if let Ok(text) = app.clipboard().read_text() {
                return Ok(text);
            }
        }
        if Instant::now() >= deadline {
            return Err("未复制到选中文字，请确认已选择文字且目标应用支持复制。".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 等 Alt 等修饰键释放后发送 Ctrl+C；成功更新前不展示主窗口，以保留原应用选区。
#[cfg(windows)]
pub(super) fn copy(app: &AppHandle, target: usize) -> Result<String, String> {
    use std::time::{Duration, Instant};
    use tauri_plugin_clipboard_manager::ClipboardExt;
    use windows::Win32::{
        System::DataExchange::GetClipboardSequenceNumber,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::GetForegroundWindow},
    };
    let same_window = || unsafe { GetForegroundWindow().0 as usize == target };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if !same_window() {
            return Err("前台窗口已切换，请重新选中文字。".into());
        }
        let held = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN, VK_C]
            .iter()
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 });
        if !held {
            break;
        }
        if Instant::now() >= deadline {
            return Err("请松开快捷键后重新尝试翻译。".into());
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let key = |code, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [
        key(VK_CONTROL, KEYBD_EVENT_FLAGS::default()),
        key(VK_C, KEYBD_EVENT_FLAGS::default()),
        key(VK_C, KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        if sent > 0 {
            // 部分注入也必须释放已按下的 Ctrl/C，避免影响用户后续输入。
            unsafe {
                SendInput(&inputs[2..], std::mem::size_of::<INPUT>() as i32);
            }
        }
        return Err("无法自动复制选中文字，请检查目标应用权限。".into());
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if !same_window() {
            return Err("前台窗口已切换，请重新选中文字。".into());
        }
        if unsafe { GetClipboardSequenceNumber() } != sequence {
            // 应用可能先更新序号再完成延迟渲染；剪贴板短暂占用时继续等待。
            if let Ok(text) = app.clipboard().read_text() {
                return Ok(text);
            }
        }
        if Instant::now() >= deadline {
            return Err("未复制到选中文字，请确认已选择文字且目标应用支持复制。".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
