//! 截图工具栏主动识别二维码；只保存本次结果，打开和复制均需用户点击。
use image::RgbaImage;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrItem {
    text: String,
    url: Option<String>,
}

#[derive(Clone, Default, Serialize)]
pub struct QrResult {
    id: u64,
    items: Vec<QrItem>,
}

/// 结果留在原生内存中供打开和复制使用；动作携带截图 ID 防止操作旧结果。
#[derive(Default)]
pub struct QrState(Mutex<QrResult>);

fn web_url(text: &str) -> Option<String> {
    let url = tauri::Url::parse(text.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.to_string())
}

fn decode(image: &RgbaImage) -> Vec<QrItem> {
    let gray = image::imageops::grayscale(image);
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(
        gray.width() as usize,
        gray.height() as usize,
        |x, y| gray.get_pixel(x as u32, y as u32)[0],
    );
    let mut items: Vec<QrItem> = Vec::new();
    for grid in prepared.detect_grids() {
        // 单个码损坏不影响其他码；空内容和同文重复码不重复提示。
        if let Ok((_, text)) = grid.decode() {
            if !text.is_empty() && !items.iter().any(|item| item.text == text) {
                items.push(QrItem {
                    url: web_url(&text),
                    text,
                });
            }
        }
    }
    items
}

/// 工具栏主动识别时返回结果，不隐藏截图窗口，也不唤起主页。
pub fn recognize(app: &AppHandle, id: u64, image: &RgbaImage) -> Result<QrResult, String> {
    let items = std::panic::catch_unwind(|| decode(image))
        .map_err(|_| "二维码识别失败，请调整选区后重试。".to_string())?;
    let result = QrResult { id, items };
    *app.state::<QrState>().0.lock().map_err(|e| e.to_string())? = result.clone();
    Ok(result)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QrAction {
    Open,
    Copy,
}

/// 仅接受本次识别结果的索引，不接受前端提供的任意网址或命令。
#[tauri::command]
pub async fn qr_action(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    index: usize,
    action: QrAction,
) -> Result<(), String> {
    crate::capture::require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<QrState>();
        let result = state.0.lock().map_err(|e| e.to_string())?;
        let item = result
            .items
            .get(index)
            .filter(|_| result.id == id)
            .ok_or("二维码结果已更新，请使用最新结果。")?
            .clone();
        drop(result);
        match action {
            QrAction::Copy => app
                .clipboard()
                .write_text(item.text)
                .map_err(|e| format!("复制失败：{e}")),
            QrAction::Open => {
                let url = item.url.ok_or("仅支持打开 HTTP / HTTPS 链接。")?;
                open::that(url).map_err(|e| format!("打开链接失败：{e}"))
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 使用独立编码器生成带静区的码图，覆盖真实定位、解码、多码及去重。
    fn code_image(text: &str) -> RgbaImage {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let size = code.width();
        let scale = 6;
        let mut image = RgbaImage::from_pixel(
            ((size + 8) * scale) as u32,
            ((size + 8) * scale) as u32,
            image::Rgba([255; 4]),
        );
        for y in 0..size {
            for x in 0..size {
                if code[(x, y)] == qrcode::Color::Dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            image.put_pixel(
                                ((x + 4) * scale + dx) as u32,
                                ((y + 4) * scale + dy) as u32,
                                image::Rgba([0, 0, 0, 255]),
                            );
                        }
                    }
                }
            }
        }
        image
    }

    #[test]
    fn decodes_rotated_text_and_deduplicates_multiple_codes() {
        let link = code_image("https://example.com/qr");
        let text = image::imageops::rotate90(&code_image("你好，二维码"));
        let mut canvas = RgbaImage::from_pixel(
            link.width() * 2 + text.width(),
            link.height().max(text.height()),
            image::Rgba([255; 4]),
        );
        image::imageops::replace(&mut canvas, &link, 0, 0);
        image::imageops::replace(&mut canvas, &link, i64::from(link.width()), 0);
        image::imageops::replace(&mut canvas, &text, i64::from(link.width() * 2), 0);
        let items = decode(&canvas);
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|item| item.text == "https://example.com/qr" && item.url.is_some()));
        assert!(items
            .iter()
            .any(|item| item.text == "你好，二维码" && item.url.is_none()));
    }
    #[test]
    fn allows_only_web_links() {
        assert!(web_url("https://example.com/path?a=1&b=2").is_some());
        for text in [
            "普通文本",
            "javascript:alert(1)",
            "file:///C:/test",
            "mailto:a@b.com",
        ] {
            assert!(web_url(text).is_none());
        }
    }
    #[test]
    fn blank_image_has_no_codes() {
        assert!(decode(&RgbaImage::from_pixel(100, 100, image::Rgba([255; 4]))).is_empty());
    }
}
