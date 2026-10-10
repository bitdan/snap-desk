//! 文本与颜色贴图的输入校验，以及受限 PNG 文件解码。
//! 文本继续使用现有可编辑标注，原生层只接受校验后的 PNG 字形，不接收任意路径。
use crate::annotations::{Annotation, Kind};
use image::{ImageFormat, Rgba, RgbaImage};
use serde::Deserialize;
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};
use tauri::{AppHandle, WebviewWindow};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorPinRequest {
    color: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextPinRequest {
    background: String,
    annotation: Annotation,
}

fn color(value: &str) -> Result<Rgba<u8>, String> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("颜色必须为 #RRGGBB。".into());
    }
    let value = u32::from_str_radix(&value[1..], 16).map_err(|e| e.to_string())?;
    Ok(Rgba([
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
        255,
    ]))
}

fn color_image(request: ColorPinRequest) -> Result<RgbaImage, String> {
    if !(16..=2048).contains(&request.width) || !(16..=2048).contains(&request.height) {
        return Err("颜色贴图宽高须为 16～2048 像素。".into());
    }
    Ok(RgbaImage::from_pixel(
        request.width,
        request.height,
        color(&request.color)?,
    ))
}

fn text_image(request: TextPinRequest) -> Result<(RgbaImage, Vec<Annotation>), String> {
    let a = &request.annotation;
    // 与前端一致：16 px 留白、最大 608 × 480 的字形范围；先校验浮点坐标再转整数。
    if !matches!(a.kind, Kind::Text)
        || a.start.x != 16.0
        || a.start.y != 16.0
        || a.end.x > 624.0
        || a.end.y > 496.0
        || a.text.as_ref().is_none_or(|s| s.trim().is_empty())
    {
        return Err("文本贴图内容或排版范围无效。".into());
    }
    crate::annotations::validate(std::slice::from_ref(a), 640, 512)?;
    let image = RgbaImage::from_pixel(
        (a.end.x.ceil() as u32) + 16,
        (a.end.y.ceil() as u32) + 16,
        color(&request.background)?,
    );
    Ok((image, vec![request.annotation]))
}

#[tauri::command]
pub async fn pin_color(
    app: AppHandle,
    window: WebviewWindow,
    request: ColorPinRequest,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::pins::create_visible(&app, color_image(request)?, vec![])
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pin_text(
    app: AppHandle,
    window: WebviewWindow,
    request: TextPinRequest,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (image, annotations) = text_image(request)?;
        crate::pins::create_visible(&app, image, annotations)
    })
    .await
    .map_err(|e| e.to_string())?
}

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 路径仅来自原生打开对话框。限制读取字节数及解码尺寸，避免小压缩文件占用大量内存。
pub(crate) fn read_png(path: &Path) -> Result<RgbaImage, String> {
    let file = File::open(path).map_err(|e| format!("无法打开图片：{e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("请选择不超过 64 MiB 的 PNG 文件。".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("PNG 文件超过 64 MiB。".into());
    }
    decode_png(&bytes)
}

fn decode_png(bytes: &[u8]) -> Result<RgbaImage, String> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "文件不是有效的 PNG 图片。")?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_000_000 {
        return Err("PNG 图片为空或超过 1600 万像素。".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|image| image.to_rgba8())
        .map_err(|e| format!("PNG 解码失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_pins_validate_dimensions_and_keep_exact_rgb() {
        let image = color_image(ColorPinRequest {
            color: "#12aBcF".into(),
            width: 16,
            height: 20,
        })
        .unwrap();
        assert_eq!(image.dimensions(), (16, 20));
        assert!(image.pixels().all(|p| p.0 == [18, 171, 207, 255]));
        for (w, h) in [(0, 20), (20, 15), (2049, 16), (16, u32::MAX)] {
            assert!(color_image(ColorPinRequest {
                color: "#123456".into(),
                width: w,
                height: h
            })
            .is_err());
        }
        for value in ["red", "#fff", "#12345g", "#你好"] {
            assert!(color(value).is_err());
        }
    }

    #[test]
    fn png_import_preserves_alpha_and_rejects_invalid_data() {
        let image = RgbaImage::from_pixel(2, 3, Rgba([1, 2, 3, 70]));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        assert_eq!(decode_png(png.get_ref()).unwrap(), image);
        assert!(decode_png(b"not a PNG").is_err());
        assert!(decode_png(&png.get_ref()[..20]).is_err());
        let huge = RgbaImage::new(4001, 4000);
        let mut png = Cursor::new(Vec::new());
        huge.write_to(&mut png, ImageFormat::Png).unwrap();
        assert!(decode_png(png.get_ref()).is_err());
    }

    #[test]
    fn text_pin_retains_editable_annotation_and_padding() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut png = Cursor::new(Vec::new());
        RgbaImage::from_pixel(20, 30, Rgba([1, 2, 3, 255]))
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let annotation: Annotation = serde_json::from_value(serde_json::json!({
            "kind": "text", "start": {"x":16,"y":16}, "end":{"x":36,"y":46},
            "color":"#000000","width":2,"text":"中文","fontSize":24,"fontFamily":"sans",
            "raster":STANDARD.encode(png.into_inner())
        }))
        .unwrap();
        let (image, shapes) = text_image(TextPinRequest {
            background: "#ffffff".into(),
            annotation: annotation.clone(),
        })
        .unwrap();
        assert_eq!(image.dimensions(), (52, 62));
        assert_eq!(shapes[0].text.as_deref(), Some("中文"));
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
        let mut invalid = annotation.clone();
        invalid.end.x = f64::NAN;
        assert!(text_image(TextPinRequest {
            background: "#ffffff".into(),
            annotation: invalid
        })
        .is_err());
        let mut invalid = annotation.clone();
        invalid.kind = Kind::Mosaic;
        assert!(text_image(TextPinRequest {
            background: "#ffffff".into(),
            annotation: invalid
        })
        .is_err());
        let mut invalid = annotation;
        invalid.text = Some(" \n ".into());
        assert!(text_image(TextPinRequest {
            background: "#ffffff".into(),
            annotation: invalid
        })
        .is_err());
    }
}
