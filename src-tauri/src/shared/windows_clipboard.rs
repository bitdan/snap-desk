//! Windows 图片复制：编码在打开剪贴板之前完成，缩短系统资源占用时间。
use image::{ImageFormat, RgbaImage};
use std::{io::Cursor, sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager};

static WRITER: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(super) fn benchmark_dib(image: &RgbaImage) -> Result<Vec<u8>, String> { dib(image) }

/// CF_DIBV5 使用正高度、从底向上的 BGRA；保留 alpha，兼容常见图片粘贴目标。
fn dib(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let (width, height) = image.dimensions();
    let size = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .filter(|_| {
            width > 0 && height > 0 && width <= i32::MAX as u32 && height <= i32::MAX as u32
        })
        .ok_or("图片尺寸超出剪贴板支持范围。")?;
    let mut bytes = vec![0u8; 124];
    for (offset, value) in [
        (0, 124),
        (4, width),
        (8, height),
        (16, 3),
        (20, size),
        (40, 0x00ff0000),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
        (108, 4),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
    bytes.reserve(size as usize);
    for row in image.as_raw().chunks_exact(width as usize * 4).rev() {
        for pixel in row.chunks_exact(4) {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    Ok(bytes)
}

/// 从现有后台截图/贴图线程调用；Open、Empty、Set、Close 始终发生于同一线程。
pub fn copy(app: &AppHandle, image: &RgbaImage) -> Result<(), String> {
    let bitmap = dib(image)?;
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let format = clipboard_win::register_format("PNG").ok_or("无法注册 PNG 剪贴板格式。")?;
    let window = app
        .get_webview_window("main")
        .ok_or("主窗口不可用，无法复制图片。")?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let _writer = WRITER.lock().map_err(|e| e.to_string())?;
    let mut last = String::new();
    for delay in [0, 40, 80, 160] {
        if delay != 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        // 每次重试重新打开。RAII 在成功和所有错误路径上都关闭剪贴板。
        let attempt = (|| -> Result<(), String> {
            let _clipboard =
                clipboard_win::Clipboard::new_for(hwnd.0 as _).map_err(|e| e.to_string())?;
            clipboard_win::raw::empty().map_err(|e| e.to_string())?;
            clipboard_win::raw::set_without_clear(format.get(), png.get_ref())
                .map_err(|e| e.to_string())?;
            // PNG 已写入即有可用输出，补充 DIB 失败不能重试清空已经成功的 PNG。
            if let Err(_error) = clipboard_win::raw::set_without_clear(17, &bitmap) {
                log::warn!("event=clipboard.dib_fallback_failed module=clipboard code=DIB_FALLBACK_FAILED");
            }
            Ok(())
        })();
        match attempt {
            Ok(()) => return Ok(()),
            Err(error) => last = error,
        }
    }
    Err(format!("复制图片自动重试后仍失败：{last}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dib_preserves_dimensions_orientation_channels_and_alpha() {
        let image = RgbaImage::from_raw(1, 2, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let bytes = dib(&image).unwrap();
        assert_eq!(bytes.len(), 132);
        assert_eq!(&bytes[4..12], &[1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(&bytes[12..16], &[1, 0, 32, 0]);
        assert_eq!(&bytes[124..], &[7, 6, 5, 8, 3, 2, 1, 4]);
        assert!(dib(&RgbaImage::new(0, 1)).is_err());
    }
}
