//! 翻译引擎按需安装。下载与导入共用仓库锁定的 ZIP 摘要，不执行外部选择的任意程序。
use std::path::PathBuf;
use tauri::AppHandle;

static STATUS: crate::resources::status_cache::StatusCache<bool> =
    crate::resources::status_cache::StatusCache::new();

fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(crate::resources::directory(app)?.join("translator-runtime"))
}

pub fn supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

/// 仅设置页复用引擎目录的近期校验；启动执行仍直接调用 executable 完整验证。
pub(super) fn status_ready(app: &AppHandle) -> Result<bool, String> {
    if !supported() {
        return Ok(false);
    }
    STATUS.read(&[root(app)?], || Ok(executable(app).is_ok()))
}

pub fn executable(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(windows)]
    if supported() {
        return windows::executable(app);
    }
    let _ = app;
    Err("翻译引擎下载和导入目前仅支持 Windows x64。".into())
}

pub fn install(app: &AppHandle, import: bool) -> Result<(), String> {
    #[cfg(windows)]
    if supported() {
        return windows::install(app, import);
    }
    let _ = (app, import);
    Err("翻译引擎下载和导入目前仅支持 Windows x64。".into())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::resources::{self, Asset, ResourceKind};
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::{collections::HashSet, fs, io::Read, path::Path, time::Duration};
    use tauri_plugin_dialog::DialogExt;

    const MAX_BYTES: u64 = 512 * 1024 * 1024;
    const WORKER: &str = "snap-translator-worker.exe";
    const ARCHIVE: &str = ".archive.zip";

    #[derive(Deserialize)]
    struct Lock {
        tag: String,
        sha256: String,
        runtime: String,
        protocol: u32,
        target: String,
        asset: String,
    }

    /// 与发布流水线共用信任根，避免应用和 CI 各自维护不同的引擎版本。
    fn lock() -> Result<Lock, String> {
        let value: Lock =
            serde_json::from_str(include_str!("../../../scripts/ci/translation-engine.json"))
                .map_err(|e| e.to_string())?;
        if value.runtime != "4.6.0"
            || value.protocol != 1
            || value.target != "windows-x64"
            || value.asset != "translator-runtime-windows-x64.zip"
            || !value
                .tag
                .strip_prefix("translation-engine-v4.6.0-r")
                .is_some_and(|revision| {
                    !revision.is_empty()
                        && !revision.starts_with('0')
                        && revision.bytes().all(|b| b.is_ascii_digit())
                })
            || value.sha256.len() != 64
            || !value.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("内置翻译引擎清单无效。".into());
        }
        Ok(value)
    }

    /// ZIP 必须先匹配内置摘要；之后才允许读取里面的路径与文件内容。
    fn archive(path: &Path) -> Result<zip::ZipArchive<fs::File>, String> {
        let length = fs::metadata(path).map_err(|e| e.to_string())?.len();
        if length == 0 || length > MAX_BYTES {
            return Err("翻译引擎压缩包大小无效。".into());
        }
        let asset = Asset {
            kind: ResourceKind::Ocr,
            name: String::new(),
            url: String::new(),
            size: length,
            sha256: lock()?.sha256,
        };
        if !resources::verified(path, &asset) {
            return Err("引擎包校验失败，请选择匹配版本的官方 ZIP。".into());
        }
        zip::ZipArchive::new(fs::File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }

    /// 保留已验证的原始 ZIP，启动前逐项对比安装文件，防止仅检查 EXE 存在就误报就绪。
    fn verify(dir: &Path) -> Result<(), String> {
        let mut zip = archive(&dir.join(ARCHIVE))?;
        let mut worker = false;
        let mut expected = HashSet::from([PathBuf::from(ARCHIVE)]);
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
            if entry.is_dir() {
                continue;
            }
            let relative = entry.enclosed_name().ok_or("引擎归档路径无效。")?;
            expected.insert(relative.clone());
            let path = dir.join(relative);
            if fs::symlink_metadata(&path)
                .map(|m| !m.is_file() || m.file_type().is_symlink())
                .unwrap_or(true)
            {
                return Err("翻译引擎文件缺失，请重新下载或导入。".into());
            }
            let mut hash = Sha256::new();
            std::io::copy(&mut entry, &mut hash).map_err(|e| e.to_string())?;
            let asset = Asset {
                kind: ResourceKind::Ocr,
                name: String::new(),
                url: String::new(),
                size: entry.size(),
                sha256: format!("{:x}", hash.finalize()),
            };
            if !resources::verified(&path, &asset) {
                return Err("翻译引擎文件校验失败，请重新下载或导入。".into());
            }
            worker |= entry.name() == WORKER;
        }
        if !worker {
            return Err("引擎包缺少翻译程序。".into());
        }
        // DLL 搜索使用引擎目录；拒绝额外文件及目录链接，避免加载清单之外的 DLL。
        let mut pending = vec![dir.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                if kind.is_symlink() {
                    return Err("翻译引擎目录不能包含链接。".into());
                }
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if !expected
                    .remove(entry.path().strip_prefix(dir).map_err(|e| e.to_string())?)
                {
                    return Err("翻译引擎含清单外文件，请重新安装。".into());
                }
            }
        }
        if !expected.is_empty() {
            return Err("翻译引擎文件缺失。".into());
        }
        Ok(())
    }

    pub fn executable(app: &AppHandle) -> Result<PathBuf, String> {
        let dir = root(app)?;
        verify(&dir).map_err(|_| "翻译引擎不可用，请重新下载或导入。")?;
        Ok(dir.join(WORKER))
    }

    pub fn install(app: &AppHandle, import: bool) -> Result<(), String> {
        let parent = resources::directory(app)?;
        fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        let staging = tempfile::tempdir_in(&parent).map_err(|e| e.to_string())?;
        let archive_path = staging.path().join(ARCHIVE);
        if import {
            let Some(file) = app
                .dialog()
                .file()
                .set_title(crate::shared::native_i18n::text(app, "选择翻译引擎 ZIP"))
                .add_filter("ZIP", &["zip"])
                .blocking_pick_file()
            else {
                return Ok(());
            };
            let source = file.into_path().map_err(|e| e.to_string())?;
            if fs::metadata(&source).map_err(|e| e.to_string())?.len() > MAX_BYTES {
                return Err("翻译引擎压缩包过大。".into());
            }
            fs::copy(source, &archive_path).map_err(|e| e.to_string())?;
        } else {
            let lock = lock()?;
            let client = reqwest::blocking::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let url = attempt.url();
                    if attempt.previous().len() <= 3
                        && url.scheme() == "https"
                        && url.host_str() == Some("release-assets.githubusercontent.com")
                        && url.port_or_known_default() == Some(443)
                        && url.username().is_empty()
                        && url.password().is_none()
                    {
                        attempt.follow()
                    } else {
                        attempt.error("引擎附件重定向不受允许")
                    }
                }))
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(900))
                .build()
                .map_err(|e| e.to_string())?;
            let response = client
                .get(format!(
                    "https://github.com/bitdan/snap-desk/releases/download/{}/{}",
                    lock.tag, lock.asset
                ))
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("翻译引擎下载失败，可重试：{e}"))?;
            let mut file = fs::File::create(&archive_path).map_err(|e| e.to_string())?;
            if std::io::copy(&mut response.take(MAX_BYTES + 1), &mut file)
                .map_err(|e| e.to_string())?
                > MAX_BYTES
            {
                return Err("翻译引擎压缩包过大。".into());
            }
        }
        let mut zip = archive(&archive_path)?;
        let mut seen = HashSet::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let name = entry.name().replace('\\', "/");
            let name = name.trim_end_matches('/');
            if name.is_empty()
                || name.contains(':')
                || name
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || !seen.insert(name.to_lowercase())
                || name.eq_ignore_ascii_case(ARCHIVE)
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("引擎归档含非法或重复路径。".into());
            }
            total = total
                .checked_add(entry.size())
                .ok_or("引擎解压大小超限。")?;
            if total > MAX_BYTES {
                return Err("引擎解压大小超限。".into());
            }
            let target = staging
                .path()
                .join(entry.enclosed_name().ok_or("引擎归档路径无效。")?);
            if entry.is_dir() {
                fs::create_dir_all(target).map_err(|e| e.to_string())?;
                continue;
            }
            fs::create_dir_all(target.parent().ok_or("引擎路径无效。")?)
                .map_err(|e| e.to_string())?;
            let size = entry.size();
            let mut file = fs::File::create(target).map_err(|e| e.to_string())?;
            if std::io::copy(&mut entry.take(size + 1), &mut file).map_err(|e| e.to_string())?
                != size
            {
                return Err("引擎归档文件大小不符。".into());
            }
        }
        drop(zip);
        verify(staging.path())?;
        // 同卷暂存并整体替换；失败时恢复原目录，绝不把新旧 DLL 混装。
        let target = root(app)?;
        let backup = tempfile::tempdir_in(&parent).map_err(|e| e.to_string())?;
        let previous = backup.path().join("previous");
        if target.exists() {
            fs::rename(&target, &previous).map_err(|e| e.to_string())?;
        }
        if let Err(error) = fs::rename(staging.path(), &target) {
            if previous.exists() && fs::rename(&previous, &target).is_err() {
                let _ = backup.keep();
                return Err("引擎安装失败，旧引擎已保留在资源临时目录中。".into());
            }
            return Err(error.to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn runtime_lock_matches_application_contract() {
            let value = lock().unwrap();
            assert_eq!(value.target, "windows-x64");
            assert_eq!(value.runtime, "4.6.0");
        }

        #[test]
        fn rejects_untrusted_zip_before_reading_members() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join(ARCHIVE);
            fs::write(&file, b"untrusted archive").unwrap();
            assert!(archive(&file).is_err());
            assert!(verify(dir.path()).is_err());
            fs::write(&file, b"").unwrap();
            assert!(archive(&file).is_err());
        }
    }
}
