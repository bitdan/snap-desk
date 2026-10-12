//! 无声录屏：复用系统主屏采集，逐帧落盘，停止后调用系统原生视频编码 API。
//! 使用有界磁盘缓存；实际取样时间换算为媒体时间戳，慢机器不会快放。
use super::*;
use std::{fs, path::Path};
#[path = "recording_encoder.rs"]
mod encoder;
use super::scroll_capture::chrome;

const MAX_SECONDS: f64 = 180.0;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FRAMES: usize = 1800;

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Ready,
    Recording,
    Paused,
    Encoding,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingView {
    id: u64,
    revision: u64,
    phase: Phase,
    seconds: f64,
    frames: usize,
    message: String,
    path: Option<String>,
}

/// 暂停累计时长使用单调时钟；状态锁也使暂停与当前帧取样互斥。
struct Session {
    view: RecordingView,
    elapsed: Duration,
    running_since: Option<Instant>,
    stop: bool,
    cancel: bool,
}
impl Session {
    fn elapsed(&self) -> Duration {
        self.elapsed + self.running_since.map(|t| t.elapsed()).unwrap_or_default()
    }
    fn freeze(&mut self) {
        self.elapsed = self.elapsed();
        self.running_since = None;
        self.view.seconds = self.elapsed.as_secs_f64();
    }
}

#[derive(Default)]
pub struct RecordingState(Mutex<Option<Session>>, Mutex<()>, AtomicBool);

fn active(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Ready | Phase::Recording | Phase::Paused | Phase::Encoding
    )
}

/// 托盘、截图快捷键和退出入口只发停止信号，清理始终由录屏工作线程完成。
pub fn request_stop(app: &AppHandle, cancel: bool) -> bool {
    let state = app.state::<RecordingState>();
    let Ok(mut guard) = state.0.lock() else {
        return false;
    };
    let Some(s) = guard.as_mut().filter(|s| active(s.view.phase)) else {
        return false;
    };
    s.freeze();
    s.stop = true;
    s.cancel |= cancel || s.view.phase == Phase::Ready;
    s.view.revision += 1;
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingAction {
    Start,
    Pause,
    Resume,
    Stop,
    Cancel,
}

/// 悬浮条暂停保留现场，托盘暂停可展开主页；恢复仅隐藏主页，边框和控制条不收起。
pub fn control(app: &AppHandle, id: Option<u64>, action: RecordingAction) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    // 控制动作串行，但窗口操作不得持有状态锁，避免 UI 查询状态时相互等待。
    let _control = state.1.lock().map_err(err)?;
    let mut guard = state.0.lock().map_err(err)?;
    let s = guard
        .as_mut()
        .filter(|s| active(s.view.phase) && id.is_none_or(|id| s.view.id == id))
        .ok_or("没有正在进行的录屏。")?;
    match action {
        RecordingAction::Pause
            if matches!(s.view.phase, Phase::Recording | Phase::Paused) && !s.stop =>
        {
            s.freeze();
            s.view.phase = Phase::Paused;
            s.view.message = "录屏已暂停，可恢复或停止导出。".into();
            s.view.revision += 1;
            drop(guard);
            // 悬浮条暂停不展开主窗口，录制范围及控制条持续可见；托盘仍可打开主页。
            if id.is_none() {
                show_main(app);
                let _ = app.emit_to("main", "capture-result", "录屏已暂停，可恢复或停止导出。");
            }
            return Ok(());
        }
        RecordingAction::Start | RecordingAction::Resume
            if !s.stop
                && ((matches!(action, RecordingAction::Start) && s.view.phase == Phase::Ready)
                    || (matches!(action, RecordingAction::Resume)
                        && s.view.phase == Phase::Paused)) =>
        {
            let session_id = s.view.id;
            let expected_phase = s.view.phase;
            drop(guard);
            if let Some(main) = app.get_webview_window("main") {
                main.hide().map_err(err)?;
            }
            // 此时仍是待开始/暂停状态；窗口完全移除后才开启计时和取样。
            std::thread::sleep(Duration::from_millis(250));
            let mut guard = state.0.lock().map_err(err)?;
            let s = guard
                .as_mut()
                .filter(|s| s.view.id == session_id && s.view.phase == expected_phase && !s.stop)
                .ok_or("录屏已停止。")?;
            s.running_since = Some(Instant::now());
            s.view.phase = Phase::Recording;
            s.view.message = "录屏中；托盘可暂停，截图快捷键可停止。".into();
            s.view.revision += 1;
            return Ok(());
        }
        RecordingAction::Stop | RecordingAction::Cancel => {
            s.freeze();
            s.stop = true;
            s.cancel |= matches!(action, RecordingAction::Cancel) || s.view.phase == Phase::Ready;
        }
        _ => return Err("当前录屏状态不支持此操作。".into()),
    }
    s.view.revision += 1;
    Ok(())
}

#[tauri::command]
pub async fn control_recording(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    action: RecordingAction,
) -> Result<(), String> {
    require_control_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || control(&app, Some(id), action))
        .await
        .map_err(err)?
}

#[tauri::command]
pub fn current_recording(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<RecordingView>, String> {
    require_control_window(&window)?;
    let state = app.state::<RecordingState>();
    let guard = state.0.lock().map_err(err)?;
    if window.label() == "recording-controls" {
        state.2.store(true, Ordering::SeqCst);
    }
    Ok(guard.as_ref().map(|s| {
        let mut view = s.view.clone();
        view.seconds = s.elapsed().as_secs_f64();
        view
    }))
}

fn require_control_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "recording-controls") {
        Ok(())
    } else {
        Err("当前窗口无权控制录屏。".into())
    }
}

fn validate_region(rect: Rect, size: (u32, u32)) -> Result<(), String> {
    rect.validate(size.0, size.1)?;
    if rect.width < 2
        || rect.height < 2
        || u64::from(rect.width) * u64::from(rect.height) > 16_000_000
    {
        return Err("录屏选区至少为 2×2，最多 1600 万像素。".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
) -> Result<(), String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.lock().map_err(err)?.is_some() {
            return Err("请先结束长截图。".into());
        }
        let size = state
            .session
            .lock()
            .map_err(err)?
            .as_ref()
            .filter(|s| s.frame.id == id)
            .map(|s| s.image.dimensions())
            .ok_or("截图已过期。")?;
        validate_region(rect, size)?;
        let root = app
            .path()
            .app_local_data_dir()
            .map_err(err)?
            .join("recordings");
        fs::create_dir_all(&root).map_err(err)?;
        encoder::probe(&root, rect.width, rect.height)?;
        let frames = tempfile::Builder::new()
            .prefix("frames-")
            .tempdir_in(&root)
            .map_err(err)?;
        let screen = super::scroll_capture::screen(&app)?;
        chrome::place_recording(&app, &screen, rect)?;
        window.hide().map_err(err)?;
        let recording = app.state::<RecordingState>();
        *recording.0.lock().map_err(err)? = Some(Session {
            view: RecordingView {
                id,
                revision: 1,
                phase: Phase::Ready,
                seconds: 0.0,
                frames: 0,
                message: "录屏选区已准备，请点击下方“开始录制”。".into(),
                path: None,
            },
            elapsed: Duration::ZERO,
            running_since: None,
            stop: false,
            cancel: false,
        });
        recording.2.store(false, Ordering::SeqCst);
        let ready = chrome::show_recording(&app).and_then(|_| {
            // 只有控制页挂载并成功查询原生命令才继续，防止空白工具窗不可操作。
            let deadline = Instant::now() + Duration::from_secs(5);
            while !recording.2.load(Ordering::SeqCst) {
                if Instant::now() >= deadline {
                    return Err("录屏按钮页未成功加载，请重启应用后重试。".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        });
        if let Err(error) = ready {
            chrome::hide_recording(&app);
            *recording.0.lock().map_err(err)? = None;
            let _ = window.show();
            return Err(error);
        }
        let worker_app = app.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("screen-recording".into())
            .spawn(move || {
                collect(worker_app, id, rect, size, screen, frames);
            })
        {
            chrome::hide_recording(&app);
            *recording.0.lock().map_err(err)? = None;
            let _ = window.show();
            return Err(err(e));
        }
        // 将选区会话所有权交给录屏；active 保持 true，阻止截图/长截图抢占屏幕。
        *state.session.lock().map_err(err)? = None;
        Ok(())
    })
    .await
    .map_err(err)?
}

fn collect(
    app: AppHandle,
    id: u64,
    rect: Rect,
    size: (u32, u32),
    screen: super::scroll_capture::Screen,
    frames: tempfile::TempDir,
) {
    let state = app.state::<RecordingState>();
    let mut times = Vec::new();
    let mut bytes = 0;
    let mut warning = String::new();
    // 蒙层消失后才取第一帧，不把设置弹窗写入视频。
    std::thread::sleep(Duration::from_millis(250));
    let result = (|| -> Result<Option<String>, String> {
        loop {
            let tick = Instant::now();
            // 显示器查询可能派发到 UI 线程，必须在状态锁外执行。
            let current_screen = super::scroll_capture::screen(&app)?;
            {
                let mut guard = state.0.lock().map_err(err)?;
                let s = guard.as_mut().ok_or("录屏会话丢失。")?;
                if s.cancel {
                    return Ok(None);
                }
                if s.stop {
                    break;
                }
                if s.view.phase == Phase::Recording {
                    if s.running_since.is_none() {
                        s.running_since = Some(Instant::now());
                    }
                    if s.elapsed().as_secs_f64() >= MAX_SECONDS
                        || times.len() >= MAX_FRAMES
                        || bytes >= MAX_BYTES
                    {
                        warning = "达到录制上限，已自动停止。".into();
                        s.freeze();
                        break;
                    }
                    if current_screen != screen {
                        warning = "显示器位置、尺寸或 DPI 改变，已停止。".into();
                        s.freeze();
                        break;
                    }
                    // 在状态锁内取样：暂停返回时，后台已不会继续抓取主窗口。
                    let time = s.elapsed().as_secs_f64();
                    let image = crate::platform::capture_primary()?;
                    if image.dimensions() != size {
                        return Err("显示器像素尺寸改变，请重新录屏。".into());
                    }
                    let crop =
                        image::imageops::crop_imm(&image, rect.x, rect.y, rect.width, rect.height)
                            .to_image();
                    let mut encoded = Cursor::new(Vec::new());
                    crop.write_to(&mut encoded, ImageFormat::Png).map_err(err)?;
                    let data = encoded.into_inner();
                    if bytes + data.len() as u64 > MAX_BYTES {
                        warning = "达到 512 MiB 帧缓存上限，已自动停止。".into();
                        s.freeze();
                        break;
                    }
                    fs::write(frames.path().join(format!("{:06}.png", times.len())), &data)
                        .map_err(err)?;
                    bytes += data.len() as u64;
                    times.push(time);
                    s.view.frames = times.len();
                    s.view.revision += 1;
                }
            }
            // 目标 10 fps；取样慢时不追赶补帧，实际时间戳保证播放时长。
            std::thread::sleep(Duration::from_millis(100).saturating_sub(tick.elapsed()));
        }
        let end = {
            let mut guard = state.0.lock().map_err(err)?;
            let s = guard.as_mut().ok_or("录屏会话丢失。")?;
            if s.cancel {
                return Ok(None);
            }
            s.freeze();
            s.view.phase = Phase::Encoding;
            s.view.message = "录屏已停止，正在编码 MP4…".into();
            s.view.revision += 1;
            s.view.seconds
        };
        show_main(&app);
        let _ = app.emit_to("main", "capture-result", "录屏已停止，正在编码 MP4…");
        if times.is_empty() {
            return Err("录制时间过短，没有采集到画面。".into());
        }
        let samples = encoder::sample_times(&times, end)?;
        fs::write(
            frames.path().join("timing.json"),
            serde_json::to_vec(&samples).map_err(err)?,
        )
        .map_err(err)?;
        encode(&app, frames.path(), &samples)
    })();
    let cancelled = state
        .0
        .lock()
        .ok()
        .is_some_and(|s| s.as_ref().is_some_and(|s| s.cancel));
    let (phase, message, path) = match result {
        Ok(Some(path)) => (
            Phase::Completed,
            format!("录屏已保存：{path}。{warning}"),
            Some(path),
        ),
        Ok(None) => (Phase::Cancelled, "录屏已取消，临时帧已清理。".into(), None),
        Err(_) if cancelled => (Phase::Cancelled, "录屏已取消，临时帧已清理。".into(), None),
        Err(error) => {
            // 编码/磁盘错误保留可恢复帧；不把错误输出当作成功的 MP4。
            let path = frames.keep();
            (
                Phase::Failed,
                format!(
                    "录屏失败：{error} 原始帧保留于 {}，可手动恢复或清理。",
                    path.display()
                ),
                None,
            )
        }
    };
    if let Ok(mut guard) = state.0.lock() {
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.freeze();
            s.view.phase = phase;
            s.view.message = message.clone();
            s.view.path = path;
            s.view.revision += 1;
        }
    }
    // 录屏接管后已移除截图 session，只需释放全局占用，不恢复旧的静态蒙层。
    chrome::hide_recording(&app);
    app.state::<CaptureState>()
        .active
        .store(false, Ordering::SeqCst);
    let _ = app.emit_to("main", "capture-result", &message);
    show_main(&app);
}

fn encode(
    app: &AppHandle,
    frames: &Path,
    samples: &[(i64, i64)],
) -> Result<Option<String>, String> {
    let output = tempfile::Builder::new()
        .prefix("recording-")
        .suffix(".mp4")
        .tempfile_in(frames.parent().ok_or("录屏目录不可用。")?)
        .map_err(err)?;
    // 先关闭临时文件句柄，再让系统 Sink Writer 打开，避免共享访问冲突。
    let output = output.into_temp_path();
    encoder::encode(&output, frames, samples, || {
        app.state::<RecordingState>()
            .0
            .lock()
            .map(|s| s.as_ref().is_none_or(|s| s.cancel))
            .unwrap_or(true)
    })?;
    // 完成之后才发布路径；临时 MP4 失败/取消时由 RAII 删除。
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&output)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    if file.metadata().map_err(err)?.len() == 0 {
        return Err("编码器未生成视频。".into());
    }
    let state = app.state::<RecordingState>();
    let mut guard = state.0.lock().map_err(err)?;
    if guard.as_ref().is_none_or(|s| s.cancel) {
        return Ok(None);
    }
    drop(file);
    let path = output.keep().map_err(err)?;
    // 与取消串行发布：一旦文件正式保留，后续取消不能再回复“已取消”。
    if let Some(s) = guard.as_mut() {
        s.view.phase = Phase::Completed;
    }
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_outside_and_oversized_regions() {
        for rect in [
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 20,
            },
            Rect {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 2,
            },
            Rect {
                x: 0,
                y: 0,
                width: 5000,
                height: 4000,
            },
        ] {
            assert!(validate_region(rect, (8000, 8000)).is_err());
        }
        assert!(validate_region(
            Rect {
                x: 3,
                y: 5,
                width: 101,
                height: 99
            },
            (1920, 1080)
        )
        .is_ok());
    }
    #[test]
    fn paused_time_does_not_grow() {
        let mut session = Session {
            view: RecordingView {
                id: 1,
                revision: 1,
                phase: Phase::Recording,
                seconds: 0.0,
                frames: 0,
                message: String::new(),
                path: None,
            },
            elapsed: Duration::from_secs(2),
            running_since: Some(Instant::now() - Duration::from_secs(3)),
            stop: false,
            cancel: false,
        };
        session.freeze();
        assert!(session.elapsed().as_secs_f64() >= 5.0);
        let elapsed = session.elapsed();
        assert_eq!(session.elapsed(), elapsed);
    }
}
