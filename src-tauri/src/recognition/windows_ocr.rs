//! Windows.Media.Ocr：直接从内存灰度像素识别，不写 PNG、不加载 Paddle/ONNX 模型。
use super::{ocr_config::OcrLanguage, OcrLine, OcrResult, PolygonPoint};
use std::{marker::PhantomData, rc::Rc, time::Instant};
use windows::{
    core::HSTRING,
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

/// 阻塞工作线程按次初始化 MTA，并在同一线程释放；标记阻止跨线程移动清理器。
struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
    fn enter() -> Result<Self, String> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED).map_err(failure)?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
fn failure(error: windows::core::Error) -> String {
    format!("Windows OCR 调用失败（{}）。", error.code())
}

/// 每次查询系统实际安装的语言，安装语言包后刷新即可看到，不缓存失败状态。
pub fn languages() -> Result<Vec<OcrLanguage>, String> {
    let _runtime = Runtime::enter()?;
    OcrEngine::AvailableRecognizerLanguages()
        .map_err(failure)?
        .into_iter()
        .map(|language| {
            Ok(OcrLanguage {
                tag: language.LanguageTag().map_err(failure)?.to_string(),
                name: language.NativeName().map_err(failure)?.to_string(),
            })
        })
        .collect()
}

pub fn recognize(image: &image::RgbaImage, language: &str) -> Result<OcrResult, String> {
    let started = Instant::now();
    let op = super::ocr::next_request();
    let result = recognize_inner(image, language, format!("ocr-{op}"));
    match result {
        Ok(mut result) => {
            result.elapsed_ms = started.elapsed().as_millis() as u64;
            log::info!(
                "event=ocr.finished module=ocr op=ocr-{op} duration_ms={} lines={}",
                result.elapsed_ms,
                result.lines.len()
            );
            Ok(result)
        }
        Err(error) => {
            log::error!(
                "event=ocr.failed module=ocr op=ocr-{op} code=OCR_SYSTEM_FAILED duration_ms={}",
                started.elapsed().as_millis()
            );
            Err(error)
        }
    }
}

fn recognize_inner(
    image: &image::RgbaImage,
    language: &str,
    request_id: String,
) -> Result<OcrResult, String> {
    super::ocr::validate_image(image)?;
    let _runtime = Runtime::enter()?;
    let engine = if language == "auto" {
        OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|_| "无法创建 Windows OCR 引擎，请检查系统首选语言的 OCR 语言包。")?
    } else {
        let selected = Language::CreateLanguage(&HSTRING::from(language)).map_err(failure)?;
        if !OcrEngine::IsLanguageSupported(&selected).map_err(failure)? {
            return Err(
                "所选 Windows OCR 语言包未安装，请在 Windows 语言设置中安装后重试。".into(),
            );
        }
        OcrEngine::TryCreateFromLanguage(&selected)
            .map_err(|_| "无法创建 Windows OCR 引擎，请检查所选系统 OCR 语言包。")?
    };
    let tag = engine
        .RecognizerLanguage()
        .map_err(failure)?
        .LanguageTag()
        .map_err(failure)?
        .to_string();
    let maximum = OcrEngine::MaxImageDimension().map_err(failure)?;
    if maximum == 0 {
        return Err("Windows OCR 返回了无效的图像尺寸限制。".into());
    }
    // 先铺白底再转灰度；超过系统上限时等比缩小，最终坐标还原到裁剪原图像素。
    let gray = image::DynamicImage::ImageRgb8(crate::shared::image_io::white_background(image))
        .into_luma8();
    let resized = if gray.width() > maximum || gray.height() > maximum {
        image::DynamicImage::ImageLuma8(gray)
            .thumbnail(maximum, maximum)
            .into_luma8()
    } else {
        gray
    };
    let writer = DataWriter::new().map_err(failure)?;
    writer.WriteBytes(resized.as_raw()).map_err(failure)?;
    let buffer = writer.DetachBuffer().map_err(failure)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Gray8,
        resized.width() as i32,
        resized.height() as i32,
    )
    .map_err(failure)?;
    let recognized = engine
        .RecognizeAsync(&bitmap)
        .map_err(failure)?
        .join()
        .map_err(failure)?;
    let scale_x = image.width() as f32 / resized.width() as f32;
    let scale_y = image.height() as f32 / resized.height() as f32;
    let mut lines = Vec::new();
    for line in recognized.Lines().map_err(failure)? {
        let text = line.Text().map_err(failure)?.to_string();
        if text.trim().is_empty() {
            continue;
        }
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for word in line.Words().map_err(failure)? {
            let rect = word.BoundingRect().map_err(failure)?;
            let (left, top, right, bottom) =
                (rect.X, rect.Y, rect.X + rect.Width, rect.Y + rect.Height);
            if ![left, top, right, bottom]
                .iter()
                .all(|value| value.is_finite())
                || rect.Width <= 0.0
                || rect.Height <= 0.0
            {
                continue;
            }
            bounds = Some(match bounds {
                Some((l, t, r, b)) => (l.min(left), t.min(top), r.max(right), b.max(bottom)),
                None => (left, top, right, bottom),
            });
        }
        let polygon = bounds.map(|(l, t, r, b)| {
            [(l, t), (r, t), (r, b), (l, b)]
                .into_iter()
                .map(|(x, y)| PolygonPoint {
                    x: (x * scale_x).clamp(0.0, image.width() as f32),
                    y: (y * scale_y).clamp(0.0, image.height() as f32),
                })
                .collect()
        });
        let order = lines.len();
        lines.push(OcrLine {
            id: format!("line-{:04}", order + 1),
            text,
            confidence: None,
            polygon,
            language: Some(tag.clone()),
            order,
            paragraph_id: None,
        });
    }
    Ok(OcrResult {
        request_id,
        provider: "windows-media".into(),
        image_width: image.width(),
        image_height: image.height(),
        full_text: lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        lines,
        elapsed_ms: 0,
        warnings: vec![],
    })
}
