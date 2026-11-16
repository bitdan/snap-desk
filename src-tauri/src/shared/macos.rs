//! macOS 原生设施：光标位置、截图蒙层窗口层级与隐私权限查询。
//!
//! 坐标单位均为“点”（point），原点在主屏左上角、向下为正，与 CGDisplayBounds 及 xcap
//! 的显示器坐标一致；乘以各屏 backing scale 才是 Retina 像素。这里只声明少量系统函数，
//! 不引入额外框架绑定，所有 NSWindow 调用都必须在主线程执行。
use std::ffi::c_void;
use tauri::WebviewWindow;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    /// 只读当前授权；结果在进程生命周期内缓存，用户授权后需重启应用才会变为 true。
    fn CGPreflightScreenCaptureAccess() -> bool;
    /// 首次调用弹出系统授权框并把应用加入“屏幕录制”列表；已拒绝过则不再弹窗。
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// 辅助功能授权：窗口元素吸附、直接粘贴、划词翻译等后续能力依赖它。
    fn AXIsProcessTrusted() -> bool;
}

/// 当前鼠标所在的全局点坐标；用空事件读取位置，不需要辅助功能或输入监控权限。
pub fn cursor_point() -> Option<(f64, f64)> {
    // CGEventCreate 返回 +1 引用计数的对象，读取后必须 CFRelease。
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

pub fn screen_capture_granted() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub fn request_screen_capture() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// NSStatusWindowLevel：高于菜单栏（24）和 Dock（20），低于系统弹窗与屏保层，
/// 让全屏截图蒙层盖住菜单栏而不挡住系统授权对话框。
const OVERLAY_LEVEL: isize = 25;
/// canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary：
/// 出现在当前桌面空间（含其他应用的全屏空间），不参与 Mission Control 动画和 ⌘` 切换。
const OVERLAY_BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);

/// 把截图蒙层提升到菜单栏之上并允许覆盖全屏应用。Tauri 的置顶只到浮动层（3），
/// 盖不住菜单栏；每次显示前重施，避免其他置顶调用把层级改回去。
pub fn raise_overlay(window: &WebviewWindow) -> Result<(), String> {
    let target = window.clone();
    // ns_window 指针只在主线程有效，因此在主线程闭包内读取并发送消息。
    window
        .run_on_main_thread(move || {
            let Ok(pointer) = target.ns_window() else {
                log::warn!("event=capture.overlay_level_failed module=capture");
                return;
            };
            if pointer.is_null() {
                return;
            }
            // SAFETY: 指针来自 Tauri 管理的存活 NSWindow，且当前位于主线程。
            unsafe {
                let ns_window = &*(pointer as *mut objc2::runtime::AnyObject);
                let _: () = objc2::msg_send![ns_window, setLevel: OVERLAY_LEVEL];
                let _: () = objc2::msg_send![ns_window, setCollectionBehavior: OVERLAY_BEHAVIOR];
            }
        })
        .map_err(|e| e.to_string())
}

/// 打开“系统设置 → 隐私与安全性”的对应面板；地址固定，不接受前端传入。
pub fn open_privacy_pane(screen_capture: bool) -> Result<(), String> {
    crate::shared::platform::open_web_page(if screen_capture {
        "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
    } else {
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
    })
}
