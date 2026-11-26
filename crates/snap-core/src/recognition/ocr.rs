//! OCR 结果契约与本地 PP-OCR 引擎（OAR-OCR 检测、透视裁剪与 CTC 解码，ort CPU 推理）。
//! 坐标始终相对裁剪原图，单位为像素，不能当作屏幕 CSS 坐标。
use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize)]
pub struct PolygonPoint {
    pub x: f32,
    pub y: f32,
}

/// 保留引擎阅读顺序；没有可靠坐标或语言判断时返回 null。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub id: String,
    pub text: String,
    pub confidence: Option<f32>,
    pub polygon: Option<Vec<PolygonPoint>>,
    pub language: Option<String>,
    pub order: usize,
    pub paragraph_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub request_id: String,
    pub provider: String,
    pub image_width: u32,
    pub image_height: u32,
    pub full_text: String,
    pub lines: Vec<OcrLine>,
    pub elapsed_ms: u64,
    pub warnings: Vec<String>,
    /// 由调度入口按当前设置填入，各引擎构造时一律为 false。
    pub auto_copy: bool,
    pub merge_lines: bool,
}

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// OCR 引擎共用递增编号，切换引擎后日志与结果仍能唯一关联。
pub fn next_request() -> u64 {
    REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

/// 离线引擎共用输入边界，先检查原图大小再进行缩放或分配像素缓冲区。
pub fn validate_image(image: &image::RgbaImage) -> Result<(), String> {
    if image.width() == 0 || image.height() == 0 {
        return Err("OCR 选区不能为空。".into());
    }
    if u64::from(image.width()) * u64::from(image.height()) > 16_000_000 {
        return Err("OCR 选区超过 1600 万像素，请缩小范围。".into());
    }
    Ok(())
}

pub(crate) fn cjk(c: char) -> bool {
    // 2E80–9FFF 已包含 CJK 标点（3000–303F）。
    matches!(c, '\u{2e80}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{ff00}'..='\u{ffef}')
}

/// 合并换行（与前端 `mergeLines` 一致）：英文连字符断词直接拼接，中日韩文字之间不加空格，
/// 其他换行用空格连接；空行保留为段落分隔。
pub fn merge_lines(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs: Vec<Vec<&str>> = vec![Vec::new()];
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            if !paragraphs.last().is_some_and(Vec::is_empty) {
                paragraphs.push(Vec::new());
            }
        } else if let Some(last) = paragraphs.last_mut() {
            last.push(line);
        }
    }
    paragraphs
        .iter()
        .filter(|lines| !lines.is_empty())
        .map(|lines| {
            let mut out = String::new();
            for line in lines {
                if out.is_empty() {
                    out.push_str(line);
                } else if out.ends_with('-') {
                    out.pop();
                    out.push_str(line);
                } else if out.chars().last().is_some_and(cjk) && line.chars().next().is_some_and(cjk) {
                    out.push_str(line);
                } else {
                    out.push(' ');
                    out.push_str(line);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 识别结果进入可编辑草稿前的整理：设置开启“自动合并换行”时先合并。
pub fn initial_text(result: &OcrResult) -> String {
    if result.merge_lines {
        merge_lines(&result.full_text)
    } else {
        result.full_text.clone()
    }
}

/// 本地 PP-OCR：模型首次使用时加载；识别和回收共用锁，避免卸载正在推理的会话。
#[derive(Default)]
pub struct LocalOcr(Mutex<Option<CachedEngine>>);

struct CachedEngine {
    engine: OAROCR,
    /// 从最后一次推理结束计时，失败的推理也会刷新空闲时间。
    last_used: Instant,
}

impl LocalOcr {
    /// 空闲五分钟后释放会话；运行库及分配器是否归还内存需通过实机采样判断。
    pub fn release_idle(&self) {
        if let Ok(mut slot) = self.0.try_lock() {
            if slot
                .as_ref()
                .is_some_and(|cached| cached.last_used.elapsed() >= Duration::from_secs(300))
            {
                // 在持锁期间完成析构，避免旧模型尚未释放时又初始化另一份模型。
                drop(slot.take());
                log::info!("event=ocr.engine_unloaded module=ocr engine=local-ppocr reason=idle");
            }
        }
    }

    /// `models` 只在需要加载模型时调用，返回已通过校验的资源目录。
    pub fn recognize(
        &self,
        image: &image::RgbaImage,
        models: impl FnOnce() -> Result<PathBuf, String>,
    ) -> Result<OcrResult, String> {
        let started = Instant::now();
        let op = next_request();
        let mut code = "OCR_INVALID_REGION";
        let result = (|| {
            validate_image(image)?;
            code = "OCR_BUSY";
            let mut engine = self.0.try_lock().map_err(|_| "已有文字识别正在进行。")?;
            if engine.is_none() {
                code = "OCR_RESOURCES_UNAVAILABLE";
                let dir = models()?;
                code = "OCR_ENGINE_INIT_FAILED";
                let loading = Instant::now();
                *engine = Some(CachedEngine {
                    engine: build_engine(&dir)?,
                    last_used: Instant::now(),
                });
                log::info!(
                    "event=ocr.engine_ready module=ocr engine=local-ppocr op=ocr-{op} duration_ms={}",
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
                "event=ocr.finished module=ocr engine=local-ppocr op=ocr-{op} duration_ms={} inference_ms={} lines={}",
                result.elapsed_ms,
                inference.elapsed().as_millis(),
                result.lines.len()
            );
            Ok(result)
        })();
        if result.is_err() {
            log::error!(
                "event=ocr.failed module=ocr engine=local-ppocr op=ocr-{op} code={code} duration_ms={}",
                started.elapsed().as_millis()
            );
        }
        result
    }
}

/// 与性能测试共用模型加载和推理路径，避免基准只测到与产品不同的代码。
fn build_engine(dir: &Path) -> Result<OAROCR, String> {
    // 必须先显式加载可信路径，防止 ort 回退到 PATH/环境变量搜索并在缺少 DLL 时 panic。
    #[cfg(windows)]
    super::ocr_runtime::initialize(dir)?;
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

fn recognize_regions(engine: &OAROCR, image: &image::RgbaImage) -> Result<OcrResult, String> {
    let rgb = crate::export::white_background(image);
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
        request_id: String::new(), // 外层在识别完成后统一填入本次任务编号。
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
        auto_copy: false,
        merge_lines: false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn merge_lines_follows_frontend_rules() {
        assert_eq!(super::merge_lines("hyph-\nenated word\nnext"), "hyphenated word next");
        assert_eq!(super::merge_lines("中文\n继续\n\n\nnew para\r\nline"), "中文继续\n\nnew para line");
        assert_eq!(super::merge_lines("  a  \n\n"), "a");
    }

    #[test]
    fn rejects_empty_and_oversized_images_before_loading_models() {
        assert!(super::validate_image(&image::RgbaImage::new(0, 4)).is_err());
        assert!(super::validate_image(&image::RgbaImage::new(4, 4)).is_ok());
        let local = super::LocalOcr::default();
        let error = local
            .recognize(&image::RgbaImage::new(4, 4), || Err("缺少模型".into()))
            .unwrap_err();
        assert_eq!(error, "缺少模型");
    }
}
