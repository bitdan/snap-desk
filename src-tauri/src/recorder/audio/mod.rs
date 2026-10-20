//! 音频契约、QPC 对齐混音和独立采集线程。
//! WASAPI 的共享模式负责设备格式 → 48 kHz 双声道浮点的高质量重采样；
//! 此模块按视频时间轴裁剪暂停，补静音，再向 AAC 编码器提交 PCM16。
use super::timeline::Timeline;
use std::collections::VecDeque;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::Worker;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
pub const TICKS: i64 = 10_000_000;
/// 给两路设备和线程调度留 200 ms 对齐窗口；不改变文件内媒体时间。
pub const MIX_DELAY: i64 = 2_000_000;

#[derive(Clone, Copy, Debug)]
pub enum Source {
    System = 0,
    Microphone = 1,
}

/// 采集线程复制并拥有的数据，不携带 COM 指针或借用设备缓冲。
/// samples 为交错的 L/R f32，固定 RATE Hz；source_time 为首个样本的 QPC（100 ns）。
/// 静音包也携带零样本，设备失效和无效时间戳通过 Result 显式报错。
pub struct AudioFrame {
    pub source: Source,
    pub source_time: i64,
    pub samples: Vec<f32>,
}

/// AAC 输入固定为 PCM16 双声道；时间由整数样本序号计算，避免累计舍入漂移。
pub struct AudioPacket {
    pub pts: i64,
    pub duration: i64,
    pub samples: Vec<i16>,
}

pub fn sample_time(index: i64) -> i64 {
    (i128::from(index) * i128::from(TICKS) / i128::from(RATE)) as i64
}

#[derive(Default)]
pub struct Mixer {
    /// 下一待输出样本序号；队列第 0 个元素始终对应它。
    cursor: i64,
    pending: VecDeque<[f32; CHANNELS]>,
    last_source: [Option<i64>; 2],
}

impl Mixer {
    pub fn ingest(&mut self, frame: AudioFrame, timeline: &Timeline) -> Result<(), String> {
        if frame.samples.len() % CHANNELS != 0 || frame.samples.len() > RATE as usize * CHANNELS {
            return Err("音频包长度不符合双声道契约。".into());
        }
        // 先检查整包时间范围，避免异常驱动时间戳在逐样本加法中溢出，
        // 也避免处理到半包才报错、留下已写入混音队列的部分数据。
        if frame.source_time < 0
            || frame
                .source_time
                .checked_add(sample_time((frame.samples.len() / CHANNELS) as i64))
                .is_none()
        {
            return Err("音频包 QPC 时间范围无效。".into());
        }
        for (offset, pair) in frame.samples.chunks_exact(CHANNELS).enumerate() {
            let Some(pts) = timeline.project_audio(frame.source_time + sample_time(offset as i64))
            else {
                continue;
            };
            // QPC 单位不能整除 48 kHz；四舍五入避免相邻包在边缘重复/缺失一个样本。
            let index = ((i128::from(pts) * i128::from(RATE) + i128::from(TICKS / 2))
                / i128::from(TICKS)) as i64;
            let previous = &mut self.last_source[frame.source as usize];
            if previous.is_some_and(|last| index <= last) {
                continue;
            }
            if index < self.cursor {
                // 超过混音等待窗口的设备/线程延迟不能静默丢弃并伪装录制成功。
                return Err("音频采集或编码延迟超过同步窗口，请降低录屏负载后重试。".into());
            }
            let offset = (index - self.cursor) as usize;
            if offset >= RATE as usize * 2 {
                return Err("音频时间戳跳变超过同步缓冲范围。".into());
            }
            self.pending
                .resize(self.pending.len().max(offset + 1), [0.0; CHANNELS]);
            for (output, input) in self.pending[offset].iter_mut().zip(pair) {
                // 驱动异常 NaN/Inf 不能传入编码器；每路先限制范围，混合后再限幅。
                *output += if input.is_finite() {
                    input.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
            *previous = Some(index);
        }
        Ok(())
    }

    /// 一次最多输出 10 ms；无系统播放时 loopback 可能不发包，此处显式补零。
    /// end 为允许写入的媒体终点，finish 使用真实停止时间，普通轮询使用延迟水位。
    pub fn next_packet(&mut self, end: i64) -> Option<AudioPacket> {
        let limit = (i128::from(end.max(0)) * i128::from(RATE) / i128::from(TICKS)) as i64;
        let count = (limit - self.cursor).clamp(0, 480) as usize;
        if count == 0 {
            return None;
        }
        let pts = sample_time(self.cursor);
        let mut samples = Vec::with_capacity(count * CHANNELS);
        for _ in 0..count {
            for value in self.pending.pop_front().unwrap_or([0.0; CHANNELS]) {
                samples.push((value.clamp(-1.0, 1.0) * 32767.0).round() as i16);
            }
        }
        self.cursor += count as i64;
        Some(AudioPacket {
            pts,
            duration: sample_time(self.cursor) - pts,
            samples,
        })
    }
}

#[cfg(not(target_os = "windows"))]
pub struct Worker;
#[cfg(not(target_os = "windows"))]
impl Worker {
    pub fn start(_: bool, _: bool, _: impl Fn() -> bool + Send + 'static) -> Result<Self, String> {
        Err("当前平台尚未支持音频录制。".into())
    }
    pub fn drain(&self) -> Result<Vec<AudioFrame>, String> {
        Ok(Vec::new())
    }
    pub fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(source: Source, time: i64, count: usize, value: f32) -> AudioFrame {
        AudioFrame {
            source,
            source_time: time,
            samples: vec![value; count * CHANNELS],
        }
    }

    #[test]
    fn mixes_sources_clips_and_fills_silence() {
        let mut timeline = Timeline::default();
        timeline.resume(1000);
        let mut mixer = Mixer::default();
        mixer
            .ingest(frame(Source::System, 1000, 240, 0.75), &timeline)
            .unwrap();
        mixer
            .ingest(frame(Source::Microphone, 1000, 240, 0.75), &timeline)
            .unwrap();
        let packet = mixer.next_packet(100_000).unwrap();
        assert_eq!(
            (packet.pts, packet.duration, packet.samples.len()),
            (0, 100_000, 960)
        );
        assert!(packet.samples[..480].iter().all(|v| *v == 32767));
        assert!(packet.samples[480..].iter().all(|v| *v == 0));
    }

    #[test]
    fn delayed_packets_are_clipped_at_pause_resume_and_stop() {
        let mut timeline = Timeline::default();
        timeline.resume(100_000);
        timeline.pause(200_000);
        timeline.resume(400_000);
        timeline.stop(500_000);
        timeline.stop(900_000);
        let mut mixer = Mixer::default();
        // 整包覆盖开始前、暂停、恢复和停止后；只保留两段各 10 ms。
        mixer
            .ingest(frame(Source::System, 0, 2880, 0.5), &timeline)
            .unwrap();
        for pts in [0, 100_000] {
            let packet = mixer.next_packet(200_000).unwrap();
            assert_eq!(packet.pts, pts);
            assert!(packet.samples.iter().all(|v| *v == 16384));
        }
        assert!(mixer.next_packet(200_000).is_none());
        assert!(mixer.pending.is_empty());
    }

    #[test]
    fn repeated_packets_do_not_double_volume_and_late_data_fails() {
        let mut timeline = Timeline::default();
        timeline.resume(0);
        let mut mixer = Mixer::default();
        for _ in 0..2 {
            mixer
                .ingest(frame(Source::System, 0, 480, 0.25), &timeline)
                .unwrap();
        }
        assert!(mixer
            .next_packet(100_000)
            .unwrap()
            .samples
            .iter()
            .all(|v| *v == 8192));
        assert!(mixer
            .ingest(frame(Source::Microphone, 0, 480, 1.0), &timeline)
            .is_err());
    }

    #[test]
    fn integer_sample_clock_has_no_packet_rounding_drift() {
        let mut mixer = Mixer::default();
        let end = 12_345_678;
        let mut duration = 0;
        while let Some(packet) = mixer.next_packet(end) {
            assert_eq!(packet.pts, duration);
            duration += packet.duration;
        }
        assert!(end - duration < sample_time(1) + 1);
        assert_eq!(sample_time(i64::from(RATE) * 3600), TICKS * 3600);
    }

    #[test]
    fn invalid_qpc_range_is_rejected_before_mixing() {
        let mut timeline = Timeline::default();
        timeline.resume(0);
        for time in [-1, i64::MAX - 100] {
            let mut mixer = Mixer::default();
            assert!(mixer
                .ingest(frame(Source::System, time, 480, 0.5), &timeline)
                .is_err());
            assert!(mixer.pending.is_empty());
            assert_eq!(mixer.last_source, [None, None]);
        }
    }
}
