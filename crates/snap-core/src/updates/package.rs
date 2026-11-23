//! 更新包：读取发布自带的固定清单（静态 JSON），校验其中的版本与安装包地址，
//! 下载后用 minisign 公钥校验签名（且要求签名中记录的版本
//! 与清单版本相同）。清单与镜像都不受信任：地址只能指向同一 tag 下、当前分发形式的附件。
use super::{download::Network, Found, REPOSITORY};
use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use reqwest::Url;
use semver::Version;
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

/// 清单大小上限（固定清单只有几 KiB）。
const MAX_MANIFEST: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    platforms: HashMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    signature: String,
    url: String,
}

/// 已通过地址校验的更新包；`signature` 在下载后交给 [`verify`]。
#[derive(Clone, Debug)]
pub struct Package {
    pub version: Version,
    pub url: Url,
    pub signature: String,
    pub notes: Option<String>,
}

/// 解析清单并取出 `target`（如 `windows-x86_64-nsis`）的安装包；版本、地址前缀与后缀必须与发现的版本一致。
pub fn parse(
    bytes: &[u8],
    version: &Version,
    target: &str,
    suffix: &str,
) -> Result<Package, String> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|_| "更新清单格式无效。".to_string())?;
    let announced = Version::parse(manifest.version.trim_start_matches('v'))
        .map_err(|_| "更新清单的版本或安装包类型不匹配。".to_string())?;
    let entry = manifest
        .platforms
        .get(target)
        .ok_or("更新清单未提供当前平台的安装包。")?;
    let url =
        Url::parse(&entry.url).map_err(|_| "更新清单的版本或安装包类型不匹配。".to_string())?;
    let prefix = format!("https://github.com/{REPOSITORY}/releases/download/v{version}/");
    if announced != *version
        || !url.as_str().starts_with(&prefix)
        || !url.path().ends_with(suffix)
        || url.query().is_some()
        || url.fragment().is_some()
        || entry.signature.trim().is_empty()
    {
        return Err("更新清单的版本或安装包类型不匹配。".into());
    }
    Ok(Package {
        version: announced,
        url,
        signature: entry.signature.trim().to_owned(),
        notes: manifest.notes,
    })
}

/// 阻塞读取（后台线程）：清单先直连 GitHub，失败时再经镜像；内容由 [`parse`] 与签名把关。
pub fn fetch(
    network: &Network,
    found: &Found,
    target: &str,
    suffix: &str,
) -> Result<Package, String> {
    let endpoint = Url::parse(&found.manifest).map_err(|_| "更新清单地址无效。".to_string())?;
    let routes: Vec<Url> = std::iter::once(endpoint.clone())
        .chain(network.mirrored(&endpoint))
        .collect();
    let client = network.client(Some(Duration::from_secs(30)))?;
    let mut last = String::from("读取更新清单失败。");
    for route in routes {
        let result = super::block_on(async {
            let response = client.get(route.clone()).send().await?.error_for_status()?;
            response.bytes().await
        })?;
        match result {
            Ok(bytes) if bytes.len() <= MAX_MANIFEST => {
                let mut package = parse(&bytes, &found.version, target, suffix)?;
                // 清单没写说明时沿用发布页正文。
                if package.notes.as_deref().is_none_or(|n| n.trim().is_empty()) {
                    package.notes = found.notes.clone();
                }
                return Ok(package);
            }
            Ok(_) => last = "更新清单过大。".into(),
            Err(error) => last = format!("读取更新清单失败：{error}"),
        }
    }
    Err(last)
}

fn decode(value: &str, what: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|_| format!("{what}格式无效。"))?;
    String::from_utf8(bytes).map_err(|_| format!("{what}格式无效。"))
}

/// 校验安装包签名，并要求签名可信注释里的 `version:` 与清单版本一致（防止把新版本号配上旧安装包）。
pub fn verify(
    bytes: &[u8],
    signature: &str,
    public_key: &str,
    version: &Version,
) -> Result<(), String> {
    let key = PublicKey::decode(&decode(public_key, "更新签名公钥")?)
        .map_err(|_| "更新签名公钥格式无效。".to_string())?;
    let signature = Signature::decode(&decode(signature, "更新签名")?)
        .map_err(|_| "更新签名格式无效。".to_string())?;
    key.verify(bytes, &signature, true)
        .map_err(|_| "安装包签名校验失败，已丢弃。请重试或在浏览器中下载。".to_string())?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or("安装包签名未记录版本，已丢弃。")?;
    match Version::parse(signed.trim_start_matches('v')) {
        Ok(signed) if signed == *version => Ok(()),
        _ => Err("安装包签名的版本与更新清单不一致，已丢弃。".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, url: &str) -> Vec<u8> {
        serde_json::json!({
            "version": version,
            "notes": "修复若干问题",
            "platforms": { "windows-x86_64-nsis": { "signature": "c2ln", "url": url } }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn manifest_must_point_at_the_same_tag_and_package_kind() {
        let version = Version::parse("1.2.0").unwrap();
        let good = format!("https://github.com/{REPOSITORY}/releases/download/v1.2.0/Snap.Desk_1.2.0_x64-setup.exe");
        let package = parse(
            &manifest("1.2.0", &good),
            &version,
            "windows-x86_64-nsis",
            "-setup.exe",
        )
        .unwrap();
        assert_eq!(package.url.as_str(), good);
        assert_eq!(package.signature, "c2ln");
        assert_eq!(package.notes.as_deref(), Some("修复若干问题"));

        // 另一个 tag、另一种分发形式、带查询参数或别的仓库都拒绝。
        let other_tag = format!("https://github.com/{REPOSITORY}/releases/download/v1.1.0/Snap.Desk_1.1.0_x64-setup.exe");
        assert!(parse(
            &manifest("1.2.0", &other_tag),
            &version,
            "windows-x86_64-nsis",
            "-setup.exe"
        )
        .is_err());
        assert!(parse(
            &manifest("1.2.0", &good),
            &version,
            "windows-x86_64-nsis",
            "-standalone.exe"
        )
        .is_err());
        assert!(parse(
            &manifest("1.2.0", &format!("{good}?x=1")),
            &version,
            "windows-x86_64-nsis",
            "-setup.exe"
        )
        .is_err());
        assert!(parse(
            &manifest(
                "1.2.0",
                "https://github.com/evil/repo/releases/download/v1.2.0/a-setup.exe"
            ),
            &version,
            "windows-x86_64-nsis",
            "-setup.exe"
        )
        .is_err());
        // 清单版本与发现的版本不一致、或缺少当前平台。
        assert!(parse(
            &manifest("1.3.0", &good),
            &version,
            "windows-x86_64-nsis",
            "-setup.exe"
        )
        .is_err());
        assert!(parse(
            &manifest("1.2.0", &good),
            &version,
            "darwin-aarch64-app",
            "-setup.exe"
        )
        .is_err());
    }

    #[test]
    fn signatures_are_rejected_without_a_valid_key() {
        let version = Version::parse("1.2.0").unwrap();
        assert!(verify(b"data", "not base64", "also not", &version).is_err());
        let key = base64::engine::general_purpose::STANDARD.encode("untrusted comment: x\nRWQ=");
        assert!(verify(b"data", &key, &key, &version).is_err());
    }
}
