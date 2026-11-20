//! 日志：`log` 门面 + 追加写入应用日志目录。
//!
//! 与 Tauri 版共用日志目录（`snap_core::paths::log_dir`），但写入独立文件 `snap-desk-gpui.log`，
//! 两个程序同时运行时不会交错写同一文件。日志器自身失败（磁盘满、文件被占用）只丢弃该条，
//! 不影响用户操作，也不在失败路径递归记录。
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

/// 超过此大小在启动时轮转为 `.old`，只保留上一份，避免日志无限增长。
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;
const FILE_NAME: &str = "snap-desk-gpui.log";

struct FileLogger {
    level: log::LevelFilter,
    /// Mutex 让多线程（主线程、单实例管道线程）写入的行不交错；锁中毒后放弃文件输出。
    file: Option<Mutex<File>>,
    /// 调试构建同时输出到 stderr，便于在终端里观察快捷键与托盘事件。
    stderr: bool,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {} {}\n",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
            record.level(),
            record.target(),
            record.args()
        );
        if self.stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.flush();
            }
        }
    }
}

/// 级别取环境变量 `SNAP_DESK_LOG`（error/warn/info/debug/trace），默认 info。
/// 返回日志文件路径；目录不可用时只输出到 stderr。
pub fn init() -> Option<PathBuf> {
    let level = std::env::var("SNAP_DESK_LOG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(log::LevelFilter::Info);
    let path = snap_core::paths::log_dir().map(|dir| dir.join(FILE_NAME));
    let file = path.as_ref().and_then(|path| open(path).ok());
    let logger = FileLogger {
        level,
        file: file.map(Mutex::new),
        stderr: cfg!(debug_assertions),
    };
    // 只在进程启动时调用一次；重复设置会失败，此时沿用已有日志器。
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    path
}

fn open(path: &PathBuf) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() > ROTATE_BYTES) {
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
    OpenOptions::new().create(true).append(true).open(path)
}
