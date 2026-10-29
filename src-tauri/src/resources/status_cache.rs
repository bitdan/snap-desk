//! 设置页复用近期完整校验结果；文件树变化或缓存过期后重新校验。
//! 仅用于状态展示，模型加载、引擎启动和安装仍走各自的完整校验。
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

const MAX_AGE: Duration = Duration::from_secs(300);
const MAX_ENTRIES: usize = 4096;

#[derive(PartialEq)]
struct Stamp {
    path: PathBuf,
    directory: bool,
    length: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}

struct Cached<T> {
    root: PathBuf,
    files: Vec<Stamp>,
    checked_at: Instant,
    value: T,
}

pub(crate) struct StatusCache<T>(Mutex<Option<Cached<T>>>);
impl<T: Clone> StatusCache<T> {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// 锁只协调同类状态请求，不与识别 Worker 或资源安装共用，避免重复解压和哈希。
    pub(crate) fn read(
        &self,
        root: &Path,
        check: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut cached = self.0.lock().map_err(|e| e.to_string())?;
        let before = snapshot(root).ok();
        if let (Some(previous), Some(files)) = (cached.as_ref(), before.as_ref()) {
            if previous.root == root
                && previous.files == *files
                && previous.checked_at.elapsed() < MAX_AGE
            {
                return Ok(previous.value.clone());
            }
        }
        // 失败和读取期间被修改的结果都不缓存；目录含链接时也保留原有完整检查路径。
        *cached = None;
        let value = check()?;
        if let Some(files) = before {
            if snapshot(root).ok().as_ref() == Some(&files) {
                *cached = Some(Cached {
                    root: root.to_path_buf(),
                    files,
                    checked_at: Instant::now(),
                    value: value.clone(),
                });
            }
        }
        Ok(value)
    }
}

/// 只读取元数据，不打开大模型内容；创建、替换、删除和新增文件都会使快照变化。
fn snapshot(root: &Path) -> io::Result<Vec<Stamp>> {
    let mut result = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if path == root && error.kind() == io::ErrorKind::NotFound => {
                return Ok(Vec::new())
            }
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink()
            || !(metadata.is_file() || metadata.is_dir())
            || result.len() >= MAX_ENTRIES
        {
            return Err(io::Error::other("资源状态目录无法缓存。"));
        }
        result.push(Stamp {
            path: path.clone(),
            directory: metadata.is_dir(),
            length: metadata.len(),
            modified: metadata.modified()?,
            created: metadata.created().ok(),
        });
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn unchanged_files_reuse_successful_verification() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("model.bin"), b"verified").unwrap();
        let cache = StatusCache::new();
        let calls = Cell::new(0);
        for _ in 0..3 {
            assert!(cache
                .read(root.path(), || {
                    calls.set(calls.get() + 1);
                    Ok(true)
                })
                .unwrap());
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn same_size_changes_deletion_and_new_files_invalidate_status() {
        let root = tempfile::tempdir().unwrap();
        let model = root.path().join("model.bin");
        fs::write(&model, b"abc").unwrap();
        let cache = StatusCache::new();
        assert!(cache.read(root.path(), || Ok(true)).unwrap());
        fs::write(&model, b"abd").unwrap();
        fs::File::options()
            .write(true)
            .open(&model)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
            )
            .unwrap();
        assert!(!cache.read(root.path(), || Ok(false)).unwrap());
        fs::remove_file(&model).unwrap();
        assert!(cache.read(root.path(), || Ok(true)).unwrap());
        fs::write(&model, b"abc").unwrap();
        assert!(!cache.read(root.path(), || Ok(false)).unwrap());
    }

    #[test]
    fn expiration_failures_and_files_changed_during_check_are_not_reused() {
        let root = tempfile::tempdir().unwrap();
        let cache = StatusCache::new();
        assert!(cache.read(root.path(), || Ok(true)).unwrap());
        cache.0.lock().unwrap().as_mut().unwrap().checked_at = Instant::now() - MAX_AGE;
        assert!(cache.read(root.path(), || Err("校验失败".into())).is_err());
        assert!(!cache.read(root.path(), || Ok(false)).unwrap());
        let second = StatusCache::new();
        second
            .read(root.path(), || {
                fs::write(root.path().join("new"), b"new").unwrap();
                Ok(true)
            })
            .unwrap();
        assert!(!second.read(root.path(), || Ok(false)).unwrap());
    }
}
