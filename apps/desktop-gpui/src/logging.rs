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
        if !self.enabled(record.metadata()) || benign(record) {
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

/// GPUI 框架已知无害的错误日志（目标为空）：关闭窗口后迟到的激活状态通知找不到窗口；
/// 未安装 DirectX 调试层时取调试接口失败（随后另有 WARN 说明）。它们不代表故障，只会淹没真正的错误。
fn benign(record: &log::Record) -> bool {
    if record.level() != log::Level::Error || !record.target().is_empty() {
        return false;
    }
    let message = record.args().to_string();
    (message.starts_with("handling active status change for window") && message.contains("window not found"))
        || message.contains("0x887A002D")
}

#[cfg(test)]
mod tests {
    #[test]
    fn filters_only_known_framework_noise() {
        let check = |level, target: &str, message: std::fmt::Arguments| {
            super::benign(&log::Record::builder().level(level).target(target).args(message).build())
        };
        assert!(check(log::Level::Error, "", format_args!("handling active status change for window WindowId(3): window not found")));
        assert!(check(log::Level::Error, "", format_args!("缺失的 SDK 组件。 (0x887A002D)")));
        assert!(!check(log::Level::Error, "", format_args!("handling active status change for window WindowId(3): other")));
        assert!(!check(log::Level::Error, "snap_desk_gpui", format_args!("window not found 0x887A002D")));
        assert!(!check(log::Level::Warn, "", format_args!("0x887A002D")));
    }
}
