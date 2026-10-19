//! 未发布视频的持久恢复记录。失败保留源文件，只有明确取消或成功发布才清理。
use super::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, sync::LazyLock};

static ACTIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recoverable {
    pub id: String,
    pub path: String,
    pub finalized: bool,
    pub stage: String,
    pub exported: Option<String>,
}

fn catalog(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_local_data_dir()
        .map_err(err)?
        .join("recording-recovery");
    fs::create_dir_all(&path).map_err(err)?;
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
    pub fn published(&mut self) {
        self.published = true;
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
        let _ = fs::remove_file(record);
        return;
    }
    let _path = directory.keep();
    value.stage = format!("failed:{}", value.stage);
    if let Err(_error) = store(record, value) {
        log::error!("event=recording.recovery_store_failed module=recording code=RECOVERY_STORE_FAILED");
    }
    log::warn!("event=recording.recovery_retained module=recording");
}

/// 发布到新目录时先复制、同步临时文件，再无覆盖发布；任何失败都保留源视频。
fn export(
    source: &Path,
    destination: &Path,
    mut advanced: impl FnMut(&Path),
) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    let mut input = fs::File::open(source).map_err(err)?;
    if input.metadata().map_err(err)?.len() == 0 {
        return Err("保留的视频为空。".into());
    }
    let mut target = tempfile::NamedTempFile::new_in(destination).map_err(err)?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        target
            .as_file_mut()
            .write_all(&buffer[..count])
            .map_err(err)?;
        advanced(target.path());
    }
    target.as_file().sync_all().map_err(err)?;
    let name = target
        .path()
        .file_name()
        .ok_or("导出路径无效")?
        .to_string_lossy();
    let extension = match source.extension().and_then(|s| s.to_str()) {
        Some("mp4") => "mp4",
        Some("gif") => "gif",
        _ => return Err("不支持的录制文件格式。".into()),
    };
    let output = destination.join(output::filename(
        name.trim_start_matches('.'),
        extension,
        true,
    ));
    target.persist_noclobber(&output).map_err(err)?;
    Ok(output)
}

#[tauri::command]
pub async fn recoverable_recordings(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<Recoverable>, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let active = ACTIVE.lock().map_err(err)?.clone();
        let mut items = Vec::new();
        for entry in fs::read_dir(catalog(&app)?).map_err(err)? {
            let path = entry.map_err(err)?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(value) = fs::read(&path)
                .map_err(err)
                .and_then(|b| serde_json::from_slice::<Recoverable>(&b).map_err(err))
            else {
                continue;
            };
            if !active.contains(&value.id)
                && value.exported.is_none()
                && Path::new(&value.path).is_file()
            {
                items.push(value);
            }
        }
        Ok(items)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn retry_recording_export(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<String, String> {
    require_window(&window, "main")?;
    // 前端只传后台生成的标识，不能指定任意来源路径。
    if !id.starts_with("recording-pending-")
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("录屏恢复标识无效。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = WorkerGuard::acquire(&app)?;
        let mut active = ACTIVE.lock().map_err(err)?;
        if !active.insert(id.clone()) {
            return Err("视频仍在处理，请稍后重试。".into());
        }
        drop(active);
        let result = (|| {
            let record = catalog(&app)?.join(format!("{id}.json"));
            let mut value: Recoverable =
                serde_json::from_slice(&fs::read(&record).map_err(err)?).map_err(err)?;
            if let Some(path) = value.exported {
                result::remember(&app, Path::new(&path));
                return Ok(path);
            }
            if value.id != id || !value.finalized {
                return Err("该视频未确认完成封装，已保留源文件供检查，不能直接导出。".into());
            }
            let source = PathBuf::from(&value.path);
            if source
                .file_name()
                .is_none_or(|n| n != "video.mp4" && n != "video.gif")
                || source
                    .parent()
                    .and_then(Path::file_name)
                    .is_none_or(|n| n != id.as_str())
            {
                return Err("录屏恢复路径与标识不一致。".into());
            }
            progress(&app, "retry-export", Some(&source));
            let output = export(&source, &output::prepare(&app)?, |path| {
                progress(&app, "retry-export", Some(path))
            })?;
            value.exported = Some(output.to_string_lossy().into_owned());
            value.stage = "exported".into();
            // 先记录成功再删除源文件。记录失败仍保留两份，避免丢失唯一副本。
            store(&record, &value)
                .map_err(|e| format!("视频已导出到 {}，恢复记录更新失败：{e}", output.display()))?;
            let _ = fs::remove_file(&source);
            if let Some(parent) = source.parent() {
                let _ = fs::remove_dir(parent);
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(export(&source, &root.path().join("missing"), |_| {}).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"finalized-video");
        let path = export(&source, root.path(), |_| {}).unwrap();
        assert_eq!(fs::read(path).unwrap(), fs::read(source).unwrap());
    }

    #[test]
    fn gif_retry_keeps_extension_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("video.gif");
        fs::write(&source, b"GIF89a").unwrap();
        let output = export(&source, root.path(), |_| {}).unwrap();
        assert_eq!(output.extension().unwrap(), "gif");
        assert_eq!(fs::read(output).unwrap(), fs::read(source).unwrap());
    }
}
