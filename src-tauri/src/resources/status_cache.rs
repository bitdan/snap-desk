//! 设置页按资源分区复用近期完整校验结果；只观察调用方声明的文件或子树。
//! 仅用于状态展示，模型加载、引擎启动和安装仍走各自的完整校验。
use std::{
    fs, io,
    path::PathBuf,
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
    roots: Vec<PathBuf>,
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
        roots: &[PathBuf],
        check: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut cached = self.0.lock().map_err(|e| e.to_string())?;
        // 不加入依赖路径的父目录；共享 resources 根新增字体/临时包时不连带其他分区。
        let mut roots = roots.to_vec();
        roots.sort();
        roots.dedup();
        let before = snapshot(&roots).ok();
        if let (Some(previous), Some(files)) = (cached.as_ref(), before.as_ref()) {
            if previous.roots == roots
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
            if snapshot(&roots).ok().as_ref() == Some(&files) {
                *cached = Some(Cached {
                    roots,
                    files,
                    checked_at: Instant::now(),
                    value: value.clone(),
                });
            }
        }
        Ok(value)
    }
}

/// 直接文件只观察自身；目录观察完整子树。缺失依赖允许缓存未就绪状态，安装后自动失效。
/// 不读取大模型内容；同大小替换仍通过修改/创建时间识别，实际加载始终重新验证摘要。
fn snapshot(roots: &[PathBuf]) -> io::Result<Vec<Stamp>> {
    let mut result = Vec::new();
    let mut pending = roots.to_vec();
    while let Some(path) = pending.pop() {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if roots.contains(&path) && error.kind() == io::ErrorKind::NotFound => {
                continue
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

    fn read_partitions(
        caches: &[StatusCache<usize>],
        dependencies: &[Vec<PathBuf>],
        calls: &[Cell<usize>],
    ) -> Vec<usize> {
        caches
            .iter()
            .enumerate()
            .map(|(index, cache)| {
                cache
                    .read(&dependencies[index], || {
                        calls[index].set(calls[index].get() + 1);
                        Ok(calls[index].get())
                    })
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn explicit_dependencies_isolate_shared_parent_and_sibling_resource_changes() {
        let root = tempfile::tempdir().unwrap();
        let ocr = root.path().join("ocr.onnx");
        let runtime = root.path().join("ocr-runtime");
        let font = root.path().join("export-font.ttf");
        let engine = root.path().join("translator-runtime");
        let en_zh = root.path().join("translation/en-zh");
        let zh_en = root.path().join("translation/zh-en");
        let dependencies = [
            vec![ocr.clone(), runtime.clone()],
            vec![font.clone()],
            vec![engine.clone()],
            vec![en_zh.clone()],
            vec![zh_en.clone()],
        ];
        let caches: [StatusCache<usize>; 5] = std::array::from_fn(|_| StatusCache::new());
        let calls: [Cell<usize>; 5] = std::array::from_fn(|_| Cell::new(0));
        // 所有依赖尚不存在时允许缓存缺失状态；共享父目录新增临时文件不能使其失效。
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![1, 1, 1, 1, 1]
        );
        fs::write(root.path().join("unrelated-download.tmp"), b"partial").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![1, 1, 1, 1, 1]
        );
        fs::write(&ocr, b"model").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 1, 1, 1, 1]
        );
        fs::write(&font, b"font").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 2, 1, 1, 1]
        );
        fs::create_dir(&engine).unwrap();
        fs::write(engine.join("worker.exe"), b"worker").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 2, 2, 1, 1]
        );
        fs::create_dir_all(&en_zh).unwrap();
        fs::write(en_zh.join("model.bin"), b"abc").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 2, 2, 2, 1]
        );
        fs::create_dir_all(&zh_en).unwrap();
        fs::write(zh_en.join("model.bin"), b"abc").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 2, 2, 2, 2]
        );
        // 相同字节长度的原子替换也重新检查；修改时间显式区分，兼容低精度文件系统。
        let replacement = root.path().join("replacement.tmp");
        fs::write(&replacement, b"def").unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
            )
            .unwrap();
        fs::rename(&replacement, en_zh.join("model.bin")).unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 2, 2, 3, 2]
        );
        fs::remove_file(&font).unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![2, 3, 2, 3, 2]
        );
        fs::create_dir(&runtime).unwrap();
        fs::write(runtime.join("engine.dll"), b"runtime").unwrap();
        assert_eq!(
            read_partitions(&caches, &dependencies, &calls),
            vec![3, 3, 2, 3, 2]
        );
    }

    #[test]
    fn direct_dependencies_changed_during_check_and_failed_checks_are_not_cached() {
        let root = tempfile::tempdir().unwrap();
        let model = root.path().join("new-model.bin");
        let dependencies = [model.clone()];
        let cache = StatusCache::new();
        assert!(cache
            .read(&dependencies, || {
                fs::write(&model, b"installed-during-check").unwrap();
                Ok(false)
            })
            .is_ok());
        assert!(cache.read(&dependencies, || Ok(true)).unwrap());
        fs::remove_file(&model).unwrap();
        assert!(cache
            .read(&dependencies, || Err("校验失败".into()))
            .is_err());
        assert!(!cache.read(&dependencies, || Ok(false)).unwrap());
    }

    #[test]
    fn unchanged_files_reuse_successful_verification() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("model.bin"), b"verified").unwrap();
        let cache = StatusCache::new();
        let calls = Cell::new(0);
        for _ in 0..3 {
            assert!(cache
                .read(&[root.path().to_path_buf()], || {
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
        assert!(cache
            .read(&[root.path().to_path_buf()], || Ok(true))
            .unwrap());
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
        assert!(!cache
            .read(&[root.path().to_path_buf()], || Ok(false))
            .unwrap());
        fs::remove_file(&model).unwrap();
        assert!(cache
            .read(&[root.path().to_path_buf()], || Ok(true))
            .unwrap());
        fs::write(&model, b"abc").unwrap();
        assert!(!cache
            .read(&[root.path().to_path_buf()], || Ok(false))
            .unwrap());
    }

    #[test]
    fn expiration_failures_and_files_changed_during_check_are_not_reused() {
        let root = tempfile::tempdir().unwrap();
        let cache = StatusCache::new();
        assert!(cache
            .read(&[root.path().to_path_buf()], || Ok(true))
            .unwrap());
        cache.0.lock().unwrap().as_mut().unwrap().checked_at = Instant::now() - MAX_AGE;
        assert!(cache
            .read(&[root.path().to_path_buf()], || Err("校验失败".into()))
            .is_err());
        assert!(!cache
            .read(&[root.path().to_path_buf()], || Ok(false))
            .unwrap());
        let second = StatusCache::new();
        second
            .read(&[root.path().to_path_buf()], || {
                fs::write(root.path().join("new"), b"new").unwrap();
                Ok(true)
            })
            .unwrap();
        assert!(!second
            .read(&[root.path().to_path_buf()], || Ok(false))
            .unwrap());
    }
}
