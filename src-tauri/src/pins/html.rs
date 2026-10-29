//! 富文本贴图桥：读取有界 CF_HTML，交由主 WebView 安全排版，再复用图片贴图入口。
use super::*;
use std::{sync::mpsc, time::Duration};

#[derive(Default)]
pub struct RenderState {
    next: AtomicU64,
    pending: Mutex<Option<(u64, mpsc::SyncSender<Result<RgbaImage, String>>)>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderRequest {
    id: u64,
    html: String,
}

/// 不依赖剪贴板历史是否启用，也不把富文本写入历史数据库。
#[cfg(windows)]
pub(super) fn read() -> Result<Option<String>, String> {
    use windows::{
        core::w,
        Win32::{
            Foundation::HGLOBAL,
            System::{DataExchange::*, Memory::*},
        },
    };
    let _clipboard =
        clipboard_win::Clipboard::new_attempts(5).map_err(|_| "剪贴板正在使用，请稍后重试。")?;
    unsafe {
        let format = RegisterClipboardFormatW(w!("HTML Format"));
        if format == 0 || IsClipboardFormatAvailable(format).is_err() {
            return Ok(None);
        }
        let handle = HGLOBAL(GetClipboardData(format).map_err(err)?.0);
        let length = GlobalSize(handle);
        if length == 0 || length > 1024 * 1024 + 4096 {
            return Err("富文本贴图最多支持 1 MiB 内容。".into());
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return Err("富文本暂时不可读。".into());
        }
        let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), length).to_vec();
        let _ = GlobalUnlock(handle);
        fragment(&bytes).map(Some)
    }
}
#[cfg(not(windows))]
pub(super) fn read() -> Result<Option<String>, String> {
    Ok(None)
}

/// CF_HTML 的偏移单位是 UTF-8 字节；只接受合法范围，缺失偏移时用标准片段标记。
fn fragment(bytes: &[u8]) -> Result<String, String> {
    let header = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let offset = |key: &str| {
        header
            .lines()
            .find_map(|line| line.strip_prefix(key)?.trim().parse::<usize>().ok())
    };
    let content = match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(start), Some(end)) if start < end && end <= bytes.len() => &bytes[start..end],
        _ => {
            let start_tag = b"<!--StartFragment-->";
            let end_tag = b"<!--EndFragment-->";
            let start = bytes
                .windows(start_tag.len())
                .position(|v| v == start_tag)
                .map(|i| i + start_tag.len())
                .ok_or("富文本片段头无效。")?;
            let end = bytes[start..]
                .windows(end_tag.len())
                .position(|v| v == end_tag)
                .map(|i| start + i)
                .ok_or("富文本片段尾无效。")?;
            &bytes[start..end]
        }
    };
    if content.len() > 1024 * 1024 {
        return Err("富文本贴图最多支持 1 MiB 内容。".into());
    }
    let html = std::str::from_utf8(content)
        .map_err(|_| "富文本编码无效。")?
        .trim_matches('\0')
        .to_string();
    if html.trim().is_empty() {
        return Err("剪贴板富文本为空。".into());
    }
    Ok(html)
}

/// 原生调用发生在阻塞池；等待期间不持有贴图集合锁，主窗口回调可正常完成。
pub(super) fn render(app: &AppHandle, html: String) -> Result<RgbaImage, String> {
    let state = app.state::<RenderState>();
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let (sender, receiver) = mpsc::sync_channel(1);
    {
        let mut pending = state.pending.lock().map_err(err)?;
        if pending.is_some() {
            return Err("正在转换富文本，请稍候。".into());
        }
        *pending = Some((id, sender));
    }
    let result = app
        .emit_to("main", "pin-render-html", RenderRequest { id, html })
        .map_err(err)
        .and_then(|_| {
            receiver
                .recv_timeout(Duration::from_secs(8))
                .map_err(|_| "富文本转换超时，请重试。".to_string())?
        });
    if let Ok(mut pending) = state.pending.lock() {
        if pending.as_ref().is_some_and(|(current, _)| *current == id) {
            pending.take();
        }
    }
    result
}

/// 回调只接受当前请求及有界 PNG，不接受文件路径、URL 或任意窗口提交。
#[tauri::command]
pub async fn finish_html_render(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    png: Option<String>,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let sender = {
        let state = app.state::<RenderState>();
        let mut pending = state.pending.lock().map_err(err)?;
        if !pending.as_ref().is_some_and(|(current, _)| *current == id) {
            return Err("富文本转换请求已过期。".into());
        }
        pending.take().unwrap().1
    };
    tauri::async_runtime::spawn_blocking(move || {
        let image = (|| {
            let png = png.ok_or("富文本无法转换为图片，请复制较小的文本片段重试。")?;
            if png.len() > 32 * 1024 * 1024 {
                return Err("富文本图片过大。".into());
            }
            let bytes = STANDARD
                .decode(
                    png.strip_prefix("data:image/png;base64,")
                        .ok_or("富文本图片格式无效。")?,
                )
                .map_err(err)?;
            super::content::decode_image(&bytes)
        })();
        sender
            .send(image)
            .map_err(|_| "富文本转换请求已过期。".to_string())
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragment_uses_byte_offsets_and_rejects_invalid_ranges() {
        let body = "<b>中文</b>";
        let header = format!(
            "StartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            60,
            60 + body.len()
        );
        let mut bytes = header.into_bytes();
        bytes.resize(60, b' ');
        bytes.extend_from_slice(body.as_bytes());
        assert_eq!(fragment(&bytes).unwrap(), body);
        assert!(fragment(b"StartFragment:900\r\nEndFragment:1000\r\n").is_err());
        assert_eq!(
            fragment(b"<!--StartFragment--><i>ok</i><!--EndFragment-->").unwrap(),
            "<i>ok</i>"
        );
    }
}
