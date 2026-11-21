//! 运行时日志阈值：本机持久化、立即切换（滚动截图的高频调试采样在 core `scroll`）。
use snap_core::diagnostics::{
    level::{allows, code, persist as persist_level, read as read_level},
    logs::Level,
};
use std::sync::atomic::{AtomicU8, Ordering};
use tauri::Manager;

static LEVEL: AtomicU8 = AtomicU8::new(2);

/// 返回当前实际生效阈值，供状态页与诊断包使用。
pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Error,
        1 => Level::Warn,
        3 => Level::Debug,
        _ => Level::Info,
    }
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

/// 插件上限保持 Debug，原生与前端日志统一按当前阈值过滤。
pub fn enabled(metadata: &log::Metadata<'_>) -> bool {
    allows(level(), metadata.level())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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
