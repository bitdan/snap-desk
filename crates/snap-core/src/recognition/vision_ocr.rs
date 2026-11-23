//! macOS 系统 OCR：Apple Vision 的 VNRecognizeTextRequest，直接识别内存位图，不写临时文件。
//!
//! 与 Windows.Media.Ocr 共用提供方标识 `windows-media`（即“系统 OCR”），配置、诊断与前端契约不变。
//! Vision 的框坐标是归一化值、原点在左下角；这里换算为裁剪原图的像素坐标（原点左上）。
use super::{ocr::{OcrLine, OcrResult, PolygonPoint}, ocr_config::OcrLanguage};
use crate::macos::{array_items, ns_array, ns_string, rust_string, Id};
use objc2::{
    class,
    encode::{Encode, Encoding},
    msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, Bool},
};
use std::{ffi::c_void, time::Instant};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGDataProviderCreateWithData(
        info: *mut c_void,
        data: *const c_void,
        size: usize,
        release: *const c_void,
    ) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const f64,
        interpolate: bool,
        intent: i32,
    ) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

/// 传给 Objective-C 的 CGImageRef；声明为 `^{CGImage=}` 以匹配方法签名的类型编码。
#[repr(transparent)]
#[derive(Clone, Copy)]
struct CGImageRef(*mut c_void);
unsafe impl Encode for CGImageRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

/// 内存位图包装成 CGImage：像素缓冲由调用方持有，CGImage 释放前不得丢弃。
struct Bitmap {
    image: *mut c_void,
}
impl Bitmap {
    fn new(pixels: &image::RgbImage) -> Result<Self, String> {
        let (width, height) = (pixels.width() as usize, pixels.height() as usize);
        unsafe {
            let space = CGColorSpaceCreateDeviceRGB();
            let provider = CGDataProviderCreateWithData(
                std::ptr::null_mut(),
                pixels.as_raw().as_ptr() as *const c_void,
                pixels.as_raw().len(),
                std::ptr::null(),
            );
            // 8 位 RGB、无 Alpha（kCGImageAlphaNone = 0），默认渲染意图。
            let image = CGImageCreate(
                width,
                height,
                8,
                24,
                width * 3,
                space,
                0,
                provider,
                std::ptr::null(),
                false,
                0,
            );
            CGDataProviderRelease(provider);
            CGColorSpaceRelease(space);
            if image.is_null() {
                return Err("系统 OCR 无法读取图片。".into());
            }
            Ok(Self { image })
        }
    }
}
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { CGImageRelease(self.image) }
    }
}

fn new_request() -> Result<Retained<AnyObject>, String> {
    let request: Option<Retained<AnyObject>> =
        unsafe { msg_send![class!(VNRecognizeTextRequest), new] };
    request.ok_or_else(|| "无法创建 Apple Vision 文字识别请求。".into())
}

/// 系统支持的识别语言（随系统版本更新）；名称按当前界面区域本地化。
pub fn languages() -> Result<Vec<OcrLanguage>, String> {
    let request = new_request()?;
    objc2::rc::autoreleasepool(|_| unsafe {
        let mut error: Id = std::ptr::null_mut();
        let tags: Id = msg_send![
            &*request,
            supportedRecognitionLanguagesAndReturnError: &mut error as *mut Id
        ];
        if tags.is_null() {
            return Err("无法读取 Apple Vision 支持的识别语言。".into());
        }
        let locale: Id = msg_send![class!(NSLocale), currentLocale];
        Ok(array_items(tags)
            .into_iter()
            .filter_map(|tag| {
                let code = rust_string(tag)?;
                let name: Id = msg_send![locale, localizedStringForLocaleIdentifier: tag];
                Some(OcrLanguage {
                    name: rust_string(name).unwrap_or_else(|| code.clone()),
                    tag: code,
                })
            })
            .collect())
    })
}

/// Vision 不保证阅读顺序：先按顶部排序，再把顶部与本行首个框相差不足 4 像素的框归为同一行、
/// 行内按左侧排序。分两步做是因为“相差不足 4 像素”不满足传递性，不能直接写进比较函数。
fn reading_order<T>(mut lines: Vec<(f32, f32, T)>) -> Vec<T> {
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut ordered = Vec::with_capacity(lines.len());
    let mut row: Vec<(f32, f32, T)> = Vec::new();
    for line in lines {
        if row.first().is_some_and(|first| line.0 - first.0 >= 4.0) {
            row.sort_by(|a, b| a.1.total_cmp(&b.1));
            ordered.extend(row.drain(..).map(|(_, _, value)| value));
        }
        row.push(line);
    }
    row.sort_by(|a, b| a.1.total_cmp(&b.1));
    ordered.extend(row.into_iter().map(|(_, _, value)| value));
    ordered
}

pub fn recognize(image: &image::RgbaImage, language: &str) -> Result<OcrResult, String> {
    let started = Instant::now();
    let op = super::ocr::next_request();
    let result = recognize_inner(image, language, format!("ocr-{op}"));
    // 系统 OCR 在两个平台共用同一引擎标识，日志字段才能通过诊断白名单。
    match result {
        Ok(mut result) => {
            result.elapsed_ms = started.elapsed().as_millis() as u64;
            log::info!(
                "event=ocr.finished module=ocr engine=windows-media op=ocr-{op} duration_ms={} lines={}",
                result.elapsed_ms,
                result.lines.len()
            );
            Ok(result)
        }
        Err(error) => {
            log::error!(
                "event=ocr.failed module=ocr engine=windows-media op=ocr-{op} code=OCR_SYSTEM_FAILED duration_ms={}",
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
    // 透明区域铺白底，避免被当成黑色背景影响识别。
    let pixels = crate::export::white_background(image);
    let bitmap = Bitmap::new(&pixels)?;
    let request = new_request()?;
    let (width, height) = (image.width() as f32, image.height() as f32);
    let lines: Vec<(f32, f32, OcrLine)> = objc2::rc::autoreleasepool(|_| unsafe {
        // VNRequestTextRecognitionLevelAccurate = 0：截图文字通常不多，优先准确率。
        let _: () = msg_send![&*request, setRecognitionLevel: 0isize];
        let _: () = msg_send![&*request, setUsesLanguageCorrection: Bool::YES];
        if language == "auto" {
            let _: () = msg_send![&*request, setAutomaticallyDetectsLanguage: Bool::YES];
        } else {
            let tags = ns_array(&[ns_string(language)]);
            let _: () = msg_send![&*request, setRecognitionLanguages: tags];
        }
        let options: Id = msg_send![class!(NSDictionary), dictionary];
        let allocated: Allocated<AnyObject> = msg_send![class!(VNImageRequestHandler), alloc];
        let handler: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithCGImage: CGImageRef(bitmap.image),
            options: options
        ];
        let handler = handler.ok_or("无法创建 Apple Vision 识别任务。")?;
        let requests = ns_array(&[Retained::as_ptr(&request) as Id]);
        let mut error: Id = std::ptr::null_mut();
        let performed: Bool =
            msg_send![&*handler, performRequests: requests, error: &mut error as *mut Id];
        if !performed.as_bool() {
            return Err("Apple Vision 文字识别失败。".to_string());
        }
        let results: Id = msg_send![&*request, results];
        let mut lines = Vec::new();
        for observation in array_items(results) {
            let candidates: Id = msg_send![observation, topCandidates: 1usize];
            let Some(&candidate) = array_items(candidates).first() else {
                continue;
            };
            let text_object: Id = msg_send![candidate, string];
            let Some(text) = rust_string(text_object).filter(|t| !t.trim().is_empty()) else {
                continue;
            };
            let confidence: f32 = msg_send![candidate, confidence];
            let rect: CGRect = msg_send![observation, boundingBox];
            let left = (rect.origin.x as f32 * width).clamp(0.0, width);
            let right = ((rect.origin.x + rect.size.width) as f32 * width).clamp(0.0, width);
            let top = ((1.0 - (rect.origin.y + rect.size.height)) as f32 * height).clamp(0.0, height);
            let bottom = ((1.0 - rect.origin.y) as f32 * height).clamp(0.0, height);
            let polygon = [(left, top), (right, top), (right, bottom), (left, bottom)]
                .into_iter()
                .map(|(x, y)| PolygonPoint { x, y })
                .collect();
            lines.push((
                top,
                left,
                OcrLine {
                    id: String::new(),
                    text,
                    confidence: confidence.is_finite().then_some(confidence),
                    polygon: Some(polygon),
                    language: (language != "auto").then(|| language.to_string()),
                    order: 0,
                    paragraph_id: None,
                },
            ));
        }
        Ok::<_, String>(lines)
    })?;
    drop(bitmap);
    let lines: Vec<OcrLine> = reading_order(lines)
        .into_iter()
        .enumerate()
        .map(|(order, mut line)| {
            line.id = format!("line-{:04}", order + 1);
            line.order = order;
            line
        })
        .collect();
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
        auto_copy: false,
        merge_lines: false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn lines_follow_rows_then_columns() {
        let lines = vec![(30.0, 5.0, "c"), (1.0, 50.0, "b"), (0.0, 10.0, "a"), (31.0, 1.0, "d")];
        assert_eq!(super::reading_order(lines), ["a", "b", "d", "c"]);
    }
}
