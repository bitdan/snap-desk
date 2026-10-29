//! 导出安全暂存：先编码并同步，再发布最终文件；失败时不覆盖已存在的成品。
//! 只管理本次导出声明的文件和目录，不执行启动扫描或提供恢复清理界面。
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
        store(&record, &value)?;
        let (file, _) = file.keep().map_err(error)?;
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
        store(&record, &value)?;
        // 登记成功后保留目录，交由恢复记录管理；路径已在 value 中，无需再次保存。
        let _ = directory.keep();
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
