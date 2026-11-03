//! 将纯文字转换成现有图片贴图；不执行 HTML，也不修改系统剪贴板。
use image::RgbaImage;
use tauri::AppHandle;

/// 颜色贴图拖出标准 HEX，普通文本（包括多行文件路径）保留原始内容。
pub(super) fn drag_text(text: &str) -> String {
    color_hex(text.trim()).unwrap_or_else(|| text.to_owned())
}

fn color_hex(text: &str) -> Option<String> {
    if text.chars().any(char::is_control) {
        return None;
    }
    if let Some(hex) = text.strip_prefix('#') {
        if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.bytes().all(|v| v.is_ascii_hexdigit()) {
            return None;
        }
        return Some(
            if hex.len() <= 4 {
                format!("#{}", hex.chars().flat_map(|c| [c, c]).collect::<String>())
            } else {
                text.to_owned()
            }
            .to_ascii_uppercase(),
        );
    }
    let lower = text.to_ascii_lowercase();
    let (name, body) = lower.split_once('(')?;
    let body = body.strip_suffix(')')?;
    let hsl = matches!(name, "hsl" | "hsla");
    let alpha = matches!(name, "rgba" | "hsla");
    if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla") {
        return None;
    }
    let parts = body.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != if alpha { 4 } else { 3 } {
        return None;
    }
    let mut values = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let percent = part.ends_with('%');
        if hsl && ((index == 0 && percent) || (matches!(index, 1 | 2) && !percent)) {
            return None;
        }
        let value = part.strip_suffix('%').unwrap_or(part).parse::<f64>().ok()?;
        let max = if percent {
            100.0
        } else if index == 3 {
            1.0
        } else if hsl {
            360.0
        } else {
            255.0
        };
        if !value.is_finite() || !(0.0..=max).contains(&value) {
            return None;
        }
        values.push(if percent {
            value / 100.0
        } else if index == 3 || hsl {
            value
        } else {
            value / 255.0
        });
    }
    let rgb = if hsl {
        let (h, s, l) = (values[0] / 60.0, values[1], values[2]);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 | 6 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        [r + m, g + m, b + m]
    } else {
        [values[0], values[1], values[2]]
    };
    let byte = |v: f64| (v * 255.0).round() as u8;
    let mut hex = format!(
        "#{:02X}{:02X}{:02X}",
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2])
    );
    if alpha {
        hex.push_str(&format!("{:02X}", byte(values[3])));
    }
    Some(hex)
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    #[test]
    fn colors_drag_as_hex_and_other_content_is_unchanged() {
        for (input, output) in [
            (" #abc ", "#AABBCC"),
            ("#1234", "#11223344"),
            ("rgb(255, 0, 12)", "#FF000C"),
            ("rgba(100%,0%,0%,0.5)", "#FF000080"),
            ("hsl(120,100%,50%)", "#00FF00"),
        ] {
            assert_eq!(drag_text(input), output);
        }
        for input in [
            " 原始文字\n第二行 ",
            "C:\\a.txt\nD:\\b.txt",
            "rgb(256,0,0)",
            "rgb(255,\n0,0)",
            "not a color",
        ] {
            assert_eq!(drag_text(input), input);
        }
    }
}

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
