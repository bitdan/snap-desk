//! 录屏后的轻量编辑：仅接受原生结果 ID，以只读媒体协议预览，另存时间片段。
//! MP4 交给 Windows 系统媒体组件，GIF 按帧流式裁剪；不载入整个录像、不改写原文件。
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::SystemTime,
};
use tauri::{
    http::{Request, Response, StatusCode},
    AppHandle, Manager, WebviewWindow,
};
use tauri_plugin_dialog::DialogExt;

#[cfg(windows)]
#[path = "editing_windows.rs"]
mod windows;

/// 主界面和录屏回放复用同一受限编辑能力，其他窗口不能访问媒体文件。
fn require_editor_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "recording-controls") {
        Ok(())
    } else {
        Err("当前窗口无权编辑录屏。".into())
    }
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone)]
struct MediaFile {
    path: PathBuf,
    length: u64,
    modified: SystemTime,
}
impl MediaFile {
    fn new(path: PathBuf) -> Result<Self, String> {
        crate::recovery::no_links(&path)?;
        let meta = fs::metadata(&path).map_err(error)?;
        if !meta.is_file() || meta.len() == 0 {
            return Err("录屏文件为空或已不可用。".into());
        }
        Ok(Self {
            path,
            length: meta.len(),
            modified: meta.modified().map_err(error)?,
        })
    }
    /// 打开后核对同一文件句柄的元数据，避免把已替换/仍在改写的文件当作原录像。
    fn open(&self) -> Result<File, String> {
        crate::recovery::no_links(&self.path)?;
        let file = File::open(&self.path).map_err(error)?;
        let meta = file.metadata().map_err(error)?;
        if meta.len() != self.length || meta.modified().map_err(error)? != self.modified {
            return Err("录屏文件已被修改，请关闭预览后重新打开。".into());
        }
        Ok(file)
    }
}
#[derive(Clone)]
struct Source {
    token: u64,
    file: MediaFile,
    format: String,
    duration_ms: u64,
    output: Option<MediaFile>,
}
#[derive(Default)]
struct Editor {
    owner: Option<String>,
    generation: u64,
    preparing: Option<u64>,
    source: Option<Source>,
    active: Option<(u64, Arc<AtomicBool>)>,
}
#[derive(Default)]
pub struct EditState(Mutex<Editor>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorView {
    token: u64,
    format: String,
    duration_ms: u64,
    preview_available: bool,
}

/// 原生解析最近结果并读取真实时长；前端提供的文件路径和时长不作为依据。
#[tauri::command]
pub async fn prepare_recording_editor(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
) -> Result<EditorView, String> {
    require_editor_window(&window)?;
    let path = super::result::source_path(&app, id)?;
    let token = {
        let state = app.state::<EditState>();
        let mut editor = state.0.lock().map_err(error)?;
        if editor.active.is_some() || editor.preparing.is_some() {
            return Err("请先等待当前录屏读取或裁剪结束。".into());
        }
        editor.owner = Some(window.label().to_string());
        editor.generation += 1;
        editor.source = None;
        editor.preparing = Some(editor.generation);
        editor.generation
    };
    let guard = PrepareGuard(app.clone(), token);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let file = MediaFile::new(path)?;
        let format = file
            .path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or("录屏格式无效。")?
            .to_ascii_lowercase();
        let duration_ms = if format == "gif" {
            gif_duration(file.open()?)?
        } else if format == "mp4" {
            mp4_duration(&file.path)?
        } else {
            return Err("仅支持 MP4 / GIF 录屏。".into());
        };
        file.open()?;
        if duration_ms < 20 {
            return Err("录屏过短，无法裁剪。".into());
        }
        let preview_available = format != "gif" || file.length <= MAX_FULL_RESPONSE;
        let state = app.state::<EditState>();
        let mut editor = state.0.lock().map_err(error)?;
        if editor.generation != token {
            return Err("预览请求已过期。".into());
        }
        editor.source = Some(Source {
            token,
            file,
            format: format.clone(),
            duration_ms,
            output: None,
        });
        Ok(EditorView {
            token,
            format,
            duration_ms,
            preview_available,
        })
    })
    .await
    .map_err(error)?
}

/// 取消标志由 GIF 帧循环和系统转码轮询观察；不在 UI 线程等待媒体编码。
#[tauri::command]
pub fn cancel_recording_trim(
    app: AppHandle,
    window: WebviewWindow,
    token: u64,
) -> Result<(), String> {
    require_editor_window(&window)?;
    let state = app.state::<EditState>();
    let editor = state.0.lock().map_err(error)?;
    if let Some((id, cancel)) = &editor.active {
        if *id == token {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}
#[tauri::command]
pub fn close_recording_editor(
    app: AppHandle,
    window: WebviewWindow,
    token: u64,
) -> Result<(), String> {
    cancel_recording_trim(app.clone(), window, token)?;
    let state = app.state::<EditState>();
    let mut editor = state.0.lock().map_err(error)?;
    if editor.source.as_ref().is_some_and(|s| s.token == token) {
        editor.source = None;
    }
    // 关闭先于 metadata 查询完成到达时，同样让该请求失效。
    if editor.generation == token {
        editor.generation += 1;
    }
    Ok(())
}
/// WebView 销毁不会执行 Vue 卸载回调，原生侧主动取消属于该窗口的编辑任务。
/// 主窗口使用同一编辑能力，必须核对 owner，不能误清理另一个窗口的媒体。
pub(crate) fn release_window(app: &AppHandle, label: &str) {
    if let Ok(mut editor) = app.state::<EditState>().0.lock() {
        if editor.owner.as_deref() == Some(label) {
            if let Some((_, cancel)) = &editor.active { cancel.store(true, Ordering::SeqCst); }
            editor.generation += 1;
            editor.owner = None;
            editor.source = None;
        }
    }
}
pub fn shutdown(app: &AppHandle) {
    if let Ok(editor) = app.state::<EditState>().0.lock() {
        if let Some((_, cancel)) = &editor.active {
            cancel.store(true, Ordering::SeqCst);
        }
    };
}
/// 限制同时只有一次媒体时长扫描；关闭后重开超大 GIF 也不会堆积多个解码任务。
struct PrepareGuard(AppHandle, u64);
impl Drop for PrepareGuard {
    fn drop(&mut self) {
        if let Ok(mut editor) = self.0.state::<EditState>().0.lock() {
            if editor.preparing == Some(self.1) {
                editor.preparing = None;
            }
        }
    }
}
struct JobGuard(AppHandle, u64);
impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Ok(mut editor) = self.0.state::<EditState>().0.lock() {
            if editor.active.as_ref().is_some_and(|(id, _)| *id == self.1) {
                editor.active = None;
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrimRange {
    start_ms: u64,
    end_ms: u64,
}
impl TrimRange {
    fn validate(&self, duration: u64, format: &str) -> Result<(), String> {
        if self.start_ms >= self.end_ms
            || self.end_ms > duration
            || self.end_ms - self.start_ms < 20
        {
            return Err(
                "裁剪范围无效：结束时间须大于开始时间，至少保留 0.02 秒且不能超过原片时长。".into(),
            );
        }
        if format == "gif" && (self.start_ms % 10 != 0 || self.end_ms % 10 != 0) {
            return Err("GIF 时间须精确到 0.01 秒。".into());
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimExport {
    path: String,
    duration_ms: u64,
    preview_available: bool,
}

/// 只向不存在的新文件发布；即使保存对话框确认覆盖，也不允许覆盖原片或其他文件。
fn validate_destination(source: &Path, target: &Path, format: &str) -> Result<(), String> {
    crate::recovery::no_links(target)?;
    if target == source
        || target
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
            != Some(format)
    {
        return Err("请另存为同格式的新文件，不能覆盖原录屏。".into());
    }
    if fs::symlink_metadata(target).is_ok() {
        return Err("目标文件已存在，请选择新的文件名。".into());
    }
    if !target.parent().is_some_and(Path::is_dir) {
        return Err("目标目录已不可用。".into());
    }
    Ok(())
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("裁剪已取消，原录屏保留。".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn trim_recording(
    app: AppHandle,
    window: WebviewWindow,
    token: u64,
    range: TrimRange,
) -> Result<Option<TrimExport>, String> {
    require_editor_window(&window)?;
    let (source, cancel) = {
        let state = app.state::<EditState>();
        let mut editor = state.0.lock().map_err(error)?;
        if editor.active.is_some() {
            return Err("已有裁剪正在进行。".into());
        }
        let source = editor
            .source
            .as_ref()
            .filter(|s| s.token == token)
            .ok_or("预览已关闭或过期。")?
            .clone();
        range.validate(source.duration_ms, &source.format)?;
        let cancel = Arc::new(AtomicBool::new(false));
        editor.active = Some((token, cancel.clone()));
        (source, cancel)
    };
    let guard = JobGuard(app.clone(), token);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        source.file.open()?;
        check_cancel(&cancel)?;
        let stem = source
            .file
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(crate::shared::native_i18n::text(
                &app,
                "另存裁剪片段（不覆盖已有文件）",
            ))
            .add_filter(source.format.to_uppercase(), &[source.format.as_str()])
            .set_directory(source.file.path.parent().ok_or("原录屏目录无效。")?)
            .set_file_name(format!("{stem}-trimmed.{}", source.format))
            .blocking_save_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        check_cancel(&cancel)?;
        let target = selected.into_path().map_err(error)?;
        validate_destination(&source.file.path, &target, &source.format)?;
        let mut output = crate::recovery::TrackedFile::new(
            &app,
            target.parent().ok_or("目标目录无效。")?,
            &source.format,
            "录屏裁剪片段",
        )?;
        if source.format == "gif" {
            trim_gif(source.file.open()?, output.path(), &range, &cancel)?;
        } else {
            trim_mp4(&source.file.path, output.path(), &range, &cancel)?;
        }
        check_cancel(&cancel)?;
        source.file.open()?;
        output.finalize()?;
        // 发布与关闭/取消在同一状态锁内裁决；发布失败由恢复中心保留完整片段供取回。
        let state = app.state::<EditState>();
        let mut editor = state.0.lock().map_err(error)?;
        check_cancel(&cancel)?;
        let current = editor
            .source
            .as_mut()
            .filter(|s| s.token == token)
            .ok_or("编辑器已关闭，原录屏保留。")?;
        output.publish(&target, false)?;
        current.output = MediaFile::new(target.clone()).ok();
        let preview_available = current
            .output
            .as_ref()
            .is_some_and(|file| source.format != "gif" || file.length <= MAX_FULL_RESPONSE);
        Ok(Some(TrimExport {
            path: target.to_string_lossy().into_owned(),
            duration_ms: range.end_ms - range.start_ms,
            preview_available,
        }))
    })
    .await
    .map_err(error)?
}

#[cfg(windows)]
fn mp4_duration(path: &Path) -> Result<u64, String> {
    windows::duration(path)
}
#[cfg(not(windows))]
fn mp4_duration(_: &Path) -> Result<u64, String> {
    Err("当前平台尚未接入 MP4 裁剪。".into())
}
#[cfg(windows)]
fn trim_mp4(
    input: &Path,
    output: &Path,
    range: &TrimRange,
    cancel: &AtomicBool,
) -> Result<(), String> {
    windows::trim(input, output, range, cancel)
}
#[cfg(not(windows))]
fn trim_mp4(_: &Path, _: &Path, _: &TrimRange, _: &AtomicBool) -> Result<(), String> {
    Err("当前平台尚未接入 MP4 裁剪。".into())
}

/// 本应用 GIF 每帧完整且不透明，可直接保留调色板；遇到外部改写的增量帧安全拒绝。
fn validate_gif_frame(frame: &gif::Frame<'_>, width: u16, height: u16) -> Result<(), String> {
    if frame.left != 0
        || frame.top != 0
        || frame.width != width
        || frame.height != height
        || frame.transparent.is_some()
        || frame.delay == 0
        || !matches!(
            frame.dispose,
            gif::DisposalMethod::Keep | gif::DisposalMethod::Any
        )
    {
        return Err("GIF 包含不支持的增量/透明帧，请使用本应用完整导出的 GIF。".into());
    }
    Ok(())
}
fn gif_duration(file: File) -> Result<u64, String> {
    let mut decoder = gif::DecodeOptions::new().read_info(file).map_err(error)?;
    let (w, h) = (decoder.width(), decoder.height());
    let mut duration = 0u64;
    while let Some(frame) = decoder.read_next_frame().map_err(error)? {
        validate_gif_frame(frame, w, h)?;
        duration = duration
            .checked_add(u64::from(frame.delay) * 10)
            .ok_or("GIF 时长溢出。")?;
    }
    Ok(duration)
}
fn trim_gif(
    file: File,
    output: &Path,
    range: &TrimRange,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut decoder = gif::DecodeOptions::new().read_info(file).map_err(error)?;
    let (w, h) = (decoder.width(), decoder.height());
    let palette = decoder.global_palette().unwrap_or(&[]).to_vec();
    let file = File::create(output).map_err(error)?;
    let mut encoder =
        gif::Encoder::new(std::io::BufWriter::new(file), w, h, &palette).map_err(error)?;
    encoder.set_repeat(gif::Repeat::Infinite).map_err(error)?;
    let mut time = 0u64;
    let mut written = 0u64;
    while let Some(frame) = decoder.read_next_frame().map_err(error)? {
        check_cancel(cancel)?;
        validate_gif_frame(frame, w, h)?;
        let end = time
            .checked_add(u64::from(frame.delay) * 10)
            .ok_or("GIF 时长溢出。")?;
        let overlap = end
            .min(range.end_ms)
            .saturating_sub(time.max(range.start_ms));
        if overlap > 0 {
            let mut frame = frame.clone();
            frame.delay = u16::try_from(overlap / 10).map_err(error)?;
            encoder.write_frame(&frame).map_err(error)?;
            written += overlap;
        }
        time = end;
        if time >= range.end_ms {
            break;
        }
    }
    if written != range.end_ms - range.start_ms {
        return Err("GIF 时长与裁剪范围不匹配。".into());
    }
    encoder.into_inner().map_err(error)?.flush().map_err(error)
}

const MAX_RANGE_RESPONSE: u64 = 2 * 1024 * 1024;
const MAX_FULL_RESPONSE: u64 = 64 * 1024 * 1024;
/// 浏览器寻址请求单段读取；拒绝多区间，避免任意内存分配及整数溢出。
fn byte_range(header: Option<&str>, length: u64) -> Result<(u64, u64, bool), ()> {
    if length == 0 {
        return Err(());
    }
    let Some(header) = header else {
        return Ok((0, length - 1, false));
    };
    let value = header.strip_prefix("bytes=").ok_or(())?;
    let (start, end) = value.split_once('-').ok_or(())?;
    let (start, end) = if start.is_empty() {
        let count = end.parse::<u64>().map_err(|_| ())?;
        if count == 0 {
            return Err(());
        }
        (length.saturating_sub(count), length - 1)
    } else {
        let start = start.parse::<u64>().map_err(|_| ())?;
        let end = if end.is_empty() {
            length - 1
        } else {
            end.parse::<u64>().map_err(|_| ())?.min(length - 1)
        };
        (start, end)
    };
    if start >= length || end < start {
        return Err(());
    }
    Ok((
        start,
        end.min(start.saturating_add(MAX_RANGE_RESPONSE - 1)),
        true,
    ))
}
fn response(status: StatusCode, data: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff")
        .body(data)
        .expect("fixed protocol headers")
}
/// 只读协议仅对主界面/录屏回放 WebView 的已打开编辑器生效，URL 中没有文件路径或目录通配权限。
pub fn preview(app: &AppHandle, main: bool, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if !main {
        return response(StatusCode::FORBIDDEN, vec![]);
    }
    if !matches!(request.method().as_str(), "GET" | "HEAD") {
        return response(StatusCode::METHOD_NOT_ALLOWED, vec![]);
    }
    let parts: Vec<_> = request.uri().path().split('/').collect();
    if parts.len() != 3 || !parts[1].bytes().all(|b| b.is_ascii_digit()) {
        return response(StatusCode::NOT_FOUND, vec![]);
    }
    let Ok(token) = parts[1].parse::<u64>() else {
        return response(StatusCode::NOT_FOUND, vec![]);
    };
    let selected = (|| -> Result<(MediaFile, String), String> {
        let state = app.state::<EditState>();
        let editor = state.0.lock().map_err(error)?;
        let source = editor
            .source
            .as_ref()
            .filter(|s| s.token == token)
            .ok_or("stale")?;
        let file = match parts[2] {
            "source" => source.file.clone(),
            "output" => source.output.clone().ok_or("missing")?,
            _ => return Err("invalid".into()),
        };
        Ok((file, source.format.clone()))
    })();
    let Ok((source, format)) = selected else {
        return response(StatusCode::NOT_FOUND, vec![]);
    };
    let Ok(mut file) = source.open() else {
        return response(StatusCode::GONE, vec![]);
    };
    let range = request.headers().get("Range");
    let range = match range.map(|v| v.to_str()).transpose() {
        Ok(v) => v,
        Err(_) => return response(StatusCode::RANGE_NOT_SATISFIABLE, vec![]),
    };
    let Ok((start, end, partial)) = byte_range(range, source.length) else {
        let mut result = response(StatusCode::RANGE_NOT_SATISFIABLE, vec![]);
        result.headers_mut().insert(
            "Content-Range",
            format!("bytes */{}", source.length).parse().unwrap(),
        );
        return result;
    };
    let count = end - start + 1;
    if !partial && count > MAX_FULL_RESPONSE && request.method() != "HEAD" {
        return response(StatusCode::PAYLOAD_TOO_LARGE, vec![]);
    }
    let mut data = vec![];
    if request.method() != "HEAD" {
        data.resize(count as usize, 0);
        if file
            .seek(SeekFrom::Start(start))
            .and_then(|_| file.read_exact(&mut data))
            .is_err()
        {
            return response(StatusCode::GONE, vec![]);
        }
    }
    let mut result = response(
        if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        },
        data,
    );
    let headers = result.headers_mut();
    headers.insert(
        "Content-Type",
        if format == "gif" {
            "image/gif"
        } else {
            "video/mp4"
        }
        .parse()
        .unwrap(),
    );
    headers.insert("Accept-Ranges", "bytes".parse().unwrap());
    headers.insert("Content-Length", count.to_string().parse().unwrap());
    if partial {
        headers.insert(
            "Content-Range",
            format!("bytes {start}-{end}/{}", source.length)
                .parse()
                .unwrap(),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_validate_duration_minimum_and_gif_precision() {
        for range in [
            TrimRange {
                start_ms: 20,
                end_ms: 40,
            },
            TrimRange {
                start_ms: 0,
                end_ms: 1000,
            },
        ] {
            assert!(range.validate(1000, "gif").is_ok());
        }
        for range in [
            TrimRange {
                start_ms: 1,
                end_ms: 21,
            },
            TrimRange {
                start_ms: 0,
                end_ms: 19,
            },
            TrimRange {
                start_ms: 100,
                end_ms: 100,
            },
            TrimRange {
                start_ms: 1001,
                end_ms: 1100,
            },
            TrimRange {
                start_ms: u64::MAX,
                end_ms: 0,
            },
        ] {
            assert!(range.validate(1000, "gif").is_err());
        }
        assert!(TrimRange {
            start_ms: 1,
            end_ms: 21
        }
        .validate(1000, "mp4")
        .is_ok());
    }
    #[test]
    fn http_ranges_are_bounded_and_reject_invalid_or_multi_ranges() {
        assert_eq!(byte_range(None, 100), Ok((0, 99, false)));
        assert_eq!(byte_range(Some("bytes=10-20"), 100), Ok((10, 20, true)));
        assert_eq!(byte_range(Some("bytes=90-999"), 100), Ok((90, 99, true)));
        assert_eq!(byte_range(Some("bytes=-10"), 100), Ok((90, 99, true)));
        assert_eq!(
            byte_range(Some("bytes=0-"), 100_000_000),
            Ok((0, MAX_RANGE_RESPONSE - 1, true))
        );
        for value in [
            "bytes=100-",
            "bytes=20-10",
            "bytes=-0",
            "bytes=0-1,5-6",
            "bytes=",
            "bytes=0-18446744073709551616",
            "items=0-1",
        ] {
            assert!(byte_range(Some(value), 100).is_err(), "{value}");
        }
        assert!(byte_range(None, 0).is_err());
    }
    #[test]
    fn original_and_existing_destinations_are_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("source.mp4");
        fs::write(&original, b"original").unwrap();
        let file = MediaFile::new(original.clone()).unwrap();
        assert!(file.open().is_ok());
        assert!(validate_destination(&original, &original, "mp4").is_err());
        assert!(validate_destination(&original, &root.path().join("clip.gif"), "mp4").is_err());
        let target = root.path().join("new.mp4");
        assert!(validate_destination(&original, &target, "mp4").is_ok());
        fs::write(&target, b"existing").unwrap();
        assert!(validate_destination(&original, &target, "mp4").is_err());
        fs::write(&original, b"modified-length").unwrap();
        assert!(file.open().is_err());
        assert_eq!(fs::read(&target).unwrap(), b"existing");
    }
    #[test]
    fn publish_race_retains_finalized_clip_and_existing_target() {
        let root = tempfile::tempdir().unwrap();
        let catalog = tempfile::tempdir().unwrap();
        let original = root.path().join("source.mp4");
        fs::write(&original, b"original").unwrap();
        let target = root.path().join("new.mp4");
        validate_destination(&original, &target, "mp4").unwrap();
        let mut pending =
            crate::recovery::TrackedFile::new_in(catalog.path(), root.path(), "mp4", "clip")
                .unwrap();
        pending.file_mut().write_all(b"encoded clip").unwrap();
        pending.finalize().unwrap();
        let recovery_path = pending.path().to_path_buf();
        fs::write(&target, b"race winner").unwrap();
        assert!(pending.publish(&target, false).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"original");
        assert_eq!(fs::read(&target).unwrap(), b"race winner");
        assert_eq!(fs::read(recovery_path).unwrap(), b"encoded clip");
    }
    #[test]
    fn gif_trims_both_boundaries_without_reencoding_palette_or_changing_original() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.gif");
        let output = root.path().join("clip.gif");
        let palette = [255, 0, 0, 0, 0, 255, 0, 255, 0];
        {
            let mut encoder =
                gif::Encoder::new(File::create(&source).unwrap(), 2, 2, &palette).unwrap();
            for color in 0..3u8 {
                let frame = gif::Frame {
                    width: 2,
                    height: 2,
                    delay: 10,
                    dispose: gif::DisposalMethod::Keep,
                    buffer: std::borrow::Cow::Owned(vec![color; 4]),
                    ..Default::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        let original = fs::read(&source).unwrap();
        assert_eq!(gif_duration(File::open(&source).unwrap()).unwrap(), 300);
        let cancel = AtomicBool::new(false);
        trim_gif(
            File::open(&source).unwrap(),
            &output,
            &TrimRange {
                start_ms: 50,
                end_ms: 250,
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(gif_duration(File::open(&output).unwrap()).unwrap(), 200);
        let mut decoder = gif::DecodeOptions::new()
            .read_info(File::open(&output).unwrap())
            .unwrap();
        for (index, delay) in [5, 10, 5].into_iter().enumerate() {
            let frame = decoder.read_next_frame().unwrap().unwrap();
            assert_eq!(frame.delay, delay);
            assert_eq!(frame.buffer.as_ref(), &[index as u8; 4]);
        }
        assert!(decoder.read_next_frame().unwrap().is_none());
        assert_eq!(fs::read(&source).unwrap(), original);
        cancel.store(true, Ordering::SeqCst);
        assert!(trim_gif(
            File::open(&source).unwrap(),
            &output,
            &TrimRange {
                start_ms: 0,
                end_ms: 100
            },
            &cancel
        )
        .is_err());
        assert_eq!(fs::read(&source).unwrap(), original);
    }
    #[test]
    fn gif_rejects_unsupported_delta_frames_instead_of_corrupting_the_clip() {
        let frame = gif::Frame {
            width: 2,
            height: 2,
            left: 1,
            delay: 10,
            ..Default::default()
        };
        assert!(validate_gif_frame(&frame, 2, 2).is_err());
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires Windows media components; opt in for native integration"]
    fn windows_precise_trim_reopens_output_and_preserves_original() {
        use super::super::{
            config::{EncodingPreference, OutputFormat},
            encoder,
            frame::BgraImage,
        };
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.mp4");
        let output = root.path().join("extensionless-output");
        let mut writer = encoder::create(
            &source,
            64,
            64,
            30,
            1_000_000,
            EncodingPreference::Software,
            OutputFormat::Mp4,
            false,
        )
        .unwrap();
        for index in 0..60 {
            writer
                .push(&BgraImage::new(64, 64), index * 10_000_000 / 30)
                .unwrap();
        }
        writer.finish(20_000_000).unwrap();
        drop(writer);
        let original = fs::read(&source).unwrap();
        File::create(&output).unwrap();
        trim_mp4(
            &source,
            &output,
            &TrimRange {
                start_ms: 500,
                end_ms: 1500,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let duration = mp4_duration(&output).unwrap();
        assert!((950..=1050).contains(&duration));
        assert_eq!(fs::read(&source).unwrap(), original);
    }
}
