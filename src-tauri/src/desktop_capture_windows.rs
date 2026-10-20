//! Win32 元素边界与光标合成；所有系统资源在失败路径上也由 Drop 释放。
use super::{clipped, Target};
use image::RgbaImage;
use windows::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

/// 后台线程池会复用线程，所以退出采集时必须恢复旧 DPI 上下文。
pub struct PhysicalDpi(DPI_AWARENESS_CONTEXT);
impl PhysicalDpi {
    pub fn enter() -> Result<Self, String> {
        let old =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if old.0.is_null() {
            Err("无法启用物理像素截图坐标。".into())
        } else {
            Ok(Self(old))
        }
    }
}
impl Drop for PhysicalDpi {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

struct Elements {
    targets: Vec<Target>,
    index: usize,
    parent: crate::capture::Rect,
    origin: (i32, i32),
    size: (u32, u32),
}
unsafe extern "system" fn child(hwnd: HWND, context: LPARAM) -> BOOL {
    // EnumChildWindows 同步回调，context 指向本次调用栈中的独占状态。
    let state = unsafe { &mut *(context.0 as *mut Elements) };
    if state.targets.len() >= 128 {
        return BOOL(0);
    }
    let mut r = RECT::default();
    if !unsafe { IsWindowVisible(hwnd) }.as_bool()
        || unsafe { GetWindowRect(hwnd, &mut r) }.is_err()
        || r.right <= r.left
        || r.bottom <= r.top
    {
        return BOOL(1);
    }
    if let Some(mut rect) = clipped(
        r.left,
        r.top,
        (r.right - r.left) as u32,
        (r.bottom - r.top) as u32,
        state.origin,
        state.size,
    ) {
        let right = (rect.x + rect.width).min(state.parent.x + state.parent.width);
        let bottom = (rect.y + rect.height).min(state.parent.y + state.parent.height);
        rect.x = rect.x.max(state.parent.x);
        rect.y = rect.y.max(state.parent.y);
        if right > rect.x && bottom > rect.y {
            rect.width = right - rect.x;
            rect.height = bottom - rect.y;
            state.targets.push(Target {
                rect,
                kind: "element",
                window_index: state.index,
            });
        }
    }
    BOOL(1)
}

/// 支持系统原生子窗口控件；浏览器 DOM、Canvas 等无独立 HWND 的元素回退到窗口。
pub fn elements(
    id: u32,
    index: usize,
    parent: crate::capture::Rect,
    origin: (i32, i32),
    size: (u32, u32),
) -> Vec<Target> {
    let mut state = Elements {
        targets: Vec::new(),
        index,
        parent,
        origin,
        size,
    };
    unsafe {
        let _ = EnumChildWindows(
            Some(HWND(id as usize as *mut _)),
            Some(child),
            LPARAM(&mut state as *mut _ as isize),
        );
    }
    state.targets
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.old.0.is_null() {
                SelectObject(self.dc, self.old);
            }
            if !self.bitmap.0.is_null() {
                let _ = DeleteObject(self.bitmap.into());
            }
            let _ = DeleteDC(self.dc);
        }
    }
}
struct IconInfo(ICONINFO);
impl Drop for IconInfo {
    fn drop(&mut self) {
        unsafe {
            if !self.0.hbmMask.0.is_null() {
                let _ = DeleteObject(self.0.hbmMask.into());
            }
            if !self.0.hbmColor.0.is_null() {
                let _ = DeleteObject(self.0.hbmColor.into());
            }
        }
    }
}

/// 通过系统窗口重绘接口获取完整窗口位图，再裁掉 DWM 不可见边框。
/// 不使用桌面区域回退，目标拒绝原生抓取时让用户明确选择普通区域截图。
pub fn capture_window(
    id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<RgbaImage, String> {
    use ::windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    unsafe {
        let hwnd = HWND(id as usize as *mut _);
        if !IsWindow(Some(hwnd)).as_bool() || IsHungAppWindow(hwnd).as_bool() {
            return Err("目标窗口已关闭或无响应，请使用区域截图。".into());
        }
        let mut outer = RECT::default();
        GetWindowRect(hwnd, &mut outer).map_err(|e| e.to_string())?;
        let w = outer.right - outer.left;
        let h = outer.bottom - outer.top;
        if w <= 0 || h <= 0 || i64::from(w) * i64::from(h) > 64_000_000 {
            return Err("原生窗口尺寸无效或过大。".into());
        }
        let left = x - outer.left;
        let top = y - outer.top;
        if left < 0
            || top < 0
            || i64::from(left) + i64::from(width) > i64::from(w)
            || i64::from(top) + i64::from(height) > i64::from(h)
        {
            return Err("窗口边界已变化，请重新截图。".into());
        }
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return Err("无法创建窗口截图画布。".into());
        }
        let mut surface = Surface {
            dc,
            bitmap: HBITMAP::default(),
            old: HGDIOBJ::default(),
        };
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        surface.bitmap = CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| e.to_string())?;
        surface.old = SelectObject(dc, surface.bitmap.into());
        if surface.old.0.is_null() || surface.old.0 as isize == -1 || bits.is_null() {
            surface.old = HGDIOBJ::default();
            return Err("无法选择窗口截图位图。".into());
        }
        if !PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)).as_bool() {
            return Err("目标不支持原生窗口截图，请使用区域截图。".into());
        }
        let _ = GdiFlush();
        let buffer = std::slice::from_raw_parts(bits as *const u8, w as usize * h as usize * 4);
        let mut image = RgbaImage::new(width, height);
        for (px, py, pixel) in image.enumerate_pixels_mut() {
            let offset =
                ((py as usize + top as usize) * w as usize + px as usize + left as usize) * 4;
            *pixel = image::Rgba([buffer[offset + 2], buffer[offset + 1], buffer[offset], 255]);
        }
        Ok(image)
    }
}

/// 直接在已有像素的 DIB 上绘制，兼容彩色、半透明及单色 XOR 光标。
/// 图标热点从屏幕位置扣除后再减去虚拟桌面原点；系统负责边缘裁剪。
pub fn draw_cursor(image: &mut RgbaImage, origin_x: i32, origin_y: i32) -> Result<(), String> {
    unsafe {
        let mut cursor = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        GetCursorInfo(&mut cursor).map_err(|e| e.to_string())?;
        if cursor.flags != CURSOR_SHOWING {
            return Ok(());
        }
        let mut info = IconInfo(ICONINFO::default());
        GetIconInfo(HICON(cursor.hCursor.0), &mut info.0).map_err(|e| e.to_string())?;
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return Err("无法创建光标合成画布。".into());
        }
        let mut surface = Surface {
            dc,
            bitmap: HBITMAP::default(),
            old: HGDIOBJ::default(),
        };
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: image.width() as i32,
                biHeight: -(image.height() as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        surface.bitmap = CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| e.to_string())?;
        surface.old = SelectObject(dc, surface.bitmap.into());
        if surface.old.0.is_null() || surface.old.0 as isize == -1 || bits.is_null() {
            surface.old = HGDIOBJ::default();
            return Err("无法选择光标合成位图。".into());
        }
        {
            let buffer = std::slice::from_raw_parts_mut(bits as *mut u8, image.as_raw().len());
            for (out, pixel) in buffer.chunks_exact_mut(4).zip(image.pixels()) {
                out.copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            }
        }
        DrawIconEx(
            dc,
            cursor.ptScreenPos.x - origin_x - info.0.xHotspot as i32,
            cursor.ptScreenPos.y - origin_y - info.0.yHotspot as i32,
            HICON(cursor.hCursor.0),
            0,
            0,
            0,
            None,
            DI_NORMAL,
        )
        .map_err(|e| e.to_string())?;
        let _ = GdiFlush();
        // GDI 完成写入之后重新借用内存，避免与上面的可变切片同时访问。
        let buffer = std::slice::from_raw_parts(bits as *const u8, image.as_raw().len());
        for (pixel, source) in image.pixels_mut().zip(buffer.chunks_exact(4)) {
            *pixel = image::Rgba([source[2], source[1], source[0], 255]);
        }
        Ok(())
    }
}
