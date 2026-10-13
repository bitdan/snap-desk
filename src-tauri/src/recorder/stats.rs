//! 录屏诊断计数与滚动 FPS；仅保存有界时间戳，不保留图像或输入事件。
use serde::Serialize;
use std::collections::VecDeque;

/// 累计统计不含准备阶段；采集次数包含空取样，节流次数不是编码丢帧。
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStats {
    pub capture_polls: usize,
    pub captured_frames: usize,
    /// next_frame 的总耗时，含 GPU 回读和 CPU 拷贝，不含帧事件等待。
    pub capture_millis: f64,
    /// 有画面或动画需要提交，但被目标帧率拒绝的次数。
    pub throttled_samples: usize,
    pub composed_frames: usize,
    pub composition_millis: f64,
    pub queued_frames: usize,
    pub queue_dropped_frames: usize,
    /// 编码线程拒绝的旧代次排队帧；与队列满丢弃共同组成旧 droppedFrames。
    pub stale_queued_frames: usize,
    /// 采集后因暂停/停止/过期时间戳未进入处理的源帧。
    pub rejected_capture_frames: usize,
    /// 已完成合成，但提交前遇到暂停/停止的输出帧。
    pub rejected_composed_frames: usize,
    pub recent_fps: f64,
}

/// 近两秒成功编码提交帧的媒体时间戳；暂停后恢复清空窗口，累计计数不清空。
/// 当前最高 60 FPS，窗口通常最多 121 帧；额外硬上限避免未来扩展导致无界增长。
#[derive(Default)]
pub struct FpsWindow {
    start: i64,
    samples: VecDeque<i64>,
}
impl FpsWindow {
    pub fn reset(&mut self, media_time: i64) {
        self.start = media_time;
        self.samples.clear();
    }

    pub fn submitted(&mut self, pts: i64) {
        self.samples.retain(|time| *time > pts - 20_000_000);
        if self.samples.len() == 256 {
            self.samples.pop_front();
        }
        self.samples.push_back(pts);
    }

    /// 查询时也裁除静态画面留下的旧样本；启动前 100ms 使用最小分母避免瞬间尖峰。
    pub fn fps(&self, now: i64) -> f64 {
        let span = (now - self.start).clamp(1_000_000, 20_000_000);
        let count = self
            .samples
            .iter()
            .filter(|time| **time > now - 20_000_000 && **time <= now)
            .count();
        count as f64 * 10_000_000.0 / span as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_expires_without_new_frames_and_resets_on_resume() {
        let mut window = FpsWindow::default();
        assert_eq!(window.fps(0), 0.0);
        for i in 1..=60 {
            window.submitted(i * 10_000_000 / 30);
        }
        assert_eq!(window.fps(20_000_000), 30.0);
        assert_eq!(window.fps(30_000_000), 15.0);
        assert_eq!(window.fps(40_000_000), 0.0);
        window.reset(40_000_000);
        assert_eq!(window.fps(40_000_000), 0.0);
        window.submitted(40_000_000);
        assert_eq!(window.fps(40_000_000), 10.0);
        assert_eq!(window.fps(50_000_000), 1.0);
    }

    #[test]
    fn samples_remain_bounded_even_with_identical_timestamps() {
        let mut window = FpsWindow::default();
        for _ in 0..1000 {
            window.submitted(0);
        }
        assert_eq!(window.samples.len(), 256);
        window.submitted(20_000_000);
        assert_eq!(window.samples.len(), 1);
    }
}
