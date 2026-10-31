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

/// 直接借用原图生成等比缩略图，不为 DynamicImage 的所有权接口复制整张 RGBA。
fn thumbnail(image: &RgbaImage) -> RgbaImage {
    let scale =
        (240.0 / f64::from(image.width().max(1))).min(160.0 / f64::from(image.height().max(1)));
    let width = (f64::from(image.width()) * scale).round().max(1.0) as u32;
    let height = (f64::from(image.height()) * scale).round().max(1.0) as u32;
    image::imageops::thumbnail(image, width, height)
}

/// 数据库提交前持有本次写入；失败或提前返回时只回收新文件，不触碰既有素材。
pub struct SavedBlob {
    pub blob: String,
    pub thumb: String,
    pub bytes: u64,
    created: Vec<PathBuf>,
}
impl SavedBlob {
    pub fn commit(mut self) {
        self.created.clear();
    }
}
impl Drop for SavedBlob {
    fn drop(&mut self) {
        for path in &self.created {
            // 清理失败留给下次维护，不覆盖数据库/写盘的原始错误。
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        model::{ClipboardSettings, Kind},
        repository::NewItem,
    };
    use super::*;
    #[test]
    fn quota_cleanup_removes_only_returned_paths_and_preserves_rollback_and_shared_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("clipboard")).unwrap();
        let repo = Repository::open(&dir.path().join("history.db")).unwrap();
        let image = RgbaImage::new(2, 2);
        let hash = "a".repeat(64);
        let first = store.save(&hash, &image).unwrap();
        let (blob, thumb) = (first.blob.clone(), first.thumb.clone());
        let settings = ClipboardSettings {
            max_items: 1,
            ..ClipboardSettings::default()
        };
        repo.insert(
            NewItem {
                kind: Kind::Image,
                hash: &hash,
                text: None,
                blob: Some(&blob),
                thumb: Some(&thumb),
                width: Some(2),
                height: Some(2),
                bytes: first.bytes,
            },
            &settings,
            1,
        )
        .unwrap();
        first.commit();
        repo.favorite(1, true).unwrap();
        let rejected = store.save(&"b".repeat(64), &image).unwrap();
        let rejected_blob = rejected.blob.clone();
        assert!(repo
            .insert(
                NewItem {
                    kind: Kind::Image,
                    hash: "other",
                    text: None,
                    blob: Some(&rejected.blob),
                    thumb: Some(&rejected.thumb),
                    width: Some(2),
                    height: Some(2),
                    bytes: rejected.bytes
                },
                &settings,
                2
            )
            .is_err());
        drop(rejected);
        assert!(store.reference_exists(&blob).unwrap());
        assert!(!store.reference_exists(&rejected_blob).unwrap());

        repo.favorite(1, false).unwrap();
        // 新条目复用旧原图时，淘汰旧记录只能移除旧缩略图，不能删除仍被引用的原图。
        let other_thumb = "thumbs/shared.webp";
        fs::copy(store.path(&thumb).unwrap(), store.root.join(other_thumb)).unwrap();
        let orphan = store.save(&"c".repeat(64), &image).unwrap();
        let unrelated = orphan.blob.clone();
        orphan.commit();
        let removed = repo
            .insert(
                NewItem {
                    kind: Kind::Image,
                    hash: "shared",
                    text: None,
                    blob: Some(&blob),
                    thumb: Some(other_thumb),
                    width: Some(2),
                    height: Some(2),
                    bytes: 16,
                },
                &settings,
                3,
            )
            .unwrap();
        store.remove(&removed).unwrap();
        assert!(store.reference_exists(&blob).unwrap());
        assert!(!store.reference_exists(&thumb).unwrap());
        assert!(store.reference_exists(other_thumb).unwrap());
        assert!(store.reference_exists(&unrelated).unwrap());
        let removed = repo.clear(true).unwrap();
        store.remove(&removed).unwrap();
        store.remove(&removed).unwrap();
        assert!(!store.reference_exists(&blob).unwrap());
        assert!(!store.reference_exists(other_thumb).unwrap());
    }
    #[test]
    fn borrowed_thumbnail_preserves_existing_size_pixels_and_alpha() {
        for (width, height) in [(2, 2), (601, 199), (199, 601), (1, 400), (1920, 1080)] {
            let image = RgbaImage::from_fn(width, height, |x, y| {
                image::Rgba([x as u8, y as u8, (x ^ y) as u8, x.wrapping_mul(y) as u8])
            });
            let expected = image::DynamicImage::ImageRgba8(image.clone())
                .thumbnail(240, 160)
                .to_rgba8();
            assert_eq!(thumbnail(&image), expected);
        }
    }
    #[test]
    fn rollback_only_removes_new_files_including_partial_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("clipboard")).unwrap();
        let hash = "c".repeat(64);
        let image = RgbaImage::new(2, 2);
        let saved = store.save(&hash, &image).unwrap();
        let blob = saved.blob.clone();
        let thumb = saved.thumb.clone();
        drop(saved);
        assert!(!store.reference_exists(&blob).unwrap());
        assert!(!store.reference_exists(&thumb).unwrap());

        store.save(&hash, &image).unwrap().commit();
        fs::remove_file(store.path(&thumb).unwrap()).unwrap();
        drop(store.save(&hash, &image).unwrap());
        assert!(store.reference_exists(&blob).unwrap());
        assert!(!store.reference_exists(&thumb).unwrap());

        // 让缩略图分目录创建失败，验证原图刚写完也会自动回收。
        let hash = "d".repeat(64);
        fs::write(store.root.join("thumbs/dd"), b"blocked").unwrap();
        assert!(store.save(&hash, &image).is_err());
        assert!(!store
            .reference_exists(&format!("images/dd/dd/{hash}.png"))
            .unwrap());
    }
    #[test]
    fn repair_keeps_referenced_files_and_removes_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("clipboard")).unwrap();
        let repo = Repository::open(&dir.path().join("history.db")).unwrap();
        let image = RgbaImage::new(2, 2);
        let hash = "a".repeat(64);
        let saved = store.save(&hash, &image).unwrap();
        let (blob, thumb, bytes) = (saved.blob.clone(), saved.thumb.clone(), saved.bytes);
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
        saved.commit();
        let saved = store.save(&"b".repeat(64), &image).unwrap();
        let orphan = saved.blob.clone();
        saved.commit();
        assert!(store.path("../escape").is_err());
        store.repair(&repo).unwrap();
        assert!(store.path(&blob).is_ok());
        assert!(store.path(&orphan).is_err());
        // 缩略图丢失时保留历史，并从原图恢复可解码的缩略图。
        fs::remove_file(store.path(&thumb).unwrap()).unwrap();
        store.repair(&repo).unwrap();
        assert_eq!(repo.totals().unwrap().0, 1);
        assert!(image::open(store.path(&thumb).unwrap()).is_ok());
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
    pub fn save(&self, hash: &str, image: &RgbaImage) -> Result<SavedBlob, String> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("素材摘要无效。".into());
        }
        let mut saved = SavedBlob {
            blob: String::new(),
            thumb: String::new(),
            bytes: 0,
            created: Vec::new(),
        };
        let thumb = thumbnail(image);
        let mut refs = Vec::new();
        for (folder, ext, format, pixels) in [
            ("images", "png", ImageFormat::Png, image),
            ("thumbs", "webp", ImageFormat::WebP, &thumb),
        ] {
            let dir = self.directory(&format!("{}/{}/{}", folder, &hash[..2], &hash[2..4]))?;
            let relative = format!("{}/{}/{}/{}.{}", folder, &hash[..2], &hash[2..4], hash, ext);
            let target = dir.join(format!("{hash}.{ext}"));
            // 内容寻址文件可以复用；回滚不能删除此前已有的同名文件。
            let existed = self.reference_exists(&relative)?;
            if existed {
                saved.bytes += fs::metadata(self.path(&relative)?).map_err(err)?.len();
                refs.push(relative);
                continue;
            }
            let mut temp = tempfile::NamedTempFile::new_in(self.directory("temp")?).map_err(err)?;
            pixels.write_to(temp.as_file_mut(), format).map_err(err)?;
            temp.flush().map_err(err)?;
            temp.as_file().sync_all().map_err(err)?;
            saved.bytes += temp.as_file().metadata().map_err(err)?.len();
            temp.persist(&target).map_err(err)?;
            if !existed {
                saved.created.push(target);
            }
            refs.push(relative);
        }
        saved.blob = refs.remove(0);
        saved.thumb = refs.remove(0);
        Ok(saved)
    }
    /// 只清理已提交事务返回的孤儿路径；调用方继续持有服务操作锁，防止复用素材时交错删除。
    /// 单个文件失败不阻止其余清理，失败残留仍由启动和定时 repair 兜底。
    pub fn remove(&self, references: &[String]) -> Result<(), String> {
        let mut failure = None;
        for relative in references {
            let result = (|| {
                if self.reference_exists(relative)? {
                    match fs::remove_file(self.path(relative)?) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(err(error)),
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
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
            if !blob_exists {
                missing.push(id);
            } else {
                if !thumb_exists {
                    // 解码或写盘失败时返回错误并保留记录，不能将暂时故障当作数据丢失。
                    let image = crate::pins::content::read_image(&self.path(&blob)?)?;
                    let thumbnail = image::DynamicImage::ImageRgba8(image).thumbnail(240, 160);
                    let parent = Path::new(&thumb).parent().ok_or("缩略图路径无效。")?;
                    let dir = self.directory(parent.to_str().ok_or("缩略图路径无效。")?)?;
                    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(err)?;
                    thumbnail
                        .write_to(temp.as_file_mut(), ImageFormat::WebP)
                        .map_err(err)?;
                    temp.flush().map_err(err)?;
                    temp.as_file().sync_all().map_err(err)?;
                    temp.persist(self.root.join(&thumb)).map_err(err)?;
                }
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
