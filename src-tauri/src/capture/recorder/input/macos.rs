//! macOS 键鼠特效：只读事件监听（CGEventTap ListenOnly），不拦截、不修改任何输入。
//!
//! 系统要求“输入监控”授权才能创建监听；未授权时返回明确错误，提示到系统权限页授权。
//! 位置从全局点坐标换算为各屏物理像素（点 × 该屏缩放），与录屏来源原点一致。
//! 键盘只记录可展示的命令键与功能键，不记录普通文字。
use super::{super::capture, Clicks, EventBuffer, InputEvent, InputKind};
use super::super::config::InputEffects;
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

type TapCallback =
    extern "C" fn(proxy: *mut c_void, kind: u32, event: *mut c_void, info: *mut c_void) -> *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        callback: TapCallback,
        info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventGetFlags(event: *mut c_void) -> u64;
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFMachPortCreateRunLoopSource(allocator: *const c_void, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFMachPortInvalidate(port: *mut c_void);
    fn CFRelease(object: *const c_void);
}

// CGEventType
const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const MOVED: u32 = 5;
const LEFT_DRAGGED: u32 = 6;
const RIGHT_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const SCROLL: u32 = 22;
/// 系统因回调超时或用户操作停用监听时发送，需要重新启用。
const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;

// CGEventField
const MOUSE_CLICK_STATE: u32 = 1;
const KEYBOARD_AUTOREPEAT: u32 = 8;
const KEYBOARD_KEYCODE: u32 = 9;
const SCROLL_DELTA_AXIS1: u32 = 11;

const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_OPTION: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

/// 显示器边界（点）及缩放，用于把全局点坐标换算为物理像素。
#[derive(Clone, Copy)]
struct Screen {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

fn screens() -> Vec<Screen> {
    xcap::Monitor::all()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            Some(Screen {
                x: f64::from(m.x().ok()?),
                y: f64::from(m.y().ok()?),
                width: f64::from(m.width().ok()?),
                height: f64::from(m.height().ok()?),
                scale: f64::from(m.scale_factor().ok()?),
            })
        })
        .collect()
}

/// 回调线程独占的状态；事件缓冲由录制线程定期取走。
struct Tap {
    config: InputEffects,
    screens: Vec<Screen>,
    events: Arc<Mutex<EventBuffer>>,
    clicks: Clicks,
    left: bool,
    right: bool,
    port: *mut c_void,
}

impl Tap {
    fn pixel(&self, point: CGPoint) -> (i32, i32) {
        let scale = self
            .screens
            .iter()
            .find(|s| {
                point.x >= s.x && point.x < s.x + s.width && point.y >= s.y && point.y < s.y + s.height
            })
            .map_or(1.0, |s| s.scale);
        ((point.x * scale).round() as i32, (point.y * scale).round() as i32)
    }

    fn handle(&mut self, kind: u32, event: *mut c_void) {
        let source_time = capture::now();
        let mut output = Vec::new();
        unsafe {
            match kind {
                LEFT_DOWN | RIGHT_DOWN => {
                    let right = kind == RIGHT_DOWN;
                    if right {
                        self.right = true;
                    } else {
                        self.left = true;
                    }
                    if self.config.clicks {
                        let (x, y) = self.pixel(CGEventGetLocation(event));
                        // 系统给出的连击计数已按用户的双击速度判定；Clicks 只负责去重三连击。
                        let system_double = CGEventGetIntegerValueField(event, MOUSE_CLICK_STATE) >= 2;
                        let double = self.clicks.press(source_time, x, y, right, (i64::MAX, i32::MAX, i32::MAX))
                            && system_double;
                        output.push(InputKind::Click { x, y, right, double });
                    }
                }
                LEFT_UP => self.left = false,
                RIGHT_UP => self.right = false,
                MOVED | LEFT_DRAGGED | RIGHT_DRAGGED => {
                    if self.config.movement || self.config.drag {
                        let (x, y) = self.pixel(CGEventGetLocation(event));
                        output.push(InputKind::Move {
                            x,
                            y,
                            dragging: self.left || self.right,
                        });
                    }
                }
                SCROLL => {
                    if self.config.wheel {
                        let (x, y) = self.pixel(CGEventGetLocation(event));
                        // 以行为单位；乘 120 与 Windows 的滚轮刻度一致，正值表示向上。
                        let lines = CGEventGetIntegerValueField(event, SCROLL_DELTA_AXIS1);
                        if lines != 0 {
                            output.push(InputKind::Wheel {
                                x,
                                y,
                                delta: (lines.clamp(-1000, 1000) as i32) * 120,
                            });
                        }
                    }
                }
                KEY_DOWN => {
                    if self.config.keyboard
                        && CGEventGetIntegerValueField(event, KEYBOARD_AUTOREPEAT) == 0
                    {
                        let code = CGEventGetIntegerValueField(event, KEYBOARD_KEYCODE) as u16;
                        if let Some(label) = key_label(code, CGEventGetFlags(event)) {
                            output.push(InputKind::Key(label));
                        }
                    }
                }
                TAP_DISABLED_TIMEOUT | TAP_DISABLED_USER => CGEventTapEnable(self.port, true),
                _ => {}
            }
        }
        if !output.is_empty() {
            if let Ok(mut buffer) = self.events.lock() {
                for kind in output {
                    buffer.push(InputEvent { source_time, kind });
                }
            }
        }
    }
}

/// 物理键码（ANSI 布局）转展示名称；与 Windows 一致，只允许常用命令键和功能键。
fn key_label(code: u16, flags: u64) -> Option<String> {
    let (ctrl, option, shift, command) = (
        flags & FLAG_CONTROL != 0,
        flags & FLAG_OPTION != 0,
        flags & FLAG_SHIFT != 0,
        flags & FLAG_COMMAND != 0,
    );
    let name = match code {
        51 => "BACKSPACE".into(),
        48 => "TAB".into(),
        36 | 76 => "ENTER".into(),
        53 => "ESC".into(),
        116 => "PAGE UP".into(),
        121 => "PAGE DOWN".into(),
        119 => "END".into(),
        115 => "HOME".into(),
        123 => "LEFT".into(),
        126 => "UP".into(),
        124 => "RIGHT".into(),
        125 => "DOWN".into(),
        117 => "DELETE".into(),
        122 => "F1".into(),
        120 => "F2".into(),
        99 => "F3".into(),
        118 => "F4".into(),
        96 => "F5".into(),
        97 => "F6".into(),
        98 => "F7".into(),
        100 => "F8".into(),
        101 => "F9".into(),
        109 => "F10".into(),
        103 => "F11".into(),
        111 => "F12".into(),
        // 只在按住 ⌘ 时记录字母，避免把普通输入当快捷键；⌥ 单独组合多用于输入特殊字符，不记录。
        _ if command => match code {
            0 => "A".into(),
            8 => "C".into(),
            3 => "F".into(),
            45 => "N".into(),
            35 => "P".into(),
            12 => "Q".into(),
            1 => "S".into(),
            17 => "T".into(),
            9 => "V".into(),
            13 => "W".into(),
            7 => "X".into(),
            16 => "Y".into(),
            6 => "Z".into(),
            _ => return None,
        },
        _ => return None,
    };
    let mut label = String::new();
    for (active, prefix) in [(ctrl, "CTRL+"), (option, "OPT+"), (shift, "SHIFT+"), (command, "CMD+")] {
        if active {
            label.push_str(prefix);
        }
    }
    label.push_str(&name);
    Some(label)
}

extern "C" fn callback(
    _proxy: *mut c_void,
    kind: u32,
    event: *mut c_void,
    info: *mut c_void,
) -> *mut c_void {
    if !info.is_null() {
        // SAFETY: info 指向监听线程栈上的 Tap，线程在移除监听前不会退出；回调只在该线程的运行循环中执行。
        let tap = unsafe { &mut *(info as *mut Tap) };
        tap.handle(kind, event);
    }
    // 只读监听：原样返回事件。
    event
}

/// 监听随录制线程生命周期；Drop 停止运行循环并等待线程释放监听端口。
pub struct Listener {
    events: Arc<Mutex<EventBuffer>>,
    run_loop: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start(config: InputEffects) -> Result<Self, String> {
        let events = Arc::new(Mutex::new(EventBuffer::default()));
        let run_loop = Arc::new(AtomicUsize::new(0));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (thread_events, thread_loop) = (events.clone(), run_loop.clone());
        let worker = std::thread::Builder::new()
            .name("input-effects".into())
            .spawn(move || unsafe {
                let mut tap = Tap {
                    config,
                    screens: screens(),
                    events: thread_events,
                    clicks: Clicks::default(),
                    left: false,
                    right: false,
                    port: std::ptr::null_mut(),
                };
                let mask: u64 = [
                    LEFT_DOWN, LEFT_UP, RIGHT_DOWN, RIGHT_UP, MOVED, LEFT_DRAGGED, RIGHT_DRAGGED,
                    KEY_DOWN, SCROLL,
                ]
                .iter()
                .fold(0, |mask, kind| mask | (1u64 << kind));
                // kCGSessionEventTap = 1，kCGHeadInsertEventTap = 0，kCGEventTapOptionListenOnly = 1。
                let port = CGEventTapCreate(1, 0, 1, mask, callback, &mut tap as *mut Tap as *mut c_void);
                if port.is_null() {
                    let _ = ready_tx.send(Err(
                        "录屏键鼠特效需要“输入监控”权限，请在“设置 → 通用 → 系统权限”中授权后重启应用。"
                            .to_string(),
                    ));
                    return;
                }
                tap.port = port;
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
                let current = CFRunLoopGetCurrent();
                CFRunLoopAddSource(current, source, kCFRunLoopCommonModes);
                CGEventTapEnable(port, true);
                thread_loop.store(current as usize, Ordering::Release);
                let _ = ready_tx.send(Ok(()));
                // 阻塞到 Drop 调用 CFRunLoopStop。
                CFRunLoopRun();
                CGEventTapEnable(port, false);
                CFMachPortInvalidate(port);
                CFRelease(source);
                CFRelease(port);
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                events,
                run_loop,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err("键鼠特效监听线程初始化失败。".into())
            }
        }
    }

    pub fn drain(&self) -> Result<Vec<InputEvent>, String> {
        Ok(self
            .events
            .lock()
            .map_err(|_| "键鼠事件缓冲不可用。")?
            .drain())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let run_loop = self.run_loop.load(Ordering::Acquire);
        if run_loop != 0 {
            // CFRunLoopStop 可跨线程调用，令监听线程的 CFRunLoopRun 返回。
            unsafe { CFRunLoopStop(run_loop as *mut c_void) };
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut events) = self.events.lock() {
            events.drain();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_command_shortcuts_and_function_keys_are_labeled() {
        assert_eq!(key_label(8, FLAG_COMMAND).as_deref(), Some("CMD+C"));
        assert_eq!(key_label(1, FLAG_COMMAND | FLAG_SHIFT).as_deref(), Some("SHIFT+CMD+S"));
        assert_eq!(key_label(8, 0), None);
        assert_eq!(key_label(8, FLAG_OPTION), None);
        assert_eq!(key_label(122, 0).as_deref(), Some("F1"));
        assert_eq!(key_label(53, FLAG_CONTROL).as_deref(), Some("CTRL+ESC"));
    }
}
