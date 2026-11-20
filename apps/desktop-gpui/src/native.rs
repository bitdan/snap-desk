//! GPUI 没有提供的窗口能力：原生句柄、点击穿透、切换置顶。目前只实现 Windows，
//! 其他平台返回“不支持”，由调用方记录日志（macOS/Linux 的实现随多平台验证补上）。
use gpui_kit::Window;

/// 当前窗口的 HWND（作为 Windows 剪贴板所有者等）；其他平台返回 0。
pub fn hwnd(window: &Window) -> isize {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window) {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                return handle.hwnd.get();
            }
        }
    }
    let _ = window;
    0
}

/// 点击穿透：加上分层 + 透明扩展样式后，鼠标事件落到下面的窗口；DirectComposition 渲染不受影响。
/// 透明度保持 255，显示透明度由界面自己的 opacity 决定。
pub fn set_click_through(window: &Window, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: hwnd 来自仍然存活的 GPUI 窗口，只修改扩展样式位并刷新框架。
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let bits = (WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if enabled { style | bits } else { style & !bits });
            if enabled {
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (window, enabled);
        Err("当前平台暂不支持鼠标穿透。".into())
    }
}

/// 切换置顶。PopUp 窗口创建时即为置顶；取消后变为普通层级，仍不出现在任务栏。
pub fn set_topmost(window: &Window, topmost: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        let after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
        // SAFETY: 同上；只改 Z 序，不移动、不改尺寸、不抢焦点。
        let ok = unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        if ok == 0 {
            return Err("无法切换窗口置顶。".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (window, topmost);
        Err("当前平台暂不支持切换置顶。".into())
    }
}
