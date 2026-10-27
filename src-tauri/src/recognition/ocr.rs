//! 截图文字识别：复用 OAR-OCR 的检测、透视裁剪及 CTC 解码，通过 ort 在 CPU 上推理。
use crate::recognition::{OcrLine, OcrResult, PolygonPoint};
use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{path::Path, sync::Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 模型首次使用时加载；识别和回收共用锁，避免卸载正在推理的会话。
#[derive(Default)]
pub struct OcrState(Mutex<Option<CachedEngine>>);

struct CachedEngine {
    engine: OAROCR,
    /// 从最后一次推理结束计时，失败的推理也会刷新空闲时间。
    last_used: Instant,
}

/// 空闲五分钟后释放会话；运行库及分配器是否归还内存需通过实机采样判断。
pub fn release_idle(app: &AppHandle) {
    let state = app.state::<OcrState>();
    if let Ok(mut slot) = state.0.try_lock() {
        if slot
            .as_ref()
            .is_some_and(|cached| cached.last_used.elapsed() >= Duration::from_secs(300))
        {
            // 在持锁期间完成析构，避免旧模型尚未释放时又初始化另一份模型。
            drop(slot.take());
            log::info!("event=ocr.engine_unloaded module=ocr reason=idle");
        }
    };
}

pub fn recognize(app: &AppHandle, image: &image::RgbaImage) -> Result<String, String> {
    Ok(recognize_structured(app, image)?.full_text)
}

/// 新旧入口共享模型锁与过滤规则，结构化识别不会额外加载模型。
pub fn recognize_structured(
    app: &AppHandle,
    image: &image::RgbaImage,
) -> Result<OcrResult, String> {
    let started = Instant::now();
    let op = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut code = "OCR_INVALID_REGION";
    let result = (|| {
        if image.width() == 0 || image.height() == 0 {
            return Err("OCR 选区不能为空。".into());
        }
        if u64::from(image.width()) * u64::from(image.height()) > 16_000_000 {
            return Err("OCR 选区超过 1600 万像素，请缩小范围。".into());
        }
        code = "OCR_BUSY";
        let state = app.state::<OcrState>();
        let mut engine = state.0.try_lock().map_err(|_| "已有文字识别正在进行。")?;
        if engine.is_none() {
            code = "OCR_RESOURCES_UNAVAILABLE";
            let dir = crate::resources::ocr_paths(app)?;
            code = "OCR_ENGINE_INIT_FAILED";
            let loading = Instant::now();
            *engine = Some(CachedEngine {
                engine: build_engine(&dir)?,
                last_used: Instant::now(),
            });
            log::info!(
                "event=ocr.engine_ready module=ocr op=ocr-{op} duration_ms={}",
                loading.elapsed().as_millis()
            );
        }
        code = "OCR_INFERENCE_FAILED";
        let inference = Instant::now();
        let cached = engine.as_mut().ok_or("OCR 未初始化。")?;
        let result = recognize_regions(&cached.engine, image);
        cached.last_used = Instant::now();
        let mut result = result?;
        result.request_id = format!("ocr-{op}");
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        log::info!(
            "event=ocr.finished module=ocr op=ocr-{op} duration_ms={} inference_ms={} lines={}",
            result.elapsed_ms,
            inference.elapsed().as_millis(),
            result.lines.len()
        );
        Ok(result)
    })();
    if result.is_err() {
        log::error!(
            "event=ocr.failed module=ocr op=ocr-{op} code={code} duration_ms={}",
            started.elapsed().as_millis()
        );
    }
    result
}

/// 与性能测试共用模型加载和推理路径，避免基准只测到与产品不同的代码。
fn build_engine(dir: &Path) -> Result<OAROCR, String> {
    // 必须先显式加载可信路径，防止 ort 回退到 PATH/环境变量搜索并在缺少 DLL 时 panic。
    #[cfg(windows)]
    crate::recognition::ocr_runtime::initialize(dir)?;
    // 检测与识别各有会话；默认线程数会争抢 CPU，默认整页批次还会放大张量内存。
    // 桌面交互优先：每次最多识别 8 行，每个会话最多使用两个推理线程。
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(2))
        .unwrap_or(1);
    OAROCRBuilder::new(
        dir.join("pp-ocrv6_small_det.onnx"),
        dir.join("pp-ocrv6_small_rec.onnx"),
        dir.join("ppocrv6_dict.txt"),
    )
    .region_batch_size(8)
    .ort_session(
        OrtSessionConfig::new()
            .with_intra_threads(threads)
            .with_inter_threads(1),
    )
    .build()
    .map_err(|e| format!("OCR 模型加载失败：{e}"))
}

#[cfg(test)]
fn recognize_image(engine: &OAROCR, image: &image::RgbaImage) -> Result<String, String> {
    Ok(recognize_regions(engine, image)?.full_text)
}

fn recognize_regions(engine: &OAROCR, image: &image::RgbaImage) -> Result<OcrResult, String> {
    let rgb = crate::shared::image_io::white_background(image);
    let results = engine
        .predict(vec![rgb])
        .map_err(|e| format!("文字识别失败：{e}"))?;
    let lines: Vec<OcrLine> = results
        .iter()
        .flat_map(|r| r.text_regions.iter())
        .filter_map(|region| {
            let (text, confidence) = region.text_with_confidence()?;
            if text.trim().is_empty() || !confidence.is_finite() || confidence < 0.5 {
                return None;
            }
            // OAR 已将检测坐标还原至输入图像；不再乘检测网络的缩放比例。
            let points = &region.bounding_box.points;
            let polygon = (points.len() == 4
                && points.iter().all(|p| p.x.is_finite() && p.y.is_finite()))
            .then(|| {
                points
                    .iter()
                    .map(|p| PolygonPoint {
                        x: p.x.clamp(0.0, image.width() as f32),
                        y: p.y.clamp(0.0, image.height() as f32),
                    })
                    .collect()
            });
            Some((text.to_string(), confidence, polygon))
        })
        .enumerate()
        .map(|(order, (text, confidence, polygon))| OcrLine {
            id: format!("line-{:04}", order + 1),
            text,
            confidence: Some(confidence),
            polygon,
            language: None,
            order,
            paragraph_id: None,
        })
        .collect();
    Ok(OcrResult {
        request_id: format!("ocr-{}", REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)),
        provider: "local-ppocr".into(),
        image_width: image.width(),
        image_height: image.height(),
        full_text: lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        lines,
        elapsed_ms: 0,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore = "需预先下载真实模型；用 --release --ignored --nocapture 显式测量"]
    fn perf_ocr_cpu() {
        let start = Instant::now();
        let engine = build_engine(&crate::shared::perf::assets()).unwrap();
        println!("PERF ocr_load_ms={}", start.elapsed().as_millis());
        for (width, height) in [(1200, 500), (1920, 1080)] {
            let image = crate::shared::perf::screenshot(width, height);
            let mut times = Vec::new();
            for _ in 0..4 {
                let start = Instant::now();
                let text = recognize_image(&engine, &image).unwrap();
                times.push(start.elapsed().as_millis());
                assert!(text.contains("中文"), "{text}");
                assert!(text.contains("English"), "{text}");
                assert_eq!(
                    text.lines().count(),
                    ((height - 60) / 48) as usize,
                    "批次处理不得遗漏文本行：{text}"
                );
            }
            println!("PERF ocr_{width}x{height}_ms={times:?}");
        }
        let blank = image::RgbaImage::from_pixel(3840, 2160, image::Rgba([255; 4]));
        let start = Instant::now();
        assert!(recognize_image(&engine, &blank).unwrap().is_empty());
        println!("PERF ocr_blank_4k_ms={}", start.elapsed().as_millis());
    }
}

/// 结果允许用户编辑；仅截图或仍存在的贴图窗口可写文本剪贴板，且限制消息大小。
#[tauri::command]
pub fn copy_ocr_text(app: AppHandle, window: WebviewWindow, text: String) -> Result<(), String> {
    if window.label() != "capture" {
        crate::pins::require_pin(&app, &window)?;
    }
    if text.is_empty() || text.len() > 1024 * 1024 {
        return Err("复制文本为空或超过 1 MiB。".into());
    }
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}
