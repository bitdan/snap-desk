//! 退出协调的共用部分：记录真实处理进展，并按阶段判断等待是否超时。
//!
//! 退出时外壳先请求停止录屏，再等待流水线封装、发布成品；只有长时间没有进展（帧提交、阶段切换、
//! 文件增长）或总等待超过硬上限才强制退出。不能用定时心跳伪装编码器仍有进展。
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// 最近一次真实进展。
pub struct Progress {
    pub stage: &'static str,
    pub updated: Instant,
    pub entered: Instant,
    /// 正在写入的文件；退出等待期间文件增长也算进展。
    pub path: Option<PathBuf>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            stage: "idle",
            updated: Instant::now(),
            entered: Instant::now(),
            path: None,
        }
    }
}

impl Progress {
    /// 流水线报告进展；阶段变化时记录上一阶段耗时。
    pub fn record(&mut self, stage: &'static str, path: Option<&Path>) {
        if self.stage != stage {
            log::info!(
                "event=recording.stage_changed module=recording previous={} stage={stage} duration_ms={:.0}",
                self.stage,
                self.entered.elapsed().as_secs_f64() * 1000.0
            );
            self.entered = Instant::now();
        }
        self.stage = stage;
        self.updated = Instant::now();
        self.path = path.map(Path::to_path_buf);
    }

    /// 观察到 `path` 的文件变大：仍是当前写入文件时视为进展。
    pub fn file_grew(&mut self, path: &Path) {
        if self.path.as_deref() == Some(path) {
            self.updated = Instant::now();
        }
    }
}

/// 比较两次观察到的文件大小；同一文件变大返回 true。`observed` 更新为本次观察。
pub fn grew(observed: &mut Option<(PathBuf, u64)>, path: &Path, size: u64) -> bool {
    let grew = observed
        .as_ref()
        .is_some_and(|(old, old_size)| old == path && size > *old_size);
    *observed = Some((path.to_path_buf(), size));
    grew
}

/// Finalize 属于不可细分的原生调用，给予更长无进展窗口；总等待仍有硬上限。
pub fn exit_timed_out(stage: &str, idle: Duration, total: Duration) -> bool {
    let limit = if stage == "finalizing" { 180 } else { 90 };
    idle >= Duration::from_secs(limit) || total >= Duration::from_secs(600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_timeout_distinguishes_finalizing_progress_and_hard_deadline() {
        let s = Duration::from_secs;
        assert!(!exit_timed_out("finalizing", s(100), s(100)));
        assert!(exit_timed_out("finalizing", s(180), s(180)));
        assert!(exit_timed_out("syncing", s(90), s(100)));
        assert!(!exit_timed_out("syncing", s(1), s(599)));
        assert!(exit_timed_out("syncing", s(1), s(600)));
    }

    #[test]
    fn only_growth_of_the_current_file_counts_as_progress() {
        let (a, b) = (Path::new("a.mp4"), Path::new("b.mp4"));
        let mut observed = None;
        assert!(!grew(&mut observed, a, 10));
        assert!(grew(&mut observed, a, 20));
        assert!(!grew(&mut observed, a, 20));
        assert!(!grew(&mut observed, b, 30));
        let mut progress = Progress::default();
        progress.record("encoding", Some(a));
        let before = progress.updated;
        std::thread::sleep(Duration::from_millis(5));
        progress.file_grew(b);
        assert_eq!(progress.updated, before);
        progress.file_grew(a);
        assert!(progress.updated > before);
        assert_eq!(progress.stage, "encoding");
    }
}
