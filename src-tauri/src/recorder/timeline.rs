//! 使用 WGC 同源的 QPC 时间（100 ns）计算媒体时间；暂停切换使旧帧失效。
#[derive(Default)]
pub struct Timeline {
    elapsed: i64,
    since: Option<i64>,
    pub generation: u64,
}

/// 按媒体时钟分桶采样，避免用“距上一帧 >= 间隔”把略有抖动的 60 Hz 误降为 30 FPS。
pub fn accept_sample(last: &mut Option<(u64, i64)>, generation: u64, pts: i64, fps: u32) -> bool {
    let slot = pts * i64::from(fps) / 10_000_000;
    if last.is_some_and(|(g, previous)| g == generation && slot <= previous) {
        return false;
    }
    *last = Some((generation, slot));
    true
}

impl Timeline {
    pub fn resume(&mut self, now: i64) {
        if self.since.is_none() {
            self.generation += 1;
            self.since = Some(now);
        }
    }

    pub fn pause(&mut self, now: i64) {
        if self.since.is_some() {
            self.elapsed = self.elapsed(now);
            self.since = None;
            self.generation += 1;
        }
    }

    /// 停止保留当前代次，允许排空停止前已入队的有效帧。
    pub fn stop(&mut self, now: i64) {
        self.elapsed = self.elapsed(now);
        self.since = None;
    }

    pub fn elapsed(&self, now: i64) -> i64 {
        self.elapsed
            + self
                .since
                .map(|t| now.saturating_sub(t).max(0))
                .unwrap_or(0)
    }

    pub fn project(&self, source: i64) -> Option<(u64, i64)> {
        let since = self.since?;
        (source >= since).then_some((self.generation, self.elapsed + source - since))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_tolerates_refresh_jitter_and_resets_after_pause() {
        let mut last = None;
        let accepted = (0..100)
            .filter(|i| accept_sample(&mut last, 1, 10_000 + i * 166_600, 60))
            .count();
        assert!(accepted >= 99);
        assert!(!accept_sample(&mut last, 1, 10_000, 60));
        assert!(accept_sample(&mut last, 3, 10_000, 60));
    }

    #[test]
    fn excludes_ready_pause_and_stale_frames() {
        let mut clock = Timeline::default();
        assert_eq!(clock.project(100), None);
        clock.resume(100);
        assert_eq!(clock.project(99), None);
        assert_eq!(clock.project(130), Some((1, 30)));
        clock.pause(150);
        assert_eq!(clock.elapsed(900), 50);
        assert_eq!(clock.project(160), None);
        clock.resume(900);
        assert_eq!(clock.project(899), None);
        assert_eq!(clock.project(920), Some((3, 70)));
        clock.stop(950);
        assert_eq!(clock.generation, 3);
        assert_eq!(clock.elapsed(2000), 100);
    }

    #[test]
    fn repeated_transitions_do_not_reset_time() {
        let mut clock = Timeline::default();
        clock.resume(10);
        clock.resume(20);
        clock.pause(30);
        clock.pause(40);
        clock.stop(50);
        assert_eq!(clock.elapsed(100), 20);
        assert_eq!(clock.generation, 2);
    }
}
