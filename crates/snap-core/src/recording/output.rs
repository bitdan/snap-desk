//! 录屏临时输出与保存目录记录；失败和取消依靠 RAII 清理，只有成功成品才发布。
//! 临时目录与最终文件同盘，发布不复制整段视频。目录记录文件的位置由外壳决定。
use super::config::OutputFormat;
use crate::settings::RecordingExportSettings;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

// None 尚未读取；Some(None) 使用默认目录；Some(Some(path)) 为本次运行的选择。
static SESSION_DIRECTORY: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

/// 本地日期时间便于排序；随机标识区分同毫秒导出，尺寸取实际编码区域物理像素。
pub fn filename_with_settings(
    token: &str,
    extension: &str,
    settings: &RecordingExportSettings,
    dimensions: Option<(u32, u32)>,
) -> String {
    let now = chrono::Local::now();
    let stamp = now.format("%Y%m%d-%H%M%S-%3f");
    let dimensions = if settings.include_dimensions {
        dimensions
            .map(|(width, height)| format!("-{width}x{height}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "{}-Recording-{stamp}-{token}{dimensions}.{extension}",
        settings.filename_prefix
    )
}

/// 调用方先创建此目录，再创建媒体句柄；逆序释放句柄后才清理，不接管旧文件。
pub struct Pending {
    directory: tempfile::TempDir,
    path: PathBuf,
    format: OutputFormat,
}

impl Pending {
    pub fn new(root: &Path, format: OutputFormat) -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("recording-")
            .tempdir_in(root)
            .map_err(|e| e.to_string())?;
        let path = directory
            .path()
            .join(format!("video.{}", format.extension()));
        Ok(Self {
            directory,
            path,
            format,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 消费临时输出；失败自动清理，成功后目录中已无成品，不会误删发布文件。
    pub fn publish(
        self,
        settings: &RecordingExportSettings,
        dimensions: (u32, u32),
    ) -> Result<PathBuf, String> {
        let root = self.directory.path().parent().ok_or("录屏目录无效")?;
        let token = self
            .directory
            .path()
            .file_name()
            .ok_or("录屏目录无效")?
            .to_string_lossy();
        let output = root.join(filename_with_settings(
            token.trim_start_matches("recording-"),
            self.format.extension(),
            settings,
            Some(dimensions),
        ));
        publish(&self.path, &output)?;
        Ok(output)
    }
}

/// 无覆盖发布；失败时 TempPath 与外层临时目录共同保证清理，已有成品不受影响。
fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    let temporary = tempfile::TempPath::try_from_path(source).map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(destination)
        .map_err(|e| format!("发布录屏失败：{e}"))
}

pub fn read(record: &Path) -> Result<Option<PathBuf>, String> {
    match fs::read(record) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("录屏保存目录配置损坏，请重新选择目录或恢复默认：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("无法读取录屏保存目录：{e}")),
    }
}

/// 与图片目录记录相同：写入并同步临时文件后替换，失败保持原配置。
pub fn persist(record: &Path, directory: Option<&Path>) -> Result<(), String> {
    let parent = record.parent().ok_or("录屏目录配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(file.as_file_mut(), &directory).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(record)
        .map_err(|e| format!("保存录屏目录失败，原配置保持不变：{e}"))?;
    Ok(())
}

/// 解析本次输出目录：已选择的目录，或外壳给出的默认目录（返回 true）。
pub fn resolve(record: &Path, remember: bool, default: PathBuf) -> Result<(PathBuf, bool), String> {
    match configured_directory(record, remember)? {
        Some(path) => Ok((path, false)),
        None => Ok((default, true)),
    }
}

/// 备份在设置锁内使用已读取的记忆开关，避免再次锁定设置状态。
pub fn configured_directory(record: &Path, remember: bool) -> Result<Option<PathBuf>, String> {
    let mut session = SESSION_DIRECTORY.lock().map_err(|e| e.to_string())?;
    selected_directory(record, remember, &mut session)
}

/// 选择器和配置导入共用目录提交；调用方保证与配置导入互斥。
pub fn configure_directory(
    record: &Path,
    directory: Option<&Path>,
    remember: bool,
) -> Result<(), String> {
    if let Some(directory) = directory {
        validate_directory(directory)?;
    }
    if remember || directory.is_none() {
        persist(record, directory)?;
    }
    restore_session_directory(directory.map(Path::to_owned))
}

/// 配置导入回滚只恢复运行时选择，磁盘原始记录由备份事务恢复。
pub fn restore_session_directory(directory: Option<PathBuf>) -> Result<(), String> {
    *SESSION_DIRECTORY.lock().map_err(|e| e.to_string())? = Some(directory);
    Ok(())
}

/// 将磁盘记忆和本次运行的目录分开，关闭记忆不会中断当前输出位置。
fn selected_directory(
    record: &Path,
    remember: bool,
    session: &mut Option<Option<PathBuf>>,
) -> Result<Option<PathBuf>, String> {
    if session.is_none() {
        *session = Some(if remember { read(record)? } else { None });
    }
    let selected = session.as_ref().cloned().flatten();
    // 重新启用记忆后，把本次运行的选择同步到磁盘；关闭时不保存新的目录。
    if remember && read(record)? != selected {
        persist(record, selected.as_deref())?;
    }
    Ok(selected)
}

pub fn validate_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!(
            "录屏保存目录不可用：{}。请重新选择目录或恢复默认。",
            path.display()
        ));
    }
    // 实际试建文件检查写权限；系统 ACL、只读磁盘无法仅靠 metadata 判断。
    let probe = tempfile::NamedTempFile::new_in(path)
        .map_err(|e| format!("录屏保存目录无法写入：{}：{e}", path.display()))?;
    probe
        .close()
        .map_err(|e| format!("录屏保存目录无法清理临时文件：{e}"))
}

/// 准备时固定本次目录；自定义目录被删除/移动或外置磁盘掉线时，不偷偷改存其他位置或重建目录。
pub fn prepare(record: &Path, remember: bool, default: PathBuf) -> Result<PathBuf, String> {
    let (path, is_default) = resolve(record, remember, default)?;
    if is_default {
        fs::create_dir_all(&path).map_err(|e| format!("无法创建默认录屏目录：{e}"))?;
    }
    validate_directory(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_failure_cleans_source_without_overwriting_existing_video() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("pending.mp4");
        let destination = root.path().join("existing.mp4");
        fs::write(&source, b"new video").unwrap();
        fs::write(&destination, b"old video").unwrap();
        assert!(publish(&source, &destination).is_err());
        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"old video");
    }

    #[test]
    fn committed_output_survives_temporary_directory_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let pending = Pending::new(root.path(), OutputFormat::Mp4).unwrap();
        let temporary_directory = pending.directory.path().to_owned();
        fs::write(pending.path(), b"new video").unwrap();
        let target = pending
            .publish(&RecordingExportSettings::default(), (64, 64))
            .unwrap();
        assert_eq!(target.parent(), Some(root.path()));
        assert!(!temporary_directory.exists());
        assert_eq!(fs::read(target).unwrap(), b"new video");
    }

    #[test]
    fn dropping_uncommitted_output_removes_only_its_own_files() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("finished.mp4");
        fs::write(&target, b"existing video").unwrap();
        let pending = Pending::new(root.path(), OutputFormat::Gif).unwrap();
        let temporary_directory = pending.directory.path().to_owned();
        fs::write(pending.path(), b"unfinished video").unwrap();
        drop(pending);
        assert!(!temporary_directory.exists());
        assert_eq!(fs::read(target).unwrap(), b"existing video");
    }

    #[test]
    fn directory_survives_reload_and_reset_does_not_delete_videos() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("config/recording-directory.json");
        assert_eq!(read(&record).unwrap(), None);
        let custom = root.path().join("自定义 videos");
        fs::create_dir(&custom).unwrap();
        fs::write(custom.join("existing.mp4"), b"existing video").unwrap();
        validate_directory(&custom).unwrap();
        persist(&record, Some(&custom)).unwrap();
        assert_eq!(read(&record).unwrap(), Some(custom.clone()));
        persist(&record, None).unwrap();
        assert_eq!(read(&record).unwrap(), None);
        assert_eq!(
            fs::read(custom.join("existing.mp4")).unwrap(),
            b"existing video"
        );
    }

    #[test]
    fn missing_directory_and_corrupt_record_fail_without_fallback() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing");
        assert!(validate_directory(&path).is_err());
        assert!(!path.exists());
        assert!(validate_directory(Path::new("relative")).is_err());
        let record = root.path().join("record.json");
        fs::write(&record, "invalid json").unwrap();
        assert!(read(&record).is_err());
        assert_eq!(fs::read_to_string(&record).unwrap(), "invalid json");
    }
    #[test]
    fn filename_uses_recording_prefix_and_physical_dimensions() {
        let settings = RecordingExportSettings {
            filename_prefix: "Demo".into(),
            include_dimensions: true,
            remember_directory: false,
        };
        assert!(settings.validate().is_ok());
        let name = filename_with_settings("abc123", "mp4", &settings, Some((1920, 1080)));
        assert!(name.starts_with("Demo-Recording-"));
        assert!(name.ends_with("-abc123-1920x1080.mp4"));
        let name = filename_with_settings(
            "abc123",
            "gif",
            &RecordingExportSettings::default(),
            Some((1920, 1080)),
        );
        assert!(name.ends_with("-abc123.gif"));
        let unsafe_settings = RecordingExportSettings {
            filename_prefix: "../escape".into(),
            ..settings
        };
        assert!(unsafe_settings.validate().is_err());
    }

    #[test]
    fn disabled_directory_memory_keeps_session_and_resets_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("directory.json");
        let old = root.path().join("old");
        persist(&record, Some(&old)).unwrap();
        let current = root.path().join("current");
        let mut session = Some(Some(current.clone()));
        assert_eq!(
            selected_directory(&record, false, &mut session).unwrap(),
            Some(current.clone())
        );
        assert_eq!(read(&record).unwrap(), Some(old));
        assert_eq!(selected_directory(&record, false, &mut None).unwrap(), None);
        assert_eq!(
            selected_directory(&record, true, &mut session).unwrap(),
            Some(current.clone())
        );
        assert_eq!(read(&record).unwrap(), Some(current));
    }
}
