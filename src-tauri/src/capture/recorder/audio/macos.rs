//! macOS 声音采集：系统声音用 ScreenCaptureKit 的纯音频流，麦克风用 AVCaptureSession。
//!
//! 两路都在系统回调队列上把 CMSampleBuffer 转为交错的 48 kHz 双声道 f32，时间戳取样本缓冲的
//! 呈现时间（主机时钟，与视频 `capture::now` 同源），交给与 Windows 共用的混音器。
//! 控制线程按录制状态门控：倒计时与暂停期间丢弃数据，停止时同步结束系统采集。
use super::{AudioFrame, Source, CHANNELS, RATE};
use crate::shared::macos::{
    array_items, host_ticks, ns_array, rust_string, CMTime, Id, StreamDescription,
};
use block2::RcBlock;
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};
use std::{
    ffi::{c_char, c_void},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBuffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}
/// AudioBufferList 为变长结构；预留 8 个缓冲足以容纳所请求的双声道（非交错时每声道一个）。
#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 8],
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetNumSamples(buffer: *mut c_void) -> isize;
    fn CMSampleBufferGetPresentationTimeStamp(buffer: *mut c_void) -> CMTime;
    fn CMSampleBufferGetFormatDescription(buffer: *mut c_void) -> *mut c_void;
    fn CMAudioFormatDescriptionGetStreamBasicDescription(
        description: *mut c_void,
    ) -> *const StreamDescription;
    fn CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        buffer: *mut c_void,
        size_needed: *mut usize,
        list: *mut AudioBufferList,
        list_size: usize,
        structure_allocator: *const c_void,
        block_allocator: *const c_void,
        flags: u32,
        block: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}

extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> *mut c_void;
}

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    static AVMediaTypeAudio: Id;
}

#[link(name = "AVFAudio", kind = "framework")]
extern "C" {
    static AVFormatIDKey: Id;
    static AVSampleRateKey: Id;
    static AVNumberOfChannelsKey: Id;
    static AVLinearPCMBitDepthKey: Id;
    static AVLinearPCMIsFloatKey: Id;
    static AVLinearPCMIsNonInterleaved: Id;
    static AVLinearPCMIsBigEndianKey: Id;
}

/// 'lpcm'。
const LINEAR_PCM: u32 = 0x6C70_636D;
const FLAG_FLOAT: u32 = 1;
const FLAG_BIG_ENDIAN: u32 = 2;
const FLAG_NON_INTERLEAVED: u32 = 1 << 5;
/// SCStreamOutputTypeAudio。
const STREAM_AUDIO: isize = 1;
/// AVAuthorizationStatus。
const AUTH_NOT_DETERMINED: isize = 0;
const AUTH_AUTHORIZED: isize = 3;
/// 麦克风持续无数据视为设备断开（与 Windows 一致）。
const MICROPHONE_SILENCE: i64 = 30_000_000;

/// 样本缓冲的 PCM 布局；只接受本模块请求过的线性 PCM 小端格式。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Format {
    float: bool,
    bits: u32,
    channels: usize,
    interleaved: bool,
}

fn format(description: &StreamDescription) -> Result<Format, String> {
    let flags = description.format_flags;
    if description.format_id != LINEAR_PCM || flags & FLAG_BIG_ENDIAN != 0 {
        return Err("系统返回了不支持的音频编码。".into());
    }
    if (description.sample_rate - f64::from(RATE)).abs() > 0.5 {
        return Err(format!(
            "系统音频采样率为 {} Hz，未按要求转换为 48 kHz。",
            description.sample_rate
        ));
    }
    let float = flags & FLAG_FLOAT != 0;
    let bits = description.bits_per_channel;
    let channels = description.channels_per_frame as usize;
    if !matches!((float, bits), (true, 32) | (false, 16) | (false, 32)) || !(1..=8).contains(&channels)
    {
        return Err("系统返回了不支持的音频采样格式。".into());
    }
    Ok(Format {
        float,
        bits,
        channels,
        interleaved: flags & FLAG_NON_INTERLEAVED == 0,
    })
}

fn sample(format: Format, buffer: &[u8], index: usize) -> Option<f32> {
    let width = format.bits as usize / 8;
    let bytes = buffer.get(index * width..(index + 1) * width)?;
    Some(match (format.float, format.bits) {
        (true, _) => f32::from_le_bytes(bytes.try_into().ok()?),
        (false, 16) => f32::from(i16::from_le_bytes(bytes.try_into().ok()?)) / 32768.0,
        _ => i32::from_le_bytes(bytes.try_into().ok()?) as f32 / 2_147_483_648.0,
    })
}

/// 转为交错的左右声道；单声道复制到两侧，多于两声道只取前两路。数据不足时返回 None。
fn interleave(format: Format, buffers: &[&[u8]], frames: usize) -> Option<Vec<f32>> {
    let right = usize::from(format.channels > 1);
    let mut samples = Vec::with_capacity(frames * CHANNELS);
    for frame in 0..frames {
        let (left, right) = if format.interleaved {
            let buffer = buffers.first()?;
            (
                sample(format, buffer, frame * format.channels)?,
                sample(format, buffer, frame * format.channels + right)?,
            )
        } else {
            (
                sample(format, buffers.first()?, frame)?,
                sample(format, buffers.get(right.min(buffers.len() - 1))?, frame)?,
            )
        };
        samples.push(left);
        samples.push(right);
    }
    Some(samples)
}

/// 复制样本缓冲中的 PCM；空包返回 None。
unsafe fn audio_frame(source: Source, buffer: *mut c_void) -> Result<Option<AudioFrame>, String> {
    let frames = CMSampleBufferGetNumSamples(buffer);
    if frames <= 0 {
        return Ok(None);
    }
    let source_time = CMSampleBufferGetPresentationTimeStamp(buffer)
        .ticks()
        .ok_or("音频时间戳无效。")?;
    let description =
        CMAudioFormatDescriptionGetStreamBasicDescription(CMSampleBufferGetFormatDescription(buffer));
    let format = format(description.as_ref().ok_or("音频格式不可用。")?)?;
    let mut list = AudioBufferList {
        count: 0,
        buffers: [AudioBuffer {
            channels: 0,
            size: 0,
            data: std::ptr::null_mut(),
        }; 8],
    };
    let mut block: *mut c_void = std::ptr::null_mut();
    // kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment。
    let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        buffer,
        std::ptr::null_mut(),
        &mut list,
        std::mem::size_of::<AudioBufferList>(),
        std::ptr::null(),
        std::ptr::null(),
        1,
        &mut block,
    );
    if status != 0 {
        return Err(format!("无法读取音频数据（{status}）。"));
    }
    let buffers: Vec<&[u8]> = list.buffers[..(list.count as usize).min(8)]
        .iter()
        .filter(|b| !b.data.is_null())
        .map(|b| std::slice::from_raw_parts(b.data as *const u8, b.size as usize))
        .collect();
    let samples = interleave(format, &buffers, frames as usize);
    if !block.is_null() {
        CFRelease(block);
    }
    let samples = samples.ok_or("音频数据长度与格式不符。")?;
    Ok(Some(AudioFrame {
        source,
        source_time,
        samples,
    }))
}

/// 回调与控制线程共享的状态；同一时间只有一场录制（由录屏工作守卫保证）。
struct Sink {
    sender: SyncSender<AudioFrame>,
    running: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    microphone_at: Arc<AtomicI64>,
}
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

fn fail(failure: &Mutex<Option<String>>, message: String) {
    if let Ok(mut slot) = failure.lock() {
        slot.get_or_insert(message);
    }
}

fn deliver(source: Source, buffer: *mut c_void) {
    let Ok(guard) = SINK.lock() else {
        return;
    };
    let Some(sink) = guard.as_ref() else {
        return;
    };
    if matches!(source, Source::Microphone) {
        sink.microphone_at.store(host_ticks(), Ordering::Release);
    }
    if !sink.running.load(Ordering::Acquire) {
        return;
    }
    match unsafe { audio_frame(source, buffer) } {
        Ok(Some(frame)) => match sink.sender.try_send(frame) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => fail(
                &sink.failure,
                "音频处理跟不上采集速度，请降低帧率或关闭其他高负载程序。".into(),
            ),
        },
        Ok(None) => {}
        Err(error) => fail(&sink.failure, format!("{}：{error}", source.label())),
    }
}

extern "C-unwind" fn system_output(_: &AnyObject, _: Sel, _: Id, buffer: *mut c_void, kind: isize) {
    if kind == STREAM_AUDIO {
        deliver(Source::System, buffer);
    }
}

extern "C-unwind" fn microphone_output(_: &AnyObject, _: Sel, _: Id, buffer: *mut c_void, _: Id) {
    deliver(Source::Microphone, buffer);
}

/// SCStreamDelegate：系统中止采集（如撤销屏幕录制授权、显示器断开）时结束录制。
extern "C-unwind" fn stream_stopped(_: &AnyObject, _: Sel, _: Id, error: Id) {
    let message = unsafe { error_text(error) };
    if let Ok(guard) = SINK.lock() {
        if let Some(sink) = guard.as_ref() {
            fail(&sink.failure, format!("系统声音采集已中止：{message}"));
        }
    }
}

/// 注册回调类并创建常驻实例（采集对象可能只弱引用输出与代理）。
unsafe fn receiver() -> Result<Id, String> {
    static RECEIVER: OnceLock<usize> = OnceLock::new();
    if let Some(receiver) = RECEIVER.get() {
        return Ok(*receiver as Id);
    }
    let class = match AnyClass::get(c"SnapDeskAudioReceiver") {
        Some(class) => class,
        None => {
            let mut builder = ClassBuilder::new(c"SnapDeskAudioReceiver", class!(NSObject))
                .ok_or("无法注册音频回调。")?;
            builder.add_method(
                sel!(stream:didOutputSampleBuffer:ofType:),
                system_output as extern "C-unwind" fn(_, _, _, _, _),
            );
            builder.add_method(
                sel!(captureOutput:didOutputSampleBuffer:fromConnection:),
                microphone_output as extern "C-unwind" fn(_, _, _, _, _),
            );
            builder.add_method(
                sel!(stream:didStopWithError:),
                stream_stopped as extern "C-unwind" fn(_, _, _, _),
            );
            builder.register()
        }
    };
    let instance: Option<Retained<AnyObject>> = msg_send![class, new];
    let pointer = Retained::into_raw(instance.ok_or("无法创建音频回调。")?) as usize;
    Ok(*RECEIVER.get_or_init(|| pointer) as Id)
}

unsafe fn error_text(error: Id) -> String {
    if error.is_null() {
        return "未知错误".into();
    }
    let text: Id = msg_send![error, localizedDescription];
    rust_string(text).unwrap_or_else(|| "未知错误".into())
}

unsafe fn number(value: i64) -> Id {
    msg_send![class!(NSNumber), numberWithLongLong: value]
}

unsafe fn flag(value: bool) -> Id {
    msg_send![class!(NSNumber), numberWithBool: Bool::new(value)]
}

unsafe fn dictionary(entries: &[(Id, Id)]) -> Id {
    let keys: Vec<Id> = entries.iter().map(|(k, _)| *k).collect();
    let values: Vec<Id> = entries.iter().map(|(_, v)| *v).collect();
    msg_send![
        class!(NSDictionary),
        dictionaryWithObjects: values.as_ptr(),
        forKeys: keys.as_ptr(),
        count: keys.len()
    ]
}

unsafe fn queue(label: &std::ffi::CStr) -> Result<Retained<AnyObject>, String> {
    // GCD 队列是 Objective-C 对象，交给 Retained 管理生命周期。
    Retained::from_raw(dispatch_queue_create(label.as_ptr(), std::ptr::null()) as Id)
        .ok_or_else(|| "无法创建音频回调队列。".into())
}

/// 只带一个 NSError 参数的系统完成回调及其结果通道。
fn completion() -> (RcBlock<dyn Fn(Id)>, Receiver<Option<String>>) {
    let (sender, receiver) = mpsc::sync_channel::<Option<String>>(1);
    let block = RcBlock::new(move |error: Id| {
        let _ = sender.try_send((!error.is_null()).then(|| unsafe { error_text(error) }));
    });
    (block, receiver)
}

fn completed(receiver: Receiver<Option<String>>, timeout: Duration, what: &str) -> Result<(), String> {
    match receiver.recv_timeout(timeout) {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(format!("{what}：{error}")),
        Err(_) => Err(format!("{what}：系统响应超时。")),
    }
}

/// 跨线程传回 Objective-C 对象：只在控制线程内使用，传递期间不访问。
struct Handoff(Retained<AnyObject>);
unsafe impl Send for Handoff {}

/// 系统声音：对主显示器建立只取音频的 SCStream（视频输出未注册，系统按 1 FPS 丢弃）。
struct SystemAudio {
    stream: Retained<AnyObject>,
    _queue: Retained<AnyObject>,
    started: bool,
}

impl SystemAudio {
    unsafe fn new() -> Result<Self, String> {
        let (sender, content) = mpsc::sync_channel::<Result<Handoff, String>>(1);
        let block = RcBlock::new(move |content: Id, error: Id| {
            let result = if content.is_null() {
                Err(unsafe { error_text(error) })
            } else {
                let displays: Id = unsafe { msg_send![content, displays] };
                unsafe { array_items(displays) }
                    .first()
                    .and_then(|display| unsafe { Retained::retain(*display) })
                    .map(Handoff)
                    .ok_or_else(|| "未找到可用于采集系统声音的显示器。".to_string())
            };
            let _ = sender.try_send(result);
        });
        let _: () = msg_send![
            class!(SCShareableContent),
            getShareableContentWithCompletionHandler: &*block
        ];
        let display = content
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "读取可录制内容超时。".to_string())?
            .map_err(|e| format!("无法读取可录制内容，请确认已授予“屏幕录制”权限：{e}"))?
            .0;
        let allocated: Allocated<AnyObject> = msg_send![class!(SCContentFilter), alloc];
        let filter: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithDisplay: &*display,
            excludingWindows: ns_array(&[])
        ];
        let filter = filter.ok_or("无法创建系统声音采集范围。")?;
        let config: Option<Retained<AnyObject>> = msg_send![class!(SCStreamConfiguration), new];
        let config = config.ok_or("无法创建系统声音采集配置。")?;
        let _: () = msg_send![&*config, setCapturesAudio: Bool::YES];
        let _: () = msg_send![&*config, setSampleRate: RATE as isize];
        let _: () = msg_send![&*config, setChannelCount: CHANNELS as isize];
        // 不录本应用的提示音。
        let _: () = msg_send![&*config, setExcludesCurrentProcessAudio: Bool::YES];
        // 视频只是 SCStream 的必需部分：最小尺寸、每秒一帧，且不注册视频输出。
        let _: () = msg_send![&*config, setWidth: 2usize];
        let _: () = msg_send![&*config, setHeight: 2usize];
        let _: () = msg_send![&*config, setMinimumFrameInterval: CMTime { value: 1, timescale: 1, flags: 1, epoch: 0 }];
        let _: () = msg_send![&*config, setShowsCursor: Bool::NO];
        let receiver = receiver()?;
        let allocated: Allocated<AnyObject> = msg_send![class!(SCStream), alloc];
        let stream: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithFilter: &*filter,
            configuration: &*config,
            delegate: receiver
        ];
        let stream = stream.ok_or("无法创建系统声音采集流。")?;
        let queue = queue(c"io.snapdesk.audio.system")?;
        let mut error: Id = std::ptr::null_mut();
        let added: Bool = msg_send![
            &*stream,
            addStreamOutput: receiver,
            type: STREAM_AUDIO,
            sampleHandlerQueue: &*queue,
            error: &mut error as *mut Id
        ];
        if !added.as_bool() {
            return Err(format!("无法注册系统声音输出：{}", error_text(error)));
        }
        Ok(Self {
            stream,
            _queue: queue,
            started: false,
        })
    }

    unsafe fn start(&mut self) -> Result<(), String> {
        let (block, done) = completion();
        let _: () = msg_send![&*self.stream, startCaptureWithCompletionHandler: &*block];
        completed(done, Duration::from_secs(10), "无法开始采集系统声音")?;
        self.started = true;
        Ok(())
    }
}

impl Drop for SystemAudio {
    fn drop(&mut self) {
        if self.started {
            let (block, done) = completion();
            let _: () = unsafe { msg_send![&*self.stream, stopCaptureWithCompletionHandler: &*block] };
            let _ = completed(done, Duration::from_secs(3), "停止系统声音采集");
        }
    }
}

/// 麦克风授权：首次使用时弹出系统对话框并等待用户选择。
unsafe fn microphone_permission() -> Result<(), String> {
    let denied = "麦克风权限未开启：请在“系统设置 → 隐私与安全性 → 麦克风”中允许 Snap Desk，然后重试。";
    let status: isize =
        msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeAudio];
    match status {
        AUTH_AUTHORIZED => Ok(()),
        AUTH_NOT_DETERMINED => {
            let (sender, receiver) = mpsc::sync_channel::<bool>(1);
            let block = RcBlock::new(move |granted: Bool| {
                let _ = sender.try_send(granted.as_bool());
            });
            let _: () = msg_send![
                class!(AVCaptureDevice),
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*block
            ];
            match receiver.recv_timeout(Duration::from_secs(120)) {
                Ok(true) => Ok(()),
                _ => Err(denied.into()),
            }
        }
        _ => Err(denied.into()),
    }
}

/// 麦克风：AVCaptureSession 直接输出 48 kHz 双声道交错 f32，系统负责重采样。
struct Microphone {
    session: Retained<AnyObject>,
    _queue: Retained<AnyObject>,
    started: bool,
}

impl Microphone {
    unsafe fn new() -> Result<Self, String> {
        microphone_permission()?;
        let device: Option<Retained<AnyObject>> =
            msg_send![class!(AVCaptureDevice), defaultDeviceWithMediaType: AVMediaTypeAudio];
        let device = device.ok_or("未找到默认麦克风，请连接设备后重试。")?;
        let mut error: Id = std::ptr::null_mut();
        let input: Option<Retained<AnyObject>> = msg_send![
            class!(AVCaptureDeviceInput),
            deviceInputWithDevice: &*device,
            error: &mut error as *mut Id
        ];
        let input = input.ok_or_else(|| format!("无法打开麦克风：{}", error_text(error)))?;
        let session: Option<Retained<AnyObject>> = msg_send![class!(AVCaptureSession), new];
        let session = session.ok_or("无法创建麦克风采集会话。")?;
        let can_add: Bool = msg_send![&*session, canAddInput: &*input];
        if !can_add.as_bool() {
            return Err("麦克风被其他程序占用或不可用。".into());
        }
        let _: () = msg_send![&*session, addInput: &*input];
        let output: Option<Retained<AnyObject>> = msg_send![class!(AVCaptureAudioDataOutput), new];
        let output = output.ok_or("无法创建麦克风输出。")?;
        let settings = dictionary(&[
            (AVFormatIDKey, number(i64::from(LINEAR_PCM))),
            (AVSampleRateKey, number(i64::from(RATE))),
            (AVNumberOfChannelsKey, number(CHANNELS as i64)),
            (AVLinearPCMBitDepthKey, number(32)),
            (AVLinearPCMIsFloatKey, flag(true)),
            (AVLinearPCMIsNonInterleaved, flag(false)),
            (AVLinearPCMIsBigEndianKey, flag(false)),
        ]);
        let _: () = msg_send![&*output, setAudioSettings: settings];
        let queue = queue(c"io.snapdesk.audio.microphone")?;
        let _: () = msg_send![&*output, setSampleBufferDelegate: receiver()?, queue: &*queue];
        let can_add: Bool = msg_send![&*session, canAddOutput: &*output];
        if !can_add.as_bool() {
            return Err("无法接入麦克风输出。".into());
        }
        let _: () = msg_send![&*session, addOutput: &*output];
        Ok(Self {
            session,
            _queue: queue,
            started: false,
        })
    }

    unsafe fn start(&mut self) -> Result<(), String> {
        // startRunning 同步阻塞到设备就绪，因此在控制线程调用。
        let _: () = msg_send![&*self.session, startRunning];
        let running: Bool = msg_send![&*self.session, isRunning];
        if !running.as_bool() {
            return Err("麦克风启动失败，请检查设备连接与权限。".into());
        }
        self.started = true;
        Ok(())
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        if self.started {
            let _: () = unsafe { msg_send![&*self.session, stopRunning] };
        }
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
        let (sender, receiver) = mpsc::sync_channel(128);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let running = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let microphone_at = Arc::new(AtomicI64::new(0));
        *SINK.lock().map_err(|_| "音频状态不可用。")? = Some(Sink {
            sender,
            running: running.clone(),
            failure: failure.clone(),
            microphone_at: microphone_at.clone(),
        });
        let thread = thread::Builder::new()
            .name("recording-audio".into())
            .spawn(move || {
                let prepared = objc2::rc::autoreleasepool(|_| unsafe {
                    let system = if system {
                        Some(SystemAudio::new().map_err(|e| format!("系统声音：{e}"))?)
                    } else {
                        None
                    };
                    let microphone = if microphone {
                        Some(Microphone::new().map_err(|e| format!("麦克风：{e}"))?)
                    } else {
                        None
                    };
                    Ok::<_, String>((system, microphone))
                });
                let (mut system, mut microphone) = match prepared {
                    Ok(sources) => {
                        let _ = ready_tx.send(Ok(()));
                        sources
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                let result = (|| {
                    let mut started_at = None;
                    loop {
                        let stopped = stopping.load(Ordering::Acquire);
                        let recording = !stopped && active();
                        // 首次进入录制阶段才启动设备：倒计时期间不打开麦克风指示灯。
                        if recording && started_at.is_none() {
                            objc2::rc::autoreleasepool(|_| unsafe {
                                if let Some(system) = system.as_mut() {
                                    system.start().map_err(|e| format!("系统声音：{e}"))?;
                                }
                                if let Some(microphone) = microphone.as_mut() {
                                    microphone.start().map_err(|e| format!("麦克风：{e}"))?;
                                }
                                Ok::<_, String>(())
                            })?;
                            let now = host_ticks();
                            microphone_at.store(now, Ordering::Release);
                            started_at = Some(now);
                        }
                        running.store(recording, Ordering::Release);
                        if let Some(error) = failure.lock().ok().and_then(|mut f| f.take()) {
                            return Err(error);
                        }
                        if microphone.is_some()
                            && started_at.is_some()
                            && !stopped
                            && host_ticks() - microphone_at.load(Ordering::Acquire)
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
                running.store(false, Ordering::Release);
                objc2::rc::autoreleasepool(|_| {
                    drop(microphone.take());
                    drop(system.take());
                });
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
            return Err("音频采集线程提前停止，请检查设备连接及音频权限。".into());
        }
        Ok(self.receiver.try_iter().collect())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        let result = match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| "音频采集线程异常退出。".to_string())
                .and_then(|r| r),
            None => Ok(()),
        };
        if let Ok(mut sink) = SINK.lock() {
            sink.take();
        }
        result
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

    fn description(flags: u32, bits: u32, channels: u32, rate: f64) -> StreamDescription {
        StreamDescription {
            sample_rate: rate,
            format_id: LINEAR_PCM,
            format_flags: flags,
            bits_per_channel: bits,
            channels_per_frame: channels,
            ..Default::default()
        }
    }

    #[test]
    fn accepts_requested_pcm_layouts_and_rejects_others() {
        let planar = format(&description(FLAG_FLOAT | FLAG_NON_INTERLEAVED, 32, 2, 48_000.0)).unwrap();
        assert!(planar.float && !planar.interleaved && planar.channels == 2);
        assert!(format(&description(0, 16, 1, 48_000.0)).unwrap().interleaved);
        assert!(format(&description(FLAG_FLOAT, 32, 2, 44_100.0)).unwrap_err().contains("44100"));
        assert!(format(&description(FLAG_FLOAT, 64, 2, 48_000.0)).is_err());
        assert!(format(&description(FLAG_FLOAT | FLAG_BIG_ENDIAN, 32, 2, 48_000.0)).is_err());
    }

    #[test]
    fn interleaves_planar_stereo_mono_and_int16() {
        let floats = |values: &[f32]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
        let planar = Format { float: true, bits: 32, channels: 2, interleaved: false };
        let (left, right) = (floats(&[0.1, 0.2]), floats(&[-0.1, -0.2]));
        assert_eq!(interleave(planar, &[&left, &right], 2).unwrap(), vec![0.1, -0.1, 0.2, -0.2]);
        let mono = Format { float: true, bits: 32, channels: 1, interleaved: true };
        assert_eq!(interleave(mono, &[&left], 2).unwrap(), vec![0.1, 0.1, 0.2, 0.2]);
        let int16 = Format { float: false, bits: 16, channels: 2, interleaved: true };
        let pcm: Vec<u8> = [16384i16, -32768].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(interleave(int16, &[&pcm], 1).unwrap(), vec![0.5, -1.0]);
        // 数据不足时拒绝，而不是越界读取。
        assert!(interleave(planar, &[&left, &right], 3).is_none());
    }
}
