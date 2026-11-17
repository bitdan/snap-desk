//! macOS 拖出：在窗口内容视图上发起 NSDraggingSession。
//!
//! 拖拽请求经前端转发后才到达，此时 `NSApp.currentEvent` 未必还是鼠标事件，因此按当前鼠标位置
//! 合成一个左键拖动事件作为会话起点（系统只用它确定起点与所在窗口）。会话异步进行，
//! 调用在发起后立即返回；拖拽源对象全局只建一个，只允许“复制”操作。
use super::Payload;
use crate::shared::macos::{ns_array, ns_string, Id};
use objc2::{
    class,
    encode::{Encode, Encoding},
    msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, Sel},
    sel,
};
use std::sync::OnceLock;
use tauri::WebviewWindow;

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}
unsafe impl Encode for NSPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}
unsafe impl Encode for NSSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}
unsafe impl Encode for NSRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
}

/// NSEventTypeLeftMouseDragged。
const LEFT_MOUSE_DRAGGED: usize = 6;
/// NSDragOperationCopy：不让访达等接收方移动或删除贴图的临时文件与历史文件。
const DRAG_OPERATION_COPY: usize = 1;
/// 拖拽图标边长（点）；多个文件依次错开，最多叠放 5 层。
const ICON: f64 = 48.0;

/// NSDraggingSource 唯一的必需方法：无论拖到应用内还是应用外都只允许复制。
extern "C-unwind" fn operation_mask(_: &AnyObject, _: Sel, _: Id, _: isize) -> usize {
    DRAG_OPERATION_COPY
}

/// 注册拖拽源类并创建一个常驻实例；只在主线程调用，因此无需额外同步。
unsafe fn source() -> Result<Id, String> {
    static SOURCE: OnceLock<usize> = OnceLock::new();
    if let Some(source) = SOURCE.get() {
        return Ok(*source as Id);
    }
    let class = match AnyClass::get(c"SnapDeskDragSource") {
        Some(class) => class,
        None => {
            let mut builder = ClassBuilder::new(c"SnapDeskDragSource", class!(NSObject))
                .ok_or("无法注册拖拽源。")?;
            builder.add_method(
                sel!(draggingSession:sourceOperationMaskForDraggingContext:),
                operation_mask as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            if let Some(protocol) = AnyProtocol::get(c"NSDraggingSource") {
                builder.add_protocol(protocol);
            }
            builder.register()
        }
    };
    let instance: Option<Retained<AnyObject>> = msg_send![class, new];
    // 会话不保证持有拖拽源，故有意常驻到进程退出。
    let pointer = Retained::into_raw(instance.ok_or("无法创建拖拽源。")?) as usize;
    Ok(*SOURCE.get_or_init(|| pointer) as Id)
}

/// 以鼠标为中心放置拖拽图标；`writer` 是遵循 NSPasteboardWriting 的 NSURL 或 NSString。
unsafe fn item(writer: Id, icon: Id, at: NSPoint, index: usize) -> Option<Retained<AnyObject>> {
    if writer.is_null() {
        return None;
    }
    let allocated: Allocated<AnyObject> = msg_send![class!(NSDraggingItem), alloc];
    let item: Option<Retained<AnyObject>> = msg_send![allocated, initWithPasteboardWriter: writer];
    let item = item?;
    let offset = index.min(4) as f64 * 6.0;
    let frame = NSRect {
        origin: NSPoint {
            x: at.x - ICON / 2.0 + offset,
            y: at.y - ICON / 2.0 - offset,
        },
        size: NSSize {
            width: ICON,
            height: ICON,
        },
    };
    let _: () = msg_send![&*item, setDraggingFrame: frame, contents: icon];
    Some(item)
}

unsafe fn begin(window: Id, payload: &Payload) -> Result<(), String> {
    // 大图编码期间用户可能已松手，此时不再发起，避免晚到的请求在别处意外放下。
    let buttons: usize = msg_send![class!(NSEvent), pressedMouseButtons];
    if buttons & 1 == 0 {
        return Ok(());
    }
    let view: Id = msg_send![window, contentView];
    if view.is_null() {
        return Err("窗口尚未就绪，无法开始拖拽。".into());
    }
    let none: Id = std::ptr::null_mut();
    let in_window: NSPoint = msg_send![window, mouseLocationOutsideOfEventStream];
    let at: NSPoint = msg_send![view, convertPoint: in_window, fromView: none];
    let number: isize = msg_send![window, windowNumber];
    let process: Id = msg_send![class!(NSProcessInfo), processInfo];
    let uptime: f64 = msg_send![process, systemUptime];
    let event: Id = msg_send![
        class!(NSEvent),
        mouseEventWithType: LEFT_MOUSE_DRAGGED,
        location: in_window,
        modifierFlags: 0usize,
        timestamp: uptime,
        windowNumber: number,
        context: none,
        eventNumber: 0isize,
        clickCount: 1isize,
        pressure: 1.0f32
    ];
    if event.is_null() {
        return Err("系统未能开始拖拽。".into());
    }
    let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
    let items: Vec<Retained<AnyObject>> = match payload {
        Payload::Text(text) => {
            let icon: Id = msg_send![workspace, iconForFileType: ns_string("txt")];
            item(ns_string(text), icon, at, 0).into_iter().collect()
        }
        Payload::Files(paths) => paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let path = ns_string(path);
                let url: Id = msg_send![class!(NSURL), fileURLWithPath: path];
                let icon: Id = msg_send![workspace, iconForFile: path];
                item(url, icon, at, index)
            })
            .collect(),
    };
    if items.is_empty() {
        return Err("没有可拖出的内容。".into());
    }
    let pointers: Vec<Id> = items
        .iter()
        .map(|item| Retained::as_ptr(item) as Id)
        .collect();
    let session: Id = msg_send![
        view,
        beginDraggingSessionWithItems: ns_array(&pointers),
        event: event,
        source: source()?
    ];
    if session.is_null() {
        return Err("系统未能开始拖拽。".into());
    }
    Ok(())
}

/// 可从任意线程调用；NSWindow 与拖拽会话只在主线程访问。
pub(super) fn start(window: &WebviewWindow, payload: Payload) -> Result<(), String> {
    let target = window.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let result = match target.ns_window() {
                Ok(pointer) if !pointer.is_null() => objc2::rc::autoreleasepool(|_| {
                    // SAFETY: 指针来自 Tauri 管理的存活 NSWindow，且当前位于主线程。
                    unsafe { begin(pointer as Id, &payload) }
                }),
                _ => Err("窗口尚未就绪，无法开始拖拽。".into()),
            };
            let _ = sender.send(result);
        })
        .map_err(super::err)?;
    receiver.recv().map_err(super::err)?
}
