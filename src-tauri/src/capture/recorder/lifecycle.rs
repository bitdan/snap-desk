//! 录屏退出协调：任务守卫、真实处理进度和超时兜底。
use super::*;

/// 退出意图独立于录屏阶段：Completed 可能早于窗口和资源清理。
#[derive(Default)]
pub struct ExitState {
    pub(super) requested: AtomicBool,
    pub(super) workers: std::sync::atomic::AtomicUsize,
    progress: Mutex<Progress>,
}
impl ExitState {
    /// 最后一个任务释放时才允许正常退出，不能依赖可能提前更新的视频状态。
    pub(super) fn release_worker(&self) -> bool {
        self.workers.fetch_sub(1, Ordering::SeqCst) == 1 && self.requested.load(Ordering::SeqCst)
    }
}

/// 准备任务与采集任务均持有守卫；异常返回或 panic 也会释放计数。
pub(super) struct WorkerGuard(AppHandle);
impl WorkerGuard {
    pub(super) fn acquire(app: &AppHandle) -> Result<Self, String> {
        let state = app.state::<ExitState>();
        state.workers.fetch_add(1, Ordering::SeqCst);
        let guard = Self(app.clone());
        if state.requested.load(Ordering::SeqCst) {
            return Err("程序正在退出，不能启动录屏。".into());
        }
        Ok(guard)
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let state = self.0.state::<ExitState>();
        if state.release_worker() {
            self.0.exit(0);
        }
    }
}

/// 记录真实处理进展，不能用定时心跳伪装编码器仍有进展。
struct Progress {
    stage: &'static str,
    updated: Instant,
    entered: Instant,
    path: Option<PathBuf>,
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
pub(super) fn progress(app: &AppHandle, stage: &'static str, path: Option<&std::path::Path>) {
    if let Ok(mut p) = app.state::<ExitState>().progress.lock() {
        if p.stage != stage {
            log::info!(
                "event=recording.stage_changed module=recording previous={} stage={stage} duration_ms={:.0}",
                p.stage,
                p.entered.elapsed().as_secs_f64() * 1000.0
            );
            p.entered = Instant::now();
        }
        p.stage = stage;
        p.updated = Instant::now();
        p.path = path.map(std::path::Path::to_path_buf);
    }
}
/// Finalize 属于不可细分的原生调用，给予更长无进展窗口；总等待仍有硬上限。
pub(super) fn exit_timed_out(stage: &str, idle: Duration, total: Duration) -> bool {
    let limit = if stage == "finalizing" { 180 } else { 90 };
    idle >= Duration::from_secs(limit) || total >= Duration::from_secs(600)
}

/// 退出按真实帧提交、阶段切换及文件增长延长等待，保留超时源文件与恢复记录。
pub fn request_exit(app: &AppHandle) {
    let state = app.state::<ExitState>();
    if state.requested.swap(true, Ordering::SeqCst) {
        return;
    }
    if state.workers.load(Ordering::SeqCst) == 0 {
        app.exit(0);
        return;
    }
    let watchdog = app.clone();
    // 文件系统调用可能被掉线磁盘卡住，增长观察独立于超时裁决线程。
    let observer = app.clone();
    std::thread::spawn(move || {
        let mut observed: Option<(PathBuf, u64)> = None;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = observer.state::<ExitState>();
            if state.workers.load(Ordering::SeqCst) == 0 {
                return;
            }
            let path = state.progress.try_lock().ok().and_then(|p| p.path.clone());
            let Some(path) = path else {
                continue;
            };
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let size = metadata.len();
            if observed
                .as_ref()
                .is_some_and(|(old_path, old_size)| old_path == &path && size > *old_size)
            {
                if let Ok(mut p) = state.progress.try_lock() {
                    if p.path.as_ref() == Some(&path) {
                        p.updated = Instant::now();
                    }
                }
            }
            observed = Some((path, size));
        }
    });
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut last_progress = started;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = watchdog.state::<ExitState>();
            if state.workers.load(Ordering::SeqCst) == 0 {
                return;
            }
            let (stage, updated) = match state.progress.try_lock() {
                Ok(p) => (p.stage, p.updated),
                Err(_) => ("state-unavailable", started),
            };
            last_progress = last_progress.max(updated);
            if exit_timed_out(stage, last_progress.elapsed(), started.elapsed()) {
                log::error!("退出超时：阶段={stage}，无进展={:.1}s，总等待={:.1}s；源视频与恢复记录保留，下次启动可检查。", last_progress.elapsed().as_secs_f64(), started.elapsed().as_secs_f64());
                watchdog.exit(1);
                return;
            }
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        request_stop(&app, false);
    });
}
