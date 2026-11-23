//! 常驻资源的空闲计时：隐藏或用完时开始计时，再次使用时清除；到期由回收循环释放。
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 暂存架、剪贴板窗口隐藏超过该时长后销毁 WebView，释放其渲染进程。
pub(crate) const WINDOW_IDLE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct IdleSince(Mutex<Option<Instant>>);

impl IdleSince {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub(crate) fn start(&self) {
        if let Ok(mut since) = self.0.lock() {
            *since = Some(Instant::now());
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut since) = self.0.lock() {
            *since = None;
        }
    }

    /// 到期时清除计时并返回 true；未计时或未到期返回 false。
    pub(crate) fn take_expired(&self, idle: Duration) -> bool {
        let Ok(mut since) = self.0.lock() else {
            return false;
        };
        if since.is_some_and(|at| at.elapsed() >= idle) {
            *since = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_once_and_clear_cancels() {
        let idle = IdleSince::new();
        assert!(!idle.take_expired(Duration::ZERO));
        idle.start();
        assert!(!idle.take_expired(Duration::from_secs(3600)));
        assert!(idle.take_expired(Duration::ZERO));
        assert!(!idle.take_expired(Duration::ZERO));
        idle.start();
        idle.clear();
        assert!(!idle.take_expired(Duration::ZERO));
    }
}
