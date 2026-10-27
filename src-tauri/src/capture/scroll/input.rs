//! 首次在选区内滚动时锁定轴；监听只读取方向，始终把输入传给前台应用。
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
        cell::RefCell,
        sync::{atomic::AtomicBool, mpsc},
        thread::JoinHandle,
    };
    use windows::Win32::{
        Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT},
            WindowsAndMessaging::*,
        },
    };

    thread_local! {
        // 低级鼠标回调在安装钩子的消息线程执行，选区坐标为物理桌面像素。
        static TARGET: RefCell<Option<(Direction, (i64, i64, i64, i64))>> = const { RefCell::new(None) };
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

    /// 监听跟随采集线程生命周期；Drop 等待卸载完成，取消后不会遗留系统钩子。
    pub struct Listener {
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }
    impl Listener {
        pub fn start(direction: Direction, bounds: (i64, i64, i64, i64)) -> Result<Self, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
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
                    TARGET.with(|target| *target.borrow_mut() = Some((direction, bounds)));
                    let _ = tx.send(Ok(()));
                    while !stopped.load(Ordering::Acquire) {
                        MsgWaitForMultipleObjectsEx(None, 20, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                        let mut msg = MSG::default();
                        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                            DispatchMessageW(&msg);
                        }
                    }
                    let _ = UnhookWindowsHookEx(hook);
                    TARGET.with(|target| *target.borrow_mut() = None);
                })
                .map_err(|e| e.to_string())?;
            let listener = Self {
                stop,
                worker: Some(worker),
            };
            rx.recv()
                .map_err(|_| "滚轮方向监听线程初始化失败。".to_string())??;
            Ok(listener)
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

#[cfg(windows)]
pub(super) use platform::Listener;

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
