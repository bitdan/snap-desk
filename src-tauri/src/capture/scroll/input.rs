//! 首次滚动锁定轴；滚轮透传，Esc 取消，Ctrl+C 完成并复制长截图。
use super::matcher::ScrollMode;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

/// 0 等待滚轮，1 纵向，2 横向。会话快照共享此值，不复制系统监听资源。
#[derive(Clone, Default)]
pub(super) struct Direction(Arc<AtomicU8>);
impl Direction {
    pub fn mode(&self) -> ScrollMode {
        match self.0.load(Ordering::Acquire) {
            1 => ScrollMode::Vertical,
            2 => ScrollMode::Horizontal,
            _ => ScrollMode::Auto,
        }
    }
    fn lock(&self, horizontal: bool) {
        let _ = self.0.compare_exchange(
            0,
            if horizontal { 2 } else { 1 },
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::{atomic::AtomicBool, mpsc},
        thread::JoinHandle,
    };
    use windows::Win32::{
        Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_C, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT},
            WindowsAndMessaging::*,
        },
    };

    thread_local! {
        // 低级鼠标回调在安装钩子的消息线程执行，选区坐标为物理桌面像素。
        static TARGET: RefCell<Option<(Direction, (i64, i64, i64, i64))>> = const { RefCell::new(None) };
        // 回调只置位，请求取消在消息循环中处理，避免低级钩子等待会话锁。
        static ESCAPE_REQUESTED: Cell<bool> = const { Cell::new(false) };
        static ESCAPE_ACTIVE: Cell<bool> = const { Cell::new(true) };
        static COPY_REQUESTED: Cell<bool> = const { Cell::new(false) };
        static COPY_ACTIVE: Cell<bool> = const { Cell::new(true) };
        // 消费同一轮 C 的长按和释放，避免原应用复制正文覆盖长截图。
        static COPY_HELD: Cell<bool> = const { Cell::new(false) };
    }
    unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && matches!(wparam.0 as u32, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
            let event = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            if (event.mouseData >> 16) as i16 != 0 {
                TARGET.with(|target| {
                    if let Some((direction, (left, top, right, bottom))) = target.borrow().as_ref()
                    {
                        let (x, y) = (i64::from(event.pt.x), i64::from(event.pt.y));
                        if x >= *left && x < *right && y >= *top && y < *bottom {
                            direction.lock(
                                wparam.0 as u32 == WM_MOUSEHWHEEL
                                    || GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0,
                            );
                        }
                    }
                });
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    /// 钩子回调只记录按键意图，不在系统回调内锁会话或操作剪贴板。
    fn copy_key(message: u32, key: u32, control: bool, shift: bool, alt: bool) -> bool {
        if key != u32::from(VK_C.0) { return false; }
        if matches!(message, WM_KEYUP | WM_SYSKEYUP) {
            return COPY_HELD.with(|held| held.replace(false));
        }
        if !matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) { return false; }
        if COPY_HELD.with(Cell::get) { return true; }
        if ESCAPE_ACTIVE.with(Cell::get) && COPY_ACTIVE.with(Cell::get) && control && !shift && !alt {
            COPY_HELD.with(|held| held.set(true));
            COPY_REQUESTED.with(|requested| requested.set(true));
            return true;
        }
        false
    }

    fn copy_down() -> bool {
        unsafe {
            GetAsyncKeyState(i32::from(VK_C.0)) < 0
                && GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0
                && GetAsyncKeyState(i32::from(VK_SHIFT.0)) >= 0
                && GetAsyncKeyState(i32::from(VK_MENU.0)) >= 0
        }
    }

    /// 焦点留在被截图应用时也能完成或取消；消费对应按键，避免原页面同时处理。
    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if copy_key(
                wparam.0 as u32, event.vkCode,
                GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0,
                GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0,
                GetAsyncKeyState(i32::from(VK_MENU.0)) < 0,
            ) {
                return LRESULT(1);
            }
            if event.vkCode == u32::from(VK_ESCAPE.0) && ESCAPE_ACTIVE.with(Cell::get) {
                if matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
                    ESCAPE_REQUESTED.with(|requested| requested.set(true));
                }
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    /// 监听跟随采集线程生命周期；Drop 等待卸载完成，取消后不会遗留系统钩子。
    pub struct Listener {
        stop: Arc<AtomicBool>,
        retry_copy: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }
    impl Listener {
        pub fn start(
            direction: Direction,
            bounds: (i64, i64, i64, i64),
            cancel: impl Fn() -> bool + Send + 'static,
            finish: impl Fn() -> bool + Send + 'static,
        ) -> Result<Self, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let retry_copy = Arc::new(AtomicBool::new(false));
            let retry_requested = retry_copy.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new()
                .name("scroll-direction".into())
                .spawn(move || unsafe {
                    // 钩子线程也必须使用物理 DPI，才能与负坐标副屏上的截图选区比较。
                    let _dpi = match crate::capture::desktop::PhysicalDpi::enter() {
                        Ok(dpi) => dpi,
                        Err(error) => {
                            let _ = tx.send(Err(error));
                            return;
                        }
                    };
                    let module = match GetModuleHandleW(None) {
                        Ok(module) => module,
                        Err(error) => {
                            let _ = tx.send(Err(format!("滚轮方向监听失败：{error}")));
                            return;
                        }
                    };
                    let hook = match SetWindowsHookExW(
                        WH_MOUSE_LL,
                        Some(mouse),
                        Some(HINSTANCE(module.0)),
                        0,
                    ) {
                        Ok(hook) => hook,
                        Err(error) => {
                            let _ = tx.send(Err(format!("滚轮方向监听失败：{error}")));
                            return;
                        }
                    };
                    let keyboard_hook = match SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(keyboard),
                        Some(HINSTANCE(module.0)),
                        0,
                    ) {
                        Ok(hook) => hook,
                        Err(error) => {
                            let _ = UnhookWindowsHookEx(hook);
                            let _ = tx.send(Err(format!("Esc 退出监听失败：{error}")));
                            return;
                        }
                    };
                    TARGET.with(|target| *target.borrow_mut() = Some((direction, bounds)));
                    log::info!("event=scroll.input_ready module=scroll escape_hook=true");
                    let _ = tx.send(Ok(()));
                    // 不使用异步按键状态的最低位（可能被其他进程读取），只检测当前按下边沿。
                    // 低级钩子被系统移除时，轮询仍能接住前台应用中的 Esc。
                    let mut escape_down = GetAsyncKeyState(i32::from(VK_ESCAPE.0)) < 0;
                    let mut copying = copy_down();
                    while !stopped.load(Ordering::Acquire) {
                        // 首轮完成后不再吞 Ctrl+C；输出失败时仅重启下一次按下边沿。
                        if retry_requested.swap(false, Ordering::AcqRel) {
                            COPY_ACTIVE.with(|active| active.set(true));
                            copying = copy_down();
                        }
                        MsgWaitForMultipleObjectsEx(None, 20, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                        let mut msg = MSG::default();
                        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                            DispatchMessageW(&msg);
                        }
                        let down = GetAsyncKeyState(i32::from(VK_ESCAPE.0)) < 0;
                        let hooked = ESCAPE_REQUESTED.with(|requested| requested.replace(false));
                        if ESCAPE_ACTIVE.with(Cell::get) && (hooked || (down && !escape_down)) {
                            log::info!("event=scroll.escape module=scroll hook={hooked}");
                            if cancel() {
                                // 在途匹配尚未退出时也停止吞掉后续 Esc，不干扰其他应用。
                                ESCAPE_ACTIVE.with(|active| active.set(false));
                            }
                        }
                        escape_down = down;
                        let copy_pressed = copy_down();
                        let copy_hooked = COPY_REQUESTED.with(|requested| requested.replace(false));
                        if ESCAPE_ACTIVE.with(Cell::get) && COPY_ACTIVE.with(Cell::get)
                            && (copy_hooked || (copy_pressed && !copying)) && finish()
                        {
                            // 完成只停止采集；Esc 仍可中断末帧等待或导出。
                            COPY_ACTIVE.with(|active| active.set(false));
                        }
                        copying = copy_pressed;
                    }
                    let _ = UnhookWindowsHookEx(keyboard_hook);
                    let _ = UnhookWindowsHookEx(hook);
                    TARGET.with(|target| *target.borrow_mut() = None);
                })
                .map_err(|e| e.to_string())?;
            let listener = Self {
                stop,
                retry_copy,
                worker: Some(worker),
            };
            rx.recv()
                .map_err(|_| "滚轮方向监听线程初始化失败。".to_string())??;
            Ok(listener)
        }

        /// 由采集线程在失败状态已发布后调用；消息线程负责恢复快捷键，不跨线程操作钩子。
        pub fn allow_copy_retry(&self) {
            self.retry_copy.store(true, Ordering::Release);
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn copy_chord_is_consumed_once_and_other_modifiers_are_preserved() {
            COPY_ACTIVE.with(|active| active.set(true));
            COPY_HELD.with(|held| held.set(false));
            COPY_REQUESTED.with(|requested| requested.set(false));
            let c = u32::from(VK_C.0);
            assert!(!copy_key(WM_KEYDOWN, c, false, false, false));
            assert!(!copy_key(WM_KEYDOWN, c, true, true, false));
            assert!(!copy_key(WM_KEYDOWN, c, true, false, true));
            assert!(copy_key(WM_KEYDOWN, c, true, false, false));
            assert!(COPY_REQUESTED.with(|requested| requested.replace(false)));
            assert!(copy_key(WM_KEYDOWN, c, true, false, false));
            assert!(!COPY_REQUESTED.with(Cell::get));
            assert!(copy_key(WM_KEYUP, c, false, false, false));
            COPY_ACTIVE.with(|active| active.set(false));
            assert!(!copy_key(WM_KEYDOWN, c, true, false, false));
            COPY_ACTIVE.with(|active| active.set(true));
        }

        #[test]
        fn escape_hook_requests_cancel_on_keydown_without_waiting_for_a_webview() {
            ESCAPE_ACTIVE.with(|active| active.set(true));
            ESCAPE_REQUESTED.with(|requested| requested.set(false));
            let event = KBDLLHOOKSTRUCT {
                vkCode: u32::from(VK_ESCAPE.0),
                ..Default::default()
            };
            let param = LPARAM(&event as *const KBDLLHOOKSTRUCT as isize);
            unsafe {
                assert_eq!(keyboard(0, WPARAM(WM_KEYDOWN as usize), param).0, 1);
                assert!(ESCAPE_REQUESTED.with(|requested| requested.replace(false)));
                assert_eq!(keyboard(0, WPARAM(WM_KEYUP as usize), param).0, 1);
                assert!(!ESCAPE_REQUESTED.with(Cell::get));
            }
        }
    }
}

#[cfg(windows)]
pub(super) use platform::Listener;

/// macOS：没有“输入监控”授权就不能安装全局事件监听，因此轮询按键状态检测 Esc 与 ⌘C 的按下边沿。
/// 轮询只读不拦截，按键仍会传给前台应用；完成时长截图随后写入剪贴板，会覆盖应用自身的复制结果。
/// 滚动方向不锁定，保持自动识别（ScrollMode::Auto）。
#[cfg(target_os = "macos")]
mod mac {
    use super::Direction;
    use crate::shared::macos;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::JoinHandle,
        time::Duration,
    };

    pub struct Listener {
        stop: Arc<AtomicBool>,
        retry_copy: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    /// 只认单独的 ⌘C；带 ⇧ 或 ⌥ 时多为其他应用的快捷键，不当作完成。
    fn copy_down() -> bool {
        let flags = macos::modifier_flags();
        macos::key_down(macos::KEY_C)
            && flags & macos::FLAG_COMMAND != 0
            && flags & (macos::FLAG_SHIFT | macos::FLAG_OPTION) == 0
    }

    impl Listener {
        pub fn start(
            _direction: Direction,
            _bounds: (i64, i64, i64, i64),
            cancel: impl Fn() -> bool + Send + 'static,
            finish: impl Fn() -> bool + Send + 'static,
        ) -> Result<Self, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let retry_copy = Arc::new(AtomicBool::new(false));
            let retry_requested = retry_copy.clone();
            let worker = std::thread::Builder::new()
                .name("scroll-keys".into())
                .spawn(move || {
                    // 以启动时的状态为基准，避免触发长截图时仍按着的键被当作一次按下。
                    let mut escape_down = macos::key_down(macos::KEY_ESCAPE);
                    let mut copying = copy_down();
                    let mut escape_active = true;
                    let mut copy_active = true;
                    log::info!("event=scroll.input_ready module=scroll escape_hook=false");
                    while !stopped.load(Ordering::Acquire) {
                        if retry_requested.swap(false, Ordering::AcqRel) {
                            copy_active = true;
                            copying = copy_down();
                        }
                        std::thread::sleep(Duration::from_millis(20));
                        let down = macos::key_down(macos::KEY_ESCAPE);
                        if escape_active && down && !escape_down {
                            log::info!("event=scroll.escape module=scroll hook=false");
                            if cancel() {
                                escape_active = false;
                            }
                        }
                        escape_down = down;
                        let pressed = copy_down();
                        if escape_active && copy_active && pressed && !copying && finish() {
                            // 完成只停止采集；Esc 仍可中断末帧等待或导出。
                            copy_active = false;
                        }
                        copying = pressed;
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(Self {
                stop,
                retry_copy,
                worker: Some(worker),
            })
        }

        /// 导出失败后允许再次用 ⌘C 完成。
        pub fn allow_copy_retry(&self) {
            self.retry_copy.store(true, Ordering::Release);
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}
#[cfg(target_os = "macos")]
pub(super) use mac::Listener;

/// Linux（X11）：与 macOS 相同，轮询全局键盘状态检测 Esc 与 Ctrl+C 的按下边沿。
/// X11 只有抢占整个键盘才能拦截按键，因此按键仍会传给前台应用；完成时长截图随后写入剪贴板，
/// 会覆盖应用自身的复制结果。滚动方向不锁定，保持自动识别。
#[cfg(target_os = "linux")]
mod linux {
    use super::Direction;
    use crate::shared::x11::{self, X11};
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread::JoinHandle,
        time::Duration,
    };

    const ESCAPE: u32 = 0xff1b;
    const C: u32 = 0x63;
    const CONTROL: [u32; 2] = [0xffe3, 0xffe4];
    /// Shift、Alt、Meta、Super：带这些修饰键时多为其他应用的快捷键，不当作完成。
    const OTHER: [u32; 8] = [
        0xffe1, 0xffe2, 0xffe9, 0xffea, 0xffe7, 0xffe8, 0xffeb, 0xffec,
    ];

    /// 启动时按当前键盘映射解析一次键码。
    struct Keys {
        escape: Vec<u8>,
        c: Vec<u8>,
        control: Vec<u8>,
        other: Vec<u8>,
    }

    impl Keys {
        fn new(x11: &X11) -> Self {
            let codes = |syms: &[u32]| -> Vec<u8> {
                x11.keycodes(syms)
                    .into_iter()
                    .map(|(_, code)| code)
                    .collect()
            };
            Self {
                escape: codes(&[ESCAPE]),
                c: codes(&[C]),
                control: codes(&CONTROL),
                other: codes(&OTHER),
            }
        }

        /// 返回（Esc 是否按下，单独的 Ctrl+C 是否按下）。
        fn state(&self, keymap: &[u8]) -> (bool, bool) {
            let any = |codes: &[u8]| codes.iter().any(|code| x11::pressed(keymap, *code));
            (
                any(&self.escape),
                any(&self.c) && any(&self.control) && !any(&self.other),
            )
        }
    }

    pub struct Listener {
        stop: Arc<AtomicBool>,
        retry_copy: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl Listener {
        pub fn start(
            _direction: Direction,
            _bounds: (i64, i64, i64, i64),
            cancel: impl Fn() -> bool + Send + 'static,
            finish: impl Fn() -> bool + Send + 'static,
        ) -> Result<Self, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let retry_copy = Arc::new(AtomicBool::new(false));
            let retry_requested = retry_copy.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new()
                .name("scroll-keys".into())
                .spawn(move || {
                    let x11 = match X11::connect() {
                        Ok(x11) => x11,
                        Err(error) => {
                            let _ = tx.send(Err(format!("Esc 退出监听失败：{error}")));
                            return;
                        }
                    };
                    let keys = Keys::new(&x11);
                    if keys.escape.is_empty() {
                        let _ = tx.send(Err("Esc 退出监听失败：键盘映射中没有 Esc。".to_string()));
                        return;
                    }
                    // 以启动时的状态为基准，避免触发长截图时仍按着的键被当作一次按下。
                    let (mut escape_down, mut copying) = x11
                        .keymap()
                        .map_or((true, true), |keymap| keys.state(&keymap));
                    let mut escape_active = true;
                    let mut copy_active = true;
                    log::info!("event=scroll.input_ready module=scroll escape_hook=false");
                    let _ = tx.send(Ok(()));
                    while !stopped.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(20));
                        let Some(keymap) = x11.keymap() else {
                            continue;
                        };
                        let (down, pressed) = keys.state(&keymap);
                        if retry_requested.swap(false, Ordering::AcqRel) {
                            copy_active = true;
                            copying = pressed;
                        }
                        if escape_active && down && !escape_down {
                            log::info!("event=scroll.escape module=scroll hook=false");
                            if cancel() {
                                escape_active = false;
                            }
                        }
                        escape_down = down;
                        if escape_active && copy_active && pressed && !copying && finish() {
                            // 完成只停止采集；Esc 仍可中断末帧等待或导出。
                            copy_active = false;
                        }
                        copying = pressed;
                    }
                })
                .map_err(|e| e.to_string())?;
            let listener = Self {
                stop,
                retry_copy,
                worker: Some(worker),
            };
            rx.recv()
                .map_err(|_| "Esc 退出监听线程初始化失败。".to_string())??;
            Ok(listener)
        }

        /// 导出失败后允许再次用 Ctrl+C 完成。
        pub fn allow_copy_retry(&self) {
            self.retry_copy.store(true, Ordering::Release);
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn press(keymap: &mut [u8], code: u8) {
            keymap[usize::from(code / 8)] |= 1 << (code % 8);
        }

        #[test]
        fn copy_chord_requires_control_without_other_modifiers() {
            let keys = Keys {
                escape: vec![9],
                c: vec![54],
                control: vec![37, 105],
                other: vec![50],
            };
            let mut keymap = [0u8; 32];
            assert_eq!(keys.state(&keymap), (false, false));
            press(&mut keymap, 54);
            assert_eq!(keys.state(&keymap), (false, false));
            press(&mut keymap, 105);
            assert_eq!(keys.state(&keymap), (false, true));
            press(&mut keymap, 50);
            press(&mut keymap, 9);
            assert_eq!(keys.state(&keymap), (true, false));
        }
    }
}
#[cfg(target_os = "linux")]
pub(super) use linux::Listener;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_wheel_locks_axis_for_shared_session_snapshots() {
        for horizontal in [false, true] {
            let direction = Direction::default();
            assert_eq!(direction.mode(), ScrollMode::Auto);
            let snapshot = direction.clone();
            direction.lock(horizontal);
            snapshot.lock(!horizontal);
            assert_eq!(
                snapshot.mode(),
                if horizontal {
                    ScrollMode::Horizontal
                } else {
                    ScrollMode::Vertical
                }
            );
        }
    }
}
