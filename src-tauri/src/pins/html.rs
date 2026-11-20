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

/// CF_HTML 读取与片段解析在 core（GPUI 版共用，后者另有 Rust 排版器 `snap_core::pins::html::render`）。
pub(super) use snap_core::pins::html::read;


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
