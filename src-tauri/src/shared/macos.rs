//! macOS 原生设施：光标位置、截图蒙层窗口层级、隐私权限、前台应用、键盘注入与剪贴板计数。
//!
//! 坐标单位均为“点”（point），原点在主屏左上角、向下为正，与 CGDisplayBounds 及 xcap
//! 的显示器坐标一致；乘以各屏 backing scale 才是 Retina 像素。这里只声明少量系统函数，
//! 不引入额外框架绑定，所有 NSWindow 调用都必须在主线程执行。
use objc2::{
    class, msg_send,
    runtime::{AnyObject, Bool},
};
use std::ffi::{c_char, c_void, CStr, CString};
use tauri::WebviewWindow;

/// 自动释放对象的裸指针；只在 `objc2::rc::autoreleasepool` 闭包内使用，离开池即失效。
pub(crate) type Id = *mut AnyObject;

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
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
    static kCGWindowLayer: *const c_void;
    static kCGWindowNumber: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: isize, value: *mut c_void) -> bool;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    /// kIOHIDRequestTypeListenEvent = 1：只读监听键鼠（录屏键鼠特效）所需的“输入监控”授权。
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
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

/// kIOHIDAccessTypeGranted = 0。
pub fn input_monitoring_granted() -> bool {
    unsafe { IOHIDCheckAccess(1) == 0 }
}

/// 首次调用弹出系统授权框并把应用加入“输入监控”列表。
pub fn request_input_monitoring() -> bool {
    unsafe { IOHIDRequestAccess(1) }
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

/// 系统设置“隐私与安全性”中的权限面板。
#[derive(Clone, Copy)]
pub enum PrivacyPane {
    ScreenCapture,
    Accessibility,
    InputMonitoring,
}

/// 打开“系统设置 → 隐私与安全性”的对应面板；地址固定，不接受前端传入。
pub fn open_privacy_pane(pane: PrivacyPane) -> Result<(), String> {
    crate::shared::platform::open_web_page(match pane {
        PrivacyPane::ScreenCapture => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        PrivacyPane::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        PrivacyPane::InputMonitoring => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
    })
}

/// 虚拟键码（与键盘布局无关的物理位置，ANSI 布局命名）。
pub const KEY_C: u16 = 8;
pub const KEY_V: u16 = 9;
pub const KEY_RETURN: u16 = 36;
pub const KEY_ESCAPE: u16 = 53;

/// kCGEventSourceStateCombinedSessionState：本会话内所有输入源的合并状态。
const SESSION_STATE: i32 = 0;
pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CONTROL: u64 = 0x0004_0000;
pub const FLAG_OPTION: u64 = 0x0008_0000;
pub const FLAG_COMMAND: u64 = 0x0010_0000;

/// 当前按住的修饰键位（只取 ⇧⌃⌥⌘ 四位）。读取按键状态不消费事件。
pub fn modifier_flags() -> u64 {
    let flags = unsafe { CGEventSourceFlagsState(SESSION_STATE) };
    flags & (FLAG_SHIFT | FLAG_CONTROL | FLAG_OPTION | FLAG_COMMAND)
}

pub fn key_down(key: u16) -> bool {
    unsafe { CGEventSourceKeyState(SESSION_STATE, key) }
}

/// 向当前前台应用注入一次 ⌘+键（按下、抬起）。系统要求“辅助功能”授权，
/// 未授权时事件会被静默丢弃，因此先检查并返回可读错误，而不是假装成功。
pub fn post_command_key(key: u16) -> Result<(), String> {
    if !accessibility_trusted() {
        return Err("需要在“设置 → 通用 → 系统权限”中授予辅助功能权限，内容已复制，可手动粘贴。".into());
    }
    unsafe {
        for down in [true, false] {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), key, down);
            if event.is_null() {
                return Err("无法创建键盘事件。".into());
            }
            CGEventSetFlags(event, FLAG_COMMAND);
            // kCGHIDEventTap：按硬件输入的位置注入，前台应用与真实按键无法区分。
            CGEventPost(0, event);
            CFRelease(event);
        }
    }
    Ok(())
}

/// 前台应用的进程号；读取失败返回 None。
pub fn frontmost_pid() -> Option<i32> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: Id = msg_send![workspace, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let pid: i32 = msg_send![app, processIdentifier];
        (pid > 0).then_some(pid)
    })
}

/// 前台应用的可执行文件名（与 Windows 的进程名对应），用于剪贴板排除名单。
pub fn frontmost_executable() -> Option<String> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: Id = msg_send![workspace, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let url: Id = msg_send![app, executableURL];
        if url.is_null() {
            return None;
        }
        let name: Id = msg_send![url, lastPathComponent];
        rust_string(name)
    })
}

/// 把指定进程带回前台；进程已退出时返回 false。
pub fn activate_pid(pid: i32) -> bool {
    objc2::rc::autoreleasepool(|_| unsafe {
        let app: Id = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid
        ];
        if app.is_null() {
            return false;
        }
        // NSApplicationActivateIgnoringOtherApps：本程序仍在前台时允许把焦点让给目标应用。
        let activated: Bool = msg_send![app, activateWithOptions: 2usize];
        activated.as_bool()
    })
}

/// 系统剪贴板的变更计数；任何应用写入都会递增，作用等同 Windows 剪贴板序列号。
pub fn pasteboard_change_count() -> u32 {
    objc2::rc::autoreleasepool(|_| unsafe {
        let pasteboard: Id = msg_send![class!(NSPasteboard), generalPasteboard];
        let count: isize = msg_send![pasteboard, changeCount];
        count as u32
    })
}

/// 创建自动释放的 NSString；含 NUL 的输入按空串处理。调用方须处在自动释放池内。
pub unsafe fn ns_string(value: &str) -> Id {
    let value = CString::new(value).unwrap_or_default();
    msg_send![class!(NSString), stringWithUTF8String: value.as_ptr()]
}

/// 复制 NSString 内容；空指针返回 None。
pub unsafe fn rust_string(value: Id) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let text: *const c_char = msg_send![value, UTF8String];
    (!text.is_null()).then(|| CStr::from_ptr(text).to_string_lossy().into_owned())
}

/// 由若干对象构造自动释放的 NSArray。
pub unsafe fn ns_array(items: &[Id]) -> Id {
    msg_send![class!(NSArray), arrayWithObjects: items.as_ptr(), count: items.len()]
}

/// 遍历 NSArray；空指针视为空数组。
pub unsafe fn array_items(array: Id) -> Vec<Id> {
    if array.is_null() {
        return Vec::new();
    }
    let count: usize = msg_send![array, count];
    (0..count)
        .map(|index| {
            let item: Id = msg_send![array, objectAtIndex: index];
            item
        })
        .collect()
}

/// 复制 NSData 的字节；超过 limit 时返回 Err 且不复制。
pub unsafe fn data_bytes(data: Id, limit: usize) -> Result<Option<Vec<u8>>, ()> {
    if data.is_null() {
        return Ok(None);
    }
    let length: usize = msg_send![data, length];
    if length > limit {
        return Err(());
    }
    let bytes: *const c_void = msg_send![data, bytes];
    if bytes.is_null() || length == 0 {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(
        std::slice::from_raw_parts(bytes as *const u8, length).to_vec(),
    ))
}

/// 屏幕上普通应用窗口（层级 0）的窗口号。菜单栏、Dock、状态栏图标、通知等位于更高层级，
/// 不参与截图吸附；按层级而非应用名过滤，避免系统语言改变所有者名称后失效。
pub fn normal_window_ids() -> std::collections::HashSet<u32> {
    // kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements，与 xcap 枚举一致。
    const OPTIONS: u32 = 1 | 16;
    // kCFNumberSInt32Type
    const SINT32: isize = 3;
    let mut ids = std::collections::HashSet::new();
    unsafe {
        let list = CGWindowListCopyWindowInfo(OPTIONS, 0);
        if list.is_null() {
            return ids;
        }
        for index in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, index);
            let number = |key: *const c_void| -> Option<i32> {
                let value = CFDictionaryGetValue(info, key);
                let mut out = 0i32;
                (!value.is_null()
                    && CFNumberGetValue(value, SINT32, &mut out as *mut i32 as *mut c_void))
                .then_some(out)
            };
            if number(kCGWindowLayer) == Some(0) {
                if let Some(id) = number(kCGWindowNumber) {
                    ids.insert(id as u32);
                }
            }
        }
        // Copy 规则：返回的数组由调用方释放。
        CFRelease(list);
    }
    ids
}
