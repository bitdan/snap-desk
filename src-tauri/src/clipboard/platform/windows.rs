//! HWND 只在创建线程销毁。Drop 投递 WM_CLOSE 并 join，确保暂停后不再接收系统通知。
use super::super::{model::ClipboardSettings, reader::Snapshot};
use std::{
    cell::RefCell,
    sync::mpsc::{self, SyncSender},
    thread::JoinHandle,
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            DataExchange::*,
            LibraryLoader::GetModuleHandleW,
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
        UI::WindowsAndMessaging::*,
    },
};
thread_local! { static SENDER: RefCell<Option<SyncSender<()>>> = const { RefCell::new(None) }; }
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            SENDER.with(|s| {
                if let Some(tx) = s.borrow().as_ref() {
                    let _ = tx.try_send(());
                }
            });
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = RemoveClipboardFormatListener(hwnd);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
pub struct Watcher {
    hwnd: usize,
    worker: Option<JoinHandle<()>>,
}
impl Watcher {
    pub fn start(tx: SyncSender<()>) -> Result<Self, String> {
        let (ready, rx) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("clipboard-messages".into())
            .spawn(move || unsafe {
                let instance = match GetModuleHandleW(None) {
                    Ok(v) => HINSTANCE(v.0),
                    Err(_) => {
                        let _ = ready.send(Err("剪贴板监听初始化失败。".to_string()));
                        return;
                    }
                };
                let class = w!("SnapDeskClipboardHistory");
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(proc),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                if RegisterClassW(&wc) == 0 {
                    let _ = ready.send(Err("无法注册剪贴板消息窗口。".into()));
                    return;
                }
                let window = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    Some(instance),
                    None,
                );
                if let Ok(hwnd) = window {
                    if AddClipboardFormatListener(hwnd).is_ok() {
                        SENDER.with(|s| *s.borrow_mut() = Some(tx));
                        let _ = ready.send(Ok(hwnd.0 as usize));
                        let mut msg = MSG::default();
                        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                        let _ = RemoveClipboardFormatListener(hwnd);
                        SENDER.with(|s| *s.borrow_mut() = None);
                    } else {
                        let _ = ready.send(Err("无法监听系统剪贴板，可在历史页重试。".into()));
                    }
                    if IsWindow(Some(hwnd)).as_bool() {
                        let _ = DestroyWindow(hwnd);
                    }
                } else {
                    let _ = ready.send(Err("无法创建剪贴板消息窗口。".into()));
                }
                let _ = UnregisterClassW(class, Some(instance));
            })
            .map_err(|_| "无法启动剪贴板消息线程。")?;
        match rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                worker: Some(worker),
            }),
            result => {
                let _ = worker.join();
                Err(result
                    .err()
                    .map(|_| "剪贴板消息线程提前结束。".into())
                    .unwrap_or_else(|| "剪贴板监听初始化失败。".into()))
            }
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// 在插件分配内存前检查原生存储大小，并跳过密码管理器常用的保密标记。
pub fn snapshot(settings: &ClipboardSettings) -> Result<Option<Snapshot>, String> {
    let _guard = clipboard_win::Clipboard::new().map_err(|_| "剪贴板正在被占用，请稍后重试。")?;
    unsafe {
        for name in [
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            w!("Clipboard Viewer Ignore"),
        ] {
            let format = RegisterClipboardFormatW(name);
            if format != 0 && IsClipboardFormatAvailable(format).is_ok() {
                return Ok(None);
            }
        }
        let png = RegisterClipboardFormatW(w!("PNG"));
        for format in [png, 17, 8, 13] {
            if format == 0 || IsClipboardFormatAvailable(format).is_err() {
                continue;
            }
            let image = format != 13;
            if image && !settings.save_copied_images {
                return Ok(None);
            }
            if !image && !settings.save_copied_text {
                return Ok(None);
            }
            let handle = GetClipboardData(format).map_err(|_| "剪贴板格式暂不可读。")?;
            let size = GlobalSize(windows::Win32::Foundation::HGLOBAL(handle.0));
            let limit = if image {
                64 * 1024 * 1024
            } else {
                (settings.max_text_bytes as usize) * 2 + 2
            };
            if size > limit {
                return Err("剪贴板内容超过单项大小限制，未记录。".into());
            }
            let global = HGLOBAL(handle.0);
            let ptr = GlobalLock(global);
            if ptr.is_null() {
                return Err("剪贴板内存不可读。".into());
            }
            if image {
                // 先查看固定长度的 PNG/DIB 头，再让插件解码；压缩文件小不代表像素少。
                let header = std::slice::from_raw_parts(ptr as *const u8, size.min(40));
                let dimensions = image_dimensions(header, format == png);
                let (width, height) = match dimensions {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = GlobalUnlock(global);
                        return Err(e);
                    }
                };
                if width == 0
                    || height == 0
                    || u64::from(width) * u64::from(height) > u64::from(settings.max_image_pixels)
                {
                    let _ = GlobalUnlock(global);
                    return Err("图片超过历史像素限制，未记录。".into());
                }
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
            let _ = GlobalUnlock(global);
            return Ok(Some(if !image {
                Snapshot::Text(bytes)
            } else if format == png {
                Snapshot::Png(bytes)
            } else {
                Snapshot::Dib(bytes)
            }));
        }
    }
    Ok(None)
}

/// 只读取已验证长度的头部；DIB 的负高度表示从上到下，不改变像素配额。
fn image_dimensions(bytes: &[u8], png: bool) -> Result<(u32, u32), String> {
    if png && bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" && &bytes[12..16] == b"IHDR"
    {
        return Ok((
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        ));
    }
    if !png && bytes.len() >= 40 && u32::from_le_bytes(bytes[..4].try_into().unwrap()) >= 40 {
        let width = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let height = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if width > 0 {
            return Ok((width as u32, height.unsigned_abs()));
        }
    }
    Err("剪贴板图片头无效，未记录。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headers_reject_truncation_and_preserve_top_down_dimensions() {
        assert!(image_dimensions(b"PNG", true).is_err());
        let mut dib = [0u8; 40];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&4000i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-3000i32).to_le_bytes());
        assert_eq!(image_dimensions(&dib, false).unwrap(), (4000, 3000));
    }
}
