//! macOS 单窗口录制：ScreenCaptureKit 的独立窗口过滤器只采集目标窗口本身，
//! 窗口被遮挡、移动或位于其他桌面空间时内容仍完整。
//!
//! 系统在回调队列上交付 BGRA 帧；这里复制选区后只保留最新一帧，由录制线程按需取走。
//! 窗口关闭、最小化或尺寸改变时与 Windows 一致：明确结束录制，不拉伸或黑边填充。
use super::{BgraImage, CaptureBackend, FramePool, RecorderConfig, VideoFrame};
use crate::capture::Rect;
use crate::macos::{
    array_items, completed, completion, dispatch_queue, error_text, CMTime, Handoff, Id,
};
use block2::RcBlock;
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};
use std::{
    ffi::c_void,
    sync::{mpsc, Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetImageBuffer(buffer: *mut c_void) -> *mut c_void;
    fn CMSampleBufferGetPresentationTimeStamp(buffer: *mut c_void) -> CMTime;
    fn CMSampleBufferGetSampleAttachmentsArray(buffer: *mut c_void, create: bool) -> Id;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
}

#[link(name = "ScreenCaptureKit", kind = "framework")]
extern "C" {
    static SCStreamFrameInfoStatus: Id;
}

/// SCStreamOutputTypeScreen 与 SCFrameStatusComplete。
const STREAM_SCREEN: isize = 0;
const FRAME_COMPLETE: isize = 0;
/// kCVPixelFormatType_32BGRA，与流水线帧格式一致。
const BGRA: u32 = 0x4247_5241;
/// kCVPixelBufferLock_ReadOnly。
const READ_ONLY: u64 = 1;
const CLOSED: &str = "录制窗口已关闭或最小化，录制已结束。";

/// 窗口在桌面中的物理像素范围（点坐标 × 所在屏缩放，与截图坐标一致）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub(super) fn geometry(id: u32, pid: u32) -> Result<Geometry, String> {
    let window = xcap::Window::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|w| w.id().ok() == Some(id) && w.pid().ok() == Some(pid))
        .ok_or(CLOSED)?;
    if window.is_minimized().unwrap_or(false) {
        return Err(CLOSED.into());
    }
    let scale = window
        .current_monitor()
        .ok()
        .and_then(|m| m.scale_factor().ok())
        .map(f64::from)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    let px = |value: f64| (value * scale).round();
    let geometry = Geometry {
        x: px(f64::from(window.x().map_err(|e| e.to_string())?)) as i32,
        y: px(f64::from(window.y().map_err(|e| e.to_string())?)) as i32,
        width: px(f64::from(window.width().map_err(|e| e.to_string())?)) as u32,
        height: px(f64::from(window.height().map_err(|e| e.to_string())?)) as u32,
    };
    if geometry.width < 2 || geometry.height < 2 {
        return Err("录制来源尺寸无效。".into());
    }
    Ok(geometry)
}

/// 回调队列与录制线程共享的状态。
struct Shared {
    latest: Mutex<Option<(i64, BgraImage)>>,
    arrived: Condvar,
    failure: Mutex<Option<String>>,
    size: (u32, u32),
    region: Rect,
    pool: Arc<FramePool>,
}

impl Shared {
    fn fail(&self, message: String) {
        if let Ok(mut slot) = self.failure.lock() {
            slot.get_or_insert(message);
        }
        self.arrived.notify_all();
    }

    /// 只接受完整帧；空闲帧（画面未变化）不带新内容，由编码端延长上一帧。
    unsafe fn receive(&self, sample: *mut c_void) {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, false);
        if let Some(first) = array_items(attachments).first() {
            let status: Id = msg_send![*first, objectForKey: SCStreamFrameInfoStatus];
            if !status.is_null() {
                let status: isize = msg_send![status, integerValue];
                if status != FRAME_COMPLETE {
                    return;
                }
            }
        }
        let buffer = CMSampleBufferGetImageBuffer(sample);
        if buffer.is_null() {
            return;
        }
        let Some(time) = CMSampleBufferGetPresentationTimeStamp(sample).ticks() else {
            return;
        };
        let (width, height) = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer));
        if (width, height) != (self.size.0 as usize, self.size.1 as usize) {
            self.fail("录制窗口尺寸已改变，录制已结束；请重新选择来源。".into());
            return;
        }
        if CVPixelBufferLockBaseAddress(buffer, READ_ONLY) != 0 {
            return;
        }
        let base = CVPixelBufferGetBaseAddress(buffer) as *const u8;
        let stride = CVPixelBufferGetBytesPerRow(buffer);
        let r = self.region;
        let row = r.width as usize * 4;
        let image = (!base.is_null() && stride >= width * 4).then(|| {
            let mut image = self.pool.acquire(r.width, r.height);
            for y in 0..r.height as usize {
                let source = std::slice::from_raw_parts(
                    base.add((r.y as usize + y) * stride + r.x as usize * 4),
                    row,
                );
                let target = &mut image.data[y * row..(y + 1) * row];
                target.copy_from_slice(source);
                // 窗口圆角外是透明像素；视频不带透明通道，统一视为不透明。
                for alpha in target.iter_mut().skip(3).step_by(4) {
                    *alpha = 255;
                }
            }
            image
        });
        CVPixelBufferUnlockBaseAddress(buffer, READ_ONLY);
        if let (Some(image), Ok(mut latest)) = (image, self.latest.lock()) {
            *latest = Some((time, image));
            self.arrived.notify_all();
        }
    }
}

/// 当前录制的共享状态；回调按全局槽位找到它（同一时间只有一场录制）。
static SINK: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

fn current() -> Option<Arc<Shared>> {
    SINK.lock().ok().and_then(|sink| sink.clone())
}

extern "C-unwind" fn screen_output(_: &AnyObject, _: Sel, _: Id, sample: *mut c_void, kind: isize) {
    if kind == STREAM_SCREEN {
        if let Some(shared) = current() {
            unsafe { shared.receive(sample) };
        }
    }
}

/// 窗口关闭或授权被撤销时系统中止采集。
extern "C-unwind" fn stream_stopped(_: &AnyObject, _: Sel, _: Id, error: Id) {
    if let Some(shared) = current() {
        let reason = unsafe { error_text(error) };
        shared.fail(format!("窗口录制已被系统中止：{reason}"));
    }
}

unsafe fn receiver() -> Result<Id, String> {
    static RECEIVER: OnceLock<usize> = OnceLock::new();
    if let Some(receiver) = RECEIVER.get() {
        return Ok(*receiver as Id);
    }
    let class = match AnyClass::get(c"SnapDeskWindowReceiver") {
        Some(class) => class,
        None => {
            let mut builder = ClassBuilder::new(c"SnapDeskWindowReceiver", class!(NSObject))
                .ok_or("无法注册窗口录制回调。")?;
            builder.add_method(
                sel!(stream:didOutputSampleBuffer:ofType:),
                screen_output as extern "C-unwind" fn(_, _, _, _, _),
            );
            builder.add_method(
                sel!(stream:didStopWithError:),
                stream_stopped as extern "C-unwind" fn(_, _, _, _),
            );
            builder.register()
        }
    };
    let instance: Option<Retained<AnyObject>> = msg_send![class, new];
    let pointer = Retained::into_raw(instance.ok_or("无法创建窗口录制回调。")?) as usize;
    Ok(*RECEIVER.get_or_init(|| pointer) as Id)
}

/// 按 CGWindowID 找到 ScreenCaptureKit 的窗口对象（包括其他桌面空间中的窗口）。
unsafe fn shareable_window(id: u32) -> Result<Retained<AnyObject>, String> {
    let (sender, content) = mpsc::sync_channel::<Result<Handoff, String>>(1);
    let block = RcBlock::new(move |shareable: Id, error: Id| {
        let result = if shareable.is_null() {
            Err(unsafe { error_text(error) })
        } else {
            let windows: Id = unsafe { msg_send![shareable, windows] };
            unsafe { array_items(windows) }
                .into_iter()
                .find(|window| {
                    let window_id: u32 = unsafe { msg_send![*window, windowID] };
                    window_id == id
                })
                .and_then(|window| unsafe { Retained::retain(window) })
                .map(Handoff)
                .ok_or_else(|| CLOSED.to_string())
        };
        let _ = sender.try_send(result);
    });
    let _: () = msg_send![
        class!(SCShareableContent),
        getShareableContentExcludingDesktopWindows: Bool::YES,
        onScreenWindowsOnly: Bool::NO,
        completionHandler: &*block
    ];
    Ok(content
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "读取可录制窗口超时。".to_string())?
        .map_err(|e| format!("无法读取可录制窗口，请确认已在“系统设置 → 隐私与安全性 → 录屏与系统录音”中允许 Snap Desk：{e}"))?
        .0)
}

pub(super) struct WindowCapture {
    stream: Retained<AnyObject>,
    _queue: Retained<AnyObject>,
    shared: Arc<Shared>,
    id: u32,
    pid: u32,
    size: (u32, u32),
    started: bool,
}

impl WindowCapture {
    pub fn new(
        config: &RecorderConfig,
        size: (u32, u32),
        id: u32,
        pid: u32,
        pool: Arc<FramePool>,
    ) -> Result<Self, String> {
        let r = config.region;
        if r.x.checked_add(r.width).is_none_or(|v| v > size.0)
            || r.y.checked_add(r.height).is_none_or(|v| v > size.1)
        {
            return Err("录屏选区需在来源范围内。".into());
        }
        objc2::rc::autoreleasepool(|_| unsafe {
            let window = shareable_window(id)?;
            let allocated: Allocated<AnyObject> = msg_send![class!(SCContentFilter), alloc];
            let filter: Option<Retained<AnyObject>> =
                msg_send![allocated, initWithDesktopIndependentWindow: &*window];
            let filter = filter.ok_or("无法创建窗口录制范围。")?;
            let settings: Option<Retained<AnyObject>> =
                msg_send![class!(SCStreamConfiguration), new];
            let settings = settings.ok_or("无法创建窗口录制配置。")?;
            let _: () = msg_send![&*settings, setWidth: size.0 as usize];
            let _: () = msg_send![&*settings, setHeight: size.1 as usize];
            let _: () = msg_send![&*settings, setPixelFormat: BGRA];
            let fps = config.fps.max(1) as i32;
            let interval = CMTime {
                value: 1,
                timescale: fps,
                flags: 1,
                epoch: 0,
            };
            let _: () = msg_send![&*settings, setMinimumFrameInterval: interval];
            // 与显示器录制一致包含光标（能力中不提供关闭选项）。
            let _: () = msg_send![&*settings, setShowsCursor: Bool::YES];
            let _: () = msg_send![&*settings, setQueueDepth: 5isize];
            let receiver = receiver()?;
            let allocated: Allocated<AnyObject> = msg_send![class!(SCStream), alloc];
            let stream: Option<Retained<AnyObject>> = msg_send![
                allocated,
                initWithFilter: &*filter,
                configuration: &*settings,
                delegate: receiver
            ];
            let stream = stream.ok_or("无法创建窗口录制流。")?;
            let queue = dispatch_queue(c"io.snapdesk.recording.window")?;
            let mut error: Id = std::ptr::null_mut();
            let added: Bool = msg_send![
                &*stream,
                addStreamOutput: receiver,
                type: STREAM_SCREEN,
                sampleHandlerQueue: &*queue,
                error: &mut error as *mut Id
            ];
            if !added.as_bool() {
                return Err(format!("无法注册窗口录制输出：{}", error_text(error)));
            }
            let shared = Arc::new(Shared {
                latest: Mutex::new(None),
                arrived: Condvar::new(),
                failure: Mutex::new(None),
                size,
                region: r,
                pool,
            });
            Ok(Self {
                stream,
                _queue: queue,
                shared,
                id,
                pid,
                size,
                started: false,
            })
        })
    }
}

impl CaptureBackend for WindowCapture {
    fn desktop_origin(&self) -> Result<(i32, i32), String> {
        let geometry = geometry(self.id, self.pid)?;
        Ok((geometry.x, geometry.y))
    }
    fn check_source(&self) -> Result<(), String> {
        if let Some(error) = self.shared.failure.lock().map_err(|e| e.to_string())?.clone() {
            return Err(error);
        }
        let geometry = geometry(self.id, self.pid)?;
        if (geometry.width, geometry.height) != self.size {
            return Err("录制窗口尺寸已改变，录制已结束；请重新选择来源。".into());
        }
        Ok(())
    }
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String> {
        // 与显示器录制一致：第一次拉帧（用户点击开始后）才启动系统采集。
        if !self.started {
            *SINK.lock().map_err(|e| e.to_string())? = Some(self.shared.clone());
            objc2::rc::autoreleasepool(|_| unsafe {
                let (block, done) = completion();
                let _: () = msg_send![&*self.stream, startCaptureWithCompletionHandler: &*block];
                completed(done, Duration::from_secs(10), "无法开始窗口录制")
            })?;
            self.started = true;
        }
        if let Some(error) = self.shared.failure.lock().map_err(|e| e.to_string())?.clone() {
            return Err(error);
        }
        let latest = self.shared.latest.lock().map_err(|e| e.to_string())?.take();
        Ok(latest.map(|(source_time, image)| VideoFrame { source_time, image }))
    }
    fn wait_for_frame(&self, timeout: Duration) {
        let Ok(latest) = self.shared.latest.lock() else {
            return;
        };
        if latest.is_some() {
            return;
        }
        let _ = self.shared.arrived.wait_timeout(latest, timeout);
    }
    fn refresh(&mut self) -> Result<(), String> {
        // 丢弃暂停前的帧；画面静止时系统可能暂不发新帧，由编码端延长上一帧。
        if let Ok(mut latest) = self.shared.latest.lock() {
            latest.take();
        }
        Ok(())
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        if self.started {
            let (block, done) = completion();
            let _: () = unsafe { msg_send![&*self.stream, stopCaptureWithCompletionHandler: &*block] };
            let _ = completed(done, Duration::from_secs(3), "停止窗口录制");
        }
        if let Ok(mut sink) = SINK.lock() {
            if sink.as_ref().is_some_and(|s| Arc::ptr_eq(s, &self.shared)) {
                sink.take();
            }
        }
    }
}
