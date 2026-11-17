//! 截图与录屏选区共用物理桌面坐标、窗口候选及原生窗口身份。
use image::RgbaImage;
use serde::Serialize;

#[cfg(windows)]
#[path = "desktop_windows.rs"]
mod windows;
#[cfg(windows)]
pub use windows::AppWindow as NativeWindow;
#[cfg(windows)]
pub(crate) use windows::PhysicalDpi;
#[cfg(not(windows))]
pub type NativeWindow = xcap::Window;
#[cfg(target_os = "macos")]
#[path = "desktop_cursor_macos.rs"]
mod cursor;
#[cfg(target_os = "macos")]
#[path = "desktop_accessibility_macos.rs"]
mod accessibility;
#[cfg(not(any(windows, target_os = "macos")))]
#[path = "desktop_linux.rs"]
mod linux;
#[cfg(not(any(windows, target_os = "macos")))]
pub use linux::capture;
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) use linux::window_frame_of as linux_window_frame;

/// 把当前光标叠加到截图上；`origin` 为截图左上角的全局点坐标。读不到光标时保持原图。
#[cfg(target_os = "macos")]
fn draw_cursor(image: &mut RgbaImage, origin: (f64, f64), scale: f64) {
    if let Some(cursor) = cursor::current(origin, scale) {
        image::imageops::overlay(image, &cursor.image, cursor.left, cursor.top);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    pub scale: f64,
}

/// 候选坐标相对虚拟桌面左上角；window_index 用于前端按父窗口分组，不暴露 HWND。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub rect: crate::capture::Rect,
    pub kind: &'static str,
    pub window_index: usize,
    /// 仅用于录屏来源选择，在本机界面显示，不写入日志；子控件为空。
    pub title: String,
    pub app_name: String,
    /// 最小化、托盘隐藏或屏幕外的应用仅参与录屏来源列表，不参与截图吸附。
    pub recording_only: bool,
    /// 受限的应用图标 PNG Base64，只用于来源菜单。
    pub icon: Option<String>,
}

pub struct Desktop {
    pub image: RgbaImage,
    pub x: i32,
    pub y: i32,
    pub displays: Vec<Display>,
    pub targets: Vec<Target>,
    pub windows: Vec<NativeWindow>,
    pub include_cursor: bool,
}

/// 减去虚拟桌面原点后再裁剪，支持位于主屏左侧、上方的负坐标屏幕。
pub fn clipped(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    origin: (i32, i32),
    size: (u32, u32),
) -> Option<crate::capture::Rect> {
    let left = (i64::from(x) - i64::from(origin.0)).max(0);
    let top = (i64::from(y) - i64::from(origin.1)).max(0);
    let right = (i64::from(x) - i64::from(origin.0) + i64::from(width)).min(i64::from(size.0));
    let bottom = (i64::from(y) - i64::from(origin.1) + i64::from(height)).min(i64::from(size.1));
    (right > left && bottom > top).then(|| crate::capture::Rect {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

/// 某个窗口内的控件候选：边界先换算为截图像素，再裁剪到父窗口并去重。
/// Windows（Win32 子窗口、UIA）与 macOS（辅助功能）共用这套规则。
#[cfg(any(windows, target_os = "macos"))]
pub(crate) struct Elements {
    pub targets: Vec<Target>,
    pub index: usize,
    pub parent: crate::capture::Rect,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}
#[cfg(any(windows, target_os = "macos"))]
impl Elements {
    /// 参数为截图所用的全局像素坐标（右、下边界不含）。
    pub fn push_edges(&mut self, left: i32, top: i32, right: i32, bottom: i32) {
        if right <= left || bottom <= top {
            return;
        }
        let Some(mut rect) = clipped(
            left,
            top,
            (i64::from(right) - i64::from(left)).min(u32::MAX as i64) as u32,
            (i64::from(bottom) - i64::from(top)).min(u32::MAX as i64) as u32,
            self.origin,
            self.size,
        ) else {
            return;
        };
        let right = (rect.x + rect.width).min(self.parent.x + self.parent.width);
        let bottom = (rect.y + rect.height).min(self.parent.y + self.parent.height);
        rect.x = rect.x.max(self.parent.x);
        rect.y = rect.y.max(self.parent.y);
        if right <= rect.x || bottom <= rect.y {
            return;
        }
        rect.width = right - rect.x;
        rect.height = bottom - rect.y;
        // 忽略文本碎片、与整窗重合的容器和重复边界，但遍历器仍会深入这些容器。
        if rect.width < 12
            || rect.height < 12
            || same_bounds(rect, self.parent)
            || self
                .targets
                .iter()
                .any(|target| same_bounds(target.rect, rect))
        {
            return;
        }
        self.targets.push(Target {
            rect,
            kind: "element",
            window_index: self.index,
            title: String::new(),
            app_name: String::new(),
            recording_only: false,
            icon: None,
        });
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn same_bounds(a: crate::capture::Rect, b: crate::capture::Rect) -> bool {
    a.x.abs_diff(b.x) <= 2
        && a.y.abs_diff(b.y) <= 2
        && (a.x + a.width).abs_diff(b.x + b.width) <= 2
        && (a.y + a.height).abs_diff(b.y + b.height) <= 2
}

/// 将虚拟桌面选区转换为单屏局部像素，跨屏或空选区必须拒绝。
pub fn within_display(
    rect: crate::capture::Rect,
    origin: (i32, i32),
    display: &Display,
) -> Option<crate::capture::Rect> {
    let x = i64::from(rect.x) + i64::from(origin.0) - i64::from(display.x);
    let y = i64::from(rect.y) + i64::from(origin.1) - i64::from(display.y);
    if rect.width == 0
        || rect.height == 0
        || x < 0
        || y < 0
        || x + i64::from(rect.width) > i64::from(display.width)
        || y + i64::from(rect.height) > i64::from(display.height)
    {
        return None;
    }
    Some(crate::capture::Rect {
        x: x as u32,
        y: y as u32,
        ..rect
    })
}

/// 跨屏窗口的控制条放在可见面积最大的显示器上；返回虚拟桌面图片内的交集。
/// 该裁剪仅用于辅助窗口布局，WGC 仍采集完整窗口，不能据此裁剪视频。
pub fn display_intersection(
    rect: crate::capture::Rect,
    origin: (i32, i32),
    displays: &[Display],
) -> Option<crate::capture::Rect> {
    displays
        .iter()
        .filter_map(|d| {
            let left = i64::from(rect.x).max(i64::from(d.x) - i64::from(origin.0));
            let top = i64::from(rect.y).max(i64::from(d.y) - i64::from(origin.1));
            let right = (i64::from(rect.x) + i64::from(rect.width))
                .min(i64::from(d.x) - i64::from(origin.0) + i64::from(d.width));
            let bottom = (i64::from(rect.y) + i64::from(rect.height))
                .min(i64::from(d.y) - i64::from(origin.1) + i64::from(d.height));
            (right > left && bottom > top).then(|| crate::capture::Rect {
                x: left as u32,
                y: top as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            })
        })
        .max_by_key(|r| u64::from(r.width) * u64::from(r.height))
}

/// Windows 使用物理像素拼接，不按各屏 DPI 再缩放位图。
/// 其他平台暂沿用主屏路径，避免将 macOS 点坐标与 Retina 像素直接混合。
#[cfg(windows)]
pub fn capture(primary_only: bool, include_cursor: bool, snap: bool) -> Result<Desktop, String> {
    let _dpi = windows::PhysicalDpi::enter()?;
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let mut displays = Vec::new();
    for monitor in &monitors {
        let primary = monitor.is_primary().map_err(|e| e.to_string())?;
        if primary_only && !primary {
            continue;
        }
        displays.push(Display {
            x: monitor.x().map_err(|e| e.to_string())?,
            y: monitor.y().map_err(|e| e.to_string())?,
            width: monitor.width().map_err(|e| e.to_string())?,
            height: monitor.height().map_err(|e| e.to_string())?,
            primary,
            scale: f64::from(monitor.scale_factor().map_err(|e| e.to_string())?),
        });
    }
    displays.sort_by_key(|d| (d.x, d.y, d.width, d.height));
    let x = displays
        .iter()
        .map(|d| d.x)
        .min()
        .ok_or("没有可用显示器。")?;
    let y = displays
        .iter()
        .map(|d| d.y)
        .min()
        .ok_or("没有可用显示器。")?;
    let right = displays
        .iter()
        .map(|d| i64::from(d.x) + i64::from(d.width))
        .max()
        .unwrap();
    let bottom = displays
        .iter()
        .map(|d| i64::from(d.y) + i64::from(d.height))
        .max()
        .unwrap();
    let width = u32::try_from(right - i64::from(x)).map_err(|_| "桌面宽度超出限制。")?;
    let height = u32::try_from(bottom - i64::from(y)).map_err(|_| "桌面高度超出限制。")?;
    if u64::from(width) * u64::from(height) > 64_000_000 || width == 0 || height == 0 {
        return Err("虚拟桌面超过 6400 万像素，请降低屏幕分辨率或减少显示器。".into());
    }
    let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for monitor in monitors {
        let mx = monitor.x().map_err(|e| e.to_string())?;
        let my = monitor.y().map_err(|e| e.to_string())?;
        let primary = monitor.is_primary().map_err(|e| e.to_string())?;
        if primary_only && !primary {
            continue;
        }
        let shot = monitor.capture_image().map_err(|e| e.to_string())?;
        if !displays
            .iter()
            .any(|d| (d.x, d.y, d.width, d.height) == (mx, my, shot.width(), shot.height()))
            || shot.dimensions()
                != (
                    monitor.width().map_err(|e| e.to_string())?,
                    monitor.height().map_err(|e| e.to_string())?,
                )
        {
            return Err("显示器尺寸发生变化或像素坐标不匹配，请重新截图。".into());
        }
        image::imageops::replace(
            &mut image,
            &shot,
            i64::from(mx) - i64::from(x),
            i64::from(my) - i64::from(y),
        );
    }
    if include_cursor {
        windows::draw_cursor(&mut image, x, y)?;
    }
    let mut desktop = Desktop {
        image,
        x,
        y,
        displays,
        targets: Vec::new(),
        windows: Vec::new(),
        include_cursor,
    };
    if !primary_only && snap {
        // 只为截图吸附枚举可见窗口；录屏直接使用屏幕选区。
        for window in windows::application_windows()?.into_iter().take(256) {
            let visible = window.is_visible() && !window.is_minimized().unwrap_or(true);
            let on_screen = if visible {
                window
                    .x()
                    .and_then(|wx| Ok((wx, window.y()?, window.width()?, window.height()?)))
                    .ok()
                    .and_then(|(wx, wy, ww, wh)| clipped(wx, wy, ww, wh, (x, y), (width, height)))
            } else {
                None
            };
            let Some(rect) = on_screen else {
                continue;
            };
            let index = desktop.windows.len();
            desktop.targets.push(Target {
                rect,
                kind: "window",
                window_index: index,
                recording_only: false,
                icon: window.icon(),
                title: window
                    .title()
                    .unwrap_or_default()
                    .chars()
                    .take(160)
                    .collect(),
                app_name: window
                    .app_name()
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect(),
            });
            if let Some(id) = window.id().ok()
            {
                let budget = 4096usize.saturating_sub(desktop.targets.len());
                desktop.targets.extend(
                    windows::elements(id, index, rect, (x, y), (width, height))
                        .into_iter()
                        .take(budget),
                );
            }
            desktop.windows.push(window);
        }
        windows::accessibility_elements(&desktop.windows, &mut desktop.targets, (x, y), (width, height));
    }
    Ok(desktop)
}

/// macOS 截取鼠标所在的那块屏幕（无鼠标位置时回退主屏）。
/// 一个蒙层窗口不能跨越多块各自拥有桌面空间的显示器，因此先按屏截取，跨屏框选另行实现。
/// Display 的 x/y 为“点坐标 × 该屏实际缩放”，宽高为 Retina 像素，scale 为像素/点之比；
/// 调用方据此把蒙层放到点坐标 (x/scale, y/scale)，不依赖窗口当前所在屏的缩放。
#[cfg(target_os = "macos")]
pub fn capture(primary_only: bool, include_cursor: bool, snap: bool) -> Result<Desktop, String> {
    let monitors = xcap::Monitor::all().map_err(crate::shared::platform::capture_error)?;
    let cursor = (!primary_only)
        .then(crate::shared::macos::cursor_point)
        .flatten();
    // 点坐标的屏幕边界；任一属性读取失败的显示器直接跳过，不影响其余屏幕。
    let bounds = |m: &xcap::Monitor| -> Option<(f64, f64, f64, f64)> {
        Some((
            f64::from(m.x().ok()?),
            f64::from(m.y().ok()?),
            f64::from(m.width().ok()?),
            f64::from(m.height().ok()?),
        ))
    };
    let under_cursor = cursor.and_then(|(cx, cy)| {
        monitors.iter().find(|&m| {
            bounds(m).is_some_and(|(x, y, w, h)| cx >= x && cx < x + w && cy >= y && cy < y + h)
        })
    });
    let monitor = under_cursor
        .or_else(|| monitors.iter().find(|m| m.is_primary().unwrap_or(false)))
        .or(monitors.first())
        .ok_or("未找到可用显示器，请检查显示器连接。")?;
    let (x, y, width, _) = bounds(monitor).ok_or("读取显示器位置失败。")?;
    let mut image = monitor
        .capture_image()
        .map_err(crate::shared::platform::capture_error)?;
    // 以实际像素宽度反推缩放，比系统报告的 scale_factor 更贴合截图位图（含缩放分辨率模式）。
    let scale = if width > 0.0 {
        f64::from(image.width()) / width
    } else {
        1.0
    };
    let (origin_x, origin_y) = ((x * scale).round() as i32, (y * scale).round() as i32);
    if include_cursor {
        draw_cursor(&mut image, (x, y), scale);
    }
    let size = image.dimensions();
    let mut desktop = Desktop {
        x: origin_x,
        y: origin_y,
        displays: vec![Display {
            x: origin_x,
            y: origin_y,
            width: image.width(),
            height: image.height(),
            primary: monitor.is_primary().unwrap_or(false),
            scale,
        }],
        image,
        targets: Vec::new(),
        windows: Vec::new(),
        include_cursor,
    };
    if snap && !primary_only {
        snap_windows(&mut desktop, scale, size);
    }
    Ok(desktop)
}

/// 点坐标的窗口边界换算为截图像素：与蒙层原点使用同一缩放和取整，
/// 原生窗口截图（native_window_inner）也按此换算，两者才能逐像素对齐。
#[cfg(target_os = "macos")]
fn window_pixels(window: &NativeWindow, scale: f64) -> Option<(i32, i32, u32, u32)> {
    let px = |value: f64| (value * scale).round();
    Some((
        px(f64::from(window.x().ok()?)) as i32,
        px(f64::from(window.y().ok()?)) as i32,
        px(f64::from(window.width().ok()?)) as u32,
        px(f64::from(window.height().ok()?)) as u32,
    ))
}

/// 吸附候选只取普通层级、位于本屏且未最小化的窗口；顺序沿用系统的前后层次（前者在上）。
/// 授予“辅助功能”权限后，再从这些窗口的辅助功能树补充控件候选。
#[cfg(target_os = "macos")]
fn snap_windows(desktop: &mut Desktop, scale: f64, size: (u32, u32)) {
    let Ok(windows) = xcap::Window::all() else {
        return;
    };
    let normal = crate::shared::macos::normal_window_ids();
    let own = std::process::id();
    for window in windows.into_iter().take(256) {
        let eligible = window.id().is_ok_and(|id| normal.contains(&id))
            && window.pid().is_ok_and(|pid| pid != own)
            && !window.is_minimized().unwrap_or(true);
        // 小于 40 点的窗口多为工具提示或浮标，吸附反而干扰框选。
        let Some(rect) = eligible
            .then(|| window_pixels(&window, scale))
            .flatten()
            .filter(|(_, _, w, h)| f64::from(*w) >= 40.0 * scale && f64::from(*h) >= 40.0 * scale)
            .and_then(|(wx, wy, ww, wh)| clipped(wx, wy, ww, wh, (desktop.x, desktop.y), size))
        else {
            continue;
        };
        let index = desktop.windows.len();
        desktop.targets.push(Target {
            rect,
            kind: "window",
            window_index: index,
            recording_only: false,
            icon: None,
            title: window.title().unwrap_or_default().chars().take(160).collect(),
            app_name: window.app_name().unwrap_or_default().chars().take(80).collect(),
        });
        desktop.windows.push(window);
    }
    let origin = (desktop.x, desktop.y);
    accessibility::collect(&desktop.windows, &mut desktop.targets, origin, size, scale);
}

/// 原生窗口抓取不接受任意窗口句柄，只允许当前会话已识别的窗口。
pub fn native_window(
    window: &NativeWindow,
    include_cursor: bool,
) -> Result<(RgbaImage, i32, i32), String> {
    // PrintWindow 可能等待目标进程。超时后主会话恢复，后台线程持有自己的 GDI 资源直到返回；
    // 全局限一个原生任务，避免对无响应窗口反复点击创建无界线程。
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("上次原生窗口截图仍在等待目标响应，请使用区域截图。".into());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let window = window.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    if let Err(error) = std::thread::Builder::new()
        .name("native-window-capture".into())
        .spawn(move || {
            let _reset = Reset;
            let _ = sender.send(native_window_inner(&window, include_cursor));
        })
    {
        RUNNING.store(false, Ordering::SeqCst);
        return Err(error.to_string());
    }
    receiver
        .recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| "原生窗口截图超时或中断，请使用区域截图。".to_string())?
}

fn native_window_inner(
    window: &NativeWindow,
    include_cursor: bool,
) -> Result<(RgbaImage, i32, i32), String> {
    #[cfg(windows)]
    let _dpi = windows::PhysicalDpi::enter()?;
    if window.is_minimized().map_err(|e| e.to_string())? {
        return Err("目标窗口已最小化，请重新截图。".into());
    }
    // macOS 的点坐标需换算为像素，单独处理；X11 需按外框补齐窗口装饰。
    #[cfg(target_os = "macos")]
    return native_window_points(window, include_cursor);
    #[cfg(not(any(windows, target_os = "macos")))]
    return linux::native_window(window, include_cursor);
    #[cfg(windows)]
    let x = window.x().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let y = window.y().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if u64::from(window.width().map_err(|e| e.to_string())?)
        * u64::from(window.height().map_err(|e| e.to_string())?)
        > 64_000_000
    {
        return Err("窗口超过 6400 万像素，请缩小窗口后重试。".into());
    }
    #[cfg(windows)]
    let mut image = windows::capture_window(
        window.id().map_err(|e| e.to_string())?,
        x,
        y,
        window.width().map_err(|e| e.to_string())?,
        window.height().map_err(|e| e.to_string())?,
    )?;
    #[cfg(windows)]
    if image.dimensions()
        != (
            window.width().map_err(|e| e.to_string())?,
            window.height().map_err(|e| e.to_string())?,
        )
    {
        return Err("原生窗口图片与窗口尺寸不一致，请使用区域截图。".into());
    }
    #[cfg(windows)]
    if include_cursor {
        windows::draw_cursor(&mut image, x, y)?;
    }
    #[cfg(windows)]
    return Ok((image, x, y));
}

/// macOS 的窗口边界是点坐标，截图位图却是 Retina 像素：按位图宽度反推该屏缩放，
/// 再用 window_pixels 的同一取整规则得到像素位置，与截图会话中的候选框逐像素一致。
#[cfg(target_os = "macos")]
fn native_window_points(
    window: &NativeWindow,
    include_cursor: bool,
) -> Result<(RgbaImage, i32, i32), String> {
    let image = window
        .capture_image()
        .map_err(|e| format!("原生窗口截图失败：{e}"))?;
    let width = window.width().map_err(|e| e.to_string())?;
    if width == 0 {
        return Err("原生窗口图片与窗口尺寸不一致，请使用区域截图。".into());
    }
    let scale = f64::from(image.width()) / f64::from(width);
    let (x, y, w, h) = window_pixels(window, scale).ok_or("读取窗口位置失败。")?;
    // 宽度按定义一致；高度允许 1 像素取整误差，超出说明窗口在截图过程中改变了尺寸。
    if w != image.width() || h.abs_diff(image.height()) > 1 {
        return Err("原生窗口图片与窗口尺寸不一致，请使用区域截图。".into());
    }
    let mut image = if h == image.height() {
        image
    } else {
        image::imageops::resize(&image, w, h, image::imageops::FilterType::Nearest)
    };
    if include_cursor {
        // 与 window_pixels 一致：窗口原点的全局点坐标即像素位置除以缩放。
        let origin = (f64::from(x) / scale, f64::from(y) / scale);
        draw_cursor(&mut image, origin, scale);
    }
    Ok((image, x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_screen_window_placement_uses_largest_visible_intersection() {
        let displays = vec![
            Display {
                x: -1920,
                y: -200,
                width: 1920,
                height: 1080,
                primary: false,
                scale: 1.25,
            },
            Display {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                primary: true,
                scale: 1.5,
            },
        ];
        let window = crate::capture::Rect {
            x: 1800,
            y: 100,
            width: 600,
            height: 500,
        };
        let placement = display_intersection(window, (-1920, -200), &displays).unwrap();
        assert_eq!(
            (placement.x, placement.y, placement.width, placement.height),
            (1920, 200, 480, 400)
        );
        assert!(within_display(placement, (-1920, -200), &displays[1]).is_some());
        assert!(display_intersection(
            crate::capture::Rect {
                x: 2100,
                y: 0,
                width: 40,
                height: 40
            },
            (-1920, -200),
            &displays
        )
        .is_none());
    }
    #[test]
    fn secondary_region_uses_physical_pixels_with_negative_xy_and_mixed_dpi() {
        let display = Display {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1080,
            primary: false,
            scale: 1.25,
        };
        let rect = crate::capture::Rect {
            x: 80,
            y: 40,
            width: 400,
            height: 300,
        };
        let local = within_display(rect, (-1920, -200), &display).unwrap();
        assert_eq!(
            (local.x, local.y, local.width, local.height),
            (80, 40, 400, 300)
        );
        assert!(within_display(
            crate::capture::Rect { x: 1800, ..rect },
            (-1920, -200),
            &display
        )
        .is_none());
        // DPI 只用于浮窗布局，不能改变截图像素和来源内偏移。
        let other_scale = Display {
            scale: 1.75,
            ..display
        };
        let other = within_display(rect, (-1920, -200), &other_scale).unwrap();
        assert_eq!(
            (other.x, other.y, other.width, other.height),
            (80, 40, 400, 300)
        );
    }
    #[test]
    fn clips_negative_origin_and_offscreen_window() {
        let r = clipped(-2000, -100, 300, 200, (-1920, 0), (4480, 1440)).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 220, 100));
        assert!(clipped(5000, 0, 20, 20, (-1920, 0), (4480, 1440)).is_none());
    }
    #[test]
    fn primary_region_translates_and_rejects_cross_screen() {
        let display = Display {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            primary: true,
            scale: 1.5,
        };
        let rect = crate::capture::Rect {
            x: 2000,
            y: 200,
            width: 400,
            height: 300,
        };
        let local = within_display(rect, (-1920, -100), &display).unwrap();
        assert_eq!(
            (local.x, local.y, local.width, local.height),
            (80, 100, 400, 300)
        );
        assert!(within_display(
            crate::capture::Rect { x: 1800, ..rect },
            (-1920, -100),
            &display
        )
        .is_none());
        assert!(within_display(
            crate::capture::Rect {
                width: 3000,
                ..rect
            },
            (-1920, -100),
            &display
        )
        .is_none());
    }
}
