//! WASAPI 共享模式：默认播放设备 loopback 和默认麦克风各一个 client。
//! 所有 COM 对象只属于音频线程；Drop 请求退出并 join，避免麦克风在取消后继续占用。
use super::{AudioFrame, Source, CHANNELS, RATE};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{Media::Audio::*, System::Com::*};

fn error(e: windows::core::Error) -> String {
    format!("WASAPI 音频录制失败：{e}。请检查默认音频设备、麦克风权限或设备连接，然后重新录制。")
}

struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
    fn new() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(error)?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

/// 先释放采集服务和 client，最后才退出 COM apartment。
struct Device {
    capture: IAudioCaptureClient,
    client: IAudioClient,
    source: Source,
    running: bool,
}
impl Device {
    fn new(enumerator: &IMMDeviceEnumerator, source: Source) -> Result<Self, String> {
        unsafe {
            let flow = match source {
                Source::System => eRender,
                Source::Microphone => eCapture,
            };
            let endpoint = enumerator
                .GetDefaultAudioEndpoint(flow, eConsole)
                .map_err(error)?;
            let client: IAudioClient = endpoint.Activate(CLSCTX_ALL, None).map_err(error)?;
            // Windows 音频引擎处理 44.1/48/96 kHz、单声道/环绕声等设备格式，
            // AUTOCONVERTPCM + SRC_DEFAULT_QUALITY 请求高质量重采样和声道转换。
            // 固定 IEEE_FLOAT (3) 输入契约后，无需把设备混合格式暴露给混音器。
            let format = WAVEFORMATEX {
                wFormatTag: 3,
                nChannels: CHANNELS as u16,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * CHANNELS as u32 * 4,
                nBlockAlign: CHANNELS as u16 * 4,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                | if matches!(source, Source::System) {
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                } else {
                    0
                };
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, &format, None)
                .map_err(error)?;
            let capture = client.GetService().map_err(error)?;
            Ok(Self {
                capture,
                client,
                source,
                running: false,
            })
        }
    }

    fn set_running(&mut self, running: bool) -> Result<(), String> {
        if self.running != running {
            unsafe {
                if running {
                    self.client.Start().map_err(error)?;
                } else {
                    self.client.Stop().map_err(error)?;
                    self.client.Reset().map_err(error)?;
                }
            }
            self.running = running;
        }
        Ok(())
    }

    /// GetBuffer 的 QPC 已转换为 100 ns，不可再次按性能计数器频率换算。
    /// 每次成功取包均在同线程 ReleaseBuffer；缓冲只在此作用域内读取。
    fn drain(&self, sender: &SyncSender<AudioFrame>) -> Result<(), String> {
        unsafe {
            while self.capture.GetNextPacketSize().map_err(error)? > 0 {
                let (mut data, mut count, mut flags, mut qpc) = (std::ptr::null_mut(), 0, 0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut count, &mut flags, None, Some(&mut qpc))
                    .map_err(error)?;
                if count == 0 {
                    break;
                }
                let result: Result<AudioFrame, String> = (|| {
                    if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                        return Err("音频设备返回无效 QPC 时间戳，无法保证音视频同步。".into());
                    }
                    if qpc > i64::MAX as u64 || count > RATE {
                        return Err("音频设备返回异常时间戳或过大的数据包。".into());
                    }
                    let length = count as usize * CHANNELS;
                    let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                        // SILENT 时 data 可以为空，绝不能解引用。
                        vec![0.0; length]
                    } else {
                        if data.is_null() {
                            return Err("音频设备返回空缓冲。".into());
                        }
                        // 逐字节转换不要求设备缓冲满足 Rust f32 指针对齐。
                        std::slice::from_raw_parts(data, length * 4)
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                            .collect()
                    };
                    if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                        // 首包也可能标记 discontinuity；后续缺口按 QPC 留空，由混音器补静音。
                        log::debug!(
                            "event=recording.audio_discontinuity source={:?}",
                            self.source
                        );
                    }
                    Ok(AudioFrame {
                        source: self.source,
                        source_time: qpc as i64,
                        samples,
                    })
                })();
                self.capture.ReleaseBuffer(count).map_err(error)?;
                // 不能阻塞设备线程，也不能无声丢包；队列满会使本次录制明确失败。
                sender
                    .try_send(result?)
                    .map_err(|_| "音频队列已满或编码线程已退出。".to_string())?;
            }
        }
        Ok(())
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        if self.running {
            unsafe {
                let _ = self.client.Stop();
            }
        }
    }
}

/// 有界音频队列与线程的所有权；错误通过 JoinHandle 传回，不会被满队列吞掉。
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
        let (sender, receiver) = mpsc::sync_channel(128);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::Builder::new()
            .name("recording-audio".into())
            .spawn(move || {
                let prepared = (|| {
                    let runtime = Runtime::new()?;
                    let enumerator: IMMDeviceEnumerator = unsafe {
                        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(error)?
                    };
                    let mut devices = Vec::new();
                    if system {
                        devices.push(Device::new(&enumerator, Source::System)?);
                    }
                    if microphone {
                        devices.push(Device::new(&enumerator, Source::Microphone)?);
                    }
                    Ok::<_, String>((runtime, devices))
                })();
                match prepared {
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        Err(error)
                    }
                    Ok((runtime, mut devices)) => {
                        let _ = ready_tx.send(Ok(()));
                        let result = (|| {
                            loop {
                                let running = active();
                                let stopped = stopping.load(Ordering::Acquire);
                                for device in &mut devices {
                                    // 暂停/停止前先取完已产生的包，最终按时间轴裁剪边界。
                                    if device.running {
                                        device.drain(&sender)?;
                                    }
                                    device.set_running(running && !stopped)?;
                                }
                                if stopped {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(5));
                            }
                            Ok(())
                        })();
                        drop(devices);
                        drop(runtime);
                        result
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut worker = Self {
            receiver,
            stop,
            thread: Some(thread),
        };
        ready_rx.recv().map_err(|e| e.to_string())??;
        // 初始化只打开设备，不开始采集；Ready/倒计时/暂停均不启用麦克风。
        if worker.thread.as_ref().is_some_and(|t| t.is_finished()) {
            worker.stop()?;
        }
        Ok(worker)
    }

    pub fn drain(&mut self) -> Result<Vec<AudioFrame>, String> {
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            self.stop()?;
            return Err("音频采集线程提前停止，请检查设备连接及音频权限。".into());
        }
        Ok(self.receiver.try_iter().collect())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| "音频采集线程异常退出。".to_string())??;
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
