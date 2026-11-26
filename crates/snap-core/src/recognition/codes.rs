//! 二维码与条形码识别：只读入内存中的像素，不读取文件、不联网。
//!
//! 解码库为 rxing（ZXing 的 Rust 移植，Apache-2.0），只启用解码与下列格式，不带图片解码和结果解析等默认功能。
//! 流程：
//! 1. RGBA 按透明度叠在白底上转灰度（透明底的码图也能识别），四周补白边：用户常贴着码边框选，
//!    而 QR 需要 4 个模块、EAN 约 7–11 个模块宽的空白静区；
//! 2. QR / Micro QR 用 zxing-cpp 移植的读取器，一次找出图中全部的码；
//! 3. 其余二维格式与一维条码由 `GenericMultipleBarcodeReader` 找到一个后，
//!    在它上下左右的剩余区域继续找（一维条码含竖排，读取器会再转 90° 扫描）；
//! 4. 一个都没找到时按反色（深底浅码，常见于深色主题）再识别一次。
//!
//! 同格式同内容的码只保留一个；结果按阅读顺序排列，HTTP/HTTPS 内容附带可打开的网址。
use image::RgbaImage;
use rxing::{
    common::HybridBinarizer,
    multi::{GenericMultipleBarcodeReader, MultipleBarcodeReader},
    qrcode::cpp_port::QrReader,
    BarcodeFormat, BinaryBitmap, DecodeHints, Luma8Source, MultiUseMultiFormatReader, Point,
    RXingResult,
};
use serde::Serialize;
use std::collections::HashSet;

/// 支持的码制。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeFormat {
    QrCode,
    MicroQrCode,
    DataMatrix,
    Aztec,
    Pdf417,
    Code128,
    Code39,
    Code93,
    Codabar,
    Ean13,
    Ean8,
    UpcA,
    UpcE,
    Itf,
    DataBar,
    DataBarExpanded,
}

/// 识别的格式与 rxing 格式的对应。未列出的（MaxiCode、rMQR、Telepen、DX 胶片码等）不识别：
/// 截图里几乎不会出现，候选格式越多，把文字、图标误当成码的机会也越多。
const FORMATS: [(CodeFormat, BarcodeFormat); 16] = [
    (CodeFormat::QrCode, BarcodeFormat::QR_CODE),
    (CodeFormat::MicroQrCode, BarcodeFormat::MICRO_QR_CODE),
    (CodeFormat::DataMatrix, BarcodeFormat::DATA_MATRIX),
    (CodeFormat::Aztec, BarcodeFormat::AZTEC),
    (CodeFormat::Pdf417, BarcodeFormat::PDF_417),
    (CodeFormat::Code128, BarcodeFormat::CODE_128),
    (CodeFormat::Code39, BarcodeFormat::CODE_39),
    (CodeFormat::Code93, BarcodeFormat::CODE_93),
    (CodeFormat::Codabar, BarcodeFormat::CODABAR),
    (CodeFormat::Ean13, BarcodeFormat::EAN_13),
    (CodeFormat::Ean8, BarcodeFormat::EAN_8),
    (CodeFormat::UpcA, BarcodeFormat::UPC_A),
    (CodeFormat::UpcE, BarcodeFormat::UPC_E),
    (CodeFormat::Itf, BarcodeFormat::ITF),
    (CodeFormat::DataBar, BarcodeFormat::RSS_14),
    (CodeFormat::DataBarExpanded, BarcodeFormat::RSS_EXPANDED),
];

impl CodeFormat {
    /// 界面显示的格式名：各语言通用的标准写法，不进词典。
    pub fn name(self) -> &'static str {
        match self {
            Self::QrCode => "QR Code",
            Self::MicroQrCode => "Micro QR Code",
            Self::DataMatrix => "Data Matrix",
            Self::Aztec => "Aztec",
            Self::Pdf417 => "PDF417",
            Self::Code128 => "Code 128",
            Self::Code39 => "Code 39",
            Self::Code93 => "Code 93",
            Self::Codabar => "Codabar",
            Self::Ean13 => "EAN-13",
            Self::Ean8 => "EAN-8",
            Self::UpcA => "UPC-A",
            Self::UpcE => "UPC-E",
            Self::Itf => "ITF",
            Self::DataBar => "GS1 DataBar",
            Self::DataBarExpanded => "GS1 DataBar Expanded",
        }
    }

    /// 一维条码（含 GS1 DataBar）：位置只有一条扫描线。PDF417 虽是堆叠的条，按二维码处理。
    pub fn is_linear(self) -> bool {
        !matches!(
            self,
            Self::QrCode | Self::MicroQrCode | Self::DataMatrix | Self::Aztec | Self::Pdf417
        )
    }

    fn from_rxing(format: BarcodeFormat) -> Option<Self> {
        FORMATS.iter().find(|(_, f)| *f == format).map(|(code, _)| *code)
    }

    fn is_qr(self) -> bool {
        matches!(self, Self::QrCode | Self::MicroQrCode)
    }
}

/// 码在原图中的位置（像素，左上角为原点）：检测到的角点或扫描线端点的外接矩形。
/// 一维条码只给出所在的一条扫描线，高度可能为 0。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CodeBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeItem {
    pub format: CodeFormat,
    pub text: String,
    /// 内容是 HTTP/HTTPS 网址时可打开的地址（界面确认后才打开）。
    pub url: Option<String>,
    pub bounds: Option<CodeBounds>,
}

/// 只接受带主机名的 HTTP / HTTPS 网址。
pub fn web_url(text: &str) -> Option<String> {
    let url = url::Url::parse(text.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.to_string())
}

/// 补在灰度图四周的白边（像素）。截图里一个模块通常只有几像素，32 像素足够当静区。
const MARGIN: u32 = 32;

/// 透明度叠在白底上转灰度（Rec. 709 系数，与 `image::imageops::grayscale` 一致），并在四周补白边；
/// `inverted` 时先反色再补白边，深底浅码也能得到浅色静区。
fn luma(image: &RgbaImage, inverted: bool) -> (Vec<u8>, u32, u32) {
    let (width, height) = image.dimensions();
    let stride = (width + 2 * MARGIN) as usize;
    let mut luma = vec![255u8; stride * (height + 2 * MARGIN) as usize];
    for (x, y, pixel) in image.enumerate_pixels() {
        let [r, g, b, a] = pixel.0.map(u32::from);
        let gray = (2126 * r + 7152 * g + 722 * b) / 10_000;
        let gray = (gray * a + 255 * (255 - a)) / 255;
        let index = (y + MARGIN) as usize * stride + (x + MARGIN) as usize;
        luma[index] = if inverted { 255 - gray as u8 } else { gray as u8 };
    }
    (luma, width + 2 * MARGIN, height + 2 * MARGIN)
}

fn hints(qr: bool) -> DecodeHints {
    let formats: HashSet<BarcodeFormat> = FORMATS
        .iter()
        .filter(|(code, _)| code.is_qr() == qr)
        .map(|(_, format)| *format)
        .collect();
    DecodeHints {
        PossibleFormats: Some(formats),
        // 识别由用户手动触发、在后台线程执行：多扫行、试旋转，换取更高的识别率。
        TryHarder: Some(true),
        ..Default::default()
    }
}

/// 识别一遍（正色或反色）。`padded` 为补边后的灰度图尺寸，`size` 为原图尺寸，用于换算坐标。
/// 结果可能重复：一维条码在相邻的剩余区域里会被再次找到。
fn collect(luma: &[u8], padded: (u32, u32), size: (u32, u32)) -> Vec<CodeItem> {
    let Ok(source) = Luma8Source::new_with_slice(luma, padded.0, padded.1) else {
        return Vec::new();
    };
    // 同一张二值图（混合二值化，按块取阈值，适应截图里的深浅背景）先后交给两个读取器，二值化只算一次。
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(source));
    // 找不到码时两个读取器都返回 NOT_FOUND，按空结果处理。
    let mut results = QrReader.decode_multiple_with_hints(&mut bitmap, &hints(true)).unwrap_or_default();
    let mut reader = GenericMultipleBarcodeReader::new(MultiUseMultiFormatReader::default());
    results.extend(reader.decode_multiple_with_hints(&mut bitmap, &hints(false)).unwrap_or_default());
    results.iter().filter_map(|result| item(result, size)).collect()
}

/// 空内容和不在支持列表里的格式不提示。
fn item(result: &RXingResult, size: (u32, u32)) -> Option<CodeItem> {
    let format = CodeFormat::from_rxing(*result.getBarcodeFormat())?;
    let text = result.getText();
    (!text.is_empty()).then(|| CodeItem {
        format,
        url: web_url(text),
        text: text.to_owned(),
        bounds: bounds(result.getPoints(), size),
    })
}

/// 补边图坐标换回原图坐标，并限制在原图范围内。
fn bounds(points: &[Point], (width, height): (u32, u32)) -> Option<CodeBounds> {
    let first = points.first()?;
    let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
    for point in points {
        left = left.min(point.x);
        top = top.min(point.y);
        right = right.max(point.x);
        bottom = bottom.max(point.y);
    }
    let margin = MARGIN as f32;
    let x = |v: f32| (v - margin).clamp(0.0, width as f32);
    let y = |v: f32| (v - margin).clamp(0.0, height as f32);
    Some(CodeBounds {
        x: x(left),
        y: y(top),
        width: x(right) - x(left),
        height: y(bottom) - y(top),
    })
}

/// 按阅读顺序排列：先按上沿排序，竖直中心落在当前行范围内的码并入同一行，行内从左到右。
/// 一行一行地归并而不写成比较函数：“同一行”不满足传递性，不能直接交给 `sort_by`。
fn reading_order(mut items: Vec<CodeItem>) -> Vec<CodeItem> {
    let top = |item: &CodeItem| item.bounds.map_or(f32::MAX, |b| b.y);
    let left = |item: &CodeItem| item.bounds.map_or(f32::MAX, |b| b.x);
    items.sort_by(|a, b| top(a).total_cmp(&top(b)));
    let mut rows: Vec<(f32, Vec<CodeItem>)> = Vec::new();
    for item in items {
        let (center, bottom) = item
            .bounds
            .map_or((f32::MAX, f32::MAX), |b| (b.y + b.height / 2.0, b.y + b.height));
        match rows.last_mut() {
            Some((row_bottom, row)) if center <= *row_bottom => {
                *row_bottom = row_bottom.max(bottom);
                row.push(item);
            }
            _ => rows.push((bottom, vec![item])),
        }
    }
    rows.into_iter()
        .flat_map(|(_, mut row)| {
            row.sort_by(|a, b| left(a).total_cmp(&left(b)));
            row
        })
        .collect()
}

fn scan(image: &RgbaImage) -> Vec<CodeItem> {
    let size = image.dimensions();
    if size.0 == 0 || size.1 == 0 {
        return Vec::new();
    }
    // 迭代器是惰性的：正色一遍找到码就不再做反色那一遍。
    let items = [false, true]
        .into_iter()
        .map(|inverted| {
            let (pixels, width, height) = luma(image, inverted);
            collect(&pixels, (width, height), size)
        })
        .find(|items| !items.is_empty())
        .unwrap_or_default();
    // 先排好阅读顺序再去重：同格式同内容的码只保留最靠左上的一个，结果稳定。
    let mut unique: Vec<CodeItem> = Vec::new();
    for item in reading_order(items) {
        if !unique.iter().any(|kept| kept.format == item.format && kept.text == item.text) {
            unique.push(item);
        }
    }
    unique
}

/// 识别失败的提示（词典键，界面按语言翻译）。
const FAILED: &str = "二维码 / 条形码识别失败，请重试。";

/// 解码库遇到异常图形可能 panic，隔离为普通错误。
pub fn decode(image: &RgbaImage) -> Result<Vec<CodeItem>, String> {
    std::panic::catch_unwind(|| scan(image)).map_err(|_| FAILED.to_string())
}

// 不 `use super::*`：rxing 的 `BarcodeFormat` 与本模块的格式枚举同在作用域里容易混用，测试按需引入。
#[cfg(test)]
mod tests {
    use super::{decode, reading_order, CodeBounds, CodeFormat, CodeItem};
    use image::{Rgba, RgbaImage};
    use rxing::{BarcodeFormat, MultiFormatWriter, Writer};

    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
    const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);

    /// 用 rxing 编码器生成码图：每个模块放大 `scale` 像素，四周留 `border` 像素的背景。
    /// 一维条码编码 40 个模块高；二维码按自然尺寸（编码器自带的静区也保留）。
    fn render(format: BarcodeFormat, text: &str, scale: u32, border: u32, ink: Rgba<u8>, paper: Rgba<u8>) -> RgbaImage {
        let linear = super::CodeFormat::from_rxing(format).is_some_and(CodeFormat::is_linear);
        let matrix = MultiFormatWriter
            .encode(text, &format, 0, if linear { 40 } else { 0 })
            .unwrap_or_else(|error| panic!("encode {format:?}: {error}"));
        let (width, height) = (matrix.width() * scale + 2 * border, matrix.height() * scale + 2 * border);
        RgbaImage::from_fn(width, height, |x, y| {
            let inside = x >= border && y >= border && x < width - border && y < height - border;
            if inside && matrix.get((x - border) / scale, (y - border) / scale) {
                ink
            } else {
                paper
            }
        })
    }

    fn plain(format: BarcodeFormat, text: &str) -> RgbaImage {
        render(format, text, 3, 20, BLACK, WHITE)
    }

    /// 裁到深色像素的外接矩形：模拟贴着码边框选、没有静区的截图。
    fn tight(image: &RgbaImage) -> RgbaImage {
        let dark: Vec<(u32, u32)> = image
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0[0] < 128)
            .map(|(x, y, _)| (x, y))
            .collect();
        let (x0, x1) = (dark.iter().map(|p| p.0).min().unwrap(), dark.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (dark.iter().map(|p| p.1).min().unwrap(), dark.iter().map(|p| p.1).max().unwrap());
        image::imageops::crop_imm(image, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image()
    }

    fn found(image: &RgbaImage) -> Vec<(CodeFormat, String)> {
        decode(image).unwrap().into_iter().map(|item| (item.format, item.text)).collect()
    }

    #[test]
    fn web_urls_require_http_scheme_and_host() {
        assert_eq!(super::web_url(" https://example.com/a ").as_deref(), Some("https://example.com/a"));
        assert!(super::web_url("javascript:alert(1)").is_none());
        assert!(super::web_url("file:///etc/passwd").is_none());
        assert!(super::web_url("hello").is_none());
        assert!(decode(&RgbaImage::new(8, 8)).unwrap().is_empty());
        assert!(decode(&RgbaImage::new(0, 0)).unwrap().is_empty());
        let english = crate::i18n::for_language(&crate::settings::Language::English, super::FAILED);
        assert_ne!(english, super::FAILED, "missing English entry");
    }

    #[test]
    fn decodes_linear_barcodes() {
        // EAN / UPC 的最后一位是校验位；EAN-13 以 0 开头时按惯例报告为 UPC-A。
        let cases = [
            (BarcodeFormat::CODE_128, CodeFormat::Code128, "Snap-Desk 128"),
            (BarcodeFormat::CODE_39, CodeFormat::Code39, "SNAP39"),
            (BarcodeFormat::CODE_93, CodeFormat::Code93, "SNAP93"),
            (BarcodeFormat::EAN_13, CodeFormat::Ean13, "5901234123457"),
            (BarcodeFormat::EAN_8, CodeFormat::Ean8, "96385074"),
            (BarcodeFormat::UPC_A, CodeFormat::UpcA, "036000291452"),
            (BarcodeFormat::UPC_E, CodeFormat::UpcE, "01234565"),
            (BarcodeFormat::ITF, CodeFormat::Itf, "1234567890"),
        ];
        for (encode, format, text) in cases {
            let items = decode(&plain(encode, text)).unwrap();
            assert_eq!(items.len(), 1, "{format:?}: {items:?}");
            assert_eq!((items[0].format, items[0].text.as_str()), (format, text));
            assert!(items[0].bounds.is_some() && items[0].url.is_none());
        }
    }

    #[test]
    fn decodes_two_dimensional_codes() {
        let cases = [
            (BarcodeFormat::QR_CODE, CodeFormat::QrCode, "https://example.com/snap?q=1"),
            (BarcodeFormat::DATA_MATRIX, CodeFormat::DataMatrix, "SnapDesk-DM-42"),
            (BarcodeFormat::PDF_417, CodeFormat::Pdf417, "Snap Desk PDF417"),
            (BarcodeFormat::AZTEC, CodeFormat::Aztec, "Snap Desk Aztec"),
        ];
        for (encode, format, text) in cases {
            assert_eq!(found(&plain(encode, text)), vec![(format, text.to_owned())], "{format:?}");
        }
        let items = decode(&plain(BarcodeFormat::QR_CODE, "https://example.com/snap?q=1")).unwrap();
        assert_eq!(items[0].url.as_deref(), Some("https://example.com/snap?q=1"));
        assert_eq!(CodeFormat::Ean13.name(), "EAN-13");
    }

    #[test]
    fn finds_every_code_in_reading_order() {
        // 两行：QR 与 EAN-13、QR 与 Code 128；同一张 QR 再贴一次只报告一次。
        let mut canvas = RgbaImage::from_pixel(1000, 700, WHITE);
        let paste = |canvas: &mut RgbaImage, image: &RgbaImage, x: i64, y: i64| image::imageops::overlay(canvas, image, x, y);
        paste(&mut canvas, &plain(BarcodeFormat::QR_CODE, "first"), 20, 20);
        paste(&mut canvas, &plain(BarcodeFormat::EAN_13, "5901234123457"), 420, 40);
        paste(&mut canvas, &plain(BarcodeFormat::QR_CODE, "second"), 20, 360);
        paste(&mut canvas, &plain(BarcodeFormat::CODE_128, "ROW-2"), 420, 400);
        paste(&mut canvas, &plain(BarcodeFormat::QR_CODE, "first"), 760, 360);
        let codes: Vec<(CodeFormat, String)> = found(&canvas);
        let expected = [
            (CodeFormat::QrCode, "first"),
            (CodeFormat::Ean13, "5901234123457"),
            (CodeFormat::QrCode, "second"),
            (CodeFormat::Code128, "ROW-2"),
        ];
        assert_eq!(codes, expected.map(|(f, t)| (f, t.to_owned())).to_vec());
        // 位置换算回原图坐标：第一个 QR 在左上角附近。
        let first = decode(&canvas).unwrap()[0].bounds.unwrap();
        assert!(first.x >= 20.0 && first.x < 60.0 && first.y >= 20.0 && first.y < 60.0, "{first:?}");
    }

    #[test]
    fn tolerates_tight_crops_inverted_and_transparent_codes() {
        let qr = plain(BarcodeFormat::QR_CODE, "tight");
        assert_eq!(found(&tight(&qr)), vec![(CodeFormat::QrCode, "tight".to_owned())]);
        let ean = plain(BarcodeFormat::EAN_13, "5901234123457");
        assert_eq!(found(&tight(&ean)), vec![(CodeFormat::Ean13, "5901234123457".to_owned())]);
        // 深色主题里的浅色码。
        let inverted = render(BarcodeFormat::QR_CODE, "dark", 3, 20, WHITE, Rgba([24, 24, 27, 255]));
        assert_eq!(found(&inverted), vec![(CodeFormat::QrCode, "dark".to_owned())]);
        // 透明底（全透明像素的颜色值为 0）按白底处理。
        let transparent = render(BarcodeFormat::CODE_128, "ALPHA", 3, 20, BLACK, Rgba([0, 0, 0, 0]));
        assert_eq!(found(&transparent), vec![(CodeFormat::Code128, "ALPHA".to_owned())]);
    }

    #[test]
    fn reading_order_groups_rows_then_columns() {
        let item = |text: &str, x: f32, y: f32, height: f32| CodeItem {
            format: CodeFormat::QrCode,
            text: text.to_owned(),
            url: None,
            bounds: Some(CodeBounds { x, y, width: 50.0, height }),
        };
        // 右边的码上沿高出 2 像素，仍与左边同一行；一维条码只有扫描线（高度 0），按所在行归并。
        let items = vec![
            item("row2-left", 0.0, 200.0, 100.0),
            item("row1-right", 300.0, 8.0, 100.0),
            item("row1-left", 0.0, 10.0, 100.0),
            item("row1-line", 150.0, 60.0, 0.0),
            CodeItem { bounds: None, ..item("unknown", 0.0, 0.0, 0.0) },
        ];
        let order: Vec<String> = reading_order(items).into_iter().map(|item| item.text).collect();
        assert_eq!(order, ["row1-left", "row1-line", "row1-right", "row2-left", "unknown"]);
    }
}
