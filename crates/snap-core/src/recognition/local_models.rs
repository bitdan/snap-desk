//! 本地翻译语言包（独立于 OCR）。在线安装只接受编译时清单；本地导入的目录由外壳的系统选择器授权。
//! `resources` 为资源目录；语言包位于其中的 `translation/<模型 ID>`。
use crate::resources::{self, Asset, ResourceKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub use super::translation::LOCAL_MODEL_IDS as IDS;
const FILES: [&str; 8] = [
    "model.bin",
    "config.json",
    "shared_vocabulary.json",
    "source_vocabulary.json",
    "target_vocabulary.json",
    "source.spm",
    "target.spm",
    "LICENSE.txt",
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pack {
    pub model_id: String,
    pub source_revision: String,
    pub runtime: String,
    pub tokenizer_profile: String,
    pub target_language_token: String,
    pub license: String,
    pub files: Vec<PackFile>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    name: String,
    size: u64,
    sha256: String,
    #[serde(default)]
    url: String,
}

impl Pack {
    fn validate(&self) -> Result<(), String> {
        if !IDS.contains(&self.model_id.as_str())
            || self.runtime != "4.6.0"
            || self.tokenizer_profile != "transformers-marian-spm-v1"
            || self.source_revision.len() != 40
            || !self.source_revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("语言包身份、模型 revision 或运行时版本不受支持。".into());
        }
        let english = self.model_id == IDS[0];
        if self.target_language_token != if english { ">>cmn_Hans<<" } else { "" }
            || self.license != if english { "Apache-2.0" } else { "CC-BY-4.0" }
        {
            return Err("语言包目标标签或许可不匹配。".into());
        }
        let mut names = HashSet::new();
        let mut size = 0u64;
        for file in &self.files {
            if !FILES.contains(&file.name.as_str())
                || !names.insert(file.name.as_str())
                || file.size == 0
                || file.size > 1024 * 1024 * 1024
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("语言包文件清单无效。".into());
            }
            size += file.size;
        }
        if size > 1536 * 1024 * 1024
            || ![
                "model.bin",
                "config.json",
                "source.spm",
                "target.spm",
                "LICENSE.txt",
            ]
            .iter()
            .all(|name| names.contains(name))
            || !(names.contains("shared_vocabulary.json")
                || (names.contains("source_vocabulary.json")
                    && names.contains("target_vocabulary.json")))
        {
            return Err("语言包缺少模型、分词器、词表或许可文件。".into());
        }
        Ok(())
    }
}
impl PackFile {
    fn asset(&self) -> Asset {
        Asset {
            kind: ResourceKind::Ocr,
            name: self.name.clone(),
            url: self.url.clone(),
            size: self.size,
            sha256: self.sha256.to_ascii_lowercase(),
        }
    }
}
/// 目标语言到语言包的映射，设置校验与下载共用。
pub use super::translation::local_model_id as model_id;
/// 语言包根目录。
pub fn root(resources: &Path) -> PathBuf {
    resources.join("translation")
}
/// 编译时内置的可下载语言包清单。
pub fn catalog() -> Result<Vec<Pack>, String> {
    let packs: Vec<Pack> = serde_json::from_str(include_str!("../../../../src-tauri/assets/translation-models.json"))
        .map_err(|_| "内置语言包清单无效。".to_string())?;
    let mut ids = HashSet::new();
    for pack in &packs {
        pack.validate()?;
        validate_download_urls(pack)?;
        if !ids.insert(&pack.model_id) {
            return Err("内置语言包清单方向重复。".into());
        }
    }
    Ok(packs)
}

/// 下载入口只信任编译时固定的本仓库附件；导入包中的 URL 不用于网络请求。
fn validate_download_urls(pack: &Pack) -> Result<(), String> {
    let direction = if pack.model_id == IDS[0] {
        "en-zh"
    } else {
        "zh-en"
    };
    let revision = pack
        .source_revision
        .get(..12)
        .ok_or("模型 revision 无效。")?;
    let prefix = format!("translation-model-{direction}-{revision}-r");
    let mut tag: Option<String> = None;
    for file in &pack.files {
        let url = reqwest::Url::parse(&file.url).map_err(|_| "模型下载地址无效。")?;
        let parts: Vec<_> = url.path().split('/').collect();
        if !https_host(&url, "github.com")
            || url.query().is_some()
            || url.fragment().is_some()
            || parts.len() != 7
            || parts[1..5] != ["bitdan", "snap-desk", "releases", "download"]
            || parts[6] != file.name
        {
            return Err("模型下载仅允许内置仓库的固定 Release 附件。".into());
        }
        let suffix = parts[5].strip_prefix(&prefix).unwrap_or("");
        if suffix.is_empty()
            || suffix.starts_with('0')
            || !suffix.bytes().all(|b| b.is_ascii_digit())
            || tag.as_deref().map(|t| t != parts[5]).unwrap_or(false)
        {
            return Err("模型附件版本不一致或无效。".into());
        }
        tag = Some(parts[5].to_string());
    }
    Ok(())
}

/// 明确限制 host、协议、端口和凭据，不能用后缀匹配放行伪装域名。
fn https_host(url: &reqwest::Url, host: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
}

fn allowed_release_redirect(url: &reqwest::Url, previous_count: usize) -> bool {
    // 2026-10-16 用 cli/cli 的真实 Release 附件确认此 CDN；不放行任意 githubusercontent 子域。
    previous_count <= 3 && https_host(url, "release-assets.githubusercontent.com")
}
/// 读取并校验目录中的 pack.json（导入前检查方向）。
pub fn read_pack(dir: &Path) -> Result<Pack, String> {
    let mut bytes = Vec::new();
    fs::File::open(dir.join("pack.json"))
        .map_err(|_| "请先安装语言包。")?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("语言包清单过大。".into());
    }
    let pack: Pack = serde_json::from_slice(&bytes).map_err(|_| "语言包清单格式无效。")?;
    pack.validate()?;
    Ok(pack)
}
fn verify(dir: &Path, pack: &Pack) -> Result<(), String> {
    for file in &pack.files {
        let path = dir.join(&file.name);
        if fs::symlink_metadata(&path)
            .map(|m| !m.is_file() || m.file_type().is_symlink())
            .unwrap_or(true)
            || !resources::verified(&path, &file.asset())
        {
            return Err(format!(
                "语言包文件 {} 缺失或校验失败，请重新安装。",
                file.name
            ));
        }
    }
    Ok(())
}
pub fn verified_directory(resources: &Path, id: &str) -> Result<PathBuf, String> {
    if !IDS.contains(&id) {
        return Err("语言方向无效。".into());
    }
    let dir = root(resources).join(id);
    let pack = read_pack(&dir)?;
    if pack.model_id != id {
        return Err("语言包方向不匹配。".into());
    }
    verify(&dir, &pack)?;
    Ok(dir)
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub model_id: String,
    pub ready: bool,
    pub downloadable: bool,
    pub revision: Option<String>,
    pub download_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    pub runtime_ready: bool,
    pub runtime_supported: bool,
    pub models: Vec<ModelStatus>,
}

/// 单个语言包的状态（完整摘要校验；外壳可按目录缓存）。
pub fn model_status(resources: &Path, catalog: &[Pack], id: &str) -> ModelStatus {
    let installed = read_pack(&root(resources).join(id)).ok();
    let available = catalog
        .iter()
        .find(|p| p.model_id == id && p.validate().is_ok());
    ModelStatus {
        model_id: id.to_string(),
        ready: verified_directory(resources, id).is_ok(),
        downloadable: available.is_some(),
        revision: installed.map(|p| p.source_revision),
        download_bytes: available
            .map(|p| p.files.iter().map(|f| f.size).sum())
            .unwrap_or(0),
    }
}

/// 引擎与全部语言包的状态（不缓存）。
pub fn status(resources: &Path) -> Result<LocalStatus, String> {
    let catalog = catalog()?;
    Ok(LocalStatus {
        runtime_ready: super::local_runtime::supported()
            && super::local_runtime::executable(resources).is_ok(),
        runtime_supported: super::local_runtime::supported(),
        models: IDS.iter().map(|id| model_status(resources, &catalog, id)).collect(),
    })
}

/// 清单中可下载的语言包。
pub fn downloadable(id: &str) -> Result<Pack, String> {
    catalog()?
        .into_iter()
        .find(|p| p.model_id == id)
        .ok_or_else(|| "此版本尚未发布该语言包下载，请导入可信的离线包。".into())
}

/// 删除已安装的语言包（调用方先停止使用它的 worker）。
pub fn delete(resources: &Path, id: &str) -> Result<(), String> {
    if !IDS.contains(&id) {
        return Err("未知语言包。".into());
    }
    let path = root(resources).join(id);
    if path.exists() {
        fs::remove_dir_all(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 先完整复制/下载并校验到 staging，再替换目录；失败时恢复旧包。`source` 为导入目录（None 为下载）。
pub fn install(
    resources: &Path,
    pack: &Pack,
    source: Option<&Path>,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<(), String> {
    pack.validate()?;
    if source.is_none() {
        validate_download_urls(pack)?;
    }
    let root = root(resources);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let staging = tempfile::tempdir_in(&root).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if allowed_release_redirect(attempt.url(), attempt.previous().len()) {
                attempt.follow()
            } else {
                attempt.error("模型附件重定向目标或次数不受允许")
            }
        }))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|e| e.to_string())?;
    for file in &pack.files {
        let target = staging.path().join(&file.name);
        if let Some(source) = source {
            let path = source.join(&file.name);
            if !resources::verified(&path, &file.asset()) {
                return Err("待导入文件校验失败。".into());
            }
            fs::copy(path, target).map_err(|e| e.to_string())?;
        } else {
            resources::download_file(&client, staging.path(), &file.asset(), &mut |received, total| {
                progress(&file.name, received, total)
            })?
                .persist(target)
                .map_err(|e| e.to_string())?;
        }
    }
    fs::write(
        staging.path().join("pack.json"),
        serde_json::to_vec(pack).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    verify(staging.path(), pack)?;
    let target = root.join(&pack.model_id);
    let backup = tempfile::tempdir_in(&root).map_err(|e| e.to_string())?;
    let previous = backup.path().join("previous");
    if target.exists() {
        fs::rename(&target, &previous).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(staging.path(), &target) {
        if previous.exists() {
            if fs::rename(&previous, &target).is_err() {
                let _ = backup.keep();
                return Err("安装失败，旧包已保留在 translation 临时目录中。".into());
            }
        }
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_redirects_reject_untrusted_hosts_and_excess_hops() {
        let valid =
            reqwest::Url::parse("https://release-assets.githubusercontent.com/file").unwrap();
        assert!(allowed_release_redirect(&valid, 3));
        assert!(!allowed_release_redirect(&valid, 4));
        for value in [
            "http://release-assets.githubusercontent.com/file",
            "https://release-assets.githubusercontent.com.evil.test/file",
            "https://127.0.0.1/file",
            "https://release-assets.githubusercontent.com:444/file",
            "https://user:secret@release-assets.githubusercontent.com/file",
            "https://github.com/file",
        ] {
            assert!(!allowed_release_redirect(
                &reqwest::Url::parse(value).unwrap(),
                1
            ));
        }
    }
    #[test]
    fn only_supported_directions_resolve() {
        assert_eq!(model_id("en").unwrap(), IDS[1]);
        assert!(model_id("ja").is_err());
        assert!(model_id("../en").is_err());
    }
    #[test]
    fn manifest_rejects_paths_duplicates_and_missing_tokenizer() {
        let mut pack = Pack {
            model_id: IDS[0].into(),
            source_revision: "a".repeat(40),
            runtime: "4.6.0".into(),
            tokenizer_profile: "transformers-marian-spm-v1".into(),
            target_language_token: ">>cmn_Hans<<".into(),
            license: "Apache-2.0".into(),
            files: [
                "model.bin",
                "config.json",
                "source.spm",
                "target.spm",
                "shared_vocabulary.json",
                "LICENSE.txt",
            ]
            .iter()
            .map(|name| PackFile {
                name: name.to_string(),
                size: 3,
                sha256: "a".repeat(64),
                url: String::new(),
            })
            .collect(),
        };
        assert!(pack.validate().is_ok());
        for file in &mut pack.files {
            file.url = format!(
                "https://github.com/bitdan/snap-desk/releases/download/translation-model-en-zh-aaaaaaaaaaaa-r1/{}",
                file.name
            );
        }
        assert!(validate_download_urls(&pack).is_ok());
        let valid_url = pack.files[0].url.clone();
        for value in [
            valid_url.replace("-r1/", "-r2/"),
            valid_url.replace("github.com/", "github.com.evil.test/"),
            valid_url.replace("bitdan/", "other/"),
            valid_url.replace("aaaaaaaaaaaa", "bbbbbbbbbbbb"),
            format!("{valid_url}?redirect=https://evil.test"),
        ] {
            pack.files[0].url = value;
            assert!(validate_download_urls(&pack).is_err());
        }
        pack.files[0].url = valid_url;
        pack.files[0].name = "../model.bin".into();
        assert!(pack.validate().is_err());
        pack.files[0].name = "model.bin".into();
        pack.files.push(pack.files[0].clone());
        assert!(pack.validate().is_err());
        pack.files.pop();
        pack.files.retain(|f| f.name != "target.spm");
        assert!(pack.validate().is_err());
    }
}
