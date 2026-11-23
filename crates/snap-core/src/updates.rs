//! 应用版本发现：在 GitHub 发布列表中选出比当前版本新、且带固定更新清单的应用发布。
//!
//! 同一仓库还发布翻译模型/引擎等资源，只匹配 SemVer 标签与固定清单地址；测试版是否纳入由调用方决定。
//! 下载与签名校验：Tauri 版用 updater 插件；GPUI 版用这里的 `download`（断点续传、镜像）与 `package`
//! （读取固定清单、校验地址与 minisign 签名）。安装（运行 NSIS 安装器或替换独立 EXE）由外壳完成。
pub mod download;
pub mod package;
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

/// 在当前线程上运行异步下载（调用方在后台线程）；reqwest 需要带 IO 与计时器的 tokio 运行时。
pub fn block_on<F: std::future::Future>(future: F) -> Result<F::Output, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("无法启动更新下载：{e}"))?;
    Ok(runtime.block_on(future))
}

pub const REPOSITORY: &str = "bitdan/snap-desk";

#[derive(Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}
#[derive(Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
    #[serde(default)]
    pub body: Option<String>,
}

/// 仅匹配应用 SemVer 标签和固定清单名称，跳过翻译模型/引擎等独立发布。返回版本与清单地址。
pub fn candidate(
    release: &Release,
    current: &Version,
    include_preview: bool,
) -> Option<(Version, String)> {
    if release.draft {
        return None;
    }
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    if version <= *current || (!include_preview && (release.prerelease || !version.pre.is_empty()))
    {
        return None;
    }
    let expected = format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/updater.json",
        release.tag_name
    );
    release
        .assets
        .iter()
        .find(|asset| asset.name == "updater.json" && asset.browser_download_url == expected)
        .map(|_| (version, expected))
}

/// 候选版本的发布页；没有候选版本时打开最新发布。版本号来自已校验的清单，只含 SemVer 字符。
pub fn release_page(version: Option<&str>) -> String {
    match version {
        Some(version) => format!("https://github.com/{REPOSITORY}/releases/tag/v{version}"),
        None => format!("https://github.com/{REPOSITORY}/releases/latest"),
    }
}

/// 在浏览器中打开发布页；版本号须为合法 SemVer（不接受任意地址）。
pub fn open_release_page(version: Option<&str>) -> Result<(), String> {
    if let Some(version) = version {
        Version::parse(version).map_err(|_| "版本号无效。".to_string())?;
    }
    crate::platform::open_trusted(&release_page(version))
}

/// 找到的新版本。
pub struct Found {
    pub version: Version,
    /// 固定的更新清单地址（安装流程使用）。
    pub manifest: String,
    pub notes: Option<String>,
}

/// 阻塞查询（在后台线程调用）：分页读取发布列表，取最新的候选。`proxy` 为设置中的更新代理。
pub fn latest(
    current: &Version,
    include_preview: bool,
    proxy: Option<&str>,
) -> Result<Option<Found>, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent("Snap-Desk-Updater")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30));
    if let Some(proxy) = proxy.filter(|p| !p.is_empty()) {
        builder =
            builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "代理地址无效。".to_string())?);
    }
    let client = builder
        .build()
        .map_err(|_| "无法初始化更新网络客户端。".to_string())?;
    let mut newest: Option<Found> = None;
    // 同仓库还发布模型资源，需要分页，不能依赖 /latest 或列表第一项。
    for page in 1..=20 {
        let response = client
            .get(format!(
                "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100&page={page}"
            ))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("检查更新失败：{e}"))?;
        if response
            .content_length()
            .is_some_and(|n| n > 8 * 1024 * 1024)
        {
            return Err("更新列表过大。".into());
        }
        let bytes = response.bytes().map_err(|e| format!("检查更新失败：{e}"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("更新列表过大。".into());
        }
        let releases: Vec<Release> =
            serde_json::from_slice(&bytes).map_err(|_| "更新列表格式无效。".to_string())?;
        for release in &releases {
            if let Some((version, manifest)) = candidate(release, current, include_preview) {
                if newest.as_ref().is_none_or(|old| version > old.version) {
                    newest = Some(Found {
                        version,
                        manifest,
                        notes: release.body.clone(),
                    });
                }
            }
        }
        if releases.len() < 100 {
            break;
        }
        if page == 20 {
            return Err("更新列表超出查询范围，请稍后重试。".into());
        }
    }
    Ok(newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![ReleaseAsset {
                name: "updater.json".into(),
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/{tag}/updater.json"
                ),
            }],
            body: None,
        }
    }

    #[test]
    fn release_page_points_at_the_candidate_or_latest_release() {
        assert_eq!(
            release_page(Some("0.2.6-beta.2")),
            format!("https://github.com/{REPOSITORY}/releases/tag/v0.2.6-beta.2")
        );
        assert_eq!(
            release_page(None),
            format!("https://github.com/{REPOSITORY}/releases/latest")
        );
    }

    #[test]
    fn channels_use_semver_and_ignore_resource_releases() {
        let current = Version::parse("0.2.0-beta.2").unwrap();
        assert!(candidate(&release("v0.2.0-beta.10", true), &current, false).is_none());
        assert!(candidate(&release("v0.2.0-beta.10", true), &current, true).is_some());
        assert!(candidate(&release("v0.2.0", false), &current, false).is_some());
        assert!(candidate(&release("v0.1.9", false), &current, false).is_none());
        assert!(candidate(&release("translation-engine-v3", true), &current, true).is_none());
        let mut forged = release("v0.2.1", false);
        forged.assets[0].browser_download_url = "https://example.com/updater.json".into();
        assert!(candidate(&forged, &current, true).is_none());
        forged = release("v0.2.1", false);
        forged.draft = true;
        assert!(candidate(&forged, &current, true).is_none());
    }

    #[test]
    fn release_bodies_are_optional_in_the_listing() {
        let json = r#"[{"tag_name":"v9.0.0","draft":false,"prerelease":false,"assets":[]}]"#;
        let releases: Vec<Release> = serde_json::from_str(json).unwrap();
        assert!(releases[0].body.is_none());
    }
}
