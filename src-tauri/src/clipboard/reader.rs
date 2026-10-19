//! 原生剪贴板打开期间只拷贝有上限的字节；关闭后才解码，避免占用系统剪贴板做重任务。
use super::{model::ClipboardSettings, platform, service::Content};
use image::{ImageDecoder, ImageFormat};
use std::io::Cursor;
pub enum Snapshot {
    Text(Vec<u8>),
    Png(Vec<u8>),
    Dib(Vec<u8>),
    Files(Vec<u8>),
}

pub fn read(settings: &ClipboardSettings) -> Result<Option<Content>, String> {
    let Some(raw) = platform::snapshot(settings)? else {
        return Ok(None);
    };
    match raw {
        Snapshot::Files(bytes) => {
            super::files::decode(&bytes).map(|paths| Some(Content::Files(paths)))
        }
        Snapshot::Text(bytes) => {
            let units = bytes
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .take_while(|c| *c != 0)
                .collect::<Vec<_>>();
            let text = String::from_utf16_lossy(&units);
            if text.len() > settings.max_text_bytes as usize {
                return Err("文本超过历史大小限制，未记录。".into());
            }
            Ok(Some(Content::Text(text)))
        }
        Snapshot::Png(bytes) => {
            decode(bytes, ImageFormat::Png, settings).map(|i| Some(Content::Image(i)))
        }
        Snapshot::Dib(bytes) => {
            decode(bmp(bytes)?, ImageFormat::Bmp, settings).map(|i| Some(Content::Image(i)))
        }
    }
}
fn decode(
    bytes: Vec<u8>,
    format: ImageFormat,
    settings: &ClipboardSettings,
) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|_| "剪贴板图片编码无效。")?;
    let (w, h) = decoder.dimensions();
    if w == 0
        || h == 0
        || u64::from(w) * u64::from(h) > u64::from(settings.max_image_pixels)
        || decoder.total_bytes() > 64 * 1024 * 1024
    {
        return Err("图片超过历史像素限制，未记录。".into());
    }
    image::DynamicImage::from_decoder(decoder)
        .map(|i| i.to_rgba8())
        .map_err(|_| "剪贴板图片解码失败。".into())
}
/// CF_DIB 没有 BMP 文件头；根据 DIB 头、掩码和调色板计算像素偏移，禁止按来源路径读取。
fn bmp(dib: Vec<u8>) -> Result<Vec<u8>, String> {
    if dib.len() < 40 {
        return Err("DIB 数据不完整。".into());
    }
    let field = |offset| u32::from_le_bytes(dib[offset..offset + 4].try_into().unwrap());
    let header = field(0) as usize;
    let compression = field(16);
    let colors = field(32) as usize;
    let bits = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    if ![40, 52, 56, 108, 124].contains(&header)
        || header > dib.len()
        || ![0, 3, 6].contains(&compression)
        || ![1, 4, 8, 16, 24, 32].contains(&bits)
    {
        return Err("暂不支持此 DIB 位图格式。".into());
    }
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let colors = if colors == 0 && bits <= 8 {
        1usize << bits
    } else {
        colors
    };
    let offset = header
        .checked_add(masks)
        .and_then(|n| n.checked_add(colors.checked_mul(4)?))
        .filter(|n| *n <= dib.len())
        .ok_or("DIB 调色板无效。")?;
    let mut output = Vec::with_capacity(dib.len() + 14);
    output.extend_from_slice(b"BM");
    output.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&((offset + 14) as u32).to_le_bytes());
    output.extend_from_slice(&dib);
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_and_oversized_images_fail_before_full_decode() {
        assert!(bmp(vec![0; 39]).is_err());
        assert!(decode(
            b"broken".to_vec(),
            ImageFormat::Png,
            &ClipboardSettings::default()
        )
        .is_err());
        let image = image::RgbaImage::new(3, 2);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let settings = ClipboardSettings {
            max_image_pixels: 5,
            ..ClipboardSettings::default()
        };
        assert!(decode(png.into_inner(), ImageFormat::Png, &settings).is_err());
    }
}
