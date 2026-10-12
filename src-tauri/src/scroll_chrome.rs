//! 长截图辅助窗口：边框放在选区外，不盖住原页面；右侧预览和下方操作条独立显示。
use super::{err, Rect, Screen};
use std::{thread, time::Duration};
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
    let controls_w = (320.0 * scale) as u32;
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

pub(super) fn place(app: &AppHandle, screen: &Screen, rect: Rect) -> Result<Vec<String>, String> {
    let mut overlaps = Vec::new();
    for (index, (label, position)) in LABELS.iter().zip(layout(screen, rect)).enumerate() {
        let window = if let Some(window) = window(app, label) {
            window
        } else if index >= 2 {
            // 使用稳定窗口接口加载无脚本纯色页面；不挂载业务界面，也不申请 IPC 权限。
            // WindowBuilder 本身需要 unstable，不能在当前依赖配置下直接使用。
            WebviewWindowBuilder::new(app, *label, WebviewUrl::App("scroll-edge.html".into()))
                .title("长截图选区")
                // 无边框不等于无最小尺寸：Windows 默认仍有最小跟踪宽高，
                // 不显式设置时，2px 边框会被撑成约 136px 宽、38px 高的色块。
                .min_inner_size(1.0, 1.0)
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
        if index >= 2 {
            window.set_ignore_cursor_events(true).map_err(err)?;
        }
        // 同时覆盖复用窗口的约束；使用物理像素，避免 DPI 再次放大下限。
        window
            .set_min_size(Some(PhysicalSize::new(1u32, 1u32)))
            .map_err(err)?;
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
        // 若平台仍无法创建细条窗口，停止启动并保留截图蒙层，不能显示大色块污染采集。
        if index >= 2 && (actual_size.width != position.w || actual_size.height != position.h) {
            hide(app);
            return Err(format!(
                "选区边框尺寸异常：期望 {}×{}，实际 {}×{}。",
                position.w, position.h, actual_size.width, actual_size.height
            ));
        }
        let actual = BoxRect {
            x: actual_position.x - screen.x,
            y: actual_position.y - screen.y,
            w: actual_size.width,
            h: actual_size.height,
        };
        // 避让依据实际窗口矩形，而不是请求尺寸，避免系统调整尺寸后辅助界面被截入图片。
        if actual.overlaps(rect) {
            overlaps.push((*label).to_string());
        }
    }
    Ok(overlaps)
}

pub(super) fn show(app: &AppHandle) -> Result<(), String> {
    for label in LABELS {
        if let Some(w) = window(app, label) {
            w.show().map_err(err)?;
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
            let _ = w.show();
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
