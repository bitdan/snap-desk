//! 截图与录屏选区共用物理桌面坐标、窗口候选及原生窗口身份。
use image::RgbaImage;
use serde::Serialize;

#[cfg(windows)]
#[path = "desktop_capture_windows.rs"]
mod windows;
#[cfg(windows)]
pub use windows::AppWindow as NativeWindow;
#[cfg(windows)]
pub(crate) use windows::{activate_window, PhysicalDpi};
#[cfg(not(windows))]
pub type NativeWindow = xcap::Window;

#[cfg(not(windows))]
pub(crate) fn activate_window(_window: &NativeWindow) -> Result<(), String> {
    Err("当前平台暂不支持应用窗口录制。".into())
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
    if !primary_only {
        // 窗口候选同时服务录屏来源选择；截图关闭吸附时仍可切换到窗口录屏。
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
            let recording_only = on_screen.is_none();
            // 未恢复的窗口没有可吸附坐标，选择后由 select_recording_window 更新真实范围。
            let rect = on_screen.unwrap_or(crate::capture::Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
            let index = desktop.windows.len();
            desktop.targets.push(Target {
                rect,
                kind: "window",
                window_index: index,
                recording_only,
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
            if let Some(id) = (snap && !recording_only)
                .then(|| window.id().ok())
                .flatten()
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
    }
    Ok(desktop)
}

#[cfg(not(windows))]
pub fn capture(_primary_only: bool, include_cursor: bool, _snap: bool) -> Result<Desktop, String> {
    if include_cursor {
        return Err("当前平台暂不支持截图包含光标。".into());
    }
    Ok(Desktop {
        image: crate::platform::capture_primary()?,
        x: 0,
        y: 0,
        displays: Vec::new(),
        targets: Vec::new(),
        windows: Vec::new(),
        include_cursor: false,
    })
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
    let x = window.x().map_err(|e| e.to_string())?;
    let y = window.y().map_err(|e| e.to_string())?;
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
    #[cfg(not(windows))]
    let mut image = window
        .capture_image()
        .map_err(|e| format!("原生窗口截图失败：{e}"))?;
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
