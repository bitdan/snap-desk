//! 将纯文字转换成现有图片贴图；不执行 HTML，也不修改系统剪贴板。
use image::RgbaImage;
use tauri::AppHandle;

/// 优先复用用户选择的字体；未下载时使用本机中文字体，让普通复制文字可以直接贴图。
fn font_bytes(app: &AppHandle) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = crate::resources::font_bytes(app) {
        return Ok(bytes);
    }
    #[cfg(target_os = "windows")]
    let paths = {
        let windows = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        vec![std::path::PathBuf::from(windows).join("Fonts/msyh.ttc")]
    };
    #[cfg(target_os = "macos")]
    let paths = vec![std::path::PathBuf::from(
        "/System/Library/Fonts/PingFang.ttc",
    )];
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let paths = vec![
        std::path::PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        std::path::PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ];
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            if ab_glyph::FontRef::try_from_slice(&bytes).is_ok() {
                return Ok(bytes);
            }
        }
    }
    Err("文字贴图字体不可用，请在通用设置下载或选择字体。".into())
}

/// 历史全文和当前剪贴板使用同一排版，限制输入字节与输出像素，避免过大文本占满内存。
pub(super) fn render(app: &AppHandle, text: &str) -> Result<RgbaImage, String> {
    if text.trim().is_empty() {
        return Err("没有可贴图的文字。".into());
    }
    if text.len() > 1024 * 1024 {
        return Err("文字贴图最多支持 1 MiB 文本，请分段贴图。".into());
    }
    crate::shared::text_image::render(text, &font_bytes(app)?, 640, super::MAX_IMAGE_PIXELS)
}
