//! 长截图辅助窗口：边框放在选区外，不盖住原页面；右侧预览和下方操作条独立显示。
use super::{err, Rect, Screen};
#[cfg(target_os = "windows")]
use std::{collections::BTreeMap, sync::Mutex};
use std::{thread, time::Duration};
#[cfg(target_os = "windows")]
static EDGE_SIZES: Mutex<BTreeMap<String, (u32, u32)>> = Mutex::new(BTreeMap::new());
#[cfg(target_os = "windows")]
static CLIPPING_EDGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
use tauri::{
    window::Color, AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow as Window, WebviewWindowBuilder,
};

const LABELS: [&str; 6] = [
    "scroll-preview",
    "scroll-controls",
    "scroll-edge-top",
    "scroll-edge-bottom",
    "scroll-edge-left",
    "scroll-edge-right",
];
const RECORDING_LABELS: [&str; 5] = [
    "recording-controls",
    "recording-edge-top",
    "recording-edge-bottom",
    "recording-edge-left",
    "recording-edge-right",
];

#[derive(Clone, Copy, Debug)]
struct BoxRect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}
impl BoxRect {
    fn overlaps(self, r: Rect) -> bool {
        self.x < (r.x + r.width) as i32
            && self.y < (r.y + r.height) as i32
            && self.x + self.w as i32 > r.x as i32
            && self.y + self.h as i32 > r.y as i32
    }
}

/// 所有坐标均为主屏物理像素；边缘空间不足时限制在屏幕内，并由取样保护器暂时隐藏。
fn layout(screen: &Screen, r: Rect) -> [BoxRect; 6] {
    layout_with_controls(screen, r, 320.0)
}

fn layout_with_controls(screen: &Screen, r: Rect, controls_width: f64) -> [BoxRect; 6] {
    let scale = f64::from_bits(screen.scale);
    let gap = (8.0 * scale).round() as i32;
    let edge = (2.0 * scale).round().max(1.0) as u32;
    let fit = |x: i32, y: i32, w: u32, h: u32| {
        let w = w.min(screen.width).max(1);
        let h = h.min(screen.height).max(1);
        BoxRect {
            x: x.clamp(0, (screen.width - w) as i32),
            y: y.clamp(0, (screen.height - h) as i32),
            w,
            h,
        }
    };
    let x = r.x as i32;
    let y = r.y as i32;
    let right = (r.x + r.width) as i32;
    let bottom = (r.y + r.height) as i32;
    let available_right = screen.width.saturating_sub(r.x + r.width + gap as u32);
    let preferred_width = (240.0 * scale) as u32;
    // 右侧至少能放 100 CSS 像素时缩窄预览；实在放不下才贴屏幕右边并在取样时避让。
    let preview_width = if available_right >= (100.0 * scale) as u32 {
        preferred_width.min(available_right)
    } else {
        preferred_width
    };
    let controls_w = (controls_width * scale) as u32;
    let controls_h = (48.0 * scale) as u32;
    let controls_y = if bottom + gap + controls_h as i32 <= screen.height as i32 {
        bottom + gap
    } else if y >= gap + controls_h as i32 {
        y - gap - controls_h as i32
    } else {
        bottom + gap
    };
    [
        fit(
            right + gap,
            y,
            preview_width,
            r.height.max((180.0 * scale) as u32),
        ),
        fit(
            right - controls_w as i32,
            controls_y,
            controls_w,
            controls_h,
        ),
        fit(x, y - edge as i32, r.width, edge),
        fit(x, bottom, r.width, edge),
        fit(x - edge as i32, y, edge, r.height),
        fit(right, y, edge, r.height),
    ]
}

fn window(app: &AppHandle, label: &str) -> Option<Window> {
    app.get_webview_window(label)
}

/// Windows 显示/重算窗口样式时可能重新套用系统最小跟踪尺寸。
/// 将可绘制区域在原生层裁成预期细条，即使外框被撑大也不能绘制紫色色块。
/// 区域相对窗口左上角，使用物理像素；成功后 HRGN 所有权交给系统。
#[cfg(target_os = "windows")]
fn clip_edge(window: &Window, width: u32, height: u32) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    // SetWindowRgn 同步发送窗口消息，主线程回调不得再次进入同一裁剪过程。
    if CLIPPING_EDGE.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CLIPPING_EDGE.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            CreateRectRgn, DeleteObject, GetRgnBox, GetWindowRgn, SetWindowRgn, HGDIOBJ,
        },
    };
    let hwnd = HWND(window.hwnd().map_err(err)?.0);
    unsafe {
        // WM_SIZE 回调也会进入此处，已正确裁剪时不重复 SetWindowRgn，避免消息递归。
        let current = CreateRectRgn(0, 0, 0, 0);
        if !current.0.is_null() {
            let mut bounds = windows::Win32::Foundation::RECT::default();
            let kind = GetWindowRgn(hwnd, current);
            GetRgnBox(current, &mut bounds);
            let _ = DeleteObject(HGDIOBJ(current.0));
            if kind.0 == 2
                && bounds.left == 0
                && bounds.top == 0
                && bounds.right == width as i32
                && bounds.bottom == height as i32
            {
                return Ok(());
            }
        }
        let region = CreateRectRgn(0, 0, width as i32, height as i32);
        if region.0.is_null() {
            return Err("无法创建选区边框裁剪区域。".into());
        }
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let error = std::io::Error::last_os_error();
            let _ = DeleteObject(HGDIOBJ(region.0));
            return Err(format!("无法裁剪选区边框：{error}"));
        }
    }
    Ok(())
}

/// 显示窗口之后及系统尺寸变化后重施裁剪，不能只在布局时设置一次。
pub(crate) fn restore_edge(window: &Window) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let size = EDGE_SIZES.lock().map_err(err)?.get(window.label()).copied();
        if let Some((width, height)) = size {
            clip_edge(window, width, height)?;
        }
    }
    Ok(())
}

fn show_window(window: &Window) -> Result<(), String> {
    window.show().map_err(err)?;
    // 同步读取形成窗口消息屏障，保证裁剪在 ShowWindow 的样式更新之后执行。
    let _ = window.outer_size().map_err(err)?;
    restore_edge(window)
}

pub(super) fn place(app: &AppHandle, screen: &Screen, rect: Rect) -> Result<Vec<String>, String> {
    place_windows(app, screen, rect, &LABELS, &layout(screen, rect), false)
}

/// 录屏复用细边框和操作条定位；排除预览窗，并通过系统捕获保护保持窗口常显。
pub(crate) fn place_recording(app: &AppHandle, screen: &Screen, rect: Rect) -> Result<(), String> {
    let result = place_windows(
        app,
        screen,
        rect,
        &RECORDING_LABELS,
        &layout_with_controls(screen, rect, 480.0)[1..],
        true,
    );
    if result.is_err() {
        hide_recording(app);
    }
    result.map(|_| ())
}

fn place_windows(
    app: &AppHandle,
    screen: &Screen,
    rect: Rect,
    labels: &[&str],
    boxes: &[BoxRect],
    recording: bool,
) -> Result<Vec<String>, String> {
    let mut overlaps = Vec::new();
    for (label, position) in labels.iter().zip(boxes) {
        let edge = label.contains("-edge-");
        let window = if let Some(window) = window(app, label) {
            window
        } else if edge {
            // 使用稳定窗口接口加载无脚本纯色页面；不挂载业务界面，也不申请 IPC 权限。
            // WindowBuilder 本身需要 unstable，不能在当前依赖配置下直接使用。
            WebviewWindowBuilder::new(app, *label, WebviewUrl::App("scroll-edge.html".into()))
                .title(if recording {
                    "录屏选区"
                } else {
                    "长截图选区"
                })
                // 无边框不等于无最小尺寸：Windows 默认仍有最小跟踪宽高，
                // 不显式设置时，2px 边框会被撑成约 136px 宽、38px 高的色块。
                .min_inner_size(1.0, 1.0)
                .inner_size(1.0, 1.0)
                .decorations(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .focused(false)
                .focusable(false)
                .visible(false)
                .background_color(Color(139, 92, 246, 255))
                .build()
                .map_err(err)?
        } else {
            return Err(format!("长截图辅助窗口不可用：{label}"));
        };
        if edge {
            window.set_ignore_cursor_events(true).map_err(err)?;
        }
        if recording {
            // Windows 映射为 WDA_EXCLUDEFROMCAPTURE，满屏时也无需反复隐藏窗口。
            window.set_content_protected(true).map_err(err)?;
        }
        // 同时覆盖复用窗口的约束；使用物理像素，避免 DPI 再次放大下限。
        window
            .set_min_size(Some(PhysicalSize::new(1u32, 1u32)))
            .map_err(err)?;
        if edge {
            // 复用窗口时先解除上次选区的最大尺寸，再设置本次尺寸。
            window
                .set_max_size(None::<PhysicalSize<u32>>)
                .map_err(err)?;
            window
                .set_max_size(Some(PhysicalSize::new(position.w, position.h)))
                .map_err(err)?;
            #[cfg(target_os = "windows")]
            {
                EDGE_SIZES
                    .lock()
                    .map_err(err)?
                    .insert((*label).to_string(), (position.w, position.h));
                clip_edge(&window, position.w, position.h)?;
            }
        }
        window
            .set_size(PhysicalSize::new(position.w, position.h))
            .map_err(err)?;
        window
            .set_position(PhysicalPosition::new(
                screen.x + position.x,
                screen.y + position.y,
            ))
            .map_err(err)?;
        let actual_size = window.outer_size().map_err(err)?;
        let actual_position = window.outer_position().map_err(err)?;
        // Windows 以裁剪区域为准，外框允许更大，但不能小于需要绘制的区域。
        // 其他平台没有原生裁剪兜底，仍要求外框尺寸完全相等。
        let invalid_edge_size = if cfg!(target_os = "windows") {
            actual_size.width < position.w || actual_size.height < position.h
        } else {
            actual_size.width != position.w || actual_size.height != position.h
        };
        if edge && invalid_edge_size {
            hide(app);
            return Err(format!(
                "选区边框尺寸异常：期望 {}×{}，实际 {}×{}。",
                position.w, position.h, actual_size.width, actual_size.height
            ));
        }
        let actual = BoxRect {
            x: actual_position.x - screen.x,
            y: actual_position.y - screen.y,
            w: if edge && cfg!(target_os = "windows") {
                position.w
            } else {
                actual_size.width
            },
            h: if edge && cfg!(target_os = "windows") {
                position.h
            } else {
                actual_size.height
            },
        };
        // 避让依据实际窗口矩形，而不是请求尺寸，避免系统调整尺寸后辅助界面被截入图片。
        if actual.overlaps(rect) {
            overlaps.push((*label).to_string());
        }
    }
    Ok(overlaps)
}

pub(crate) fn show_recording(app: &AppHandle) -> Result<(), String> {
    for label in RECORDING_LABELS {
        show_window(&window(app, label).ok_or("录屏辅助窗口不可用。")?)?;
    }
    Ok(())
}
pub(crate) fn hide_recording(app: &AppHandle) {
    for label in RECORDING_LABELS {
        if let Some(w) = window(app, label) {
            let _ = w.hide();
        }
    }
}

pub(super) fn show(app: &AppHandle) -> Result<(), String> {
    for label in LABELS {
        if let Some(w) = window(app, label) {
            show_window(&w)?;
        }
    }
    Ok(())
}
pub(crate) fn hide(app: &AppHandle) {
    for label in LABELS {
        if let Some(w) = window(app, label) {
            let _ = w.hide();
        }
    }
}

/// 只有与选区相交的辅助窗口需要避让，普通布局的四条边框会一直显示。
pub(super) struct SampleGuard(Vec<Window>);
impl Drop for SampleGuard {
    fn drop(&mut self) {
        for w in &self.0 {
            let _ = show_window(w);
        }
    }
}
pub(super) fn hide_for_sample(app: &AppHandle, labels: &[String]) -> Result<SampleGuard, String> {
    let mut guard = SampleGuard(Vec::new());
    for label in labels {
        if let Some(w) = window(app, label) {
            w.hide().map_err(err)?;
            guard.0.push(w);
        }
    }
    if !guard.0.is_empty() {
        thread::sleep(Duration::from_millis(80));
    }
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn screen(scale: f64) -> Screen {
        Screen {
            name: None,
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale: scale.to_bits(),
        }
    }
    #[test]
    fn ordinary_selection_keeps_all_chrome_outside_capture() {
        let rect = Rect {
            x: 100,
            y: 100,
            width: 900,
            height: 600,
        };
        let boxes = layout(&screen(1.0), rect);
        assert!(boxes.iter().all(|b| !b.overlaps(rect)));
        assert!(boxes[0].x > 1000);
        assert!(boxes[1].y > 700);
    }
    #[test]
    fn fullscreen_and_high_dpi_stay_on_screen_and_detect_overlap() {
        for scale in [1.0, 1.5, 2.0] {
            let screen = screen(scale);
            let rect = Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            };
            for b in layout(&screen, rect) {
                assert!(b.x >= 0 && b.y >= 0);
                assert!(b.x + b.w as i32 <= 1920 && b.y + b.h as i32 <= 1080);
                assert!(b.overlaps(rect));
            }
        }
    }
}
