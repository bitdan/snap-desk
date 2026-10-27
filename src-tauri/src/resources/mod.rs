//! 用户主动下载 OCR 引擎、模型和字体；资源存入可写的数据目录，绝不写安装目录。
//! 临时文件经过大小、摘要校验后原子替换，失败不破坏已有文件。
pub(crate) mod lifecycle;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

static OPERATION: Mutex<()> = Mutex::new(());
const MAX_FONT: u64 = 64 * 1024 * 1024;

/// 前端只能选择资源类别，下载地址、文件名和摘要由内置清单决定。
#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    Ocr,
    Font,
}

/// 发布资源的固定身份；size 为字节数，sha256 用于发现截断或内容被替换。
#[derive(Deserialize)]
pub(crate) struct Asset {
    pub(crate) kind: ResourceKind,
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

/// 设置页展示的实际可用状态，不把“文件存在”直接当作“资源已就绪”。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    directory: String,
    ocr_ready: bool,
    models_ready: bool,
    runtime_ready: bool,
    runtime_download_mib: u64,
    font_ready: bool,
}

/// 下载进度通过主窗口事件发送；received 和 total 均以字节为单位。
#[derive(Clone, Serialize)]
pub struct Progress {
    name: String,
    received: u64,
    total: u64,
}

fn manifest() -> Result<Vec<Asset>, String> {
    serde_json::from_str(include_str!("../../assets/downloads.json")).map_err(|e| e.to_string())
}

pub fn directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("resources"))
        .map_err(|e| e.to_string())
}

pub(crate) fn verified(path: &Path, asset: &Asset) -> bool {
    if fs::metadata(path).map(|m| m.len()).ok() != Some(asset.size) {
        return false;
    }
    // 模型和 DLL 用流式摘要，避免检查设置状态时额外分配整份资源的内存。
    (|| -> std::io::Result<bool> {
        let mut file = fs::File::open(path)?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash)?;
        Ok(format!("{:x}", hash.finalize()) == asset.sha256)
    })()
    .unwrap_or(false)
}

/// 仅接受单个静态 TrueType 字体；可变字体和 TTC 集合不能可靠嵌入当前 PDF 引擎。
pub fn validate_font(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 12 || bytes.len() as u64 > MAX_FONT || bytes[..4] != [0, 1, 0, 0] {
        return Err("请选择不超过 64 MiB 的静态 TTF 字体（不支持 TTC、OTF 或可变字体）。".into());
    }
    let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let directory = bytes.get(12..12 + tables * 16).ok_or("字体表损坏。")?;
    if directory.chunks_exact(16).any(|t| &t[..4] == b"fvar") {
        return Err("请先将可变字体转换为静态 TTF，或下载推荐字体。".into());
    }
    ab_glyph::FontRef::try_from_slice(bytes).map_err(|_| "字体文件不可用。")?;
    // 同时验证 PDF 引擎，避免字体能画长图却不能嵌入 PDF。
    printpdf::PdfDocument::empty("font validation")
        .add_external_font(std::io::Cursor::new(bytes))
        .map_err(|_| "该字体无法嵌入 PDF。")?;
    Ok(())
}

fn read_font(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "请先在通用设置下载或选择导出字体。")?;
    let mut bytes = Vec::new();
    file.take(MAX_FONT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    validate_font(&bytes)?;
    Ok(bytes)
}

/// 每次导出取得独立字节快照，导出中途替换字体不会改变换行和排版。
pub fn font_bytes(app: &AppHandle) -> Result<Vec<u8>, String> {
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| "资源正在更新，请稍后导出。")?;
    read_font(&directory(app)?.join("export-font.ttf"))
}

pub fn ocr_paths(app: &AppHandle) -> Result<PathBuf, String> {
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| "资源正在更新，请稍后识别。")?;
    let dir = directory(app)?;
    for asset in manifest()?.iter().filter(|a| a.kind == ResourceKind::Ocr) {
        if !verified(&dir.join(&asset.name), asset) {
            return Err("OCR 模型未下载或校验失败，请在“识别与翻译”页下载识别资源。".into());
        }
    }
    Ok(dir)
}

fn status(app: &AppHandle) -> Result<ResourceStatus, String> {
    let dir = directory(app)?;
    let models_ready = manifest()?
        .iter()
        .filter(|a| a.kind == ResourceKind::Ocr)
        .all(|a| verified(&dir.join(&a.name), a));
    #[cfg(windows)]
    let (runtime_ready, runtime_download_mib) = (
        crate::recognition::ocr_runtime::ready(&dir),
        crate::recognition::ocr_runtime::download_mib(),
    );
    #[cfg(not(windows))]
    let (runtime_ready, runtime_download_mib) = (true, 0);
    Ok(ResourceStatus {
        models_ready,
        runtime_ready,
        runtime_download_mib,
        font_ready: read_font(&dir.join("export-font.ttf")).is_ok(),
        ocr_ready: models_ready && runtime_ready,
        directory: dir.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn resource_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ResourceStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || status(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// 下载仅使用编译时白名单地址，不接受来自 WebView 的 URL 或文件名。
#[tauri::command]
pub async fn download_resources(
    app: AppHandle,
    window: WebviewWindow,
    kind: ResourceKind,
) -> Result<ResourceStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION.try_lock().map_err(|_| "已有资源操作正在进行。")?;
        let dir = directory(&app)?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| e.to_string())?;
        #[cfg(windows)]
        if kind == ResourceKind::Ocr {
            crate::recognition::ocr_runtime::download(&app, &client, &dir)?;
        }
        for asset in manifest()?.iter().filter(|a| a.kind == kind) {
            let path = dir.join(&asset.name);
            if verified(&path, asset) {
                continue;
            }
            let file = download_file(&app, &client, &dir, asset)?;
            if kind == ResourceKind::Font {
                read_font(file.path())?;
            }
            file.as_file().sync_all().map_err(|e| e.to_string())?;
            file.persist(path).map_err(|e| e.to_string())?;
        }
        status(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 下载到临时文件，大小和摘要通过后才返回；出错自动删除，调用者负责安装。
pub(crate) fn download_file(
    app: &AppHandle,
    client: &reqwest::blocking::Client,
    dir: &Path,
    asset: &Asset,
) -> Result<tempfile::NamedTempFile, String> {
    let mut response = client
        .get(&asset.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{} 下载失败，可重试：{e}", asset.name))?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    let mut last_progress = Instant::now();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|e| format!("下载中断，可重试：{e}"))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > asset.size {
            return Err("下载文件大小与清单不符。".into());
        }
        hash.update(&buffer[..count]);
        file.write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        // 网络分块可能很小，限频避免下载期间大量 IPC 刷新拖慢设置页。
        if last_progress.elapsed() >= Duration::from_millis(100) || received == asset.size {
            let _ = app.emit_to(
                "main",
                "resource-download-progress",
                Progress {
                    name: asset.name.clone(),
                    received,
                    total: asset.size,
                },
            );
            last_progress = Instant::now();
        }
    }
    if received != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err(format!("{} 校验失败，请重试。", asset.name));
    }
    Ok(file)
}

/// 由系统对话框授权读取文件，并复制到应用目录；原文件移走后仍可导出。
#[tauri::command]
pub async fn choose_export_font(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ResourceStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION.try_lock().map_err(|_| "已有资源操作正在进行。")?;
        if let Some(path) = app
            .dialog()
            .file()
            .set_title(crate::shared::native_i18n::text(&app, "选择静态 TTF 导出字体"))
            .add_filter(crate::shared::native_i18n::text(&app, "TrueType 字体"), &["ttf"])
            .blocking_pick_file()
        {
            let bytes = read_font(&path.into_path().map_err(|e| e.to_string())?)?;
            let dir = directory(&app)?;
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let mut file = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.as_file().sync_all().map_err(|e| e.to_string())?;
            file.persist(dir.join("export-font.ttf"))
                .map_err(|e| e.to_string())?;
        }
        status(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需真实模型及完整字体，用 release 显式测量资源检查开销"]
    fn perf_resource_validation() {
        let dir = crate::shared::perf::assets();
        for _ in 0..3 {
            let start = Instant::now();
            for asset in manifest()
                .unwrap()
                .iter()
                .filter(|a| a.kind == ResourceKind::Ocr)
            {
                assert!(verified(&dir.join(&asset.name), asset));
            }
            println!("PERF verify_models_ms={}", start.elapsed().as_millis());
            let start = Instant::now();
            assert!(!read_font(&dir.join("export-font.ttf")).unwrap().is_empty());
            println!("PERF read_validate_font_ms={}", start.elapsed().as_millis());
        }
    }

    #[test]
    fn font_validation_rejects_corrupt_and_variable_fonts() {
        assert!(validate_font(b"not a font").is_err());
        let mut header = vec![0u8; 28];
        header[..4].copy_from_slice(&[0, 1, 0, 0]);
        header[5] = 1;
        header[12..16].copy_from_slice(b"fvar");
        assert!(validate_font(&header).unwrap_err().contains("可变字体"));
        validate_font(include_bytes!("../../tests/fixtures/export-test.ttf")).unwrap();
    }

    #[test]
    fn checksum_rejects_same_size_corruption_and_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model");
        let asset = Asset {
            kind: ResourceKind::Ocr,
            name: "model".into(),
            url: String::new(),
            size: 3,
            sha256: format!("{:x}", Sha256::digest(b"abc")),
        };
        assert!(!verified(&path, &asset));
        fs::write(&path, b"ab").unwrap();
        assert!(!verified(&path, &asset));
        fs::write(&path, b"abd").unwrap();
        assert!(!verified(&path, &asset));
        fs::write(&path, b"abc").unwrap();
        assert!(verified(&path, &asset));
    }
}

/// 导出只投影实际校验结果，不序列化含绝对目录的 ResourceStatus。
pub(crate) fn diagnostic_summary(app: &AppHandle) -> serde_json::Value {
    match status(app) {
        Ok(value) => serde_json::json!({"provider":"local-ppocr", "model":"pp-ocrv6-small", "ready":value.ocr_ready, "runtimeReady":value.runtime_ready, "modelsReady":value.models_ready}),
        Err(_) => serde_json::json!({"provider":"local-ppocr", "state":"unavailable"}),
    }
}
