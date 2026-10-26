//! Win32 元素边界与光标合成；所有系统资源在失败路径上也由 Drop 释放。
use super::{clipped, Target};
use image::RgbaImage;
use windows::{
    core::{BOOL, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, RECT},
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        Graphics::Gdi::*,
        System::Threading::{
            AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

/// 自己保存 HWND 与首次枚举的 PID，避免枚举库丢弃托盘窗口，也防止句柄被复用后录错应用。
#[derive(Clone)]
pub struct AppWindow {
    id: u32,
    pid: u32,
    title: String,
    app_name: String,
}
impl AppWindow {
    fn hwnd(&self) -> HWND {
        HWND(self.id as usize as *mut _)
    }
    fn validate(&self) -> Result<(), String> {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(self.hwnd(), Some(&mut pid));
        }
        if pid != self.pid || !unsafe { IsWindow(Some(self.hwnd())) }.as_bool() {
            return Err("应用窗口已关闭或被替换，请重新选择。".into());
        }
        Ok(())
    }
    fn bounds(&self) -> Result<RECT, String> {
        self.validate()?;
        let mut rect = RECT::default();
        unsafe {
            if DwmGetWindowAttribute(
                self.hwnd(),
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut rect as *mut RECT as *mut _,
                std::mem::size_of::<RECT>() as u32,
            )
            .is_err()
            {
                GetWindowRect(self.hwnd(), &mut rect).map_err(|e| e.to_string())?;
            }
        }
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Err("应用窗口尺寸无效。".into());
        }
        Ok(rect)
    }
    pub fn id(&self) -> Result<u32, String> {
        self.validate()?;
        Ok(self.id)
    }
    pub fn pid(&self) -> Result<u32, String> {
        self.validate()?;
        Ok(self.pid)
    }
    pub fn title(&self) -> Result<String, String> {
        Ok(self.title.clone())
    }
    pub fn app_name(&self) -> Result<String, String> {
        Ok(self.app_name.clone())
    }
    pub fn x(&self) -> Result<i32, String> {
        Ok(self.bounds()?.left)
    }
    pub fn y(&self) -> Result<i32, String> {
        Ok(self.bounds()?.top)
    }
    pub fn width(&self) -> Result<u32, String> {
        let r = self.bounds()?;
        Ok((r.right - r.left) as u32)
    }
    pub fn height(&self) -> Result<u32, String> {
        let r = self.bounds()?;
        Ok((r.bottom - r.top) as u32)
    }
    pub fn is_minimized(&self) -> Result<bool, String> {
        self.validate()?;
        Ok(unsafe { IsIconic(self.hwnd()) }.as_bool())
    }
    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd()) }.as_bool()
    }
}

/// 用进程可执行文件名标识应用；查询权限不足时仍保留窗口，标题足以供用户识别。
fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(handle);
        if result.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

struct WindowList {
    windows: Vec<AppWindow>,
    names: std::collections::HashMap<u32, String>,
}
unsafe extern "system" fn application_window(hwnd: HWND, context: LPARAM) -> BOOL {
    // EnumWindows 同步调用；context 仅借用本次枚举栈上的列表，不跨线程保留。
    let state = unsafe { &mut *(context.0 as *mut WindowList) };
    if state.windows.len() >= 256 {
        return BOOL(0);
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == std::process::id() {
        return BOOL(1);
    }
    let mut title = [0u16; 512];
    let length = unsafe { GetWindowTextW(hwnd, &mut title) }.max(0) as usize;
    let title = String::from_utf16_lossy(&title[..length]);
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let style = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32);
    if title.trim().is_empty()
        && (style.contains(WS_EX_TOOLWINDOW) || (!visible && !style.contains(WS_EX_APPWINDOW)))
    {
        return BOOL(1);
    }
    // 有标题的隐藏窗口同样保留，不按 owner 一刀切，兼容 Qt / 微信的窗口所有者结构。
    let mut class = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    if matches!(
        String::from_utf16_lossy(&class[..length]).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "tooltips_class32"
    ) {
        return BOOL(1);
    }
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
        return BOOL(1);
    }
    let mut rect = placement.rcNormalPosition;
    if rect.right - rect.left < 64 || rect.bottom - rect.top < 64 {
        // 部分自绘窗口没有正常态位置，不能仅凭 WINDOWPLACEMENT 把它排除。
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err()
            || rect.right - rect.left < 64
            || rect.bottom - rect.top < 64
        {
            return BOOL(1);
        }
    }
    let name = state
        .names
        .entry(pid)
        .or_insert_with(|| process_name(pid))
        .clone();
    state.windows.push(AppWindow {
        id: hwnd.0 as usize as u32,
        pid,
        title,
        app_name: name,
    });
    BOOL(1)
}

/// 不使用 xcap 的“可见窗口”过滤；录屏来源同时需要最小化及托盘隐藏的主窗口。
pub fn application_windows() -> Result<Vec<AppWindow>, String> {
    let mut state = WindowList {
        windows: Vec::new(),
        names: Default::default(),
    };
    let result = unsafe {
        EnumWindows(
            Some(application_window),
            LPARAM(&mut state as *mut WindowList as isize),
        )
    };
    if state.windows.len() < 256 {
        result.map_err(|e| e.to_string())?;
    }
    Ok(state.windows)
}

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

/// 来源列表选择应用时恢复并置前目标；不还原已最大化窗口，避免主动改变录制尺寸。
pub fn activate_window(window: &AppWindow) -> Result<(), String> {
    let hwnd = HWND(window.id().map_err(|e| e.to_string())? as usize as *mut _);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err("应用窗口已关闭，请重新选择。".into());
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != window.pid().map_err(|e| e.to_string())? {
            return Err("应用窗口已被替换，请重新选择。".into());
        }
        // 原生命令运行在线程池。先创建消息队列并临时连接前台/目标输入线程，
        // 避免“选择动作来自本应用，但线程池无法接管焦点”的情况；Drop 总会解除连接。
        let mut message = MSG::default();
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let target = GetWindowThreadProcessId(hwnd, None);
        let _foreground = InputLink::connect(current, foreground);
        let _target = (target != foreground).then(|| InputLink::connect(current, target));
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        BringWindowToTop(hwnd).map_err(|e| e.to_string())?;
        let _ = SetForegroundWindow(hwnd);
        for _ in 0..5 {
            let active = GetForegroundWindow();
            let mut active_pid = 0;
            GetWindowThreadProcessId(active, Some(&mut active_pid));
            // 应用可能激活自己的模态窗口；同一主窗口拥有的弹窗也视为应用已置前。
            if active == hwnd
                || (active_pid == pid
                    && GetAncestor(active, GA_ROOTOWNER) == GetAncestor(hwnd, GA_ROOTOWNER))
            {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        return Err("无法将应用窗口置前，请先打开该应用后重试。".into());
    }
}

/// 输入线程连接仅用于本次显式选择应用，绝不长期共享输入状态。
struct InputLink {
    current: u32,
    other: u32,
    attached: bool,
}
impl InputLink {
    fn connect(current: u32, other: u32) -> Self {
        let attached = other != 0
            && current != other
            && unsafe { AttachThreadInput(current, other, true) }.as_bool();
        Self {
            current,
            other,
            attached,
        }
    }
}
impl Drop for InputLink {
    fn drop(&mut self) {
        if self.attached {
            let _ = unsafe { AttachThreadInput(self.current, self.other, false) };
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
                title: String::new(),
                app_name: String::new(),
                recording_only: false,
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
