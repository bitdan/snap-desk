//! 日志故障隔离：控制台、磁盘或第三方日志器失败不能让用户操作退出进程。
use std::{
    cell::Cell,
    fs::OpenOptions,
    io::Write,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::AppHandle;
use tauri_plugin_log::{Target, TargetKind};

thread_local! { static IN_LOGGER: Cell<bool> = const { Cell::new(false) }; }

/// 第三方 sink 一旦 panic 就停用；不在失败路径递归记录，也不继续使用可能中毒的内部锁。
struct IsolatedLogger {
    inner: Box<dyn log::Log>,
    failed: AtomicBool,
}

impl IsolatedLogger {
    fn attempt<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        if self.failed.load(Ordering::Relaxed) {
            return None;
        }
        IN_LOGGER
            .try_with(|active| {
                if active.replace(true) {
                    return None;
                }
                let result = catch_unwind(AssertUnwindSafe(operation));
                active.set(false);
                match result {
                    Ok(value) => Some(value),
                    Err(_) => {
                        self.failed.store(true, Ordering::Relaxed);
                        None
                    }
                }
            })
            .ok()
            .flatten()
    }
}

impl log::Log for IsolatedLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.attempt(|| self.inner.enabled(metadata))
            .unwrap_or(false)
    }
    fn log(&self, record: &log::Record<'_>) {
        self.attempt(|| self.inner.log(record));
    }
    fn flush(&self) {
        self.attempt(|| self.inner.flush());
    }
}

/// 在业务初始化前安装；日志初始化失败时仍保留截图、贴图等业务功能。
pub(crate) fn initialize(app: &AppHandle) {
    let panic_path = super::location(app)
        .ok()
        .map(|(root, base)| root.join(format!("{base}.log")));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        // hook 内绝不调用 log。日志器自身 panic 时也不写它正在使用的文件或控制台。
        if IN_LOGGER.try_with(Cell::get).unwrap_or(true) {
            return;
        }
        if let Some(path) = &panic_path {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let (line, column) = panic
                    .location()
                    .map(|p| (p.line(), p.column()))
                    .unwrap_or((0, 0));
                let _ = writeln!(file, "{} [ERROR] [snap_desk_lib::diagnostics] event=app.panic module=app code=RUST_PANIC line={line} column={column}",
                    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"));
            }
        }
        if cfg!(debug_assertions) {
            previous(panic);
        }
    }));

    // Windows 独立版没有控制台。fern 的 stdout/stderr 双重失败会 panic，不能使用默认目标。
    let mut targets = vec![Target::new(TargetKind::LogDir { file_name: None })];
    if cfg!(debug_assertions) {
        targets.push(Target::new(TargetKind::Stdout));
    }
    let configured = tauri_plugin_log::Builder::new()
        .targets(targets)
        .level(log::LevelFilter::Debug)
        .filter(super::runtime::enabled)
        .max_file_size(5_000_000)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
        .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
        .filter(|metadata| {
            metadata.target().starts_with("snap_desk_lib")
                || metadata.target() == tauri_plugin_log::WEBVIEW_TARGET
                || metadata.target().starts_with("webview::")
        })
        .format(|out, message, record| {
            let target = if record.target().starts_with("webview") {
                "frontend"
            } else {
                record.target()
            };
            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
                record.level(),
                target,
                message
            ));
        })
        .split(app);
    if let Ok((_, level, inner)) = configured {
        let _ = tauri_plugin_log::attach_logger(
            level,
            Box::new(IsolatedLogger {
                inner,
                failed: AtomicBool::new(false),
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;
    use std::sync::{atomic::AtomicUsize, Arc};

    struct BrokenSink(Arc<AtomicUsize>);
    impl Log for BrokenSink {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, _: &log::Record<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("simulated sink failure");
        }
        fn flush(&self) {
            panic!("simulated flush failure");
        }
    }

    #[test]
    fn broken_sink_is_disabled_without_failing_the_operation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let logger = IsolatedLogger {
            inner: Box::new(BrokenSink(calls.clone())),
            failed: AtomicBool::new(false),
        };
        let record = log::Record::builder()
            .args(format_args!("event=app.operation_failed"))
            .level(log::Level::Warn)
            .build();
        logger.log(&record);
        logger.log(&record);
        logger.flush();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(!logger.enabled(record.metadata()));
        assert!(!IN_LOGGER.with(Cell::get));
    }

    #[test]
    fn flush_failure_is_also_isolated() {
        let logger = IsolatedLogger {
            inner: Box::new(BrokenSink(Arc::new(AtomicUsize::new(0)))),
            failed: AtomicBool::new(false),
        };
        logger.flush();
        assert!(logger.failed.load(Ordering::Relaxed));
        assert!(!IN_LOGGER.with(Cell::get));
    }
}
