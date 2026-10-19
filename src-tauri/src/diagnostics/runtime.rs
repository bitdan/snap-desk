//! 会话内的临时详细日志：用单调时钟过滤，重启自动复位，不依赖设置页轮询来关闭。
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use std::time::Instant;

static CLOCK: OnceLock<Instant> = OnceLock::new();
static DEBUG_UNTIL: AtomicU64 = AtomicU64::new(0);
static LAST_SAMPLE: AtomicU64 = AtomicU64::new(0);

fn millis() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

pub fn remaining_seconds() -> u64 {
    DEBUG_UNTIL
        .load(Ordering::Relaxed)
        .saturating_sub(millis())
        .div_ceil(1000)
}

pub fn set_debug(enabled: bool) {
    DEBUG_UNTIL.store(
        if enabled {
            millis() + 30 * 60 * 1000
        } else {
            0
        },
        Ordering::Relaxed,
    );
}

/// 插件上限设为 Debug，真正级别由此过滤器决定；从不放行 Trace。
pub fn enabled(metadata: &log::Metadata<'_>) -> bool {
    metadata.level() <= log::Level::Info
        || (metadata.level() == log::Level::Debug && remaining_seconds() > 0)
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
    fn debug_expires_without_ui_or_restart() {
        let debug = log::Metadata::builder().level(log::Level::Debug).build();
        let info = log::Metadata::builder().level(log::Level::Info).build();
        let trace = log::Metadata::builder().level(log::Level::Trace).build();
        set_debug(true);
        assert!(remaining_seconds() <= 1800 && remaining_seconds() > 1790);
        assert!(enabled(&debug) && enabled(&info) && !enabled(&trace));
        DEBUG_UNTIL.store(millis().saturating_sub(1), Ordering::Relaxed);
        assert_eq!(remaining_seconds(), 0);
        assert!(!enabled(&debug) && enabled(&info));
        set_debug(false);
        assert_eq!(remaining_seconds(), 0);
    }
}
