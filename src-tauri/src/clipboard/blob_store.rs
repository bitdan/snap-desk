//! 内容寻址图片与启动回收；所有路径均从数据库相对引用解析，不接受前端路径。
use super::repository::Repository;
use image::{ImageFormat, RgbaImage};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

pub struct BlobStore {
    root: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::super::{
        model::{ClipboardSettings, Kind},
        repository::NewItem,
    };
    use super::*;
    #[test]
    fn repair_keeps_referenced_files_and_removes_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("clipboard")).unwrap();
        let repo = Repository::open(&dir.path().join("history.db")).unwrap();
        let image = RgbaImage::new(2, 2);
        let hash = "a".repeat(64);
        let (blob, thumb, bytes) = store.save(&hash, &image).unwrap();
        repo.insert(
            NewItem {
                kind: Kind::Image,
                hash: &hash,
                text: None,
                blob: Some(&blob),
                thumb: Some(&thumb),
                width: Some(2),
                height: Some(2),
                bytes,
            },
            &ClipboardSettings::default(),
            1,
        )
        .unwrap();
        let (orphan, _, _) = store.save(&"b".repeat(64), &image).unwrap();
        assert!(store.path("../escape").is_err());
        store.repair(&repo).unwrap();
        assert!(store.path(&blob).is_ok());
        assert!(store.path(&orphan).is_err());
        fs::remove_file(store.path(&blob).unwrap()).unwrap();
        store.repair(&repo).unwrap();
        assert_eq!(repo.totals().unwrap().0, 0);
        assert!(store.path(&thumb).is_err());
    }
}
fn err(_: impl std::fmt::Display) -> String {
    "历史图片读写失败，请检查应用数据目录。".into()
}
impl BlobStore {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(err)?;
        let root = root.canonicalize().map_err(err)?;
        let store = Self { root };
        for name in ["images", "thumbs", "temp"] {
            store.directory(name)?;
        }
        Ok(store)
    }
    fn directory(&self, relative: &str) -> Result<PathBuf, String> {
        let mut path = self.root.clone();
        for part in Path::new(relative).components() {
            let Component::Normal(part) = part else {
                return Err("历史路径无效。".into());
            };
            path.push(part);
            if !path.exists() {
                fs::create_dir(&path).map_err(err)?;
            }
            let meta = fs::symlink_metadata(&path).map_err(err)?;
            if meta.file_type().is_symlink()
                || !meta.is_dir()
                || !path.canonicalize().map_err(err)?.starts_with(&self.root)
            {
                return Err("历史目录不能链接到外部位置。".into());
            }
        }
        Ok(path)
    }
    pub fn path(&self, relative: &str) -> Result<PathBuf, String> {
        if Path::new(relative)
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err("历史路径无效。".into());
        }
        let path = self.root.join(relative);
        let mut current = self.root.clone();
        for part in Path::new(relative).components() {
            current.push(part);
            if fs::symlink_metadata(&current)
                .map_err(err)?
                .file_type()
                .is_symlink()
            {
                return Err("历史文件不能是符号链接。".into());
            }
        }
        let resolved = path.canonicalize().map_err(err)?;
        if !resolved.starts_with(&self.root) {
            return Err("历史路径越界。".into());
        }
        Ok(resolved)
    }
    pub fn save(&self, hash: &str, image: &RgbaImage) -> Result<(String, String, u64), String> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("素材摘要无效。".into());
        }
        let mut size = 0;
        let thumb = image::DynamicImage::ImageRgba8(image.clone())
            .thumbnail(240, 160)
            .to_rgba8();
        let mut refs = Vec::new();
        for (folder, ext, format, pixels) in [
            ("images", "png", ImageFormat::Png, image),
            ("thumbs", "webp", ImageFormat::WebP, &thumb),
        ] {
            let dir = self.directory(&format!("{}/{}/{}", folder, &hash[..2], &hash[2..4]))?;
            let relative = format!("{}/{}/{}/{}.{}", folder, &hash[..2], &hash[2..4], hash, ext);
            let target = dir.join(format!("{hash}.{ext}"));
            // 即使崩溃留下同名孤儿，也通过原子替换写入已验证内容。
            let mut temp = tempfile::NamedTempFile::new_in(self.directory("temp")?).map_err(err)?;
            pixels.write_to(temp.as_file_mut(), format).map_err(err)?;
            temp.flush().map_err(err)?;
            temp.as_file().sync_all().map_err(err)?;
            size += temp.as_file().metadata().map_err(err)?.len();
            temp.persist(target).map_err(err)?;
            refs.push(relative);
        }
        Ok((refs.remove(0), refs.remove(0), size))
    }
    /// 调用方持服务操作锁，避免清理与新素材写盘/导出快照交错；数据库锁不跨文件 IO。
    pub fn repair(&self, repo: &Repository) -> Result<(), String> {
        let refs = repo.references()?;
        let mut live = HashSet::new();
        let mut missing = Vec::new();
        for (id, blob, thumb) in refs {
            // 只有明确不存在才移除记录；权限/磁盘暂不可用不能被误认为丢失文件。
            let blob_exists = self.reference_exists(&blob)?;
            let thumb_exists = self.reference_exists(&thumb)?;
            if !blob_exists || !thumb_exists {
                missing.push(id);
            } else {
                live.insert(blob.replace('\\', "/"));
                live.insert(thumb.replace('\\', "/"));
            }
        }
        repo.delete(&missing)?;
        for folder in ["images", "thumbs", "temp"] {
            self.sweep(&self.directory(folder)?, &live)?;
        }
        Ok(())
    }
    fn reference_exists(&self, relative: &str) -> Result<bool, String> {
        if Path::new(relative)
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err("历史路径无效。".into());
        }
        match fs::symlink_metadata(self.root.join(relative)) {
            Ok(_) => {
                let path = self.path(relative)?;
                if !path.is_file() {
                    return Err("历史引用不是普通文件。".into());
                }
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(err(e)),
        }
    }
    fn sweep(&self, dir: &Path, live: &HashSet<String>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(err)? {
            let entry = entry.map_err(err)?;
            let ty = entry.file_type().map_err(err)?;
            if ty.is_symlink() {
                continue;
            }
            let path = entry.path();
            if !path.canonicalize().map_err(err)?.starts_with(&self.root) {
                continue;
            }
            if ty.is_dir() {
                self.sweep(&path, live)?;
            } else if ty.is_file()
                && !live.contains(
                    &path
                        .strip_prefix(&self.root)
                        .map_err(err)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                )
            {
                fs::remove_file(&path).map_err(err)?;
            }
        }
        Ok(())
    }
}
