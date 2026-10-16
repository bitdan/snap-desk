//! 截图文字识别：复用 OAR-OCR 的检测、透视裁剪及 CTC 解码，通过 ort 在 CPU 上推理。
use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::prelude::*;
use std::{path::Path, sync::Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// 模型首次使用时加载，后续复用；互斥锁限制同时只有一个识别任务，控制内存峰值。
#[derive(Default)]
pub struct OcrState(Mutex<Option<OAROCR>>);

pub fn recognize(app: &AppHandle, image: &image::RgbaImage) -> Result<String, String> {
    if u64::from(image.width()) * u64::from(image.height()) > 16_000_000 {
        return Err("OCR 选区超过 1600 万像素，请缩小范围。".into());
    }
    let state = app.state::<OcrState>();
    let mut engine = state.0.try_lock().map_err(|_| "已有文字识别正在进行。")?;
    if engine.is_none() {
        let dir = crate::resources::ocr_paths(app)?;
        *engine = Some(build_engine(&dir)?);
    }
    recognize_image(engine.as_ref().ok_or("OCR 未初始化。")?, image)
}

/// 与性能测试共用模型加载和推理路径，避免基准只测到与产品不同的代码。
fn build_engine(dir: &Path) -> Result<OAROCR, String> {
    // 检测与识别各有会话；默认线程数会争抢 CPU，默认整页批次还会放大张量内存。
    // 桌面交互优先：每次最多识别 8 行，每个会话最多使用两个推理线程。
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(2))
        .unwrap_or(1);
    OAROCRBuilder::new(
        dir.join("pp-ocrv5_mobile_det.onnx"),
        dir.join("pp-ocrv5_mobile_rec.onnx"),
        dir.join("ppocrv5_dict.txt"),
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

fn recognize_image(engine: &OAROCR, image: &image::RgbaImage) -> Result<String, String> {
    let rgb = crate::image_io::white_background(image);
    let results = engine
        .predict(vec![rgb])
        .map_err(|e| format!("文字识别失败：{e}"))?;
    Ok(results
        .iter()
        .flat_map(|r| r.text_regions.iter())
        .filter_map(|r| r.text_with_confidence())
        .filter(|(text, confidence)| !text.trim().is_empty() && *confidence >= 0.5)
        .map(|(text, _)| text.to_string())
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore = "需预先下载真实模型；用 --release --ignored --nocapture 显式测量"]
    fn perf_ocr_cpu() {
        let start = Instant::now();
        let engine = build_engine(&crate::perf::assets()).unwrap();
        println!("PERF ocr_load_ms={}", start.elapsed().as_millis());
        for (width, height) in [(1200, 500), (1920, 1080)] {
            let image = crate::perf::screenshot(width, height);
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

/// 结果允许用户编辑；仅截图窗口可写文本剪贴板，且限制消息大小。
#[tauri::command]
pub fn copy_ocr_text(app: AppHandle, window: WebviewWindow, text: String) -> Result<(), String> {
    crate::capture::require_window(&window, "capture")?;
    if text.is_empty() || text.len() > 1024 * 1024 {
        return Err("复制文本为空或超过 1 MiB。".into());
    }
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}
