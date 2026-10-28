//! 启动恢复中心：仅检查本应用登记的导出文件和录屏记录，不遍历用户目录。
//! 运行标记只用于说明上次是否正常退出，绝不反序列化截图会话或自动重启采集设备。
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

static ACTIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn milliseconds(time: SystemTime) -> Option<u64> {
    Some(time.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunMarker {
    started_at: u64,
    pid: u32,
}
pub struct StartupState {
    previous: Option<RunMarker>,
    warning: Option<String>,
    marker: PathBuf,
    current_started_at: u64,
}

fn local(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_local_data_dir().map_err(error)
}
fn store(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("恢复记录目录无效。")?;
    fs::create_dir_all(parent).map_err(error)?;
    no_links(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    serde_json::to_writer(file.as_file_mut(), value).map_err(error)?;
    file.as_file().sync_all().map_err(error)?;
    file.persist(path).map_err(error)?;
    Ok(())
}
/// 检查全部祖先；不能只检查末级目录，否则被替换的 junction 会让清理进入其他位置。
pub(crate) fn no_links(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("恢复路径必须为不含上级跳转的绝对路径。".into());
    }
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err("恢复路径含重解析点，已保留文件。".into());
                    }
                }
                if meta.file_type().is_symlink() {
                    return Err("恢复路径含符号链接，已保留文件。".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e)),
        }
    }
    Ok(())
}
/// 各恢复目录共用有界读取；损坏、链接或超大 JSON 不能被当成可信来源路径。
pub(crate) fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    no_links(path)?;
    if fs::metadata(path).map_err(error)?.len() > 64 * 1024 {
        return Err("恢复记录过大。".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)
}
fn previous_marker(path: &Path) -> Result<Option<RunMarker>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => read(path).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(error(e)),
    }
}
fn clear_current_marker(path: &Path, started_at: u64) -> Result<bool, String> {
    let Some(marker) = previous_marker(path)? else {
        return Ok(false);
    };
    if marker.pid != std::process::id() || marker.started_at != started_at {
        return Ok(false);
    }
    fs::remove_file(path).map_err(error)?;
    Ok(true)
}

/// setup 在创建主窗口前调用；失败只报告警告，不覆盖已有标记，也不删除任何恢复数据。
pub fn initialize(app: &AppHandle) {
    let result = (|| -> Result<StartupState, String> {
        let marker = local(app)?.join("run-state.json");
        let (previous, mut warning) = match previous_marker(&marker) {
            Ok(previous) => (previous, None),
            Err(_) => (
                None,
                Some("上次运行标记无法读取，无法确认是否正常退出。".into()),
            ),
        };
        if let Some(previous) = &previous {
            log::warn!(
                "event=app.previous_exit_unclean module=app previous_started_ms={} previous_pid={}",
                previous.started_at,
                previous.pid
            );
        }
        let current_started_at = now();
        if store(
            &marker,
            &RunMarker {
                started_at: current_started_at,
                pid: std::process::id(),
            },
        )
        .is_err()
        {
            warning = Some("无法更新运行标记，本次异常退出可能无法识别。".into());
        }
        Ok(StartupState {
            previous,
            warning,
            marker,
            current_started_at,
        })
    })();
    let state = result.unwrap_or_else(|_| StartupState {
        previous: None,
        warning: Some("恢复中心初始化失败，请检查应用数据目录。".into()),
        marker: PathBuf::new(),
        current_started_at: 0,
    });
    app.manage(state);
}
/// 真正 Exit 且其他资源释放完成后清除标记；关闭主窗口至托盘不算退出。
pub fn shutdown(app: &AppHandle) {
    // 普通退出若仍打断导出，也保留未完成标记；不能把正在写入的文件当成已安全收尾。
    if !ACTIVE.try_lock().is_ok_and(|active| active.is_empty()) {
        log::warn!("event=app.exit_pending_exports module=app");
        return;
    }
    if let Some(state) = app.try_state::<StartupState>() {
        if !state.marker.as_os_str().is_empty() {
            // 仅清除本次成功写入的标记；初始化写入失败时保留原始异常证据。
            if clear_current_marker(&state.marker, state.current_started_at).is_err() {
                log::warn!("event=app.session_marker_failed module=app");
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact {
    id: String,
    path: PathBuf,
    title: String,
    extension: Option<String>,
    created_at: u64,
    finalized: bool,
    published: bool,
    members: Vec<String>,
}
pub(crate) fn catalog(app: &AppHandle) -> Result<PathBuf, String> {
    let root = local(app)?.join("export-recovery");
    fs::create_dir_all(&root).map_err(error)?;
    no_links(&root)?;
    Ok(root)
}
fn valid_id(id: &str) -> bool {
    (id.starts_with("snapdesk-pending-") || id.starts_with("snapdesk-work-"))
        && id.len() > 18
        && id.len() <= 96
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn record_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("恢复标识无效。".into());
    }
    Ok(catalog(app)?.join(format!("{id}.json")))
}
fn validated(value: &Artifact, id: &str) -> Result<(), String> {
    if !valid_id(id)
        || value.id != id
        || value.path.file_name().is_none_or(|n| n != id)
        || value.members.len() > 1000
        || value.title.len() > 128
    {
        return Err("恢复记录与文件标识不一致。".into());
    }
    if let Some(ext) = &value.extension {
        if !["png", "jpg", "webp", "pdf", "mp4", "gif"].contains(&ext.as_str())
            || !id.starts_with("snapdesk-pending-")
        {
            return Err("恢复文件格式无效。".into());
        }
    } else if !id.starts_with("snapdesk-work-") {
        return Err("临时目录标识无效。".into());
    }
    if value.members.iter().any(|n| {
        n.is_empty()
            || n.len() > 64
            || !Path::new(n)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
            || n.contains('/')
            || n.contains('\\')
    }) {
        return Err("临时目录成员无效。".into());
    }
    no_links(&value.path)
}
fn members(value: &Artifact) -> Result<Vec<PathBuf>, String> {
    if value.extension.is_some() {
        return Ok(vec![value.path.clone()]);
    }
    let mut paths = vec![];
    for item in fs::read_dir(&value.path).map_err(error)? {
        let item = item.map_err(error)?;
        if !value.members.iter().any(|n| item.file_name() == n.as_str())
            || !item.file_type().map_err(error)?.is_file()
        {
            return Err("临时目录出现未登记文件或子目录，已保留全部内容。".into());
        }
        no_links(&item.path())?;
        paths.push(item.path());
    }
    Ok(paths)
}
/// 不递归删除；检查所有成员后才删除登记文件，遇到异常保留记录供重试。
fn discard(value: &Artifact, record: &Path) -> Result<(), String> {
    validated(value, &value.id)?;
    if value.published {
        return fs::remove_file(record).map_err(error);
    }
    match fs::symlink_metadata(&value.path) {
        Ok(_) => {
            let files = members(value)?;
            for path in files {
                fs::remove_file(path).map_err(error)?;
            }
            if value.extension.is_none() {
                fs::remove_dir(&value.path).map_err(error)?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(error(e)),
    }
    fs::remove_file(record).map_err(error)
}

/// 同目录导出暂存文件。编码前持久登记，sync 后才能标记为可恢复；发布失败保留已编码内容。
pub struct TrackedFile {
    file: Option<fs::File>,
    value: Artifact,
    record: PathBuf,
}
impl TrackedFile {
    pub fn new(
        app: &AppHandle,
        parent: &Path,
        extension: &str,
        title: &str,
    ) -> Result<Self, String> {
        Self::new_in(&catalog(app)?, parent, extension, title)
    }
    pub(crate) fn new_in(
        catalog: &Path,
        parent: &Path,
        extension: &str,
        title: &str,
    ) -> Result<Self, String> {
        no_links(parent)?;
        let file = tempfile::Builder::new()
            .prefix("snapdesk-pending-")
            .tempfile_in(parent)
            .map_err(error)?;
        let value = artifact(file.path(), Some(extension.into()), title, vec![])?;
        let record = catalog.join(format!("{}.json", value.id));
        let mut active = ACTIVE.lock().map_err(error)?;
        store(&record, &value)?;
        let (file, _) = file.keep().map_err(error)?;
        active.insert(value.id.clone());
        Ok(Self {
            file: Some(file),
            value,
            record,
        })
    }
    pub fn file_mut(&mut self) -> &mut fs::File {
        self.file
            .as_mut()
            .expect("tracked file is open before publish")
    }
    pub fn id(&self) -> &str {
        &self.value.id
    }
    pub fn path(&self) -> &Path {
        &self.value.path
    }
    /// 系统编码器可能独占打开输出路径，调用前释放本应用的读写句柄。
    /// 恢复登记与 ACTIVE 占用持续保留；成功后重开句柄供 finalize 同步，失败交给 Drop 清理。
    pub(crate) fn write_external(
        &mut self,
        write: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        self.file.take();
        no_links(self.path())?;
        write(self.path())?;
        no_links(self.path())?;
        self.file = Some(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.path())
                .map_err(error)?,
        );
        Ok(())
    }
    pub fn finalize(&mut self) -> Result<(), String> {
        self.file_mut().sync_all().map_err(error)?;
        self.value.finalized = true;
        store(&self.record, &self.value)
    }
    pub fn publish(mut self, target: &Path, overwrite: bool) -> Result<(), String> {
        if !self.value.finalized {
            return Err("导出尚未完成编码。".into());
        }
        self.file.take();
        no_links(target)?;
        let mut temporary = tempfile::TempPath::try_from_path(&self.value.path).map_err(error)?;
        temporary.disable_cleanup(true);
        if overwrite {
            temporary.persist(target)
        } else {
            temporary.persist_noclobber(target)
        }
        .map_err(error)?;
        // 源路径已被移动；删除登记失败时最多留下“源文件缺失”，不会清理最终输出。
        let _ = fs::remove_file(&self.record);
        Ok(())
    }
}
impl Drop for TrackedFile {
    fn drop(&mut self) {
        self.file.take();
        if !self.value.finalized {
            let _ = discard(&self.value, &self.record);
        }
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.value.id);
        }
    }
}
fn artifact(
    path: &Path,
    extension: Option<String>,
    title: &str,
    members: Vec<String>,
) -> Result<Artifact, String> {
    let id = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("暂存标识无效。")?
        .to_owned();
    let value = Artifact {
        id: id.clone(),
        path: path.to_owned(),
        title: title.into(),
        extension,
        created_at: now(),
        finalized: false,
        published: false,
        members,
    };
    validated(&value, &id)?;
    Ok(value)
}
/// 剪贴板快照/独立文件批次的平面目录；只允许调用方预先声明的文件名。
pub struct Workspace {
    value: Artifact,
    record: PathBuf,
}
impl Workspace {
    pub fn temporary(app: &AppHandle, title: &str, names: Vec<String>) -> Result<Self, String> {
        let root = local(app)?.join("export-work");
        fs::create_dir_all(&root).map_err(error)?;
        Self::new(app, &root, title, names)
    }
    pub fn new(
        app: &AppHandle,
        parent: &Path,
        title: &str,
        names: Vec<String>,
    ) -> Result<Self, String> {
        no_links(parent)?;
        let directory = tempfile::Builder::new()
            .prefix("snapdesk-work-")
            .tempdir_in(parent)
            .map_err(error)?;
        let value = artifact(directory.path(), None, title, names)?;
        let record = record_path(app, &value.id)?;
        let mut active = ACTIVE.lock().map_err(error)?;
        store(&record, &value)?;
        // 登记成功后保留目录，交由恢复记录管理；路径已在 value 中，无需再次保存。
        let _ = directory.keep();
        active.insert(value.id.clone());
        Ok(Self { value, record })
    }
    pub fn path(&self) -> &Path {
        &self.value.path
    }
    pub fn publish(mut self) -> Result<(), String> {
        // 先标记已交付。即使随后崩溃或记录删除失败，也不得将完整批次列为可清理临时目录。
        self.value.published = true;
        store(&self.record, &self.value)?;
        let _ = fs::remove_file(&self.record);
        Ok(())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        if !self.value.published {
            let _ = discard(&self.value, &self.record);
        }
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.value.id);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryItem {
    id: String,
    kind: &'static str,
    title: String,
    path: String,
    bytes: Option<u64>,
    modified_at: Option<u64>,
    ready: bool,
    source_exists: bool,
    stage: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    previous_started_at: Option<u64>,
    previous_pid: Option<u32>,
    warning: Option<String>,
    items: Vec<RecoveryItem>,
    unreadable_records: usize,
}
fn stats(value: &Artifact) -> Result<(u64, Option<u64>), String> {
    let mut bytes = 0u64;
    for file in members(value)? {
        let metadata = fs::metadata(file).map_err(error)?;
        if !metadata.is_file() {
            return Err("暂存源不是普通文件。".into());
        }
        bytes = bytes.saturating_add(metadata.len());
    }
    Ok((
        bytes,
        fs::metadata(&value.path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(milliseconds),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[cfg(windows)]
    #[test]
    fn external_writer_can_open_exclusively_then_finalize_and_publish() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("registry");
        let mut output = TrackedFile::new_in(&registry, root.path(), "mp4", "测试裁剪").unwrap();
        let record = output.record.clone();
        let id = output.id().to_owned();
        output.write_external(|path| {
            assert!(record.exists());
            assert!(ACTIVE.lock().unwrap().contains(&id));
            assert!(!read::<Artifact>(&record).unwrap().finalized);
            let mut exclusive = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .share_mode(0)
                .open(path)
                .map_err(error)?;
            exclusive.write_all(b"encoded clip").map_err(error)
        }).unwrap();
        output.finalize().unwrap();
        let target = root.path().join("clip.mp4");
        output.publish(&target, false).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"encoded clip");
        assert!(!record.exists());
        assert!(!ACTIVE.lock().unwrap().contains(&id));
    }

    #[test]
    fn external_writer_failure_cleans_unfinished_output_without_finalizing() {
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("registry");
        let mut output = TrackedFile::new_in(&registry, root.path(), "mp4", "测试裁剪").unwrap();
        let path = output.path().to_owned();
        let record = output.record.clone();
        let result = output.write_external(|path| {
            fs::write(path, b"partial").map_err(error)?;
            Err("系统裁剪失败".into())
        });
        assert_eq!(result.unwrap_err(), "系统裁剪失败");
        assert!(!read::<Artifact>(&record).unwrap().finalized);
        drop(output);
        assert!(!path.exists());
        assert!(!record.exists());
    }

    #[test]
    fn previous_run_is_read_without_restoring_any_session_and_only_own_marker_clears() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("run-state.json");
        assert!(previous_marker(&marker).unwrap().is_none());
        store(
            &marker,
            &RunMarker {
                started_at: 123,
                pid: std::process::id(),
            },
        )
        .unwrap();
        assert_eq!(previous_marker(&marker).unwrap().unwrap().started_at, 123);
        assert!(!clear_current_marker(&marker, 456).unwrap());
        assert!(marker.exists());
        assert!(clear_current_marker(&marker, 123).unwrap());
        assert!(!marker.exists());
        fs::write(&marker, b"damaged").unwrap();
        assert!(previous_marker(&marker).is_err());
        assert!(clear_current_marker(&marker, 123).is_err());
        assert_eq!(fs::read(marker).unwrap(), b"damaged");
    }

    #[test]
    fn incomplete_normal_failure_cleans_but_encoded_failure_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("registry");
        let mut incomplete =
            TrackedFile::new_in(&registry, root.path(), "png", "测试图片").unwrap();
        incomplete.file_mut().write_all(b"partial").unwrap();
        let partial_path = incomplete.path().to_owned();
        let partial_record = incomplete.record.clone();
        drop(incomplete);
        assert!(!partial_path.exists());
        assert!(!partial_record.exists());

        let mut completed = TrackedFile::new_in(&registry, root.path(), "png", "测试图片").unwrap();
        completed.file_mut().write_all(b"encoded-image").unwrap();
        completed.finalize().unwrap();
        let record = completed.record.clone();
        let path = completed.path().to_owned();
        drop(completed);
        let restored: Artifact = read(&record).unwrap();
        assert!(restored.finalized);
        assert_eq!(fs::read(&path).unwrap(), b"encoded-image");
        assert_eq!(stats(&restored).unwrap().0, 13);
        discard(&restored, &record).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn failed_publish_preserves_unique_source_and_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("existing.png");
        fs::write(&destination, b"old-output").unwrap();
        let mut output = TrackedFile::new_in(
            &root.path().join("registry"),
            root.path(),
            "png",
            "测试导出",
        )
        .unwrap();
        output.file_mut().write_all(b"new-image").unwrap();
        output.finalize().unwrap();
        let record = output.record.clone();
        let path = output.path().to_owned();
        assert!(output.publish(&destination, false).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"old-output");
        assert_eq!(fs::read(&path).unwrap(), b"new-image");
        let restored: Artifact = read(&record).unwrap();
        discard(&restored, &record).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"old-output");
    }

    #[test]
    fn successful_publish_does_not_leave_final_output_in_cleanup_catalog() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("saved.png");
        let mut output = TrackedFile::new_in(
            &root.path().join("registry"),
            root.path(),
            "png",
            "测试导出",
        )
        .unwrap();
        output.file_mut().write_all(b"image").unwrap();
        output.finalize().unwrap();
        let record = output.record.clone();
        output.publish(&destination, false).unwrap();
        assert!(!record.exists());
        assert_eq!(fs::read(destination).unwrap(), b"image");
    }

    #[test]
    fn directory_cleanup_refuses_unknown_files_before_removing_anything() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("snapdesk-work-fixture123");
        fs::create_dir(&directory).unwrap();
        let value = artifact(&directory, None, "快照", vec!["0.png".into()]).unwrap();
        let record = root.path().join("record.json");
        store(&record, &value).unwrap();
        fs::write(directory.join("0.png"), b"owned").unwrap();
        fs::write(directory.join("notes.txt"), b"keep").unwrap();
        assert!(discard(&value, &record).is_err());
        assert!(directory.join("0.png").exists());
        assert!(record.exists());
        fs::remove_file(directory.join("notes.txt")).unwrap();
        discard(&value, &record).unwrap();
        assert!(!directory.exists());
    }

    #[test]
    fn published_batch_and_missing_source_never_delete_final_files() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("snapdesk-work-fixture123");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("001.txt"), b"delivered").unwrap();
        let mut value = artifact(&directory, None, "已导出批次", vec!["001.txt".into()]).unwrap();
        value.published = true;
        let record = root.path().join("record.json");
        store(&record, &value).unwrap();
        discard(&value, &record).unwrap();
        assert_eq!(fs::read(directory.join("001.txt")).unwrap(), b"delivered");
        value.published = false;
        value.path = root.path().join("absent").join(&value.id);
        store(&record, &value).unwrap();
        discard(&value, &record).unwrap();
        assert!(!record.exists());
        assert!(directory.join("001.txt").exists());
    }

    #[test]
    fn traversal_and_mismatched_ids_are_rejected_without_touching_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("snapdesk-pending-fixture123");
        fs::write(&path, b"keep").unwrap();
        let mut value = artifact(&path, Some("png".into()), "图片", vec![]).unwrap();
        assert!(validated(&value, "../../outside").is_err());
        value.path = root.path().join("other").join("..").join(&value.id);
        assert!(discard(&value, &root.path().join("record.json")).is_err());
        assert_eq!(fs::read(path).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn ancestor_symlink_cannot_redirect_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let name = "snapdesk-pending-fixture123";
        fs::write(outside.path().join(name), b"keep").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("redirect")).unwrap();
        let value = Artifact {
            id: name.into(),
            path: root.path().join("redirect").join(name),
            title: "图片".into(),
            extension: Some("png".into()),
            created_at: 1,
            finalized: true,
            published: false,
            members: vec![],
        };
        assert!(discard(&value, &root.path().join("record.json")).is_err());
        assert_eq!(fs::read(outside.path().join(name)).unwrap(), b"keep");
    }
}

#[tauri::command]
pub async fn recovery_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<RecoveryStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let startup = app.state::<StartupState>();
        let mut status = RecoveryStatus {
            previous_started_at: startup.previous.as_ref().map(|m| m.started_at),
            previous_pid: startup.previous.as_ref().map(|m| m.pid),
            warning: startup.warning.clone(),
            items: vec![],
            unreadable_records: 0,
        };
        {
            // 与新登记/恢复/删除互斥，不能把当前仍在写的导出当成上次残留。
            let active = ACTIVE.lock().map_err(error)?;
            for entry in fs::read_dir(catalog(&app)?).map_err(error)? {
                let path = entry.map_err(error)?.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if active.contains(id) {
                    continue;
                }
                let value = match read::<Artifact>(&path).and_then(|value| {
                    validated(&value, id)?;
                    Ok(value)
                }) {
                    Ok(value) => value,
                    Err(_) => {
                        status.unreadable_records += 1;
                        continue;
                    }
                };
                if value.published {
                    continue;
                }
                let meta = fs::symlink_metadata(&value.path);
                let exists = match &meta {
                    Ok(_) => true,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                    Err(_) => {
                        status.unreadable_records += 1;
                        continue;
                    }
                };
                let mut unsafe_contents = false;
                let measured = if exists {
                    match stats(&value) {
                        Ok(stats) => Some(stats),
                        Err(_) => {
                            status.unreadable_records += 1;
                            unsafe_contents = true;
                            None
                        }
                    }
                } else {
                    None
                };
                status.items.push(RecoveryItem {
                    id: value.id,
                    kind: if value.extension.is_some() {
                        "export"
                    } else {
                        "temporary"
                    },
                    title: value.title,
                    path: value.path.to_string_lossy().into_owned(),
                    bytes: measured.map(|m| m.0),
                    modified_at: measured.and_then(|m| m.1).or(Some(value.created_at)),
                    ready: value.finalized
                        && exists
                        && value.extension.is_some()
                        && !unsafe_contents,
                    source_exists: exists,
                    stage: if unsafe_contents {
                        "unsafe"
                    } else if value.finalized {
                        "encoded"
                    } else {
                        "incomplete"
                    }
                    .into(),
                });
            }
        }
        let (recordings, unreadable) = crate::capture::recording::recovery::entries(&app)?;
        status.unreadable_records += unreadable;
        for entry in recordings {
            let value = entry.record;
            let metadata = fs::metadata(&value.path).ok();
            status.items.push(RecoveryItem {
                id: value.id,
                kind: "recording",
                title: "录屏恢复文件".into(),
                path: value.path,
                bytes: metadata.as_ref().map(|m| m.len()),
                modified_at: metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(milliseconds),
                ready: value.finalized && entry.source_exists,
                source_exists: entry.source_exists,
                stage: value.stage,
            });
        }
        status
            .items
            .sort_by_key(|item| std::cmp::Reverse(item.modified_at));
        Ok(status)
    })
    .await
    .map_err(error)?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactAction {
    Recover,
    OpenDirectory,
    Discard,
}
#[tauri::command]
pub async fn recovery_artifact_action(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    action: ArtifactAction,
) -> Result<Option<String>, String> {
    crate::capture::require_window(&window, "main")?;
    let record = record_path(&app, &id)?;
    tauri::async_runtime::spawn_blocking(move || {
        // 清理/恢复与扫描、新导出保持串行；不等待采集状态锁，不触碰正在写入的条目。
        let mut active = ACTIVE.lock().map_err(error)?;
        if active.contains(&id) {
            return Err("导出仍在使用该文件，请稍后重试。".into());
        }
        let value: Artifact = read(&record)?;
        validated(&value, &id)?;
        match action {
            ArtifactAction::Discard => {
                discard(&value, &record)?;
                Ok(None)
            }
            ArtifactAction::OpenDirectory => {
                let directory = if value.extension.is_none() {
                    value.path.as_path()
                } else {
                    value.path.parent().ok_or("文件目录无效。")?
                };
                crate::shared::platform::open_directory(directory)?;
                Ok(None)
            }
            ArtifactAction::Recover => {
                if value.published || !value.finalized {
                    return Err("文件尚未完成编码，只能检查或清理。".into());
                }
                let ext = value
                    .extension
                    .as_deref()
                    .ok_or("临时目录请打开后检查所需文件。")?;
                // 对话框期间不持全局锁，但将源标为使用中，拒绝其他恢复或清理。
                active.insert(id.clone());
                drop(active);
                let result = (|| {
                    let Some(path) = app
                        .dialog()
                        .file()
                        .set_title(crate::shared::native_i18n::text(&app, "另存恢复文件"))
                        .add_filter(crate::shared::native_i18n::text(&app, "恢复文件"), &[ext])
                        .set_file_name(format!("SnapDesk-Recovered-{}.{ext}", now()))
                        .blocking_save_file()
                    else {
                        return Ok(None);
                    };
                    let target = path.into_path().map_err(error)?;
                    if target
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case(ext))
                    {
                        return Err(format!("请选择 .{ext} 扩展名。"));
                    }
                    if target == value.path {
                        return Err("恢复目标不能是暂存源文件。".into());
                    }
                    validated(&value, &id)?;
                    let mut input = fs::File::open(&value.path).map_err(error)?;
                    let mut output = TrackedFile::new(
                        &app,
                        target.parent().ok_or("恢复目标目录无效。")?,
                        ext,
                        "恢复文件导出",
                    )?;
                    std::io::copy(&mut input, output.file_mut()).map_err(error)?;
                    output.finalize()?;
                    output.publish(&target, true)?;
                    // 恢复成功也保留原始副本，用户可在中心明确确认后再清理。
                    Ok(Some(target.to_string_lossy().into_owned()))
                })();
                if let Ok(mut active) = ACTIVE.lock() {
                    active.remove(&id);
                }
                result
            }
        }
    })
    .await
    .map_err(error)?
}
