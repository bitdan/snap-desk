//! 隐藏消息窗口接收后台 Raw Input；不安装全局钩子，不屏蔽前台应用输入。
//! WM_INPUT 读取后交给 DefWindowProc 清理。所有 HWND 操作及注销均在所属线程完成。
use super::super::{capture, config::InputEffects};
use super::{Clicks, EventBuffer, InputEvent, InputKind, Keys};
use std::{
    mem::size_of,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
};
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WAIT_FAILED, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            Input::{KeyboardAndMouse::GetDoubleClickTime, *},
            WindowsAndMessaging::*,
        },
    },
};

fn native_error(error: windows::core::Error) -> String {
    format!("键鼠监听失败：{error}")
}

// WNDPROC 要求 Windows 系统 ABI；windows crate 的普通 Rust 包装函数不能直接充当回调。
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, message, wparam, lparam)
}

/// Drop 发送停止信号并等待消息线程释放设备、窗口及内存，不遗留后台监听。
pub struct Listener {
    stop: Arc<AtomicBool>,
    events: Arc<Mutex<EventBuffer>>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}
impl Listener {
    pub fn start(config: InputEffects) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(EventBuffer::default()));
        let error = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (thread_stop, thread_events, thread_error) =
            (stop.clone(), events.clone(), error.clone());
        let worker = std::thread::Builder::new()
            .name("recording-raw-input".into())
            .spawn(move || {
                let window = match InputWindow::new(config) {
                    Ok(window) => window,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                if let Err(error) = pump(&window, config, &thread_stop, &thread_events) {
                    if let Ok(mut value) = thread_error.lock() {
                        *value = Some(error);
                    }
                }
                // window 随线程结束在本线程注销，随后 Listener::drop 清空事件。
            })
            .map_err(|e| e.to_string())?;
        let listener = Self {
            stop,
            events,
            error,
            worker: Some(worker),
        };
        ready_rx
            .recv()
            .map_err(|_| "键鼠监听线程初始化失败。".to_string())??;
        Ok(listener)
    }

    pub fn drain(&self) -> Result<Vec<InputEvent>, String> {
        if let Some(error) = self
            .error
            .lock()
            .map_err(|_| "键鼠监听状态不可用。")?
            .as_ref()
        {
            return Err(error.clone());
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err("键鼠监听线程已提前结束。".into());
        }
        Ok(self
            .events
            .lock()
            .map_err(|_| "键鼠事件缓冲不可用。")?
            .drain())
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut events) = self.events.lock() {
            events.drain();
        }
    }
}

/// Raw Input 注册是进程级的；保存已有路由，退出时仅在路由仍属于自己时恢复。
struct InputWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    class: HSTRING,
    usages: Vec<u16>,
    previous: Vec<RAWINPUTDEVICE>,
}

fn registrations() -> Result<Vec<RAWINPUTDEVICE>, String> {
    unsafe {
        let mut count = 0;
        let size = size_of::<RAWINPUTDEVICE>() as u32;
        if GetRegisteredRawInputDevices(None, &mut count, size) == u32::MAX {
            return Err(native_error(windows::core::Error::from_thread()));
        }
        // 注册表可能由 UI 线程改变，最多重试三次，禁止无限分配。
        for _ in 0..3 {
            if count > 1024 {
                return Err("进程 Raw Input 注册数量异常。".into());
            }
            let mut devices = vec![RAWINPUTDEVICE::default(); count as usize];
            if count == 0 {
                return Ok(devices);
            }
            let read = GetRegisteredRawInputDevices(Some(devices.as_mut_ptr()), &mut count, size);
            if read != u32::MAX {
                devices.truncate(read as usize);
                return Ok(devices);
            }
        }
        Err("无法读取现有 Raw Input 注册。".into())
    }
}

impl InputWindow {
    fn new(config: InputEffects) -> Result<Self, String> {
        static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);
        unsafe {
            // 独立线程生命周期内使用物理坐标；不改变 Tauri UI 线程的 DPI 上下文。
            if SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                .0
                .is_null()
            {
                return Err(native_error(windows::core::Error::from_thread()));
            }
            let instance = HINSTANCE(GetModuleHandleW(None).map_err(native_error)?.0);
            let class = HSTRING::from(format!(
                "SnapDeskRawInput{}",
                NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
            ));
            let previous = registrations()?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: windows::core::PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                return Err(native_error(windows::core::Error::from_thread()));
            }
            let mut window = Self {
                hwnd: HWND::default(),
                instance,
                class,
                usages: Vec::new(),
                previous,
            };
            window.hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                &window.class,
                &HSTRING::from(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )
            .map_err(native_error)?;
            let mut devices = Vec::new();
            if config.keyboard {
                window.usages.push(6);
            }
            if config.clicks || config.movement || config.drag || config.wheel {
                window.usages.push(2);
            }
            for &usage in &window.usages {
                devices.push(RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: usage,
                    dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
                    hwndTarget: window.hwnd,
                });
            }
            RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32)
                .map_err(native_error)?;
            Ok(window)
        }
    }
}
impl Drop for InputWindow {
    fn drop(&mut self) {
        unsafe {
            if let Ok(current) = registrations() {
                for &usage in &self.usages {
                    if !current.iter().any(|d| {
                        d.usUsagePage == 1 && d.usUsage == usage && d.hwndTarget == self.hwnd
                    }) {
                        continue;
                    }
                    let previous = self
                        .previous
                        .iter()
                        .find(|d| d.usUsagePage == 1 && d.usUsage == usage)
                        .filter(|d| {
                            d.hwndTarget.0.is_null() || IsWindow(Some(d.hwndTarget)).as_bool()
                        })
                        .copied();
                    let restore = previous.unwrap_or(RAWINPUTDEVICE {
                        usUsagePage: 1,
                        usUsage: usage,
                        dwFlags: RIDEV_REMOVE,
                        hwndTarget: HWND::default(),
                    });
                    if let Err(error) =
                        RegisterRawInputDevices(&[restore], size_of::<RAWINPUTDEVICE>() as u32)
                    {
                        log::warn!("Raw Input 注销/恢复失败：{error}");
                    }
                }
            }
            if !self.hwnd.0.is_null() {
                let _ = DestroyWindow(self.hwnd);
            }
            let _ = UnregisterClassW(&self.class, Some(self.instance));
        }
    }
}

/// 状态先在消息线程消费，输出事件携带当前拖动状态；上游丢事件不影响抬起语义。
#[derive(Default)]
struct MouseState {
    left: bool,
    right: bool,
    clicks: Clicks,
}

fn pump(
    window: &InputWindow,
    config: InputEffects,
    stop: &AtomicBool,
    events: &Mutex<EventBuffer>,
) -> Result<(), String> {
    let mut keys = Keys::default();
    let mut mouse = MouseState::default();
    unsafe {
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            // 系统消息唤醒；25ms 超时仅用于可靠结束线程，不对设备做高频轮询。
            if MsgWaitForMultipleObjectsEx(None, 25, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                == WAIT_FAILED
            {
                return Err(native_error(windows::core::Error::from_thread()));
            }
            let mut msg = MSG::default();
            while !stop.load(Ordering::Acquire)
                && PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool()
            {
                if msg.message == WM_QUIT {
                    return Err("键鼠消息线程收到意外退出请求。".into());
                }
                let result = if msg.hwnd == window.hwnd && msg.message == WM_INPUT {
                    receive(
                        HRAWINPUT(msg.lParam.0 as *mut _),
                        config,
                        &mut keys,
                        &mut mouse,
                        events,
                    )
                } else {
                    if msg.message == WM_INPUT_DEVICE_CHANGE {
                        keys = Keys::default();
                        mouse = MouseState::default();
                    }
                    Ok(())
                };
                // 即使读取失败也调用默认窗口过程，释放 WM_INPUT 系统资源。
                DispatchMessageW(&msg);
                result?;
            }
        }
    }
}

unsafe fn receive(
    handle: HRAWINPUT,
    config: InputEffects,
    keys: &mut Keys,
    mouse: &mut MouseState,
    events: &Mutex<EventBuffer>,
) -> Result<(), String> {
    let source_time = capture::now();
    // 仅注册标准鼠标/键盘；RAWINPUT 是对齐的固定容量缓冲，不读取可变长 HID。
    let mut raw = RAWINPUT::default();
    let mut bytes = size_of::<RAWINPUT>() as u32;
    let read = GetRawInputData(
        handle,
        RID_INPUT,
        Some((&mut raw as *mut RAWINPUT).cast()),
        &mut bytes,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if read == u32::MAX {
        return Err(native_error(windows::core::Error::from_thread()));
    }
    if read < size_of::<RAWINPUTHEADER>() as u32 {
        return Err("Raw Input 数据不完整。".into());
    }
    let mut output = Vec::new();
    if raw.header.dwType == RIM_TYPEKEYBOARD.0 && config.keyboard {
        if read < (size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>()) as u32 {
            return Err("键盘输入数据不完整。".into());
        }
        let key = raw.data.keyboard;
        // RI_KEY_E0=2，RI_KEY_BREAK=1。修饰键归一化为左右独立的虚拟键。
        let virtual_key = match key.VKey {
            0x10 => {
                if key.MakeCode == 0x36 {
                    0xa1
                } else {
                    0xa0
                }
            }
            0x11 => {
                if key.Flags & 2 != 0 {
                    0xa3
                } else {
                    0xa2
                }
            }
            0x12 => {
                if key.Flags & 2 != 0 {
                    0xa5
                } else {
                    0xa4
                }
            }
            key => key,
        };
        if let Some(label) = keys.update(virtual_key, key.Flags & 1 == 0) {
            output.push(InputKind::Key(label));
        }
    } else if raw.header.dwType == RIM_TYPEMOUSE.0 {
        if read < (size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>()) as u32 {
            return Err("鼠标输入数据不完整。".into());
        }
        let raw_mouse = raw.data.mouse;
        let buttons = raw_mouse.Anonymous.Anonymous;
        let flags = buttons.usButtonFlags;
        let mut point = POINT::default();
        // 安全桌面切换等情况下位置不可用时不画鼠标，抬起状态仍正常更新。
        let position_valid = GetCursorPos(&mut point).is_ok();
        for (down_flag, up_flag, right) in [(1, 2, false), (4, 8, true)] {
            if flags & down_flag != 0 {
                if right {
                    mouse.right = true;
                } else {
                    mouse.left = true;
                }
                if position_valid && config.clicks {
                    let double = mouse.clicks.press(
                        source_time,
                        point.x,
                        point.y,
                        right,
                        (
                            i64::from(GetDoubleClickTime()) * 10_000,
                            GetSystemMetrics(SM_CXDOUBLECLK) / 2,
                            GetSystemMetrics(SM_CYDOUBLECLK) / 2,
                        ),
                    );
                    output.push(InputKind::Click {
                        x: point.x,
                        y: point.y,
                        right,
                        double,
                    });
                }
            }
            if flags & up_flag != 0 {
                if right {
                    mouse.right = false;
                } else {
                    mouse.left = false;
                }
            }
        }
        if position_valid {
            if config.movement || config.drag {
                output.push(InputKind::Move {
                    x: point.x,
                    y: point.y,
                    dragging: mouse.left || mouse.right,
                });
            }
            // RI_MOUSE_WHEEL=0x400；必须先解释为有符号 16 位再扩展。
            if config.wheel && flags & 0x400 != 0 {
                output.push(InputKind::Wheel {
                    x: point.x,
                    y: point.y,
                    delta: i32::from(buttons.usButtonData as i16),
                });
            }
        }
    }
    if !output.is_empty() {
        let mut buffer = events.lock().map_err(|_| "键鼠事件缓冲不可用。")?;
        for kind in output {
            buffer.push(InputEvent { source_time, kind });
        }
    }
    Ok(())
}
