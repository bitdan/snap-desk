//! 翻译语言包独立于 OCR。在线安装只接受编译时清单；本地导入由系统选择器授权。
use crate::resources::{self, Asset, ResourceKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

const IDS: [&str; 2] = ["opus-mt-en-zh-int8", "opus-mt-zh-en-int8"];
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
pub fn model_id(target: &str) -> Result<&'static str, String> {
    match target {
        "zh-CN" => Ok(IDS[0]),
        "en" => Ok(IDS[1]),
        _ => Err("本地翻译仅支持英译中和中译英。".into()),
    }
}
fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(resources::directory(app)?.join("translation"))
}
fn catalog() -> Result<Vec<Pack>, String> {
    let packs: Vec<Pack> = serde_json::from_str(include_str!("../assets/translation-models.json"))
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
fn read_pack(dir: &Path) -> Result<Pack, String> {
    let mut bytes = Vec::new();
    fs::File::open(dir.join("pack.json"))
        .map_err(|_| "未安装语言包，请在识别与翻译设置中安装。")?
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
pub fn verified_directory(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if !IDS.contains(&id) {
        return Err("语言方向无效。".into());
    }
    let dir = root(app)?.join(id);
    let pack = read_pack(&dir)?;
    if pack.model_id != id {
        return Err("语言包方向不匹配。".into());
    }
    verify(&dir, &pack)?;
    Ok(dir)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    model_id: String,
    ready: bool,
    downloadable: bool,
    revision: Option<String>,
    download_bytes: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    runtime_ready: bool,
    models: Vec<ModelStatus>,
}
fn status(app: &AppHandle) -> Result<LocalStatus, String> {
    let catalog = catalog()?;
    let models = IDS
        .iter()
        .map(|id| {
            let installed = read_pack(&root(app)?.join(id)).ok();
            let available = catalog
                .iter()
                .find(|p| p.model_id == *id && p.validate().is_ok());
            Ok(ModelStatus {
                model_id: id.to_string(),
                ready: verified_directory(app, id).is_ok(),
                downloadable: available.is_some(),
                revision: installed.map(|p| p.source_revision),
                download_bytes: available
                    .map(|p| p.files.iter().map(|f| f.size).sum())
                    .unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(LocalStatus {
        runtime_ready: crate::local_translation::executable(app).is_ok(),
        models,
    })
}
#[tauri::command]
pub async fn translation_model_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<LocalStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || status(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// 先完整复制/下载并校验到 staging，再替换目录；失败时恢复旧包。
fn install(app: &AppHandle, pack: &Pack, source: Option<&Path>) -> Result<(), String> {
    pack.validate()?;
    if source.is_none() {
        validate_download_urls(pack)?;
    }
    let root = root(app)?;
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
            resources::download_file(app, &client, staging.path(), &file.asset())?
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

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelAction {
    Download,
    Import,
    Delete,
}
#[tauri::command]
pub async fn translation_model_action(
    app: AppHandle,
    window: WebviewWindow,
    model_id: String,
    action: ModelAction,
) -> Result<LocalStatus, String> {
    crate::capture::require_window(&window, "main")?;
    if !IDS.contains(&model_id.as_str()) {
        return Err("未知语言包。".into());
    }
    let state = app.state::<crate::local_translation::LocalTranslationState>();
    let mut slot = state
        .0
        .clone()
        .try_lock_owned()
        .map_err(|_| "翻译正在进行，请结束后再管理语言包。")?;
    // 卸载缓存模型后再替换磁盘资源，避免使用已删除的版本。
    if let Some(worker) = slot.take() {
        worker.stop().await;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // guard 随阻塞任务存活，即使调用方关闭窗口，安装中途也不能启动翻译。
        let _guard = slot;
        match action {
            ModelAction::Download => {
                let pack = catalog()?
                    .into_iter()
                    .find(|p| p.model_id == model_id)
                    .ok_or("此版本尚未发布该语言包下载，请导入可信的离线包。")?;
                install(&app, &pack, None)?;
            }
            ModelAction::Import => {
                if let Some(folder) = app
                    .dialog()
                    .file()
                    .set_title(crate::native_i18n::text(
                        &app,
                        "选择可信的离线语言包目录（含 pack.json）",
                    ))
                    .blocking_pick_folder()
                {
                    let source = folder.into_path().map_err(|e| e.to_string())?;
                    let pack = read_pack(&source)?;
                    if pack.model_id != model_id {
                        return Err("所选语言包方向不匹配。".into());
                    }
                    install(&app, &pack, Some(&source))?;
                }
            }
            ModelAction::Delete => {
                let path = root(&app)?.join(&model_id);
                if path.exists() {
                    fs::remove_dir_all(path).map_err(|e| e.to_string())?;
                }
            }
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

/// 不导出安装包自带的 revision 文本或任何路径，仅保留固定模型 ID 与校验状态。
pub(crate) fn diagnostic_summary(app: &AppHandle) -> serde_json::Value {
    match status(app) {
        Ok(value) => serde_json::json!({"provider":"local-ctranslate2", "runtimeReady":value.runtime_ready, "models":value.models.iter().map(|m| serde_json::json!({"model":m.model_id,"ready":m.ready})).collect::<Vec<_>>()}),
        Err(_) => serde_json::json!({"provider":"local-ctranslate2", "state":"unavailable"}),
    }
}
