//! 未发布视频的持久恢复记录。失败保留源文件，只有明确取消或成功发布才清理。
use super::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, sync::LazyLock};

static ACTIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// 目录来自后台持久记录；只允许访问该随机目录的固定文件，不接受前端路径。
fn validate_id(id: &str) -> Result<(), String> {
    if id.len() <= "recording-pending-".len()
        || !id.starts_with("recording-pending-")
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("录屏恢复标识无效。".into());
    }
    Ok(())
}

fn source_path(value: &Recoverable, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    let source = PathBuf::from(&value.path);
    crate::shared::export_staging::no_links(&source)?;
    if value.id != id
        || !source.is_absolute()
        || source
            .file_name()
            .is_none_or(|n| n != "video.mp4" && n != "video.gif")
        || source
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|n| n != id)
        || source
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("录屏恢复路径与标识不一致。".into());
    }
    // Windows junction 与 symlink 均属于 reparse point；绝不沿重定向清理。
    for path in [source.parent().ok_or("录屏目录无效")?, source.as_path()] {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("录屏恢复路径包含重解析点，拒绝操作。".into());
                    }
                }
                if metadata.file_type().is_symlink() {
                    return Err("录屏恢复路径包含符号链接，拒绝操作。".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(source)
}

/// 只删除已确认的单个源文件和空目录；额外文件存在时整体拒绝，绝不递归删除。
fn discard_source(value: &Recoverable, id: &str) -> Result<(), String> {
    let source = source_path(value, id)?;
    let directory = source.parent().ok_or("录屏目录无效")?;
    if directory.exists() {
        for entry in fs::read_dir(directory).map_err(err)? {
            if entry.map_err(err)?.path() != source {
                return Err("录屏目录包含其他文件，已保留全部内容，请手动检查。".into());
            }
        }
        match fs::remove_file(&source) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
        fs::remove_dir(directory).map_err(err)?;
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recoverable {
    pub id: String,
    pub path: String,
    pub finalized: bool,
    pub stage: String,
    pub exported: Option<String>,
}

/// 文件是否还在属于查询时状态，不写回持久记录，避免旧记录格式迁移。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    #[serde(flatten)]
    pub(crate) record: Recoverable,
    pub(crate) source_exists: bool,
}

fn catalog(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_local_data_dir()
        .map_err(err)?
        .join("recording-recovery");
    fs::create_dir_all(&path).map_err(err)?;
    crate::shared::export_staging::no_links(&path)?;
    Ok(path)
}
fn store(path: &Path, value: &Recoverable) -> Result<(), String> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("恢复记录路径无效")?).map_err(err)?;
    serde_json::to_writer(file.as_file_mut(), value).map_err(err)?;
    file.as_file().sync_all().map_err(err)?;
    file.persist(path).map_err(err)?;
    Ok(())
}

pub(super) struct Pending {
    directory: Option<tempfile::TempDir>,
    record: PathBuf,
    value: Recoverable,
    app: AppHandle,
    session: u64,
    published: bool,
}
impl Pending {
    pub fn new(
        app: &AppHandle,
        session: u64,
        root: &Path,
        format: OutputFormat,
    ) -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("recording-pending-")
            .tempdir_in(root)
            .map_err(err)?;
        let id = directory
            .path()
            .file_name()
            .ok_or("录屏目录无效")?
            .to_string_lossy()
            .into_owned();
        let value = Recoverable {
            id: id.clone(),
            path: directory
                .path()
                .join(format!("video.{}", format.extension()))
                .to_string_lossy()
                .into_owned(),
            finalized: false,
            stage: "recording".into(),
            exported: None,
        };
        let record = catalog(app)?.join(format!("{id}.json"));
        let mut active = ACTIVE.lock().map_err(err)?;
        store(&record, &value)?;
        active.insert(id);
        Ok(Self {
            directory: Some(directory),
            record,
            value,
            app: app.clone(),
            session,
            published: false,
        })
    }
    pub fn path(&self) -> PathBuf {
        PathBuf::from(&self.value.path)
    }
    pub fn stage(&mut self, stage: &str, finalized: bool) -> Result<(), String> {
        self.value.stage = stage.into();
        self.value.finalized = finalized;
        store(&self.record, &self.value)
    }
    pub fn published(&mut self, output: &Path) {
        self.published = true;
        self.value.exported = Some(output.to_string_lossy().into_owned());
    }
    pub fn output(&self, root: &Path) -> PathBuf {
        root.join(output::filename(
            self.value.id.trim_start_matches("recording-pending-"),
            self.path()
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("mp4"),
            false,
        ))
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        let cancelled = self
            .app
            .state::<RecordingState>()
            .0
            .lock()
            .ok()
            .is_some_and(|s| {
                s.as_ref()
                    .is_some_and(|s| s.view.id == self.session && s.cancel)
            });
        if let Some(directory) = self.directory.take() {
            settle(
                directory,
                &self.record,
                &mut self.value,
                self.published || cancelled,
            );
        }
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.value.id);
        }
    }
}

/// 恢复元数据更新失败也必须先保留目录；磁盘满时不能因为写不出 JSON 再删除视频。
fn settle(directory: tempfile::TempDir, record: &Path, value: &mut Recoverable, discard: bool) {
    if discard {
        // 显式观察清理结果；Windows 文件占用时保留目录对应的记录供用户重试。
        if directory.close().is_ok() {
            let _ = fs::remove_file(record);
        } else {
            value.stage = "cleanup-pending".into();
            let _ = store(record, value);
        }
        return;
    }
    let _path = directory.keep();
    value.stage = format!("failed:{}", value.stage);
    if let Err(_error) = store(record, value) {
        log::error!(
            "event=recording.recovery_store_failed module=recording code=RECOVERY_STORE_FAILED"
        );
    }
    log::warn!("event=recording.recovery_retained module=recording");
}

/// 发布到新目录时先复制、同步临时文件，再无覆盖发布；任何失败都保留源视频。
fn export(
    source: &Path,
    destination: &Path,
    registry: &Path,
    mut advanced: impl FnMut(&Path),
) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    let mut input = fs::File::open(source).map_err(err)?;
    if input.metadata().map_err(err)?.len() == 0 {
        return Err("保留的视频为空。".into());
    }
    let extension = match source.extension().and_then(|s| s.to_str()) {
        Some("mp4") => "mp4",
        Some("gif") => "gif",
        _ => return Err("不支持的录制文件格式。".into()),
    };
    let mut target = crate::shared::export_staging::TrackedFile::new_in(
        registry,
        destination,
        extension,
        "录屏恢复导出",
    )?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        target.file_mut().write_all(&buffer[..count]).map_err(err)?;
        advanced(target.path());
    }
    target.finalize()?;
    let name = target.id();
    let output = destination.join(output::filename(
        name.trim_start_matches('.'),
        extension,
        true,
    ));
    target.publish(&output, false)?;
    Ok(output)
}

#[tauri::command]
pub async fn recoverable_recordings(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<RecoveryEntry>, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || entries(&app).map(|(items, _)| items))
        .await
        .map_err(err)?
}

/// 录屏页查询已登记的录制文件；损坏记录单独计数，避免静默报告“没有残留”。
pub(crate) fn entries(app: &AppHandle) -> Result<(Vec<RecoveryEntry>, usize), String> {
    let active = ACTIVE.lock().map_err(err)?;
    let mut items = Vec::new();
    let mut unreadable = 0;
    for entry in fs::read_dir(catalog(app)?).map_err(err)? {
        let path = entry.map_err(err)?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(value) = crate::shared::export_staging::read::<Recoverable>(&path) else {
            unreadable += 1;
            continue;
        };
        if !active.contains(&value.id)
            && path
                .file_stem()
                .is_some_and(|name| name == value.id.as_str())
            && source_path(&value, &value.id).is_ok()
        {
            items.push(RecoveryEntry {
                source_exists: Path::new(&value.path).is_file(),
                record: value,
            });
        } else if !active.contains(&value.id) {
            unreadable += 1;
        }
    }
    Ok((items, unreadable))
}

#[tauri::command]
pub async fn retry_recording_export(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<String, String> {
    require_window(&window, "main")?;
    // 前端只传后台生成的标识，不能指定任意来源路径。
    validate_id(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = WorkerGuard::acquire(&app)?;
        let mut active = ACTIVE.lock().map_err(err)?;
        if !active.insert(id.clone()) {
            return Err("视频仍在处理，请稍后重试。".into());
        }
        drop(active);
        let result = (|| {
            let record = catalog(&app)?.join(format!("{id}.json"));
            let mut value: Recoverable = crate::shared::export_staging::read(&record)?;
            let source = source_path(&value, &id)?;
            if let Some(path) = value.exported.as_ref() {
                if fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0) {
                    result::remember(&app, Path::new(path));
                    return Ok(path.clone());
                }
                // 已导出的文件可能被用户移动/删除；若原始副本还在，仍可再次导出。
                value.exported = None;
            }
            if value.id != id || !value.finalized {
                return Err("该视频未确认完成封装，已保留源文件供检查，不能直接导出。".into());
            }
            progress(&app, "retry-export", Some(&source));
            let output = export(
                &source,
                &output::prepare(&app)?,
                &crate::shared::export_staging::catalog(&app)?,
                |path| progress(&app, "retry-export", Some(path)),
            )?;
            value.exported = Some(output.to_string_lossy().into_owned());
            value.stage = "exported".into();
            // 先记录成功再删除源文件。记录失败仍保留两份，避免丢失唯一副本。
            store(&record, &value)
                .map_err(|e| format!("视频已导出到 {}，恢复记录更新失败：{e}", output.display()))?;
            if discard_source(&value, &id).is_ok() {
                let _ = fs::remove_file(&record);
            }
            result::remember(&app, &output);
            Ok(output.to_string_lossy().into_owned())
        })();
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&id);
        }
        result
    })
    .await
    .map_err(err)?
}

/// 用户确认后清理一条残留；和恢复导出共用工作锁，不能删除仍在录制的文件。
#[tauri::command]
pub async fn discard_recording_recovery(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<(), String> {
    require_window(&window, "main")?;
    validate_id(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = WorkerGuard::acquire(&app)?;
        let mut active = ACTIVE.lock().map_err(err)?;
        if !active.insert(id.clone()) {
            return Err("视频仍在处理，请稍后重试。".into());
        }
        drop(active);
        let result = (|| {
            let record = catalog(&app)?.join(format!("{id}.json"));
            let value: Recoverable = crate::shared::export_staging::read(&record)?;
            discard_source(&value, &id)?;
            // 文件或空目录清理失败时保留记录，下次仍可发现和重试。
            fs::remove_file(record).map_err(err)
        })();
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&id);
        }
        result
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn failed_cancel_cleanup_retains_catalog_for_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = tempfile::Builder::new()
            .prefix("recording-pending-")
            .tempdir_in(root.path())
            .unwrap();
        let source = directory.path().join("video.mp4");
        fs::write(&source, b"raw").unwrap();
        let id = directory
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let record = root.path().join("record.json");
        let mut value = Recoverable {
            id: id.clone(),
            path: source.to_string_lossy().into_owned(),
            finalized: false,
            stage: "recording".into(),
            exported: None,
        };
        store(&record, &value).unwrap();
        let occupied = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&source)
            .unwrap();
        settle(directory, &record, &mut value, true);
        assert!(source.exists());
        let restored: Recoverable = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        assert_eq!(restored.stage, "cleanup-pending");
        drop(occupied);
        discard_source(&restored, &id).unwrap();
        assert!(!source.exists());
    }

    #[test]
    fn cleanup_removes_only_owned_source_and_keeps_exported_video() {
        let root = tempfile::tempdir().unwrap();
        let id = "recording-pending-test";
        let directory = root.path().join(id);
        fs::create_dir(&directory).unwrap();
        let source = directory.join("video.mp4");
        let output = root.path().join("export.mp4");
        fs::write(&source, b"raw").unwrap();
        fs::write(&output, b"final").unwrap();
        let value = Recoverable {
            id: id.into(),
            path: source.to_string_lossy().into_owned(),
            finalized: false,
            stage: "failed:recording".into(),
            exported: Some(output.to_string_lossy().into_owned()),
        };
        let unexpected = directory.join("keep.txt");
        fs::write(&unexpected, b"user").unwrap();
        assert!(discard_source(&value, id).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"raw");
        assert_eq!(fs::read(&unexpected).unwrap(), b"user");
        fs::remove_file(unexpected).unwrap();
        assert!(discard_source(&value, "../escape").is_err());
        assert!(discard_source(&value, "recording-pending-other").is_err());
        discard_source(&value, id).unwrap();
        assert!(!directory.exists());
        assert_eq!(fs::read(output).unwrap(), b"final");
        // 文件已经不在时仍允许清理元数据，不让列表出现永远删不掉的记录。
        discard_source(&value, id).unwrap();
    }

    #[test]
    fn cleanup_rejects_parent_components_even_with_matching_filename() {
        let root = tempfile::tempdir().unwrap();
        let id = "recording-pending-test";
        let value = Recoverable {
            id: id.into(),
            path: root
                .path()
                .join(id)
                .join("..")
                .join(id)
                .join("video.mp4")
                .to_string_lossy()
                .into_owned(),
            finalized: false,
            stage: "recording".into(),
            exported: None,
        };
        assert!(source_path(&value, id).is_err());
    }
    #[test]
    fn cancelled_output_is_removed_but_failed_output_survives_metadata_failure() {
        let root = tempfile::tempdir().unwrap();
        for discard in [false, true] {
            let directory = tempfile::tempdir_in(root.path()).unwrap();
            let source = directory.path().join("video.mp4");
            fs::write(&source, b"finalized-video").unwrap();
            let mut value = Recoverable {
                id: "test".into(),
                path: source.to_string_lossy().into_owned(),
                finalized: true,
                stage: "syncing".into(),
                exported: None,
            };
            // 模拟恢复元数据路径不可写；失败的视频仍必须保留。
            settle(
                directory,
                &root.path().join("missing/record.json"),
                &mut value,
                discard,
            );
            assert_eq!(source.exists(), !discard);
        }
    }
    #[test]
    fn finalized_recovery_record_survives_reload() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("record.json");
        let value = Recoverable {
            id: "recording-pending-test".into(),
            path: "video.mp4".into(),
            finalized: true,
            stage: "syncing".into(),
            exported: None,
        };
        store(&path, &value).unwrap();
        let restored: Recoverable = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(restored.finalized);
        assert_eq!(restored.id, value.id);
        assert!(restored.exported.is_none());
    }
    #[test]
    fn failed_export_preserves_source_and_retry_publishes_identical_bytes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("video.mp4");
        fs::write(&source, b"finalized-video").unwrap();
        assert!(export(
            &source,
            &root.path().join("missing"),
            &root.path().join("registry"),
            |_| {}
        )
        .is_err());
        assert_eq!(fs::read(&source).unwrap(), b"finalized-video");
        let path = export(&source, root.path(), &root.path().join("registry"), |_| {}).unwrap();
        assert_eq!(fs::read(path).unwrap(), fs::read(source).unwrap());
    }

    #[test]
    fn gif_retry_keeps_extension_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("video.gif");
        fs::write(&source, b"GIF89a").unwrap();
        let output = export(&source, root.path(), &root.path().join("registry"), |_| {}).unwrap();
        assert_eq!(output.extension().unwrap(), "gif");
        assert_eq!(fs::read(output).unwrap(), fs::read(source).unwrap());
    }
}
