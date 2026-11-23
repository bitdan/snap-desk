//! macOS 原生设施：光标位置、截图蒙层窗口层级、隐私权限、前台应用、键盘注入与剪贴板计数。
//!
//! 坐标单位均为“点”（point），原点在主屏左上角、向下为正，与 CGDisplayBounds 及 xcap
//! 的显示器坐标一致；乘以各屏 backing scale 才是 Retina 像素。这里只声明少量系统函数，
//! 不引入额外框架绑定，所有 NSWindow 调用都必须在主线程执行。
use objc2::{
    class,
    encode::Encode,
    msg_send,
    runtime::{AnyObject, Bool},
};
use std::ffi::{c_char, c_void, CStr, CString};
use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use block2::RcBlock;
use objc2::rc::Retained;

/// 自动释放对象的裸指针；只在 `objc2::rc::autoreleasepool` 闭包内使用，离开池即失效。
pub type Id = *mut AnyObject;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
unsafe impl objc2::encode::Encode for CGPoint {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "CGPoint",
        &[f64::ENCODING, f64::ENCODING],
    );
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
unsafe impl objc2::encode::Encode for CGSize {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "CGSize",
        &[f64::ENCODING, f64::ENCODING],
    );
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
unsafe impl objc2::encode::Encode for CGRect {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "CGRect",
        &[CGPoint::ENCODING, CGSize::ENCODING],
    );
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
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
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

/// 把截图蒙层提升到菜单栏之上并允许覆盖全屏应用。窗口框架自带的置顶通常只到浮动层（3），
/// 盖不住菜单栏；每次显示前重施，避免其他置顶调用把层级改回去。
///
/// # Safety
/// `ns_window` 必须是存活的 NSWindow 指针，且只能在 Cocoa 主线程调用。
/// 窗口指针由 raw-window-handle 取得，须在主线程调用。
pub unsafe fn raise_ns_window(ns_window: *mut c_void) {
    if ns_window.is_null() {
        return;
    }
    let ns_window = &*(ns_window as *mut AnyObject);
    let _: () = msg_send![ns_window, setLevel: OVERLAY_LEVEL];
    let _: () = msg_send![ns_window, setCollectionBehavior: OVERLAY_BEHAVIOR];
}

extern "C" {
    fn pthread_main_np() -> i32;
}

/// 当前线程是否为 Cocoa 主线程。
pub fn is_main_thread() -> bool {
    unsafe { pthread_main_np() != 0 }
}

/// 同步设置窗口外框，并在同一次调用中读回 NSWindow 的实际 frame。
///
/// `requested` 为左上角原点、向下为正的全局物理像素 `(x, y, width, height)`；
/// `scale` 为目标屏的 backing scale（须为正有限数，由调用方校验）。返回值同样是物理像素。
/// 框架的 set_size / set_position 常经 GCD 主队列异步执行，不能用来做“设置后立即校验”。
///
/// # Safety
/// `ns_window` 必须是存活且非空的 NSWindow 指针，且只能在 Cocoa 主线程调用。
pub unsafe fn set_ns_window_frame(
    ns_window: *mut c_void,
    requested: (i32, i32, u32, u32),
    scale: f64,
) -> (i32, i32, u32, u32) {
    let ns_window = &*(ns_window as *mut AnyObject);
    // Cocoa 全局坐标以主屏左下角为原点、向上为正。
    let primary = CGDisplayBounds(CGMainDisplayID()).size.height;
    let (x, y, width, height) = requested;
    let frame = cocoa_frame(requested, scale, primary);
    let _: () = msg_send![ns_window, setFrame: frame, display: Bool::YES];
    let actual: CGRect = msg_send![ns_window, frame];
    let result = top_left_pixels(actual, scale, primary);
    if result != (x, y, width, height) {
        log::warn!(
            "event=capture.frame_mismatch module=capture code=CAPTURE_FRAME_MISMATCH width={width} height={height} actual_width={} actual_height={} offset_x={} offset_y={}",
            result.2, result.3, (result.0 - x).unsigned_abs(), (result.1 - y).unsigned_abs()
        );
    }
    result
}

fn cocoa_frame(requested: (i32, i32, u32, u32), scale: f64, primary: f64) -> CGRect {
    let (x, y, width, height) = requested;
    let width = f64::from(width) / scale;
    let height = f64::from(height) / scale;
    CGRect {
        origin: CGPoint {
            x: f64::from(x) / scale,
            y: primary - f64::from(y) / scale - height,
        },
        size: CGSize { width, height },
    }
}

fn top_left_pixels(frame: CGRect, scale: f64, primary: f64) -> (i32, i32, u32, u32) {
    let top = primary - frame.origin.y - frame.size.height;
    (
        (frame.origin.x * scale).round() as i32,
        (top * scale).round() as i32,
        (frame.size.width * scale).round().max(0.0) as u32,
        (frame.size.height * scale).round().max(0.0) as u32,
    )
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
    crate::platform::open_web_page(match pane {
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

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *const c_void;
    fn TISGetInputSourceProperty(source: *const c_void, key: *const c_void) -> *const c_void;
    static kTISPropertyInputSourceID: *const c_void;
    fn IsSecureEventInputEnabled() -> u8;
}

/// 当前键盘输入源的标识，如 `com.apple.keylayout.ABC`、`com.apple.inputmethod.SCIM.ITABC`。
/// 文字输入服务要求在主线程调用。
pub fn input_source_id() -> Option<String> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return None;
        }
        // 属性值归输入源所有，在释放输入源之前复制出来；CFString 与 NSString 可直接互换。
        let id = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
        let text = rust_string(id as Id);
        CFRelease(source);
        text
    })
}

/// 是否有应用开启了“安全输入”（密码框、终端的“安全键盘输入”等）；开启期间系统会限制键盘事件的分发。
pub fn secure_input_enabled() -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
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

/// CoreMedia 时间值。录屏流水线统一使用 100 ns 刻度；系统采集回调的时间戳与
/// [`host_ticks`] 同属主机时钟（mach_absolute_time），可直接相减。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}
// CMTime 是匿名 typedef 结构体，运行时类型编码为 {?=qiIq}。
unsafe impl objc2::encode::Encode for CMTime {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "?",
        &[
            <i64 as objc2::encode::Encode>::ENCODING,
            <i32 as objc2::encode::Encode>::ENCODING,
            <u32 as objc2::encode::Encode>::ENCODING,
            <i64 as objc2::encode::Encode>::ENCODING,
        ],
    );
}
impl CMTime {
    /// 100 ns 刻度的时间（kCMTimeFlags_Valid = 1）。
    pub fn from_ticks(value: i64) -> Self {
        Self {
            value,
            timescale: 10_000_000,
            flags: 1,
            epoch: 0,
        }
    }
    /// 转为 100 ns 刻度；无效或时间刻度非正时返回 None。
    pub fn ticks(self) -> Option<i64> {
        (self.flags & 1 != 0 && self.timescale > 0).then(|| {
            (i128::from(self.value) * 10_000_000 / i128::from(self.timescale)) as i64
        })
    }
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMClockGetHostTimeClock() -> *const c_void;
    fn CMClockGetTime(clock: *const c_void) -> CMTime;
}

/// 主机时钟的当前时间（100 ns）；视频帧与音频包都以它为时间轴零点之外的共同参照。
pub fn host_ticks() -> i64 {
    unsafe { CMClockGetTime(CMClockGetHostTimeClock()) }
        .ticks()
        .unwrap_or(0)
}

/// CoreAudio 的 AudioStreamBasicDescription。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StreamDescription {
    pub sample_rate: f64,
    pub format_id: u32,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
    pub reserved: u32,
}

// ---- 系统异步接口（ScreenCaptureKit、AVFoundation）共用设施 ----

extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> *mut c_void;
}

/// NSError 的本地化描述；空指针返回“未知错误”。
pub unsafe fn error_text(error: Id) -> String {
    if error.is_null() {
        return "未知错误".into();
    }
    let text: Id = msg_send![error, localizedDescription];
    rust_string(text).unwrap_or_else(|| "未知错误".into())
}

/// 创建串行 GCD 队列作为系统回调队列。
pub unsafe fn dispatch_queue(label: &std::ffi::CStr) -> Result<Retained<AnyObject>, String> {
    // GCD 队列是 Objective-C 对象，交给 Retained 管理生命周期。
    Retained::from_raw(dispatch_queue_create(label.as_ptr(), std::ptr::null()) as Id)
        .ok_or_else(|| "无法创建系统回调队列。".into())
}

/// 只带一个 NSError 参数的系统完成回调及其结果通道。
pub fn completion() -> (RcBlock<dyn Fn(Id)>, Receiver<Option<String>>) {
    let (sender, receiver) = mpsc::sync_channel::<Option<String>>(1);
    let block = RcBlock::new(move |error: Id| {
        let _ = sender.try_send((!error.is_null()).then(|| unsafe { error_text(error) }));
    });
    (block, receiver)
}

pub fn completed(
    receiver: Receiver<Option<String>>,
    timeout: Duration,
    what: &str,
) -> Result<(), String> {
    match receiver.recv_timeout(timeout) {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(format!("{what}：{error}")),
        Err(_) => Err(format!("{what}：系统响应超时。")),
    }
}

/// 跨线程传回 Objective-C 对象：只在接收方线程内使用，传递期间不访问。
pub struct Handoff(pub Retained<AnyObject>);
unsafe impl Send for Handoff {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_conversion_round_trips_retina_pixels() {
        for scale in [1.0, 2.0, 3.0] {
            for requested in [(0, 0, 2, 2), (120, 80, 2228, 4), (-1728, 40, 4, 1300), (3456, 0, 640, 96)] {
                let frame = cocoa_frame(requested, scale, 1117.0);
                assert_eq!(top_left_pixels(frame, scale, 1117.0), requested);
            }
        }
    }

    #[test]
    fn frame_origin_is_bottom_left_of_primary_screen() {
        let frame = cocoa_frame((200, 100, 400, 60), 2.0, 900.0);
        assert_eq!(frame.origin.x, 100.0);
        assert_eq!(frame.size.height, 30.0);
        // 顶边 50pt，主屏高 900pt，底边位于 900-50-30。
        assert_eq!(frame.origin.y, 820.0);
    }
}

/// 从 NSView 取所在 NSWindow（GPUI 的 raw-window-handle 只给出 NSView）。
///
/// # Safety
/// `ns_view` 必须是存活的 NSView 指针，且只能在 Cocoa 主线程调用。
pub unsafe fn ns_window_of_view(ns_view: *mut c_void) -> *mut c_void {
    if ns_view.is_null() {
        return std::ptr::null_mut();
    }
    let view = &*(ns_view as *mut AnyObject);
    let window: Id = msg_send![view, window];
    window as *mut c_void
}

/// 鼠标穿透：忽略鼠标事件后点击落到下面的窗口。
///
/// # Safety
/// `ns_window` 必须是存活且非空的 NSWindow 指针，且只能在 Cocoa 主线程调用。
pub unsafe fn set_ns_window_ignores_mouse(ns_window: *mut c_void, ignore: bool) {
    let window = &*(ns_window as *mut AnyObject);
    let _: () = msg_send![window, setIgnoresMouseEvents: Bool::new(ignore)];
}

/// 设置窗口层级（0 普通、3 浮动、101 弹出菜单层）。
///
/// # Safety
/// 同 [`set_ns_window_ignores_mouse`]。
pub unsafe fn set_ns_window_level(ns_window: *mut c_void, level: isize) {
    let window = &*(ns_window as *mut AnyObject);
    let _: () = msg_send![window, setLevel: level];
}

/// 隐藏（orderOut）或不抢焦点地重新显示（orderFrontRegardless）。
///
/// # Safety
/// 同 [`set_ns_window_ignores_mouse`]。
pub unsafe fn set_ns_window_visible(ns_window: *mut c_void, visible: bool) {
    let window = &*(ns_window as *mut AnyObject);
    if visible {
        let _: () = msg_send![window, orderFrontRegardless];
    } else {
        let nil: Id = std::ptr::null_mut();
        let _: () = msg_send![window, orderOut: nil];
    }
}
