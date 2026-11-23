//! GPUI 没有提供的窗口能力：原生句柄、点击穿透、切换置顶、临时隐藏。
//! Windows 用 Win32；macOS 经 NSView 取 NSWindow（`snap_core::macos`）；Linux 只支持 X11
//! （`snap_core::x11`：XFixes 输入形状、EWMH 置顶、映射/取消映射），Wayland 会话返回“不支持”。
use gpui_kit::Window;

/// macOS：窗口的 NSWindow 指针；取不到时为空。
#[cfg(target_os = "macos")]
fn ns_window(window: &Window) -> *mut std::ffi::c_void {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        // SAFETY: 句柄来自存活的 GPUI 窗口，且 GPUI 事件处理在 Cocoa 主线程上运行。
        Ok(RawWindowHandle::AppKit(handle)) => unsafe {
            snap_core::macos::ns_window_of_view(handle.ns_view.as_ptr())
        },
        _ => std::ptr::null_mut(),
    }
}

/// Linux：X11 窗口 id；Wayland 会话返回 None。
#[cfg(target_os = "linux")]
fn x11_window(window: &Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
        _ => None,
    }
}

/// macOS GPUI PopUp 窗口的层级（NSPopUpWindowLevel）；取消置顶时降为普通层级。
#[cfg(target_os = "macos")]
const POPUP_LEVEL: isize = 101;

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

/// 按住鼠标拖动窗口。gpui-pre 在 Windows 上的 `start_window_move` 是空实现，这里改为释放鼠标捕获后
/// 投递“按下标题栏”消息，交给系统的移动循环（投递而非同步发送，避免在 GPUI 回调里重入窗口过程）。
pub fn start_move(window: &Window) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{
            Input::KeyboardAndMouse::ReleaseCapture,
            WindowsAndMessaging::{PostMessageW, HTCAPTION, WM_NCLBUTTONDOWN},
        };
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if !hwnd.is_null() {
            // SAFETY: hwnd 来自存活窗口；消息参数为常量。
            unsafe {
                ReleaseCapture();
                PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
            }
        }
    }
    #[cfg(not(windows))]
    window.start_window_move();
}

/// 把窗口移到全局逻辑坐标 `at`（按窗口缩放换算成物理像素），不改尺寸与层级、不激活。
pub fn move_to(window: &Window, at: gpui_kit::Point<gpui_kit::Pixels>) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return false;
        }
        let scale = window.scale_factor();
        let (x, y) = ((f32::from(at.x) * scale).round() as i32, (f32::from(at.y) * scale).round() as i32);
        // 同线程调用会同步重入窗口过程（WM_MOVE），在 GPUI 回调里触发 App 重复借用；从辅助线程带
        // SWP_ASYNCWINDOWPOS 调用，移动请求进入窗口所属线程的消息队列，由消息循环正常处理。
        let raw = hwnd as isize;
        std::thread::spawn(move || {
            // SAFETY: 句柄失效时 SetWindowPos 只返回失败，不会访问无效内存。
            unsafe {
                SetWindowPos(raw as _, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
            }
        });
        true
    }
    #[cfg(not(windows))]
    {
        let _ = (window, at);
        false
    }
}

/// 去掉系统给无边框弹出窗口画的 1px 边框（关闭 DWM 非客户区渲染），透明窗口只显示自己画的内容。
pub fn frameless(window: &Window) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY};
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return;
        }
        let policy = DWMNCRP_DISABLED;
        // SAFETY: hwnd 来自存活窗口；属性值指向栈上的整数，长度与类型一致。
        unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY as u32, &policy as *const _ as *const _, std::mem::size_of_val(&policy) as u32);
        }
    }
    #[cfg(not(windows))]
    let _ = window;
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
    #[cfg(target_os = "macos")]
    {
        let ns_window = ns_window(window);
        if ns_window.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: 指针来自存活窗口，在主线程调用。
        unsafe { snap_core::macos::set_ns_window_ignores_mouse(ns_window, enabled) };
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let id = x11_window(window).ok_or("Wayland 会话暂不支持鼠标穿透。")?;
        snap_core::x11::window_passthrough(id, enabled)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
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
    #[cfg(target_os = "macos")]
    {
        let ns_window = ns_window(window);
        if ns_window.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: 指针来自存活窗口，在主线程调用。
        unsafe { snap_core::macos::set_ns_window_level(ns_window, if topmost { POPUP_LEVEL } else { 0 }) };
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let id = x11_window(window).ok_or("Wayland 会话暂不支持切换置顶。")?;
        snap_core::x11::window_above(id, topmost)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (window, topmost);
        Err("当前平台暂不支持切换置顶。".into())
    }
}

/// 无边框的工具窗口（剪贴板历史、翻译）：置顶且不出现在任务栏。
/// 以普通窗口创建以保留可调整大小的边框，再改成工具窗口样式；Windows 需重新显示一次任务栏才会移除按钮。
pub fn utility_window(window: &Window) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: hwnd 来自存活窗口；只改扩展样式与 Z 序，显示切换以异步消息投递，避免窗口过程重入。
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let next = (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
            if next != style {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
                ShowWindowAsync(hwnd, SW_HIDE);
                ShowWindowAsync(hwnd, SW_SHOW);
            }
        }
    }
    set_topmost(window, true)
}

/// 录屏辅助窗口（红框、控制条）不进入屏幕采集画面：Windows 10 2004+ 的 WDA_EXCLUDEFROMCAPTURE。
/// 其他平台暂未实现（macOS 需在 ScreenCaptureKit 过滤器中排除窗口）。
pub fn exclude_from_capture(window: &Window) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: hwnd 来自存活窗口，只修改其显示亲和性。
        if unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } == 0 {
            return Err(format!("无法排除录屏辅助窗口：{}", std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("当前平台暂不支持排除录屏辅助窗口。".into())
    }
}

/// 临时隐藏或不抢焦点地重新显示窗口（保存/打开对话框期间隐藏置顶贴图）。
pub fn set_visible(window: &Window, visible: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: hwnd 来自存活窗口；重新显示时不激活，避免抢走对话框关闭后的焦点。
        unsafe { ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let ns_window = ns_window(window);
        if ns_window.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: 指针来自存活窗口，在主线程调用。
        unsafe { snap_core::macos::set_ns_window_visible(ns_window, visible) };
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let id = x11_window(window).ok_or("Wayland 会话暂不支持隐藏窗口。")?;
        snap_core::x11::window_mapped(id, visible)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (window, visible);
        Err("当前平台暂不支持隐藏窗口。".into())
    }
}

/// 与 [`set_visible`] 相同，但在 Windows 上投递消息（ShowWindowAsync）而不是同步发送：
/// 在 GPUI 回调里（应用状态已借用）同步显示/隐藏会让窗口过程重入并丢帧。
pub fn set_visible_async(window: &Window, visible: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = hwnd(window) as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return Err("窗口原生句柄不可用。".into());
        }
        // SAFETY: hwnd 来自存活窗口；重新显示时不激活。
        unsafe { ShowWindowAsync(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        Ok(())
    }
    #[cfg(not(windows))]
    {
        set_visible(window, visible)
    }
}

/// 以“隐藏/最小化”方式启动（快捷方式“运行方式: 最小化”、计划任务、`Start-Process -WindowStyle Hidden`）时，
/// 系统会把 STARTUPINFO 的显示方式套用到进程第一次以普通方式显示的窗口上，导致首个截图浮层不可见。
/// 启动时用一个屏幕外 1×1 的工具窗口先消耗掉这个标志。
#[cfg(windows)]
pub fn consume_startup_show_state() {
    use windows_sys::Win32::{
        System::Threading::{GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW},
        UI::WindowsAndMessaging::*,
    };
    unsafe {
        let mut info: STARTUPINFOW = std::mem::zeroed();
        info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        GetStartupInfoW(&mut info);
        if info.dwFlags & STARTF_USESHOWWINDOW == 0 {
            return;
        }
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            std::ptr::null(),
            WS_OVERLAPPED,
            -32000,
            -32000,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if hwnd.is_null() {
            log::warn!("event=gpui.startup_show_state_failed");
            return;
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        DestroyWindow(hwnd);
        log::info!("event=gpui.startup_show_state_consumed show={}", info.wShowWindow);
    }
}

#[cfg(not(windows))]
pub fn consume_startup_show_state() {}

/// 鼠标位置，换算为 GPUI 的全局逻辑坐标（与 `PlatformDisplay::bounds` 同一坐标系）；取不到时为 None。
/// Windows：GPUI 的显示器边界是“物理像素 ÷ 该屏缩放”，按光标所在显示器的 DPI 换算。
/// macOS：CGEvent 的全局点坐标即左上角原点的逻辑点。Linux 暂不支持（窗口居中显示）。
pub fn cursor_point() -> Option<gpui_kit::Point<gpui_kit::Pixels>> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST},
            UI::{
                HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
                WindowsAndMessaging::GetCursorPos,
            },
        };
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: 输出参数指向栈上有效的 POINT；显示器句柄由系统返回，只用于查询 DPI。
        unsafe {
            if GetCursorPos(&mut point) == 0 {
                return None;
            }
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) != 0 {
                dpi_x = 96;
            }
            let scale = dpi_x.max(1) as f32 / 96.0;
            Some(gpui_kit::point(gpui_kit::px(point.x as f32 / scale), gpui_kit::px(point.y as f32 / scale)))
        }
    }
    #[cfg(target_os = "macos")]
    {
        snap_core::macos::cursor_point().map(|(x, y)| gpui_kit::point(gpui_kit::px(x as f32), gpui_kit::px(y as f32)))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}
