//! Windows 自带 MediaComposition 的精确时间裁剪；音视频作为同一片段重新封装/编码。
//! 所有 WinRT 对象只在 MTA 阻塞线程内使用，析构顺序保证对象先于运行时释放。
use super::{check_cancel, TrimRange};
use ::windows::{
    core::HSTRING,
    Foundation::TimeSpan,
    Media::{
        Editing::{MediaClip, MediaComposition, MediaTrimmingPreference},
        MediaProperties::MediaEncodingProfile,
        Transcoding::TranscodeFailureReason,
    },
    Storage::StorageFile,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
fn native_error(e: ::windows::core::Error) -> String {
    format!("Windows 媒体处理失败：{e}。请检查媒体组件、文件是否被占用及磁盘空间；Windows N 版本需启用 Media Feature Pack。")
}
struct Runtime;
impl Runtime {
    fn new() -> Result<Self, String> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED).map_err(native_error)?;
        }
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
fn storage(path: &Path) -> Result<StorageFile, String> {
    StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))
        .map_err(native_error)?
        .join()
        .map_err(native_error)
}
pub(super) fn duration(path: &Path) -> Result<u64, String> {
    let _runtime = Runtime::new()?;
    let file = storage(path)?;
    let clip = MediaClip::CreateFromFileAsync(&file)
        .map_err(native_error)?
        .join()
        .map_err(native_error)?;
    let ticks = clip.OriginalDuration().map_err(native_error)?.Duration;
    u64::try_from(ticks / 10_000).map_err(|_| "录屏媒体时长无效。".into())
}
pub(super) fn trim(
    input: &Path,
    output: &Path,
    range: &TrimRange,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let _runtime = Runtime::new()?;
    check_cancel(cancel)?;
    let input = storage(input)?;
    let clip = MediaClip::CreateFromFileAsync(&input)
        .map_err(native_error)?
        .join()
        .map_err(native_error)?;
    let duration = clip.OriginalDuration().map_err(native_error)?.Duration;
    let start = i64::try_from(range.start_ms)
        .ok()
        .and_then(|n| n.checked_mul(10_000))
        .ok_or("开始时间溢出。")?;
    let end = i64::try_from(range.end_ms)
        .ok()
        .and_then(|n| n.checked_mul(10_000))
        .filter(|end| *end <= duration)
        .ok_or("结束时间超过原片。")?;
    clip.SetTrimTimeFromStart(TimeSpan { Duration: start })
        .map_err(native_error)?;
    clip.SetTrimTimeFromEnd(TimeSpan {
        Duration: duration - end,
    })
    .map_err(native_error)?;
    let composition = MediaComposition::new().map_err(native_error)?;
    composition
        .Clips()
        .map_err(native_error)?
        .Append(&clip)
        .map_err(native_error)?;
    // 复用原 MP4 的 H.264/AAC 编码参数，音轨跟随同一时间片段，避免只裁视频导致错位。
    let profile = MediaEncodingProfile::CreateFromFileAsync(&input)
        .map_err(native_error)?
        .join()
        .map_err(native_error)?;
    let destination = storage(output)?;
    check_cancel(cancel)?;
    let operation = composition
        .RenderToFileWithProfileAsync(&destination, MediaTrimmingPreference::Precise, &profile)
        .map_err(native_error)?;
    let mut cancel_sent = false;
    // 等待异步操作进入终态再销毁临时文件；Cancel 是系统协作取消，不能强抢驱动内部调用。
    while operation.Status().map_err(native_error)?.0 == 0 {
        if cancel.load(Ordering::SeqCst) && !cancel_sent {
            operation.Cancel().map_err(native_error)?;
            cancel_sent = true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let result = operation.join();
    check_cancel(cancel)?;
    let reason = result.map_err(native_error)?;
    if reason != TranscodeFailureReason::None {
        return Err(format!(
            "Windows 无法完成此录屏的裁剪（原因 {}），原录屏已保留。",
            reason.0
        ));
    }
    Ok(())
}
