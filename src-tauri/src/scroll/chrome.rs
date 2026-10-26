//! 滚动截图辅助窗口：边框放在选区外；预览优先右侧、空间不足时左侧，操作条独立显示。
use super::super::{err, Rect};
use super::session::Screen;
#[cfg(target_os = "windows")]
use std::collections::BTreeMap;
use std::{sync::Mutex, thread, time::Duration};
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

/// 回放复用录制选区，坐标为屏幕物理像素；子区域转为百分比，前端不再重复换算 DPI。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplayLayout {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    video: ReplayRegion,
    toolbar: ReplayRegion,
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct ReplayRegion {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}
#[derive(Default)]
pub(crate) struct RecordingPreviewState(Mutex<Option<ReplayLayout>>);

/// 工具条优先放选区下方，其次上方；全屏选区在原框内等比缩小回放，为工具条留位置。
/// 窄选区也保留可用工具条宽度，透明 WebView 中未占用的区域仍显示原桌面。
fn replay_layout(screen: &Screen, rect: Rect) -> ReplayLayout {
    let scale = f64::from_bits(screen.scale);
    let gap = (8.0 * scale).round() as u32;
    let toolbar_h = ((128.0 * scale).round() as u32)
        .min(screen.height.saturating_sub(1))
        .max(1);
    let toolbar_w = rect
        .width
        .max((640.0 * scale).round() as u32)
        .min(screen.width)
        .max(1);
    let mut video = BoxRect {
        x: rect.x as i32,
        y: rect.y as i32,
        w: rect.width,
        h: rect.height,
    };
    let toolbar_y = if rect.y + rect.height + gap + toolbar_h <= screen.height {
        rect.y + rect.height + gap
    } else if rect.y >= gap + toolbar_h {
        rect.y - gap - toolbar_h
    } else {
        // 不改变选区的屏幕归属；仅缩放回放画面，不裁掉视频像素。
        let available = screen.height.saturating_sub(toolbar_h + gap).max(1);
        let ratio = (f64::from(available) / f64::from(rect.height)).min(1.0);
        video.w = (f64::from(rect.width) * ratio).round().max(1.0) as u32;
        video.h = (f64::from(rect.height) * ratio).round().max(1.0) as u32;
        video.x = (rect.x + (rect.width - video.w) / 2) as i32;
        video.y = rect.y.min(available.saturating_sub(video.h)) as i32;
        video.y as u32 + video.h + gap
    };
    let toolbar_x = (rect.x + rect.width)
        .saturating_sub(toolbar_w)
        .min(screen.width - toolbar_w);
    let x = video.x.min(toolbar_x as i32);
    let y = video.y.min(toolbar_y as i32);
    let width = (video.x + video.w as i32).max((toolbar_x + toolbar_w) as i32) - x;
    let height = (video.y + video.h as i32).max((toolbar_y + toolbar_h) as i32) - y;
    let relative = |area: BoxRect| ReplayRegion {
        left: f64::from(area.x - x) * 100.0 / f64::from(width),
        top: f64::from(area.y - y) * 100.0 / f64::from(height),
        width: f64::from(area.w) * 100.0 / f64::from(width),
        height: f64::from(area.h) * 100.0 / f64::from(height),
    };
    ReplayLayout {
        x: screen.x + x,
        y: screen.y + y,
        width: width as u32,
        height: height as u32,
        video: relative(video),
        toolbar: relative(BoxRect {
            x: toolbar_x as i32,
            y: toolbar_y as i32,
            w: toolbar_w,
            h: toolbar_h,
        }),
    }
}

pub(crate) fn recording_preview_layout(app: &AppHandle) -> Option<ReplayLayout> {
    app.state::<RecordingPreviewState>()
        .0
        .lock()
        .ok()
        .and_then(|value| value.clone())
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
    let available_left = r.x.saturating_sub(gap as u32);
    let preferred_width = (320.0 * scale) as u32;
    let minimum_width = (100.0 * scale) as u32;
    // 优先完整放在右侧，右侧不足则尝试左侧；两侧都不足时才缩窄。
    // 间距和宽度均先转换为物理像素，左侧位置以选区左边界减去间距及预览宽度。
    let (preview_x, preview_width) = if available_right >= preferred_width {
        (right + gap, preferred_width)
    } else if available_left >= preferred_width {
        (x - gap - preferred_width as i32, preferred_width)
    } else if available_right >= minimum_width {
        (right + gap, available_right)
    } else if available_left >= minimum_width {
        (x - gap - available_left as i32, available_left)
    } else {
        // 全屏或两侧都过窄：沿用屏幕内限位，遮挡选区时由取样保护器暂时隐藏。
        (right + gap, preferred_width)
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
            preview_x,
            y,
            preview_width,
            r.height
                .max((240.0 * scale) as u32)
                .min((600.0 * scale) as u32),
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
/// 仅供 capture 模块内部调用，可见范围与 Screen 保持一致。
pub(in crate::capture) fn place_recording(
    app: &AppHandle,
    screen: &Screen,
    rect: Rect,
) -> Result<(), String> {
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
    } else {
        *app.state::<RecordingPreviewState>().0.lock().map_err(err)? =
            Some(replay_layout(screen, rect));
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
            // 共用 Vue 入口挂载纯色边框组件；不挂载业务界面，也不申请业务 IPC 权限。
            // WindowBuilder 本身需要 unstable，不能在当前依赖配置下直接使用。
            WebviewWindowBuilder::new(
                app,
                *label,
                WebviewUrl::App("index.html?window=selection-edge".into()),
            )
            .title(crate::native_i18n::text(
                app,
                if recording {
                    "录屏选区"
                } else {
                    "滚动截图选区"
                },
            ))
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
            // 在原生 HWND 创建时设置，早于 WebView2 挂载和合成表面初始化。
            // 与 recording-controls 的 contentProtected 配置保持一致。
            .content_protected(recording)
            .background_color(Color(139, 92, 246, 255))
            .build()
            .map_err(err)?
        } else {
            return Err(format!("滚动截图辅助窗口不可用：{label}"));
        };
        if edge {
            window.set_ignore_cursor_events(true).map_err(err)?;
        }
        if recording {
            // Windows 映射为 WDA_EXCLUDEFROMCAPTURE，满屏时也无需反复隐藏窗口。
            #[cfg(not(target_os = "windows"))]
            window.set_content_protected(true).map_err(err)?;
            #[cfg(target_os = "windows")]
            protect_recording_window(&window)?;
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

/// 已排除的窗口只读校验，避免 WebView2 初始化后重复设置触发系统错误。
/// 创建期未生效时才补设；不接受 WDA_MONITOR（它会将窗口录成黑块）。
#[cfg(target_os = "windows")]
fn protect_recording_window(window: &Window) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
        },
    };
    let hwnd = HWND(window.hwnd().map_err(err)?.0);
    let label = window.label();
    unsafe {
        let mut affinity = 0;
        GetWindowDisplayAffinity(hwnd, &mut affinity)
            .map_err(|e| format!("无法读取录屏辅助窗口 {label} 的捕获排除状态：{e}"))?;
        if affinity == WDA_EXCLUDEFROMCAPTURE.0 {
            return Ok(());
        }
        // 不清除已经生效的保护，不把失败降级成黑块或静默录入辅助界面。
        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).map_err(|e| {
            format!("无法排除录屏辅助窗口 {label}（当前状态 0x{affinity:02X}）：{e}")
        })?;
        GetWindowDisplayAffinity(hwnd, &mut affinity)
            .map_err(|e| format!("无法确认录屏辅助窗口 {label} 的捕获排除状态：{e}"))?;
        if affinity != WDA_EXCLUDEFROMCAPTURE.0 {
            return Err(format!(
                "录屏辅助窗口 {label} 未启用捕获排除（当前状态 0x{affinity:02X}）。"
            ));
        }
    }
    Ok(())
}

pub(crate) fn show_recording(app: &AppHandle) -> Result<(), String> {
    // 先显示四条边框，再显示并激活按钮窗，避免辅助窗口的置顶顺序遮住操作区。
    for label in &RECORDING_LABELS[1..] {
        show_window(&window(app, label).ok_or("录屏辅助窗口不可用。")?)?;
    }
    let controls = window(app, "recording-controls").ok_or("录屏控制窗口不可用。")?;
    show_window(&controls)?;
    // 显示和窗口样式更新后再次校验；此时还未启动 WGC，不会泄漏辅助界面。
    #[cfg(target_os = "windows")]
    for label in RECORDING_LABELS {
        protect_recording_window(&window(app, label).ok_or("录屏辅助窗口不可用。")?)?;
    }
    controls.set_focus().map_err(err)?;
    Ok(())
}
pub(crate) fn hide_recording(app: &AppHandle) {
    for label in RECORDING_LABELS {
        if let Some(w) = window(app, label) {
            let _ = w.hide();
        }
    }
}

/// 结束后仍在原选区回放，工具条紧邻选区，不再居中打开一块独立的编辑界面。
pub(crate) fn show_recording_preview(app: &AppHandle) -> Result<(), String> {
    let controls = window(app, "recording-controls").ok_or("录屏控制窗口不可用。")?;
    let layout = recording_preview_layout(app).ok_or("录屏选区位置不可用。")?;
    controls
        .set_size(PhysicalSize::new(layout.width, layout.height))
        .map_err(err)?;
    controls
        .set_position(PhysicalPosition::new(layout.x, layout.y))
        .map_err(err)?;
    show_window(&controls)?;
    controls.set_focus().map_err(err)?;
    Ok(())
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
            layout: Vec::new(),
            name: None,
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale: scale.to_bits(),
        }
    }
    #[test]
    fn replay_stays_on_original_selection_with_dpi_and_negative_origin() {
        for scale in [1.0, 1.25, 2.0] {
            let mut screen = screen(scale);
            screen.width = (1920.0 * scale) as u32;
            screen.height = (1080.0 * scale) as u32;
            screen.x = -(screen.width as i32);
            for y in [100, 800] {
                let rect = Rect {
                    x: (300.0 * scale) as u32,
                    y: (f64::from(y) * scale) as u32,
                    width: (700.0 * scale) as u32,
                    height: (180.0 * scale) as u32,
                };
                let layout = replay_layout(&screen, rect);
                let video_x = layout.x as f64 + layout.video.left / 100.0 * f64::from(layout.width);
                let video_y = layout.y as f64 + layout.video.top / 100.0 * f64::from(layout.height);
                assert!((video_x - f64::from(screen.x + rect.x as i32)).abs() < 0.01);
                assert!((video_y - f64::from(screen.y + rect.y as i32)).abs() < 0.01);
                assert!(
                    (layout.video.width / 100.0 * f64::from(layout.width) - f64::from(rect.width))
                        .abs()
                        < 0.01
                );
                assert!(layout.x >= screen.x && layout.y >= screen.y);
                assert!(layout.x + layout.width as i32 <= screen.x + screen.width as i32);
                assert!(layout.y + layout.height as i32 <= screen.y + screen.height as i32);
            }
            let full = replay_layout(
                &screen,
                Rect {
                    x: 0,
                    y: 0,
                    width: screen.width,
                    height: screen.height,
                },
            );
            assert!(full.width <= screen.width && full.height <= screen.height);
            assert!(full.video.top + full.video.height < full.toolbar.top);
            assert!(
                (full.video.width / full.video.height * f64::from(full.width)
                    / f64::from(full.height)
                    - f64::from(screen.width) / f64::from(screen.height))
                .abs()
                    < 0.01
            );
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
    fn preview_switches_sides_and_shrinks_only_when_needed() {
        for scale in [1.0, 1.5, 2.0] {
            let pixels = |value: u32| (f64::from(value) * scale) as u32;
            let mut screen = screen(scale);
            screen.width = pixels(1920);
            screen.height = pixels(1080);
            // 右侧足够、右侧不足但左侧足够，以及两侧都不足时的左右缩窄。
            for (x, width, left, preview_width) in [
                (100, 900, false, 320),
                (900, 900, true, 320),
                (450, 1250, true, 320),
                (150, 1700, true, 142),
                (50, 1720, false, 142),
            ] {
                let rect = Rect {
                    x: pixels(x),
                    y: pixels(100),
                    width: pixels(width),
                    height: pixels(600),
                };
                let preview = layout(&screen, rect)[0];
                assert!(!preview.overlaps(rect));
                assert_eq!(preview.w, pixels(preview_width));
                if left {
                    assert_eq!(
                        preview.x + preview.w as i32 + pixels(8) as i32,
                        rect.x as i32
                    );
                } else {
                    assert_eq!(preview.x, (rect.x + rect.width + pixels(8)) as i32);
                }
            }
        }
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
