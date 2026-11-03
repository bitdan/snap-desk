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

#[cfg(not(windows))]
pub(super) fn foreground() -> Result<usize, String> {
    Err("当前平台暂不支持自动复制选中文字，请在翻译页读取剪贴板。".into())
}

#[cfg(not(windows))]
pub(super) fn copy(_app: &AppHandle, _target: usize) -> Result<String, String> {
    Err("当前平台暂不支持自动复制选中文字，请在翻译页读取剪贴板。".into())
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
