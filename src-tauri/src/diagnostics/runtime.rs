//! 运行时日志阈值：本机持久化、立即切换；高频调试采样使用独立单调时钟。
use super::logs::Level;
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    OnceLock,
};
use std::time::Instant;
use std::{fs, io::Write, path::Path};
use tauri::Manager;

static CLOCK: OnceLock<Instant> = OnceLock::new();
static LEVEL: AtomicU8 = AtomicU8::new(2);
static LAST_SAMPLE: AtomicU64 = AtomicU64::new(0);

fn millis() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

fn code(level: Level) -> u8 {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
        Level::Debug => 3,
    }
}

/// 返回当前实际生效阈值，供状态页与诊断包使用。
pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Error,
        1 => Level::Warn,
        3 => Level::Debug,
        _ => Level::Info,
    }
}

fn read_level(path: &Path) -> Result<Level, String> {
    if !path.exists() {
        return Ok(Level::Info);
    }
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 64 {
        return Err("日志级别配置无效。".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "日志级别配置无效。".into())
}

/// 独立保存本机排错偏好，不混入可跨机器导入的业务配置；落盘失败保留当前阈值。
fn persist_level(path: &Path, level: Level) -> Result<(), String> {
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

/// 启动时恢复本机偏好，配置缺失默认 INFO，损坏时保留默认并记录告警。
pub fn initialize(app: &tauri::AppHandle) {
    let loaded = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())
        .and_then(|root| read_level(&root.join("logging.json")));
    match loaded {
        Ok(value) => {
            LEVEL.store(code(value), Ordering::Relaxed);
        }
        Err(_) => {
            log::warn!("event=diagnostics.level_load_failed module=diagnostics");
        }
    }
}

/// 先原子替换配置文件，再更新过滤器；调用方负责串行化修改。
pub fn set_level(app: &tauri::AppHandle, value: Level) -> Result<(), String> {
    let root = app.path().app_config_dir().map_err(|e| e.to_string())?;
    persist_level(&root.join("logging.json"), value)?;
    LEVEL.store(code(value), Ordering::Relaxed);
    Ok(())
}

fn allows(threshold: Level, value: log::Level) -> bool {
    let limit = match threshold {
        Level::Error => log::Level::Error,
        Level::Warn => log::Level::Warn,
        Level::Info => log::Level::Info,
        Level::Debug => log::Level::Debug,
    };
    value <= limit
}

/// 插件上限保持 Debug，原生与前端日志统一按当前阈值过滤。
pub fn enabled(metadata: &log::Metadata<'_>) -> bool {
    allows(level(), metadata.level())
}

/// 高频匹配细节每 500ms 最多一条；只限制 Debug，不丢弃故障与生命周期事件。
pub fn sample_debug() -> bool {
    let now = millis();
    let previous = LAST_SAMPLE.load(Ordering::Relaxed);
    now.saturating_sub(previous) >= 500
        && LAST_SAMPLE
            .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

/// 保留原 panic 行为；仅额外记录行列，不输出 payload、文件路径或环境变量。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let (line, column) = panic
            .location()
            .map(|p| (p.line(), p.column()))
            .unwrap_or((0, 0));
        log::error!("event=app.panic module=app code=RUST_PANIC line={line} column={column}");
        log::logger().flush();
        previous(panic);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_include_severity_and_never_trace() {
        for threshold in [Level::Error, Level::Warn, Level::Info, Level::Debug] {
            assert!(allows(threshold, log::Level::Error));
            assert!(!allows(threshold, log::Level::Trace));
            assert_eq!(
                allows(threshold, log::Level::Debug),
                threshold == Level::Debug
            );
            assert_eq!(
                allows(threshold, log::Level::Info),
                matches!(threshold, Level::Info | Level::Debug)
            );
            assert_eq!(
                allows(threshold, log::Level::Warn),
                threshold != Level::Error
            );
        }
    }
    #[test]
    fn level_survives_reload_and_invalid_values_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logging.json");
        assert_eq!(read_level(&path).unwrap(), Level::Info);
        for level in [Level::Debug, Level::Warn, Level::Error, Level::Info] {
            persist_level(&path, level).unwrap();
            assert_eq!(read_level(&path).unwrap(), level);
        }
        fs::write(&path, b"\"TRACE\"").unwrap();
        assert!(read_level(&path).is_err());
        fs::write(&path, [b' '; 65]).unwrap();
        assert!(read_level(&path).is_err());
    }
}
