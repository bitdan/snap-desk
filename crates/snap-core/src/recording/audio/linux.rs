//! Linux 声音采集：两路各一条 GStreamer 管线 pulsesrc → audioconvert → audioresample → appsink，
//! 系统声音读默认输出设备的监听源（`@DEFAULT_MONITOR@`），麦克风读默认输入设备；
//! PulseAudio 与 PipeWire（pipewire-pulse）都提供这两个名称。
//!
//! 管线强制使用 GStreamer 系统时钟（CLOCK_MONOTONIC，与 `capture::now` 同源），
//! 样本时间 = base_time + 缓冲时间戳，交给与其他平台共用的混音器按视频时间轴对齐。
//! 控制线程按录制状态门控：首次进入录制阶段才开始采集，倒计时与暂停期间丢弃数据。
use super::{AudioFrame, Source, CHANNELS, RATE};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// 麦克风连续 3 秒没有数据视为设备断开，与其他平台一致（100 ns）。
const MICROPHONE_SILENCE: i64 = 30_000_000;

fn now() -> i64 {
    super::super::capture::now()
}

/// 交错 f32 小端字节转为样本；不足一个样本的尾部丢弃。
fn samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes([value[0], value[1], value[2], value[3]]))
        .collect()
}

/// 回调与控制线程共享的状态。
struct Shared {
    sender: SyncSender<AudioFrame>,
    running: AtomicBool,
    failure: Mutex<Option<String>>,
    microphone_at: AtomicI64,
}

impl Shared {
    fn fail(&self, error: String) {
        if let Ok(mut failure) = self.failure.lock() {
            failure.get_or_insert(error);
        }
    }
}

struct Capture {
    pipeline: gst::Pipeline,
    source: Source,
}

impl Capture {
    fn new(source: Source, shared: Arc<Shared>) -> Result<Self, String> {
        let pulse = gst::ElementFactory::make("pulsesrc")
            .property("provide-clock", false)
            .build()
            .map_err(|_| "缺少 GStreamer 插件 pulsesrc，请安装 gstreamer1.0-pulseaudio。".to_string())?;
        if matches!(source, Source::System) {
            pulse.set_property("device", "@DEFAULT_MONITOR@");
        }
        let element = |factory: &str| {
            gst::ElementFactory::make(factory)
                .build()
                .map_err(|_| format!("缺少 GStreamer 插件 {factory}，请安装 gstreamer1.0-plugins-base。"))
        };
        let caps = gst::Caps::builder("audio/x-raw")
            .field("format", "F32LE")
            .field("layout", "interleaved")
            .field("rate", RATE as i32)
            .field("channels", CHANNELS as i32)
            .build();
        let sink = gst_app::AppSink::builder().caps(&caps).sync(false).build();
        let pipeline = gst::Pipeline::builder().build();
        let chain = [
            pulse,
            element("audioconvert")?,
            element("audioresample")?,
            sink.clone().upcast(),
        ];
        pipeline.add_many(chain.iter()).map_err(|e| e.to_string())?;
        gst::Element::link_many(chain.iter()).map_err(|e| format!("无法连接音频采集管线：{e}"))?;
        // 系统时钟即 CLOCK_MONOTONIC；不让 pulsesrc 提供自己的时钟，样本时间才能与视频对齐。
        pipeline.use_clock(Some(&gst::SystemClock::obtain()));
        // 回调由 appsink 持有，只保留弱引用，避免管线与回调互相持有而无法释放。
        let element = pipeline.downgrade();
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if !shared.running.load(Ordering::Acquire) {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    let Some(buffer) = sample.buffer() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let base = element.upgrade().and_then(|pipeline| pipeline.base_time());
                    let (Some(base), Some(pts)) = (base, buffer.pts()) else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let Ok(map) = buffer.map_readable() else {
                        shared.fail(format!("{}：无法读取音频数据。", source.label()));
                        return Err(gst::FlowError::Error);
                    };
                    let samples = samples(map.as_slice());
                    drop(map);
                    if samples.is_empty() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    let source_time = ((base + pts).nseconds() / 100) as i64;
                    if matches!(source, Source::Microphone) {
                        shared.microphone_at.store(now(), Ordering::Release);
                    }
                    let frame = AudioFrame {
                        source,
                        source_time,
                        samples,
                    };
                    // 队列满说明编码线程长时间未取走数据，明确失败而不是悄悄丢掉一段声音。
                    if shared.sender.try_send(frame).is_err() {
                        shared.fail("音频队列已满，请降低录屏负载后重试。".into());
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        // READY 只连接声音服务器、确认设备存在，不打开录音流，倒计时期间不点亮麦克风指示。
        pipeline
            .set_state(gst::State::Ready)
            .map_err(|_| format!("{}：无法连接 PulseAudio / PipeWire 声音服务。", source.label()))?;
        let capture = Self { pipeline, source };
        capture.check()?;
        Ok(capture)
    }

    fn start(&self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| format!("{}：无法开始采集。", self.source.label()))?;
        self.check()
    }

    fn check(&self) -> Result<(), String> {
        let Some(message) = self
            .pipeline
            .bus()
            .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
        else {
            return Ok(());
        };
        match message.view() {
            gst::MessageView::Error(error) => {
                Err(format!("{}：{}", self.source.label(), error.error()))
            }
            _ => Ok(()),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// 有界音频队列与控制线程的所有权；采集错误通过 JoinHandle 传回。
pub struct Worker {
    receiver: Receiver<AudioFrame>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl Worker {
    pub fn start(
        system: bool,
        microphone: bool,
        active: impl Fn() -> bool + Send + 'static,
    ) -> Result<Self, String> {
        gst::init().map_err(|e| format!("无法初始化 GStreamer：{e}"))?;
        let (sender, receiver) = mpsc::sync_channel(256);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let shared = Arc::new(Shared {
            sender,
            running: AtomicBool::new(false),
            failure: Mutex::new(None),
            microphone_at: AtomicI64::new(0),
        });
        let thread = thread::Builder::new()
            .name("recording-audio".into())
            .spawn(move || {
                let prepared = (|| {
                    let mut captures = Vec::new();
                    if system {
                        captures.push(Capture::new(Source::System, shared.clone())?);
                    }
                    if microphone {
                        captures.push(Capture::new(Source::Microphone, shared.clone())?);
                    }
                    Ok::<_, String>(captures)
                })();
                let captures = match prepared {
                    Ok(captures) => {
                        let _ = ready_tx.send(Ok(()));
                        captures
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                let result = (|| {
                    let mut started = false;
                    loop {
                        let stopped = stopping.load(Ordering::Acquire);
                        let recording = !stopped && active();
                        // 首次进入录制阶段才开始采集。
                        if recording && !started {
                            for capture in &captures {
                                capture.start()?;
                            }
                            shared.microphone_at.store(now(), Ordering::Release);
                            started = true;
                        }
                        shared.running.store(recording, Ordering::Release);
                        if let Some(error) = shared.failure.lock().ok().and_then(|mut f| f.take()) {
                            return Err(error);
                        }
                        for capture in &captures {
                            capture.check()?;
                        }
                        if microphone
                            && started
                            && !stopped
                            && now() - shared.microphone_at.load(Ordering::Acquire)
                                > MICROPHONE_SILENCE
                        {
                            return Err("麦克风超过 3 秒没有数据，请检查设备连接。".into());
                        }
                        if stopped {
                            return Ok(());
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                })();
                shared.running.store(false, Ordering::Release);
                drop(captures);
                result
            })
            .map_err(|e| e.to_string())?;
        let mut worker = Self {
            receiver,
            stop,
            thread: Some(thread),
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(worker),
            Ok(Err(error)) => {
                let _ = worker.stop();
                Err(error)
            }
            Err(error) => {
                let _ = worker.stop();
                Err(error.to_string())
            }
        }
    }

    pub fn drain(&mut self) -> Result<Vec<AudioFrame>, String> {
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            self.stop()?;
            return Err("音频采集线程提前停止，请检查声音服务及设备连接。".into());
        }
        Ok(self.receiver.try_iter().collect())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| "音频采集线程异常退出。".to_string())
                .and_then(|r| r),
            None => Ok(()),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn little_endian_floats_decode_and_drop_partial_tails() {
        let mut bytes = Vec::new();
        for value in [0.5f32, -1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(7);
        assert_eq!(samples(&bytes), vec![0.5, -1.0]);
    }
}
