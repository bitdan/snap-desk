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

#[cfg(not(windows))]
pub(super) fn set(_: &WebviewWindow, _: &[Rect]) -> Result<(), String> {
    Err("当前平台尚不支持录屏标注的局部交互区域。".into())
}
#[cfg(not(windows))]
pub(super) fn reset(_: &WebviewWindow) -> Result<(), String> {
    Ok(())
}
