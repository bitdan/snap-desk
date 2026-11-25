//! 贴图历史：关闭的贴图与退出时仍打开的贴图存到磁盘，可重新贴出。
//!
//! 每条记录一个目录：`image.png` 为贴图当前底图（含旋转、翻转等变换），`state.json` 为可编辑标注、
//! 位置与显示状态。目录名是关闭时刻的毫秒时间戳，越大越新。
//! “已关闭”最多保留 [`MAX_ENTRIES`] 条且不超过 [`MAX_BYTES`]；“上次退出”每次退出时整体替换。
use crate::annotations::Annotation;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const MAX_ENTRIES: usize = 20;
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE: &str = "image.png";
const STATE: &str = "state.json";

/// 重新贴出所需的状态；位置为图片首像素的全局物理坐标与所在屏缩放。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub drag_text: Option<String>,
    pub origin: (i32, i32),
    pub scale: f64,
    pub zoom: f64,
    pub opacity: f32,
    #[serde(default)]
    pub locked: bool,
    /// 关闭（或退出）时刻，Unix 毫秒。
    pub closed_at: i64,
}

/// 列表项：只读 `state.json` 与 PNG 头，不解码整张图。
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub closed_at: i64,
    pub width: u32,
    pub height: u32,
    pub image: PathBuf,
}

/// 一个历史目录（“已关闭”或“上次退出”）。
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 关闭的贴图：`app_local_data_dir/pins/closed`。
    pub fn closed() -> Option<Self> {
        crate::paths::local_data_dir().map(|dir| Self::new(dir.join("pins").join("closed")))
    }

    /// 上次退出时打开的贴图：`app_local_data_dir/pins/session`。
    pub fn session() -> Option<Self> {
        crate::paths::local_data_dir().map(|dir| Self::new(dir.join("pins").join("session")))
    }

    /// 新增一条记录，返回其 id；随后按条数与字节上限删去最旧的记录。
    pub fn push(&self, image: &RgbaImage, state: &State) -> Result<String, String> {
        let id = self.write(image, state)?;
        self.trim(MAX_ENTRIES, MAX_BYTES);
        Ok(id)
    }

    /// 用 `records` 整体替换目录内容（退出时保存仍打开的贴图）。
    pub fn replace(&self, records: &[(&RgbaImage, &State)]) -> Result<(), String> {
        self.clear()?;
        for (image, state) in records {
            self.write(image, state)?;
        }
        Ok(())
    }

    fn write(&self, image: &RgbaImage, state: &State) -> Result<String, String> {
        if image.width() == 0 || image.height() == 0 {
            return Err("贴图为空。".into());
        }
        let id = unique_id(state.closed_at);
        let dir = self.dir.join(&id);
        // 先写临时目录再改名：中途失败不会留下缺文件的记录。
        let partial = self.dir.join(format!(".{id}.partial"));
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(&partial).map_err(|e| format!("无法创建贴图历史目录：{e}"))?;
        let result = (|| {
            image.save(partial.join(IMAGE)).map_err(|e| format!("无法保存贴图历史图片：{e}"))?;
            let json = serde_json::to_vec(state).map_err(|e| e.to_string())?;
            fs::write(partial.join(STATE), json).map_err(|e| format!("无法保存贴图历史：{e}"))?;
            fs::rename(&partial, &dir).map_err(|e| format!("无法保存贴图历史：{e}"))
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&partial);
        }
        result.map(|()| id)
    }

    /// 最新的在前；损坏的记录跳过。
    pub fn list(&self) -> Vec<Entry> {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut entries: Vec<Entry> = read
            .flatten()
            .filter_map(|item| {
                let id = item.file_name().to_str()?.to_owned();
                if id.starts_with('.') {
                    return None;
                }
                let state = read_state(&item.path()).ok()?;
                let image = item.path().join(IMAGE);
                let (width, height) = image::image_dimensions(&image).ok()?;
                Some(Entry { id, closed_at: state.closed_at, width, height, image })
            })
            .collect();
        entries.sort_by(|a, b| b.id.cmp(&a.id));
        entries
    }

    /// 读取一条记录；id 只能是本目录下的条目名。
    pub fn load(&self, id: &str) -> Result<(RgbaImage, State), String> {
        let dir = self.entry_dir(id)?;
        let state = read_state(&dir)?;
        let image = super::content::read_image(&dir.join(IMAGE))?;
        Ok((image, state))
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let dir = self.entry_dir(id)?;
        fs::remove_dir_all(dir).map_err(|e| format!("无法删除贴图历史：{e}"))
    }

    pub fn clear(&self) -> Result<(), String> {
        match fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("无法清空贴图历史：{error}")),
        }
    }

    fn entry_dir(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || id.starts_with('.') || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("贴图历史条目无效。".into());
        }
        Ok(self.dir.join(id))
    }

    /// 保留最新的 `entries` 条，且总大小不超过 `bytes`（最新一条总是保留）。
    fn trim(&self, entries: usize, bytes: u64) {
        let mut used = 0;
        for (index, entry) in self.list().into_iter().enumerate() {
            used += dir_size(&self.dir.join(&entry.id));
            if index >= entries || (index > 0 && used > bytes) {
                let _ = self.remove(&entry.id);
            }
        }
    }
}

fn read_state(dir: &Path) -> Result<State, String> {
    let bytes = fs::read(dir.join(STATE)).map_err(|e| format!("无法读取贴图历史：{e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("贴图历史已损坏：{e}"))
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|read| read.flatten().filter_map(|item| item.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

/// 毫秒时间戳补零到 16 位（按字符串排序即按时间排序），同一毫秒内再加序号。
fn unique_id(closed_at: i64) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed) % 10_000;
    format!("{:016}-{sequence:04}", closed_at.max(0))
}

/// 当前 Unix 毫秒。
pub fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(closed_at: i64) -> State {
        State {
            annotations: Vec::new(),
            drag_text: Some("hello".into()),
            origin: (10, -20),
            scale: 1.5,
            zoom: 0.8,
            opacity: 0.6,
            locked: true,
            closed_at,
        }
    }

    fn image(width: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, 3, image::Rgba([1, 2, 3, 200]))
    }

    #[test]
    fn push_lists_newest_first_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let first = store.push(&image(4), &state(1)).unwrap();
        let second = store.push(&image(5), &state(2)).unwrap();
        let list = store.list();
        assert_eq!(list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [second.as_str(), first.as_str()]);
        assert_eq!((list[0].width, list[0].height), (5, 3));
        let (loaded, saved) = store.load(&first).unwrap();
        assert_eq!(loaded, image(4));
        assert_eq!(saved.origin, (10, -20));
        assert_eq!(saved.drag_text.as_deref(), Some("hello"));
        assert!(saved.locked);
        store.remove(&first).unwrap();
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn trim_keeps_newest_within_limits() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        for at in 0..5 {
            store.write(&image(4), &state(at)).unwrap();
        }
        store.trim(3, u64::MAX);
        let list = store.list();
        assert_eq!(list.iter().map(|e| e.closed_at).collect::<Vec<_>>(), [4, 3, 2]);
        // 字节上限再小也保留最新一条。
        store.trim(3, 1);
        assert_eq!(store.list().iter().map(|e| e.closed_at).collect::<Vec<_>>(), [4]);
    }

    #[test]
    fn replace_clears_previous_session_and_rejects_bad_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("session"));
        store.replace(&[(&image(4), &state(1)), (&image(4), &state(2))]).unwrap();
        assert_eq!(store.list().len(), 2);
        store.replace(&[]).unwrap();
        assert!(store.list().is_empty());
        assert!(store.load("../settings").is_err());
        assert!(store.remove(".partial").is_err());
    }
}
