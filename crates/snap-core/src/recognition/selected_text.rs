//! 全局翻译键先在原应用执行复制，确认剪贴板更新后再读取，避免使用历史文字。
//! X11 例外：直接读取 PRIMARY 选区（用户当前选中的文字），不经过剪贴板。
//! 读取剪贴板文字由调用方提供（GPUI 用 [`clipboard_text`]）。

/// 翻译原文的上限与空白检查（按 UTF-8 字节计，保留首尾空白）。
pub fn validate(text: String) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("没有可翻译的文字，请先选中或复制文字。".into());
    }
    if text.len() > 32_000 {
        return Err("翻译文字超过 32 KB，请减少文字后重试。".into());
    }
    Ok(text)
}

/// 读取系统剪贴板中的文字（经 core 有界读取，只取文本）。
pub fn clipboard_text() -> Result<String, String> {
    let mut settings = crate::clipboard::model::ClipboardSettings::default();
    settings.save_copied_images = false;
    settings.save_copied_files = false;
    settings.save_copied_text = true;
    match crate::clipboard::reader::read(&settings)? {
        Some(crate::clipboard::service::Content::Text(text)) => Ok(text),
        _ => Err("剪贴板中没有可读取的文字。".into()),
    }
}

/// HWND 转为纯数值跨线程传递；执行复制时重新核对前台，窗口切换即停止。
#[cfg(windows)]
pub fn foreground() -> Result<usize, String> {
    let window = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if window.0.is_null() {
        return Err("没有可复制文字的前台窗口。".into());
    }
    Ok(window.0 as usize)
}

/// X11 以 EWMH 前台窗口号代替 HWND；全局快捷键不会激活本程序，前台仍是用户选中文字的窗口。
#[cfg(not(any(windows, target_os = "macos")))]
pub fn foreground() -> Result<usize, String> {
    use crate::x11::X11;
    use xcb::Xid;
    // Wayland 读不到前台窗口；PRIMARY 经 Xwayland 同步，无需前台窗口号。
    if crate::x11::wayland() {
        return Ok(0);
    }
    let x = X11::connect()?;
    let window = x.active_window().ok_or("没有可复制文字的前台窗口。")?;
    if x.window_pid(window) == Some(std::process::id()) {
        return Err("没有可复制文字的前台窗口。".into());
    }
    Ok(window.resource_id() as usize)
}

/// X11 选中文字即进入 PRIMARY 选区，直接向选区所有者请求 UTF-8 文本，
/// 不注入 Ctrl+C，也不覆盖用户的剪贴板。终端等程序里 Ctrl+C 是中断键，注入反而有害。
#[cfg(not(any(windows, target_os = "macos")))]
pub fn copy(
    target: usize,
    _read_text: impl Fn() -> Result<String, String>,
) -> Result<String, String> {
    use crate::x11::{ReadError, SelectionReader, X11};
    use xcb::{x, Xid};
    /// 超过翻译上限的选区不必整段取回，留出余量让上层给出“超过 32 KB”的提示。
    const LIMIT: usize = 64 * 1024;
    let x11 = X11::connect_selections()?;
    if !crate::x11::wayland()
        && x11.active_window().map(|w| w.resource_id() as usize) != Some(target)
    {
        return Err("前台窗口已切换，请重新选中文字。".into());
    }
    let reader = SelectionReader::new(&x11)?;
    if reader.owner(x::ATOM_PRIMARY).is_none() {
        return Err("未读取到选中文字，请先选择文字。".into());
    }
    let mut last = None;
    for name in ["UTF8_STRING", "text/plain;charset=utf-8", "STRING"] {
        let format = x11.intern(name)?;
        match reader.read(x::ATOM_PRIMARY, format, LIMIT) {
            Ok(Some(bytes)) if !bytes.is_empty() => {
                // STRING 按规范是 Latin-1；其余目标均为 UTF-8。
                return Ok(if name == "STRING" {
                    bytes.iter().map(|b| char::from(*b)).collect()
                } else {
                    String::from_utf8_lossy(&bytes).into_owned()
                });
            }
            Ok(_) => {}
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(ReadError::TooLarge) => "翻译文字超过 32 KB，请减少文字后重试。".into(),
        Some(ReadError::Timeout) => "目标应用未响应选区请求，请重新选中文字。".into(),
        _ => "未读取到选中文字，请确认已选择文字。".into(),
    })
}

/// macOS 以前台应用进程号代替 HWND；全局快捷键不会激活本程序，前台仍是用户选中文字的应用。
#[cfg(target_os = "macos")]
pub fn foreground() -> Result<usize, String> {
    match crate::macos::frontmost_pid() {
        Some(pid) if pid as u32 != std::process::id() => Ok(pid as usize),
        _ => Err("没有可复制文字的前台窗口。".into()),
    }
}

/// 与 Windows 流程相同：等快捷键全部松开，注入 ⌘C，确认剪贴板计数变化后再读文字。
/// 注入按键需要“辅助功能”授权，未授权时直接提示而不是静默失败。
#[cfg(target_os = "macos")]
pub fn copy(
    target: usize,
    read_text: impl Fn() -> Result<String, String>,
) -> Result<String, String> {
    use crate::macos;
    use std::time::{Duration, Instant};
    if !macos::accessibility_trusted() {
        return Err(
            "自动复制选中文字需要辅助功能权限，请在“设置 → 通用 → 系统权限”中授权。".into(),
        );
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
            if let Ok(text) = read_text() {
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
pub fn copy(
    target: usize,
    read_text: impl Fn() -> Result<String, String>,
) -> Result<String, String> {
    use std::time::{Duration, Instant};
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
    // 有选区时应用通常在 100 ms 内写入剪贴板；没有选区时剪贴板不会变化，只能等到超时，
    // 超时直接决定空白翻译窗口出现前的停顿，因此控制在 500 ms。
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        if !same_window() {
            return Err("前台窗口已切换，请重新选中文字。".into());
        }
        if unsafe { GetClipboardSequenceNumber() } != sequence {
            // 应用可能先更新序号再完成延迟渲染；剪贴板短暂占用时继续等待。
            if let Ok(text) = read_text() {
                return Ok(text);
            }
        }
        if Instant::now() >= deadline {
            return Err("未复制到选中文字，请确认已选择文字且目标应用支持复制。".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    #[test]
    fn translation_input_is_bounded_in_utf8_bytes_and_preserves_whitespace() {
        assert!(validate(
            " 
	"
            .into()
        )
        .is_err());
        assert!(validate("中".repeat(11_000)).is_err());
        assert!(validate("a".repeat(32_000)).is_ok());
        assert_eq!(
            validate(
                "  hello
world  "
                    .into()
            )
            .unwrap(),
            "  hello
world  "
        );
    }
}
