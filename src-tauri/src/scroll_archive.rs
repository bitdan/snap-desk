//! 原始帧存档。PNG 与追加日志保留在本地，取消/退出不会删除，供调试与离线恢复。
use image::{ImageFormat, RgbaImage};
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
};
use tauri::Manager;

pub struct Archive {
    pub path: PathBuf,
    count: u32,
    bytes: u64,
}
impl Archive {
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
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
