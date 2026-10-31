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
    time::{Duration, Instant},
};
use windows::Win32::{Media::Audio::*, System::Com::*};

fn error(e: windows::core::Error) -> String {
    let advice = match e.code() {
        AUDCLNT_E_DEVICE_INVALIDATED => "设备已断开或失效，请重新连接设备后重新录制",
        AUDCLNT_E_DEVICE_IN_USE => "设备被其他程序独占，请关闭占用程序后重新录制",
        AUDCLNT_E_SERVICE_NOT_RUNNING => "Windows 音频服务未运行，请恢复服务后重新录制",
        code if code == windows::core::HRESULT(0x80070005_u32 as i32) => {
            "访问被拒绝，请检查 Windows 麦克风隐私权限后重新录制"
        }
        code if code == windows::core::HRESULT(0x80070490_u32 as i32) => {
            "未找到默认音频设备，请在 Windows 设置中选择设备后重新录制"
        }
        _ => "请检查默认音频设备、麦克风权限或设备连接，然后重新录制",
    };
    format!("WASAPI 音频录制失败：{e}。{advice}。")
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Self::System => "系统声音",
            Self::Microphone => "麦克风",
        }
    }

    fn flow(self) -> EDataFlow {
        match self {
            Self::System => eRender,
            Self::Microphone => eCapture,
        }
    }
}

/// 系统无播放时 loopback 可以没有包；麦克风静音应有 SILENT 包，二者不能混淆。
fn validate_delivery(source: Source, idle: Duration) -> Result<(), String> {
    if matches!(source, Source::Microphone) && idle >= Duration::from_secs(3) {
        return Err("麦克风连续 3 秒未返回数据，录制已停止，请检查权限、设备或休眠状态。".into());
    }
    Ok(())
}

fn validate_poll_gap(gap: Duration) -> Result<(), String> {
    if gap >= Duration::from_secs(5) {
        return Err(
            "音频采集因系统休眠或调度中断超过 5 秒，本次录制已停止；请检查设备后重新录制。".into(),
        );
    }
    Ok(())
}

/// GetId 返回 COM 分配的字符串；先复制，再释放，失败分支也不能泄漏。
fn endpoint_id(endpoint: &IMMDevice) -> Result<String, String> {
    unsafe {
        let id = endpoint.GetId().map_err(error)?;
        let result = id
            .to_string()
            .map_err(|e| format!("无法读取音频设备标识：{e}"));
        CoTaskMemFree(Some(id.0.cast()));
        result
    }
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
    endpoint: IMMDevice,
    endpoint_id: String,
    source: Source,
    running: bool,
    /// 静音包也是有效进展；只有麦克风长时间完全不交包才判定故障。
    last_packet: Instant,
    last_qpc: Option<u64>,
}
impl Device {
    fn new(enumerator: &IMMDeviceEnumerator, source: Source) -> Result<Self, String> {
        unsafe {
            let endpoint = enumerator
                .GetDefaultAudioEndpoint(source.flow(), eConsole)
                .map_err(error)?;
            let endpoint_id = endpoint_id(&endpoint)?;
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
                endpoint,
                endpoint_id,
                source,
                running: false,
                last_packet: Instant::now(),
                last_qpc: None,
            })
        }
    }

    /// 默认设备切换不保证使旧 client 失效；每 500 ms（暂停时也检查）以及恢复前
    /// 核对身份。明确失败而非继续录旧设备或悄悄导出静音，随后清理本次临时输出。
    fn validate_endpoint(&self, enumerator: &IMMDeviceEnumerator) -> Result<(), String> {
        unsafe {
            if self.endpoint.GetState().map_err(error)? != DEVICE_STATE_ACTIVE {
                return Err("音频设备已断开或停用，请重新选择设备后重新录制。".into());
            }
            let current = enumerator
                .GetDefaultAudioEndpoint(self.source.flow(), eConsole)
                .map_err(error)?;
            if endpoint_id(&current)? != self.endpoint_id {
                return Err("默认音频设备已切换，本次录制已停止，请确认新设备后重新录制。".into());
            }
        }
        Ok(())
    }

    fn set_running(&mut self, running: bool) -> Result<(), String> {
        if self.running != running {
            unsafe {
                if running {
                    self.client.Start().map_err(error)?;
                    self.last_packet = Instant::now();
                    self.last_qpc = None;
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
    fn drain(&mut self, sender: &SyncSender<AudioFrame>, final_drain: bool) -> Result<(), String> {
        unsafe {
            // 不让持续产包的驱动把线程永远困在 drain 中；每轮最多 32 包/20 ms，
            // 回到外层检查停止、暂停和设备状态。剩余包仍属于 WASAPI，下轮继续取。
            let started = Instant::now();
            for _ in 0..32 {
                if started.elapsed() >= Duration::from_millis(20)
                    || self.capture.GetNextPacketSize().map_err(error)? == 0
                {
                    break;
                }
                let (mut data, mut count, mut flags, mut qpc) = (std::ptr::null_mut(), 0, 0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut count, &mut flags, None, Some(&mut qpc))
                    .map_err(error)?;
                if count == 0 {
                    self.capture.ReleaseBuffer(0).map_err(error)?;
                    // 零帧不能刷新健康水位；仍需执行下面的无数据/尾包检查。
                    break;
                }
                let result: Result<AudioFrame, String> = (|| {
                    if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                        return Err("音频设备返回无效 QPC 时间戳，无法保证音视频同步。".into());
                    }
                    if qpc > i64::MAX as u64 || count > RATE {
                        return Err("音频设备返回异常时间戳或过大的数据包。".into());
                    }
                    if self.last_qpc.is_some_and(|previous| qpc <= previous) {
                        return Err(
                            "音频设备时间戳停滞或倒退，无法保证音视频同步，请重新录制。".into()
                        );
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
                self.last_packet = Instant::now();
                self.last_qpc = Some(qpc);
                // 不能阻塞设备线程，也不能无声丢包；队列满会使本次录制明确失败。
                sender
                    .try_send(result?)
                    .map_err(|_| "音频队列已满或编码线程已退出。".to_string())?;
            }
            // 暂停/停止后 Reset 会丢掉剩余包，不能把取包预算耗尽伪装成完整成功。
            if final_drain && self.capture.GetNextPacketSize().map_err(error)? > 0 {
                return Err(
                    "音频积压超过停止处理预算，无法保证尾部完整，请降低录屏负载后重试。".into(),
                );
            }
            // Loopback 在没有系统播放时允许完全不发包，不能把正常静音当作故障。
            // 麦克风（含静音）应持续发包；明确失败，避免混音器无限补零掩盖坏设备。
            validate_delivery(self.source, self.last_packet.elapsed())?;
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
                        devices.push(
                            Device::new(&enumerator, Source::System)
                                .map_err(|e| format!("系统声音：{e}"))?,
                        );
                    }
                    if microphone {
                        devices.push(
                            Device::new(&enumerator, Source::Microphone)
                                .map_err(|e| format!("麦克风：{e}"))?,
                        );
                    }
                    Ok::<_, String>((runtime, enumerator, devices))
                })();
                match prepared {
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        Err(error)
                    }
                    Ok((runtime, enumerator, mut devices)) => {
                        let _ = ready_tx.send(Ok(()));
                        let result = (|| {
                            let mut checked = Instant::now();
                            let mut last_poll = Instant::now();
                            loop {
                                let running = active();
                                let stopped = stopping.load(Ordering::Acquire);
                                // 休眠/驱动或调度长时间中断后，不能把整个空白段冒充正常音频。
                                // 停止优先，避免取消被健康检查阻挡；暂停也检查中断。
                                if !stopped {
                                    validate_poll_gap(last_poll.elapsed())?;
                                }
                                last_poll = Instant::now();
                                let check_due = checked.elapsed() >= Duration::from_millis(500);
                                for device in &mut devices {
                                    let source = device.source;
                                    let device_result = (|| {
                                        if !stopped && (check_due || (running && !device.running)) {
                                            device.validate_endpoint(&enumerator)?;
                                        }
                                        // 暂停/停止前有界排空，超预算明确失败；尾包按时间轴裁剪。
                                        if device.running {
                                            device.drain(&sender, !running || stopped)?;
                                        }
                                        device.set_running(running && !stopped)?;
                                        Ok::<_, String>(())
                                    })();
                                    device_result
                                        .map_err(|e| format!("{}：{e}", source.label()))?;
                                }
                                if check_due {
                                    checked = Instant::now();
                                }
                                if stopped {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(5));
                            }
                            Ok(())
                        })();
                        drop(devices);
                        drop(enumerator);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_microphone_packets_fail_but_idle_loopback_is_valid() {
        assert!(validate_delivery(Source::Microphone, Duration::from_millis(2999)).is_ok());
        assert!(validate_delivery(Source::Microphone, Duration::from_secs(3)).is_err());
        assert!(validate_delivery(Source::System, Duration::from_secs(3600)).is_ok());
        assert!(validate_poll_gap(Duration::from_millis(4999)).is_ok());
        assert!(validate_poll_gap(Duration::from_secs(5))
            .unwrap_err()
            .contains("休眠"));
    }

    /// 无需真实设备即可核对 Windows 错误到用户可操作提示的映射。
    #[test]
    fn audio_failures_explain_the_next_action_and_keep_native_details() {
        for (code, expected) in [
            (AUDCLNT_E_DEVICE_INVALIDATED, "重新连接设备"),
            (AUDCLNT_E_DEVICE_IN_USE, "关闭占用程序"),
            (AUDCLNT_E_SERVICE_NOT_RUNNING, "恢复服务"),
            (windows::core::HRESULT(0x80070005_u32 as i32), "隐私权限"),
            (
                windows::core::HRESULT(0x80070490_u32 as i32),
                "未找到默认音频设备",
            ),
            (windows::core::HRESULT(0x80004005_u32 as i32), "设备连接"),
        ] {
            let native = windows::core::Error::from_hresult(code);
            let details = native.to_string();
            let message = error(native);
            assert!(message.contains(expected), "{message}");
            assert!(message.contains(&details), "{message}");
        }
    }
}
