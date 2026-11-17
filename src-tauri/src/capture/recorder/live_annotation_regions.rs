//! 录屏覆盖层按选区、工具栏和菜单的并集裁剪；区域外由系统直接命中桌面。
//! 坐标相对覆盖窗口左上角，单位为物理像素，不能传入 CSS 像素或屏幕绝对坐标。
use crate::capture::Rect;
use tauri::WebviewWindow;

/// 系统接管成功提交的 HRGN；失败路径仍由本函数释放，避免反复拖动时泄漏 GDI 对象。
#[cfg(windows)]
pub(super) fn set(window: &WebviewWindow, rectangles: &[Rect]) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, RGN_OR},
    };
    let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0);
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        if region.0.is_null() {
            return Err("无法创建录屏标注交互区域。".into());
        }
        for rect in rectangles {
            let part = CreateRectRgn(
                rect.x as i32,
                rect.y as i32,
                (rect.x + rect.width) as i32,
                (rect.y + rect.height) as i32,
            );
            if part.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(region.0));
                return Err("无法创建录屏标注交互区域。".into());
            }
            let combined = CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
            let _ = DeleteObject(HGDIOBJ(part.0));
            if combined.0 == 0 {
                let _ = DeleteObject(HGDIOBJ(region.0));
                return Err("无法合并录屏标注交互区域。".into());
            }
        }
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
            return Err(format!(
                "无法设置录屏标注交互区域：{}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

/// 截图和录屏复用 capture 窗口，结束录屏交互后必须恢复完整窗口区域。
#[cfg(windows)]
pub(super) fn reset(window: &WebviewWindow) -> Result<(), String> {
    use windows::Win32::{Foundation::HWND, Graphics::Gdi::SetWindowRgn};
    let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0);
    if unsafe { SetWindowRgn(hwnd, None, true) } == 0 {
        return Err("无法恢复截图窗口区域。".into());
    }
    Ok(())
}

/// Linux 用 GTK 输入形状（X11 为 XShape 输入区域，Wayland 为 input region）：形状外的点击直达下层窗口。
/// GTK 使用逻辑像素，物理像素按窗口缩放向外取整，交互区域只会略大而不会缺边。
#[cfg(target_os = "linux")]
pub(super) fn set(window: &WebviewWindow, rectangles: &[Rect]) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?.max(1.0);
    let parts = rectangles
        .iter()
        .map(|rect| logical(rect, scale))
        .collect::<Vec<_>>();
    update(window, |state| state.regions = Some(parts))
}

#[cfg(target_os = "linux")]
pub(super) fn reset(window: &WebviewWindow) -> Result<(), String> {
    update(window, |state| state.regions = None)
}

/// 整窗穿透。Linux 上 tao 的同名接口同样改写 GTK 输入形状，但经请求队列异步执行，
/// 可能晚于随后设置的交互区域而把它覆盖；这里与 [`set`] / [`reset`] 同步串行，顺序即调用顺序。
#[cfg(target_os = "linux")]
pub(super) fn ignore_cursor(window: &WebviewWindow, ignore: bool) -> Result<(), String> {
    update(window, |state| state.ignore = ignore)
}

/// Windows 的窗口区域与整窗穿透是两项独立设置，Linux 只有一个输入形状，
/// 因此按窗口记住两者再合成：穿透优先，其次是交互区域，都没有时整窗接收输入。
#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
struct InputState {
    ignore: bool,
    regions: Option<Vec<(i32, i32, i32, i32)>>,
}

#[cfg(target_os = "linux")]
impl InputState {
    fn effective(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        if self.ignore {
            Some(Vec::new())
        } else {
            self.regions.clone()
        }
    }
}

#[cfg(target_os = "linux")]
fn update(window: &WebviewWindow, change: impl FnOnce(&mut InputState)) -> Result<(), String> {
    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
    };
    static STATES: OnceLock<Mutex<HashMap<String, InputState>>> = OnceLock::new();
    let effective = {
        let mut states = STATES
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "录屏标注交互区域状态不可用。")?;
        let state = states.entry(window.label().to_owned()).or_default();
        change(state);
        state.effective()
    };
    shape(window, effective)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn ignore_cursor(window: &WebviewWindow, ignore: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(ignore)
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn logical(rect: &Rect, scale: f64) -> (i32, i32, i32, i32) {
    let left = (f64::from(rect.x) / scale).floor() as i32;
    let top = (f64::from(rect.y) / scale).floor() as i32;
    let right = (f64::from(rect.x + rect.width) / scale).ceil() as i32;
    let bottom = (f64::from(rect.y + rect.height) / scale).ceil() as i32;
    (left, top, right - left, bottom - top)
}

/// GTK 只能在主线程调用：已在主线程时直接执行，否则投递并等待完成。
/// None 恢复整窗接收输入，空列表为整窗穿透。
#[cfg(target_os = "linux")]
fn shape(window: &WebviewWindow, parts: Option<Vec<(i32, i32, i32, i32)>>) -> Result<(), String> {
    use gtk::{cairo, prelude::*};
    let apply = move |target: &WebviewWindow| {
        target.gtk_window().map_err(|e| e.to_string()).map(|widget| {
            let region = parts.map(|parts| {
                let rectangles = parts
                    .into_iter()
                    .map(|(x, y, width, height)| cairo::RectangleInt::new(x, y, width, height))
                    .collect::<Vec<_>>();
                cairo::Region::create_rectangles(&rectangles)
            });
            widget.input_shape_combine_region(region.as_ref());
        })
    };
    let result = if gtk::is_initialized_main_thread() {
        apply(window)
    } else {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let target = window.clone();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(apply(&target));
            })
            .map_err(|e| e.to_string())?;
        receiver.recv().map_err(|e| e.to_string())?
    };
    result.map_err(|e| format!("无法设置录屏标注交互区域：{e}"))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub(super) fn set(_: &WebviewWindow, _: &[Rect]) -> Result<(), String> {
    Err("当前平台尚不支持录屏标注的局部交互区域。".into())
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub(super) fn reset(_: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    #[test]
    fn physical_rectangles_round_outward_to_logical_pixels() {
        let rect = Rect {
            x: 3,
            y: 5,
            width: 10,
            height: 7,
        };
        assert_eq!(logical(&rect, 1.0), (3, 5, 10, 7));
        assert_eq!(logical(&rect, 2.0), (1, 2, 6, 4));
    }

    #[test]
    fn passthrough_wins_over_regions_and_reset_keeps_passthrough() {
        let mut state = InputState::default();
        assert_eq!(state.effective(), None);
        state.regions = Some(vec![(0, 0, 4, 4)]);
        assert_eq!(state.effective(), Some(vec![(0, 0, 4, 4)]));
        state.ignore = true;
        assert_eq!(state.effective(), Some(vec![]));
        state.regions = None;
        assert_eq!(state.effective(), Some(vec![]));
        state.ignore = false;
        assert_eq!(state.effective(), None);
    }
}

#[cfg(target_os = "macos")]
pub(super) use mac::{reset, set};

/// macOS 不能只让窗口的一部分穿透点击：每 30 ms 读取全局光标，
/// 光标进入交互区域时让整个覆盖窗口接收鼠标，离开后改为穿透，效果与 Windows 的窗口区域一致。
#[cfg(target_os = "macos")]
mod mac {
    use super::Rect;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, OnceLock},
        time::Duration,
    };
    use tauri::WebviewWindow;

    /// 轮询线程只读这里的快照，不调用窗口取值接口：后台线程读窗口属性要等主线程应答，
    /// 若主线程正在 reset 中等待，会互相等待。录制期间覆盖窗口不移动，在 set 时取一次即可。
    struct State {
        rectangles: Vec<Rect>,
        /// 窗口左上角的物理像素位置与缩放比例。
        origin: (f64, f64),
        scale: f64,
        stopped: bool,
    }

    /// 按窗口标签保存轮询状态；同一窗口重复设置只更新区域，不再开新线程。
    fn watches() -> &'static Mutex<HashMap<String, Arc<Mutex<State>>>> {
        static WATCHES: OnceLock<Mutex<HashMap<String, Arc<Mutex<State>>>>> = OnceLock::new();
        WATCHES.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn inside(state: &State) -> bool {
        let Some((cx, cy)) = crate::shared::macos::cursor_point() else {
            return false;
        };
        let x = cx * state.scale - state.origin.0;
        let y = cy * state.scale - state.origin.1;
        state.rectangles.iter().any(|r| {
            x >= f64::from(r.x)
                && x < f64::from(r.x + r.width)
                && y >= f64::from(r.y)
                && y < f64::from(r.y + r.height)
        })
    }

    pub(in crate::capture) fn set(window: &WebviewWindow, rectangles: &[Rect]) -> Result<(), String> {
        let origin = window.outer_position().map_err(|e| e.to_string())?;
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let busy = || "录屏标注区域状态不可用。".to_string();
        let mut watches = watches().lock().map_err(|_| busy())?;
        let label = window.label().to_string();
        if let Some(state) = watches.get(&label) {
            let mut state = state.lock().map_err(|_| busy())?;
            state.rectangles = rectangles.to_vec();
            state.origin = (f64::from(origin.x), f64::from(origin.y));
            state.scale = scale;
            return Ok(());
        }
        let state = Arc::new(Mutex::new(State {
            rectangles: rectangles.to_vec(),
            origin: (f64::from(origin.x), f64::from(origin.y)),
            scale,
            stopped: false,
        }));
        let (shared, target) = (state.clone(), window.clone());
        std::thread::Builder::new()
            .name("annotation-hit-test".into())
            .spawn(move || {
                let mut interactive: Option<bool> = None;
                loop {
                    {
                        // 检查停止与切换都在同一把锁内：reset 置位后不会再有迟到的切换。
                        let Ok(state) = shared.lock() else { break };
                        if state.stopped {
                            break;
                        }
                        let current = inside(&state);
                        // 只在状态变化时切换；设置接口只投递消息，不等待主线程。
                        if interactive != Some(current) {
                            if target.set_ignore_cursor_events(!current).is_err() {
                                break;
                            }
                            interactive = Some(current);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
            })
            .map_err(|_| "无法启动录屏标注交互线程。")?;
        watches.insert(label, state);
        Ok(())
    }

    pub(in crate::capture) fn reset(window: &WebviewWindow) -> Result<(), String> {
        let state = watches().lock().ok().and_then(|mut w| w.remove(window.label()));
        if let Some(state) = state {
            if let Ok(mut state) = state.lock() {
                state.stopped = true;
            }
        }
        // 截图复用同一窗口，恢复为整窗接收鼠标。
        window
            .set_ignore_cursor_events(false)
            .map_err(|e| e.to_string())
    }
}
