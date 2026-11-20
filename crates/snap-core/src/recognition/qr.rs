//! 二维码识别：不读取文件、不联网；同文重复码只保留一个，HTTP/HTTPS 内容附带可打开的网址。
use image::RgbaImage;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrItem {
    pub text: String,
    pub url: Option<String>,
}

/// 只接受带主机名的 HTTP / HTTPS 网址。
pub fn web_url(text: &str) -> Option<String> {
    let url = url::Url::parse(text.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.to_string())
}

fn scan(image: &RgbaImage) -> Vec<QrItem> {
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

/// 解码库遇到异常图形可能 panic，隔离为普通错误。
pub fn decode(image: &RgbaImage) -> Result<Vec<QrItem>, String> {
    std::panic::catch_unwind(|| scan(image)).map_err(|_| "二维码识别失败，请重试。".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn web_urls_require_http_scheme_and_host() {
        assert_eq!(super::web_url(" https://example.com/a ").as_deref(), Some("https://example.com/a"));
        assert!(super::web_url("javascript:alert(1)").is_none());
        assert!(super::web_url("file:///etc/passwd").is_none());
        assert!(super::web_url("hello").is_none());
        assert!(super::decode(&image::RgbaImage::new(8, 8)).unwrap().is_empty());
    }
}
