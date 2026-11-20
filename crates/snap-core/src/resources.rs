//! 用户下载资源（OCR 模型、运行库、字体）的清单与校验；下载与进度事件由各外壳实现。
//! 资源存入可写的数据目录 `<data_dir>/resources`（与 Tauri 版 `app_data_dir/resources` 相同）。
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    serde_json::from_str(include_str!("../../../src-tauri/assets/downloads.json"))
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
}
