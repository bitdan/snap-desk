//! 用户下载资源（OCR 模型、运行库、字体）的清单、校验、下载与安装；进度显示由各外壳实现。
//! 资源存入可写的数据目录 `<data_dir>/resources`（与旧版相同），绝不写安装目录。
//! 临时文件经过大小、摘要校验后原子替换，失败不破坏已有文件。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// 导出字体文件名（资源目录下）。
pub const FONT_FILE: &str = "export-font.ttf";
const MAX_FONT: u64 = 64 * 1024 * 1024;

/// 资源类别；下载地址、文件名和摘要由内置清单决定。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    Ocr,
    Font,
}

/// 发布资源的固定身份；size 为字节数，sha256 用于发现截断或内容被替换。
#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub kind: ResourceKind,
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

pub fn manifest() -> Result<Vec<Asset>, String> {
    serde_json::from_str(include_str!("../../../resources/assets/downloads.json"))
        .map_err(|e| e.to_string())
}

/// 资源目录：数据目录下的 resources。
pub fn directory() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("resources"))
}

/// 大小与 SHA-256 都一致才算可用；模型和 DLL 用流式摘要，不额外分配整份资源的内存。
pub fn verified(path: &Path, asset: &Asset) -> bool {
    if fs::metadata(path).map(|m| m.len()).ok() != Some(asset.size) {
        return false;
    }
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

/// 读取并校验字体（有上限地读取）。
pub fn read_font(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "请先在通用设置下载或选择导出字体。")?;
    let mut bytes = Vec::new();
    file.take(MAX_FONT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    validate_font(&bytes)?;
    Ok(bytes)
}

/// 把用户选择的字体（已读取、已校验）原子写入资源目录；原文件移走后仍可导出。
pub fn install_font(dir: &Path, bytes: &[u8]) -> Result<(), String> {
    validate_font(bytes)?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(dir.join(FONT_FILE)).map_err(|e| e.to_string())?;
    Ok(())
}

/// 资源的实际可用状态（每次校验，不把“文件存在”当作“已就绪”）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub models_ready: bool,
    pub runtime_ready: bool,
    /// Windows 上 OCR 运行库压缩包的下载大小（MiB）；其他平台为 0。
    pub runtime_download_mib: u64,
    pub font_ready: bool,
}

impl Status {
    pub fn ocr_ready(&self) -> bool {
        self.models_ready && self.runtime_ready
    }
}

/// OCR 模型是否全部通过校验（流式摘要）。
pub fn models_ready(dir: &Path) -> Result<bool, String> {
    Ok(manifest()?
        .iter()
        .filter(|a| a.kind == ResourceKind::Ocr)
        .all(|a| verified(&dir.join(&a.name), a)))
}

/// 当前架构的 OCR 运行库是否就绪及其下载大小（只有 Windows 需要单独下载）。
pub fn runtime_status(dir: &Path) -> (bool, u64) {
    #[cfg(windows)]
    return (
        crate::recognition::ocr_runtime::ready(dir),
        crate::recognition::ocr_runtime::download_mib(),
    );
    #[cfg(not(windows))]
    {
        let _ = dir;
        (true, 0)
    }
}

pub fn font_ready(dir: &Path) -> bool {
    read_font(&dir.join(FONT_FILE)).is_ok()
}

/// 完整校验一次（不缓存；外壳可自行缓存）。
pub fn status(dir: &Path) -> Result<Status, String> {
    let (runtime_ready, runtime_download_mib) = runtime_status(dir);
    Ok(Status {
        models_ready: models_ready(dir)?,
        runtime_ready,
        runtime_download_mib,
        font_ready: font_ready(dir),
    })
}

/// 下载客户端：只走 HTTPS，连接 20 秒、单文件 5 分钟上限。
pub fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())
}

/// 下载到 `dir` 中的临时文件，大小和摘要通过后才返回；出错自动删除，调用者负责安装。
/// `progress(received, total)` 至多每 100 ms 调用一次（结束时必调用）。
pub fn download_file(
    client: &reqwest::blocking::Client,
    dir: &Path,
    asset: &Asset,
    progress: &mut dyn FnMut(u64, u64),
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
        // 网络分块可能很小，限频避免下载期间频繁刷新界面。
        if last_progress.elapsed() >= Duration::from_millis(100) || received == asset.size {
            progress(received, asset.size);
            last_progress = Instant::now();
        }
    }
    if received != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err(format!("{} 校验失败，请重试。", asset.name));
    }
    Ok(file)
}

/// 下载并安装某类资源（只用内置清单中的地址与文件名）；已通过校验的文件跳过。
/// OCR 在 Windows 上同时下载运行库。`progress(name, received, total)`。
pub fn download(
    dir: &Path,
    kind: ResourceKind,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let client = client()?;
    #[cfg(windows)]
    if kind == ResourceKind::Ocr {
        crate::recognition::ocr_runtime::download(dir, |asset| {
            download_file(&client, dir, asset, &mut |received, total| progress(&asset.name, received, total))
        })?;
    }
    for asset in manifest()?.iter().filter(|a| a.kind == kind) {
        let path = dir.join(&asset.name);
        if verified(&path, asset) {
            continue;
        }
        let file = download_file(&client, dir, asset, &mut |received, total| progress(&asset.name, received, total))?;
        if kind == ResourceKind::Font {
            read_font(file.path())?;
        }
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 本地 OCR 模型全部通过校验时返回资源目录。
pub fn ocr_models(dir: &Path) -> Result<PathBuf, String> {
    for asset in manifest()?.iter().filter(|a| a.kind == ResourceKind::Ocr) {
        if !verified(&dir.join(&asset.name), asset) {
            return Err("OCR 模型未下载或校验失败，请在“文字识别”页下载识别资源。".into());
        }
    }
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_checks_size_and_digest_and_manifest_lists_ocr_models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        fs::write(&path, b"abc").unwrap();
        let mut asset = Asset {
            kind: ResourceKind::Ocr,
            name: "a.bin".into(),
            url: String::new(),
            size: 3,
            sha256: format!("{:x}", Sha256::digest(b"abc")),
        };
        assert!(verified(&path, &asset));
        asset.sha256 = format!("{:x}", Sha256::digest(b"abd"));
        assert!(!verified(&path, &asset));
        asset.size = 4;
        assert!(!verified(&path, &asset));
        assert!(manifest().unwrap().iter().any(|a| a.kind == ResourceKind::Ocr));
        assert!(ocr_models(dir.path()).is_err());
    }

    #[test]
    fn font_validation_rejects_corrupt_and_variable_fonts() {
        assert!(validate_font(b"not a font").is_err());
        let mut header = vec![0u8; 28];
        header[..4].copy_from_slice(&[0, 1, 0, 0]);
        header[5] = 1;
        header[12..16].copy_from_slice(b"fvar");
        assert!(validate_font(&header).unwrap_err().contains("可变字体"));
        validate_font(include_bytes!("../tests/fixtures/export-test.ttf")).unwrap();
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

    #[test]
    fn installed_fonts_are_validated_and_reported_ready() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!font_ready(dir.path()));
        assert!(install_font(dir.path(), b"not a font").is_err());
        install_font(dir.path(), include_bytes!("../tests/fixtures/export-test.ttf")).unwrap();
        assert!(font_ready(dir.path()));
        assert!(!status(dir.path()).unwrap().models_ready);
    }
}
