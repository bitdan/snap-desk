//! 本机日志级别偏好（配置目录 `logging.json`，与旧版相同）：独立于可导入的业务配置。
use super::logs::Level;
use std::{fs, io::Write, path::Path};

/// 配置目录下的级别文件名。
pub const FILE_NAME: &str = "logging.json";

/// 文件缺失时默认 INFO；内容异常时报错，调用方保留默认。
pub fn read(path: &Path) -> Result<Level, String> {
    if !path.exists() {
        return Ok(Level::Info);
    }
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 64 {
        return Err("日志级别配置无效。".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "日志级别配置无效。".into())
}

/// 原子替换；落盘失败时调用方保留当前阈值。
pub fn persist(path: &Path, level: Level) -> Result<(), String> {
    let parent = path.parent().ok_or("日志级别配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(&serde_json::to_vec(&level).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// 阈值是否放行某条日志；TRACE 永不放行。
pub fn allows(threshold: Level, value: log::Level) -> bool {
    let limit = match threshold {
        Level::Error => log::Level::Error,
        Level::Warn => log::Level::Warn,
        Level::Info => log::Level::Info,
        Level::Debug => log::Level::Debug,
    };
    value <= limit
}

/// 原子存储用的编码。
pub fn code(level: Level) -> u8 {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
        Level::Debug => 3,
    }
}

pub fn from_code(code: u8) -> Level {
    match code {
        0 => Level::Error,
        1 => Level::Warn,
        3 => Level::Debug,
        _ => Level::Info,
    }
}
