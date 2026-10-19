//! 原始帧存档。PNG 与追加日志保留在本地，取消/退出不会删除，供调试与离线恢复。
use image::{ImageFormat, RgbaImage};
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
};
use tauri::Manager;

struct Writer {
    pub path: PathBuf,
    count: u32,
    bytes: u64,
}
impl Writer {
    pub fn new(app: &tauri::AppHandle) -> Result<Self, String> {
        let root = app
            .path()
            .app_local_data_dir()
            .map_err(err)?
            .join("scroll-frames");
        fs::create_dir_all(&root).map_err(err)?;
        let directory = tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(root)
            .map_err(err)?;
        Ok(Self {
            path: directory.keep(),
            count: 0,
            bytes: 0,
        })
    }
    pub fn record(&self, event: serde_json::Value) -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path.join("events.jsonl"))
            .map_err(err)?;
        writeln!(file, "{event}").map_err(err)?;
        file.sync_data().map_err(err)
    }
    /// 文件名只由后端编号生成；限额到达时暂停，避免无限写盘。
    pub fn frame(&mut self, image: &RgbaImage) -> Result<(), String> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).map_err(err)?;
        let bytes = bytes.into_inner();
        if self.count >= 128 || self.bytes + bytes.len() as u64 > 512 * 1024 * 1024 {
            return Err("原始帧存档达到 128 帧 / 512 MiB 上限，请保存当前结果。".into());
        }
        let name = format!("{:04}.png", self.count);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path.join(&name))
            .map_err(err)?;
        file.write_all(&bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        self.count += 1;
        self.bytes += bytes.len() as u64;
        self.record(serde_json::json!({"event":"frame", "file":name}))
    }
}

enum Entry {
    Event(serde_json::Value),
    Frame(Arc<RgbaImage>),
}

/// 队列最多两项，帧只共享像素所有权；编码和同步落盘都在独立线程。
/// 队列满或写盘失败立即停用调试存档，不能让慢磁盘阻塞采集。
#[derive(Clone)]
pub struct Archive {
    pub path: PathBuf,
    sender: SyncSender<Entry>,
    failed: Arc<AtomicBool>,
}
impl Archive {
    pub fn new(app: &tauri::AppHandle) -> Result<Self, String> {
        let mut writer = Writer::new(app)?;
        let path = writer.path.clone();
        let (sender, receiver) = mpsc::sync_channel(2);
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        std::thread::Builder::new()
            .name("scroll-archive".into())
            .spawn(move || {
                for entry in receiver {
                    let result = match entry {
                        Entry::Event(event) => writer.record(event),
                        Entry::Frame(image) => writer.frame(&image),
                    };
                    if let Err(_error) = result {
                        failure.store(true, Ordering::Release);
                        log::warn!("event=scroll.archive_failed module=scroll code=ARCHIVE_WRITE_FAILED");
                        break;
                    }
                }
            })
            .map_err(err)?;
        Ok(Self {
            path,
            sender,
            failed,
        })
    }
    fn enqueue(&self, entry: Entry) -> Result<(), String> {
        if self.failed.load(Ordering::Acquire) {
            return Err("原始帧存档已停用，截图继续。".into());
        }
        self.sender.try_send(entry).map_err(|_| {
            self.failed.store(true, Ordering::Release);
            "原始帧存档队列已满或写入失败，截图继续。".into()
        })
    }
    pub fn record(&self, event: serde_json::Value) -> Result<(), String> {
        self.enqueue(Entry::Event(event))
    }
    pub fn frame(&self, image: Arc<RgbaImage>) -> Result<(), String> {
        self.enqueue(Entry::Frame(image))
    }
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_archive_never_waits_when_disk_worker_is_busy() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let archive = Archive {
            path: PathBuf::new(),
            sender,
            failed: Arc::new(AtomicBool::new(false)),
        };
        let frame = Arc::new(RgbaImage::new(32, 64));
        archive.frame(frame.clone()).unwrap();
        archive.frame(frame.clone()).unwrap();
        assert!(archive.frame(frame.clone()).is_err());
        assert!(archive.failed.load(Ordering::Acquire));
        if let Entry::Frame(stored) = receiver.recv().unwrap() {
            assert!(Arc::ptr_eq(&stored, &frame));
        } else {
            panic!("应保存帧引用");
        }
        assert!(archive
            .record(serde_json::json!({"event":"match"}))
            .is_err());
    }
}
