//! 受限 PNG / JPEG / WebP 文件解码，用于导入图片贴图。
use image::{ImageDecoder, ImageFormat, RgbaImage};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 路径仅来自原生打开对话框。限制读取字节数及解码尺寸，避免小压缩文件占用大量内存。
pub(crate) fn read_image(path: &Path) -> Result<RgbaImage, String> {
    let file = File::open(path).map_err(|e| format!("无法打开图片：{e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("请选择不超过 64 MiB 的 PNG、JPEG 或 WebP 文件。".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("图片文件超过 64 MiB。".into());
    }
    decode_image(&bytes)
}

/// 根据文件内容识别格式，不信任扩展名；动画 WebP 仅读取首帧作为静态贴图。
fn decode_image(bytes: &[u8]) -> Result<RgbaImage, String> {
    let format =
        image::guess_format(bytes).map_err(|_| "文件不是有效的 PNG、JPEG 或 WebP 图片。")?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("仅支持 PNG、JPEG 和 WebP 图片。".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "图片头无效或文件已损坏。")?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_000_000 {
        return Err("图片为空或超过 1600 万像素。".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("图片解码失败：{e}"))?;
    // 先约束解码输出，再读取 EXIF 方向；手机照片按拍摄方向显示，导出与预览一致。
    if decoder.total_bytes() > 128 * 1024 * 1024 {
        return Err("图片解码所需内存超过限制。".into());
    }
    let orientation = decoder
        .orientation()
        .map_err(|e| format!("图片方向信息无效：{e}"))?;
    let mut image =
        image::DynamicImage::from_decoder(decoder).map_err(|e| format!("图片解码失败：{e}"))?;
    image.apply_orientation(orientation);
    Ok(image.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn png_import_preserves_alpha_and_rejects_invalid_data() {
        let image = RgbaImage::from_pixel(2, 3, Rgba([1, 2, 3, 70]));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        assert_eq!(decode_image(png.get_ref()).unwrap(), image);
        assert!(decode_image(b"not an image").is_err());
        assert!(decode_image(b"GIF89a").is_err());
        assert!(decode_image(&png.get_ref()[..20]).is_err());
        let huge = RgbaImage::new(4001, 4000);
        let mut png = Cursor::new(Vec::new());
        huge.write_to(&mut png, ImageFormat::Png).unwrap();
        assert!(decode_image(png.get_ref()).is_err());
    }

    #[test]
    fn webp_import_keeps_transparency() {
        let image = RgbaImage::from_pixel(2, 3, Rgba([40, 80, 120, 70]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::WebP).unwrap();
        assert_eq!(decode_image(bytes.get_ref()).unwrap(), image);
        assert!(decode_image(&bytes.get_ref()[..20]).is_err());
    }

    #[test]
    fn jpeg_import_applies_exif_orientation() {
        let image = image::RgbImage::from_pixel(2, 3, image::Rgb([40, 80, 120]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Jpeg).unwrap();
        let decoded = decode_image(bytes.get_ref()).unwrap();
        assert_eq!(decoded.dimensions(), (2, 3));
        assert!(decoded.pixels().all(|p| p.0[3] == 255));
        // SOI 后插入 EXIF APP1：小端 TIFF 的方向标签 6 表示顺时针旋转 90°。
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&bytes.get_ref()[2..]);
        assert_eq!(
            decode_image(&oriented).unwrap(),
            image::imageops::rotate90(&decoded)
        );
        assert!(decode_image(&bytes.get_ref()[..20]).is_err());
    }

    #[test]
    fn file_import_uses_content_and_rejects_oversized_files() {
        // 文件名故意使用错误扩展名，验证解码器只依据内容判别格式。
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("photo.jpg");
        let image = RgbaImage::from_pixel(2, 3, Rgba([1, 2, 3, 70]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        std::fs::write(&path, bytes.into_inner()).unwrap();
        assert_eq!(read_image(&path).unwrap(), image);
        File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(read_image(&path).unwrap_err().contains("64 MiB"));
        assert!(read_image(&directory.path().join("missing.png")).is_err());
    }
}
