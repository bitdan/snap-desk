//! 会话状态、采集循环和输出；协调采集、恢复、拼接。窗口与平台集成由外壳通过 [`Host`] 提供。
use super::{
    archive::Archive,
    capture::{self, wait},
    input::Direction,
    matcher::{Bounds, Match, ScrollMode},
    recovery, stability,
    stitcher::Stitch,
};
use crate::capture::Rect;
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize, Serializer};
use std::{
    any::Any,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 来源显示器的物理位置、尺寸、缩放与整体布局；任一变化都结束采集。
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 缩放（f64 位模式，便于精确比较）。
    pub scale: u64,
    pub layout: Vec<(i32, i32, u32, u32, u64)>,
}

/// 空洞默认白底；透明选项仅影响导出，预览始终保留透明覆盖信息。
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExportBackground {
    #[default]
    White,
    Transparent,
}

/// 外壳持有的滚动截图状态；控制命令与采集线程通过它协作。
#[derive(Default)]
pub struct ScrollState {
    pub session: Mutex<Option<ScrollSession>>,
    pub collecting: AtomicBool,
    pub stop: AtomicBool,
    pub cancel: AtomicBool,
}

#[derive(Clone)]
pub struct ScrollSession {
    pub id: u64,
    revision: u64,
    rect: Rect,
    screen: Screen,
    source_size: (u32, u32),
    stitch: Stitch,
    direction: Direction,
    recovery: recovery::Recovery,
    message: String,
    /// 预览缩略图 PNG（最长 360×600），只在可信成图改变时重新编码。
    preview: Arc<[u8]>,
    status: &'static str,
    last_growth: Instant,
    background: ExportBackground,
    // 用户选择“保存已确认部分”时冻结当前可信画布，不再采集结束前的末帧。
    confirmed_only: bool,
    /// 采样前需要暂时隐藏的外壳窗口（遮挡选区时）；内容由外壳解释。
    pub obscuring_windows: Vec<String>,
    archive: Option<Archive>,
}

/// 预览窗与控制条共用的状态快照。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollView {
    pub id: u64,
    pub revision: u64,
    pub collecting: bool,
    pub archive_path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    /// 预览 PNG；序列化为 data URL（无预览时为空字符串）。
    #[serde(serialize_with = "data_url")]
    pub preview: Arc<[u8]>,
    pub frame_height: u32,
    pub canvas_bounds: Bounds,
    pub viewport_rect: Bounds,
    pub coverage_percent: f64,
    pub has_holes: bool,
    pub direction: &'static str,
    pub status: &'static str,
    pub message: String,
}

fn data_url<S: Serializer>(png: &Arc<[u8]>, serializer: S) -> Result<S::Ok, S::Error> {
    if png.is_empty() {
        serializer.serialize_str("")
    } else {
        serializer.serialize_str(&format!("data:image/png;base64,{}", STANDARD.encode(png)))
    }
}

/// 外壳为采集循环提供的窗口与平台能力。
pub trait Host: Send + Sync + 'static {
    fn state(&self) -> &ScrollState;
    /// 与普通截图、录屏准备共用的操作互斥标志。
    fn operation(&self) -> &AtomicBool;
    /// 按物理桌面原点重查来源显示器。
    fn screen(&self, x: i32, y: i32) -> Result<Screen, String>;
    /// 采样期间暂时隐藏遮挡选区的窗口；返回值析构时恢复。
    fn hide_for_sample(&self, id: u64, windows: &[String]) -> Result<Box<dyn Any>, String>;
    /// 状态快照变化（预览、状态文案、覆盖率）。
    fn updated(&self, view: &ScrollView);
    /// 立即隐藏预览、控制条和选区边框。
    fn hide_windows(&self);
    /// 输出失败后重新显示预览和控制条供用户重试。
    fn show_output(&self) -> Result<(), String>;
    fn report_error(&self, message: String);
    /// 结束会话并释放外壳资源（窗口、截图占用）。
    fn close_session(&self, id: u64) -> Result<(), String>;
    /// 成图保存目录。
    fn results_dir(&self) -> Result<PathBuf, String>;
    /// 把 PNG 写入 `root` 并返回最终路径；默认见 [`write_png`]。
    fn publish(&self, root: &Path, png: &[u8]) -> Result<PathBuf, String> {
        write_png(root, png)
    }
    /// 把成图写入系统剪贴板（同一份 PNG 可直接复用）。
    fn copy(&self, image: &RgbaImage, png: &[u8]) -> Result<(), String>;
    /// 保存与复制之后交出成图本身（移交所有权，不再复制一份最多 3200 万像素的画布），
    /// 外壳可据此打开结果窗口继续裁剪、贴图或识别；在 [`Host::finished`] 之前调用，默认丢弃。
    fn output(&self, _image: RgbaImage) {}
    /// 输出完成：`message` 含保存位置与复制结果。
    fn finished(&self, result: &ScrollView, message: String);
}

/// 先写临时文件并落盘，再以不覆盖的方式改名为 `scroll-<毫秒时间戳>.png`。
pub fn write_png(root: &Path, png: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(err)?;
    let mut file = tempfile::Builder::new()
        .prefix(".scroll-")
        .suffix(".png")
        .tempfile_in(root)
        .map_err(err)?;
    file.write_all(png).map_err(err)?;
    file.as_file().sync_all().map_err(err)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(err)?
        .as_millis();
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("scroll-{stamp}.png")
        } else {
            format!("scroll-{stamp}-{attempt}.png")
        };
        let path = root.join(name);
        match file.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => file = e.file,
            Err(e) => return Err(e.error.to_string()),
        }
    }
    Err("滚动截图结果文件名冲突过多。".into())
}

/// 操作互斥的 RAII 守卫。
struct Operation<'a>(&'a AtomicBool);
impl<'a> Operation<'a> {
    fn acquire(flag: &'a AtomicBool) -> Result<Self, String> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "正在处理截图，请稍候。".to_string())?;
        Ok(Self(flag))
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

// 采集时只传图块缩略图，导出仍使用原始像素。
fn view(session: &ScrollSession, collecting: bool) -> Result<ScrollView, String> {
    Ok(ScrollView {
        id: session.id,
        revision: session.revision,
        collecting,
        archive_path: session
            .archive
            .as_ref()
            .map(|a| a.path.to_string_lossy().into_owned()),
        width: session.stitch.width,
        height: session.stitch.height,
        frames: session.stitch.count(),
        frame_height: session.stitch.frame_height,
        canvas_bounds: session.stitch.bounds,
        viewport_rect: session.stitch.viewport(),
        coverage_percent: session.stitch.coverage(),
        has_holes: session.stitch.has_holes(),
        direction: session.stitch.direction(),
        status: session.status,
        preview: if collecting || matches!(session.status, "failed" | "finishing") {
            session.preview.clone()
        } else {
            Arc::from(Vec::new())
        },
        message: session.message.clone(),
    })
}

/// 新会话：校验选区容量并创建（可选）帧存档；`archive_root` 为应用本地数据目录。
pub fn new_session(
    id: u64,
    seed: RgbaImage,
    rect: Rect,
    screen: Screen,
    archive_root: Option<&Path>,
    background: ExportBackground,
) -> Result<ScrollSession, String> {
    let source_size = (screen.width, screen.height);
    // 先校验选区容量，非法选区不创建持久化目录。
    let stitch = Stitch::with_mode(seed, ScrollMode::Auto)?;
    let mut archive = archive_root.and_then(|root| match Archive::new(root) {
        Ok(archive) => Some(archive),
        Err(_error) => {
            log::warn!("event=scroll.archive_failed module=scroll code=ARCHIVE_CREATE_FAILED");
            None
        }
    });
    if let Some(writer) = &archive {
        if let Err(_error) = writer
            .record(serde_json::json!({"event":"session", "version":1,
            "width":rect.width, "height":rect.height, "automatic":false}))
            .and_then(|_| writer.frame(stitch.last_shared()))
        {
            log::warn!("event=scroll.archive_failed module=scroll code=ARCHIVE_INIT_FAILED");
            archive = None;
        }
    }
    let mut session = ScrollSession {
        id,
        revision: 1,
        rect,
        screen,
        obscuring_windows: Vec::new(),
        source_size,
        stitch,
        direction: Direction::default(),
        recovery: recovery::Recovery::default(),
        message: "在选区内慢慢滚动，横向或纵向均可。".into(),
        archive,
        preview: Arc::from(Vec::new()),
        status: "sampling",
        last_growth: Instant::now(),
        background,
        confirmed_only: false,
    };
    refresh_preview(&mut session)?;
    Ok(session)
}

/// 安装会话并复位控制标志；返回首个快照。
pub fn install(state: &ScrollState, session: ScrollSession) -> Result<ScrollView, String> {
    let result = view(&session, true)?;
    *state.session.lock().map_err(err)? = Some(session);
    state.cancel.store(false, Ordering::SeqCst);
    state.stop.store(false, Ordering::SeqCst);
    state.collecting.store(true, Ordering::SeqCst);
    Ok(result)
}

/// 启动失败时撤销安装。
pub fn abort(state: &ScrollState) -> Result<(), String> {
    state.collecting.store(false, Ordering::SeqCst);
    *state.session.lock().map_err(err)? = None;
    Ok(())
}

/// 当前会话快照。
pub fn current(state: &ScrollState) -> Result<Option<ScrollView>, String> {
    let guard = state.session.lock().map_err(err)?;
    guard
        .as_ref()
        .map(|s| view(s, state.collecting.load(Ordering::SeqCst)))
        .transpose()
}

/// 完成按钮与 Ctrl+C 共用停止入口；快捷键在恢复状态只导出已确认画布。
pub fn finish(state: &ScrollState, id: u64, confirmed_only: Option<bool>) -> Result<(), String> {
    let mut guard = state.session.lock().map_err(err)?;
    let session = guard
        .as_mut()
        .filter(|s| s.id == id)
        .ok_or("滚动截图已结束。")?;
    if state.cancel.load(Ordering::SeqCst) {
        return Err("滚动截图已取消。".into());
    }
    session.confirmed_only |= confirmed_only.unwrap_or(session.status == "recovering");
    state.stop.store(true, Ordering::SeqCst);
    Ok(())
}

/// Esc 和取消按钮共用入口；立即隐藏窗口，后台保留停止标志直到在途计算安全退出。
pub fn cancel(host: &dyn Host, id: u64, source: &'static str) -> bool {
    let state = host.state();
    let Ok(guard) = state.session.lock() else {
        return false;
    };
    if !guard.as_ref().is_some_and(|s| s.id == id) {
        return false;
    }
    state.cancel.store(true, Ordering::SeqCst);
    state.stop.store(true, Ordering::SeqCst);
    // 窗口操作不能持会话锁，避免等待 UI 时阻塞采集线程收尾。
    drop(guard);
    log::info!("event=scroll.cancel_requested module=scroll op=scroll-{id} source={source}");
    host.hide_windows();
    true
}

/// 托盘和全局快捷键调用，不阻塞 UI；后台完成当前取样后直接输出成图。
pub fn stop(state: &ScrollState) -> bool {
    if !state.collecting.load(Ordering::SeqCst) {
        return false;
    }
    state.stop.store(true, Ordering::SeqCst);
    true
}

/// 后台直接落盘并复制。匹配不可靠的候选帧不会进入成图，不要求用户修接缝。
fn complete(host: &dyn Host, id: u64, message: &str) -> Result<(), String> {
    let state = host.state();
    // 取消无需分配最多 3200 万像素的连续导出画布。
    if state.cancel.load(Ordering::SeqCst) {
        return host.close_session(id);
    }
    let snapshot = {
        let mut guard = state.session.lock().map_err(err)?;
        let Some(s) = guard.as_mut().filter(|s| s.id == id) else {
            return Ok(());
        };
        s.revision += 1;
        s.message = message.into();
        s.status = "finishing";
        s.clone()
    };
    // 快照只克隆 Arc；整图分配与像素复制不占状态锁。
    let compose_started = Instant::now();
    let mut image = snapshot.stitch.compose();
    if matches!(snapshot.background, ExportBackground::White) {
        for pixel in image.pixels_mut() {
            if pixel[3] == 0 {
                *pixel = image::Rgba([255, 255, 255, 255]);
            }
        }
    }
    let compose_ms = compose_started.elapsed().as_secs_f64() * 1000.0;
    let result = view(&snapshot, false)?;
    if state.cancel.load(Ordering::SeqCst) {
        return host.close_session(id);
    }
    // 同一份 PNG 同时用于落盘和 Windows 剪贴板，长图不重复进行无损编码。
    let png_started = Instant::now();
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).map_err(err)?;
    let png_ms = png_started.elapsed().as_secs_f64() * 1000.0;
    let saved = host
        .results_dir()
        .and_then(|root| host.publish(&root, png.get_ref()));
    let clipboard_started = Instant::now();
    let copied = host.copy(&image, png.get_ref());
    // 实际复制包含 PNG/DIB 准备及系统剪贴板重试；只记录数值，不记录输出路径或图像。
    log::debug!("event=scroll.export module=scroll op=scroll-{id} compose_ms={compose_ms:.2} png_ms={png_ms:.2} clipboard_ms={:.2} rgba_bytes={} width={} height={} frames={}",
        clipboard_started.elapsed().as_secs_f64()*1000.0, image.as_raw().len(), image.width(), image.height(), snapshot.stitch.count());
    // 两种输出都失败时保留内存会话，调用方自动重试；不假报完成。
    if let (Err(save), Err(copy)) = (&saved, &copied) {
        return Err(format!("保存失败：{save}；复制失败：{copy}"));
    }
    let output = match saved {
        Ok(path) => format!(
            "已保存：{}。{}",
            path.display(),
            if copied.is_ok() {
                "已复制到剪贴板。"
            } else {
                "剪贴板不可用，PNG 已保留。"
            }
        ),
        Err(error) => format!("已复制到剪贴板，本地保存失败：{error}"),
    };
    log::info!(
        "event=scroll.finished module=scroll op=scroll-{id} frames={} width={} height={}",
        result.frames,
        result.width,
        result.height
    );
    host.output(image);
    host.close_session(id)?;
    host.finished(&result, format!("{message} {output}"));
    Ok(())
}

/// 调用方持有截图操作锁。结束前的最后取样忽略停止标志，但仍必须通过稳定性检查。
fn sample_pair(host: &dyn Host, id: u64, finishing: bool) -> Result<Option<Match>, String> {
    let state = host.state();
    let mut snapshot = {
        let guard = state.session.lock().map_err(err)?;
        guard
            .as_ref()
            .filter(|s| s.id == id)
            .cloned()
            .ok_or("滚动截图会话已结束。")?
    };
    let revision = snapshot.revision;
    // Windows 以实际滚轮输入决定轴；还没滚动时无需抓屏和匹配。
    #[cfg(windows)]
    if snapshot.direction.mode() == ScrollMode::Auto {
        return Ok(Some(Match::Unchanged));
    }
    snapshot.stitch.lock_axis(snapshot.direction.mode());
    if host.screen(snapshot.screen.x, snapshot.screen.y)? != snapshot.screen {
        return Err("显示器布局、尺寸或 DPI 发生变化，已结束采集，仅输出已确认部分。".into());
    }
    let _preview_guard = host.hide_for_sample(id, &snapshot.obscuring_windows)?;
    let settled = capture::sample_pair(
        state,
        snapshot.rect,
        snapshot.source_size,
        &snapshot.screen,
        finishing,
        &mut |frame| snapshot.recovery.observe_transit(&snapshot.stitch, frame),
    );
    if host.screen(snapshot.screen.x, snapshot.screen.y)? != snapshot.screen {
        return Err("显示器布局已变化，仅输出变化前已确认的部分。".into());
    }
    let before_pixels = snapshot.stitch.covered_pixels();
    let before_bounds = snapshot.stitch.bounds;
    let before_viewport = snapshot.stitch.viewport();
    let result = if let capture::SampleOutcome::Stable(second) = settled {
        let second = Arc::new(second);
        if !stability::stable(snapshot.stitch.last(), &second) {
            if let Some(archive) = &snapshot.archive {
                if let Err(_error) = archive.frame(second.clone()) {
                    log::warn!(
                        "event=scroll.archive_failed module=scroll code=ARCHIVE_FRAME_FAILED"
                    );
                    snapshot.archive = None;
                }
            }
        }
        let started = Instant::now();
        let result = snapshot.recovery.consider(&mut snapshot.stitch, second)?;
        if log::log_enabled!(log::Level::Debug) && super::sample_debug() {
            log::debug!(
                "event=scroll.recovery_match module=scroll duration_ms={:.2}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(archive) = &snapshot.archive {
            if let Err(_error) = archive.record(
                serde_json::json!({"event":"match", "result":format!("{result:?}"),
                "frames":snapshot.stitch.count(), "height":snapshot.stitch.height}),
            ) {
                log::warn!("event=scroll.archive_failed module=scroll code=ARCHIVE_MATCH_FAILED");
                snapshot.archive = None;
            }
        }
        Some(result)
    } else {
        match settled {
            capture::SampleOutcome::CaptureFailure(error) => return Err(error),
            capture::SampleOutcome::Cancelled | capture::SampleOutcome::Stopped => return Ok(None),
            _ => None,
        }
    };
    let canvas_changed = snapshot.stitch.covered_pixels() != before_pixels
        || snapshot.stitch.bounds != before_bounds;
    if canvas_changed {
        snapshot.last_growth = Instant::now();
    }
    let no_progress = snapshot.last_growth.elapsed() >= Duration::from_secs(3);
    let status = if finishing {
        "finishing"
    } else if snapshot.recovery.is_recovering() {
        "recovering"
    } else if no_progress {
        "no_progress"
    } else if result.is_none() {
        "sampling"
    } else {
        "collecting"
    };
    let message = if status == "no_progress" {
        if result.is_none() {
            "页面还在动，请停一下或暂停动画。"
        } else {
            "暂无新增，可继续滚动或点“完成”。"
        }
    } else {
        snapshot.recovery.message(result)
    };
    let changed = snapshot.message != message
        || snapshot.status != status
        || canvas_changed
        || snapshot.stitch.viewport() != before_viewport;
    snapshot.message = message.into();
    snapshot.status = status;
    if canvas_changed {
        refresh_preview(&mut snapshot)?;
    }
    snapshot.revision += 1;
    // 计算期间控制命令可以立即设置停止/取消；过期快照不得覆盖新会话。
    let update = if changed {
        Some(view(&snapshot, true)?)
    } else {
        None
    };
    // 拼接计算不持锁；计算期间改变屏幕布局时，候选快照也不得提交。
    if host.screen(snapshot.screen.x, snapshot.screen.y)? != snapshot.screen {
        return Err("显示器布局已变化，仅输出变化前已确认的部分。".into());
    }
    {
        let mut guard = state.session.lock().map_err(err)?;
        let current = guard
            .as_mut()
            .filter(|s| s.id == id && s.revision == revision)
            .ok_or("滚动截图状态已更新，丢弃过期计算结果。")?;
        // 控制命令和提交共用会话锁：停止后不提交普通采样；部分保存连末帧也不追加。
        if state.cancel.load(Ordering::SeqCst)
            || current.confirmed_only
            || (!finishing && state.stop.load(Ordering::SeqCst))
        {
            return Ok(None);
        }
        *current = snapshot;
    }
    if let Some(update) = update {
        host.updated(&update);
    }
    Ok(result)
}

/// 缩略图仅在可信成图改变时编码；状态查询复用缓存，不反复编码 PNG。
fn refresh_preview(session: &mut ScrollSession) -> Result<(), String> {
    let started = Instant::now();
    let mut bytes = Cursor::new(Vec::new());
    session
        .stitch
        .preview(360, 600)
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(err)?;
    session.preview = Arc::from(bytes.into_inner());
    if log::log_enabled!(log::Level::Debug) && super::sample_debug() {
        log::debug!(
            "event=scroll.preview module=scroll duration_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

/// 锁争用也发布状态；以会话 ID 和 revision 防止旧结果覆盖新会话。
fn notify_waiting(host: &dyn Host, id: u64, finishing: bool) {
    let Ok(mut guard) = host.state().session.lock() else {
        return;
    };
    let Some(session) = guard.as_mut().filter(|s| s.id == id) else {
        return;
    };
    let status = if finishing { "finishing" } else { "waiting" };
    let message = if finishing {
        "正在等待当前操作完成后保存。"
    } else {
        "正在处理其他截图，请稍候。"
    };
    if session.status == status && session.message == message {
        return;
    }
    session.status = status;
    session.message = message.into();
    session.revision += 1;
    let update = view(session, true);
    drop(guard);
    if let Ok(update) = update {
        host.updated(&update);
    }
    log::warn!("event=scroll.operation_wait module=scroll op=scroll-{id} finishing={finishing}");
}

/// 输出失败只冻结可信画布；保留预览供用户重试，不再采集新的桌面内容。
fn failed_output(session: &mut ScrollSession, error: &str) -> Result<ScrollView, String> {
    session.status = "failed";
    session.message = format!("输出失败，结果仍保留：{error}");
    session.confirmed_only = true;
    session.revision += 1;
    view(session, false)
}

fn alive(state: &ScrollState, id: u64) -> bool {
    state
        .session
        .lock()
        .ok()
        .is_some_and(|s| s.as_ref().is_some_and(|s| s.id == id))
}

/// 失败等待期间不持操作锁，取消或全局清理仍能释放会话；完成按钮沿用停止信号重试。
fn wait_for_output_action(state: &ScrollState, id: u64) -> bool {
    loop {
        if !alive(state, id) {
            return false;
        }
        if state.cancel.load(Ordering::SeqCst) || state.stop.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn collect(host: &dyn Host, id: u64, first_delay: u64, allow_copy_retry: impl Fn()) {
    let state = host.state();
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut message = "滚动截图采集结束。".to_string();
    let mut finish_latest = true;
    let mut delay = first_delay;
    let mut last_operation = Instant::now();
    while finish_latest && wait(state, delay) {
        delay = 40;
        if Instant::now() >= deadline {
            message = "采集达到 5 分钟上限，已自动结束。".into();
            finish_latest = false;
            break;
        }
        let Ok(_operation) = Operation::acquire(host.operation()) else {
            if last_operation.elapsed() >= Duration::from_secs(1) {
                notify_waiting(host, id, false);
            }
            continue;
        };
        last_operation = Instant::now();
        let decision = (|| -> Result<(), String> {
            sample_pair(host, id, false)?;
            if state.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            // 不确定匹配只退避重采，可信锚点始终保留，不按重试次数结束。
            let guard = state.session.lock().map_err(err)?;
            if let Some(session) = guard.as_ref().filter(|s| s.id == id) {
                delay = session.recovery.delay();
            }
            Ok(())
        })();
        if let Err(error) = decision {
            message = error;
            finish_latest = false;
            break;
        }
    }
    // 等待短暂的读取操作释放互斥；旧会话不得弹出窗口或覆盖新会话。
    loop {
        // 即使操作锁长期占用，旧线程也不应永久等待或污染新会话。
        if !alive(state, id) {
            break;
        }
        let Ok(_operation) = Operation::acquire(host.operation()) else {
            if last_operation.elapsed() >= Duration::from_secs(1) {
                notify_waiting(host, id, true);
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        // 旧采集线程不得关闭新会话的预览或重置其状态。
        if !alive(state, id) {
            break;
        }
        if state.cancel.load(Ordering::SeqCst) {
            let _ = host.close_session(id);
            break;
        }
        let confirmed_only = state.session.lock().ok().is_some_and(|s| {
            s.as_ref()
                .is_some_and(|s| s.id == id && (s.confirmed_only || s.recovery.is_recovering()))
        });
        if confirmed_only && !message.ends_with(" 仅输出已确认部分；未匹配画面未追加。")
        {
            message.push_str(" 仅输出已确认部分；未匹配画面未追加。");
        }
        if finish_latest && !confirmed_only {
            // 等待托盘菜单收起；采集原窗口的最终静止画面，避免刚滚动完就结束而漏掉末尾。
            std::thread::sleep(Duration::from_millis(180));
            match sample_pair(host, id, true) {
                Ok(Some(Match::Uncertain)) => {
                    message = "末帧无法可靠拼接，已自动排除，仅输出已确认部分。".into()
                }
                Ok(Some(_)) => {}
                Ok(None) => message = "结束时画面仍在运动，末帧未追加，仅输出已确认部分。".into(),
                Err(error) => message = error,
            }
        }
        if state.cancel.load(Ordering::SeqCst) {
            let _ = host.close_session(id);
            break;
        }
        host.hide_windows();
        state.collecting.store(false, Ordering::SeqCst);
        state.stop.store(true, Ordering::SeqCst);
        let mut failure = None;
        for _ in 0..3 {
            match complete(host, id, &message) {
                Ok(()) => {
                    failure = None;
                    break;
                }
                Err(error) => {
                    failure = Some(error);
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        if let Some(error) = failure {
            log::error!(
                "event=scroll.failed module=scroll op=scroll-{id} code=SCROLL_OUTPUT_FAILED"
            );
            let update = if let Ok(mut guard) = state.session.lock() {
                if let Some(session) = guard.as_mut().filter(|s| s.id == id) {
                    // 与控制命令共用会话锁，先清掉旧完成信号，再开放下一次重试。
                    state.stop.store(false, Ordering::SeqCst);
                    failed_output(session, &error).ok()
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(update) = update {
                allow_copy_retry();
                host.updated(&update);
                if let Err(window_error) = host.show_output() {
                    host.report_error(window_error);
                }
            }
            // 用户的 Esc 可能在重新显示窗口期间到达，必须恢复其立即隐藏的效果。
            if state.cancel.load(Ordering::SeqCst) {
                host.hide_windows();
            }
            host.report_error(format!("滚动截图输出重试失败，内存结果仍保留：{error}"));
            finish_latest = false;
            drop(_operation);
            if wait_for_output_action(state, id) {
                continue;
            }
        }
        break;
    }
}

/// 安装滚轮/Esc/Ctrl+C 监听并启动采集线程。监听器随线程结束释放。
pub fn launch(host: Arc<dyn Host>, id: u64, first_delay: u64) -> Result<(), String> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    let listener = {
        let guard = host.state().session.lock().map_err(err)?;
        let s = guard
            .as_ref()
            .filter(|s| s.id == id)
            .ok_or("滚动截图会话已结束。")?;
        let x = i64::from(s.screen.x) + i64::from(s.rect.x);
        let y = i64::from(s.screen.y) + i64::from(s.rect.y);
        let direction = s.direction.clone();
        let rect = s.rect;
        drop(guard);
        let cancel_host = host.clone();
        let finish_host = host.clone();
        super::input::Listener::start(
            direction,
            (x, y, x + i64::from(rect.width), y + i64::from(rect.height)),
            move || cancel(cancel_host.as_ref(), id, "escape"),
            move || finish(finish_host.state(), id, None).is_ok(),
        )?
    };
    std::thread::Builder::new()
        .name("scroll-capture".into())
        .spawn(move || {
            #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
            let allow_copy_retry = || listener.allow_copy_retry();
            #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
            let allow_copy_retry = || {};
            collect(host.as_ref(), id, first_delay, allow_copy_retry)
        })
        .map(|_| ())
        .map_err(err)
}

#[cfg(test)]
mod output_tests {
    use super::*;

    fn session(id: u64) -> ScrollSession {
        ScrollSession {
            id,
            revision: 4,
            rect: Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 64,
            },
            screen: Screen {
                name: None,
                x: 0,
                y: 0,
                width: 100,
                height: 100,
                scale: 1.0f64.to_bits(),
                layout: Vec::new(),
            },
            source_size: (100, 100),
            stitch: Stitch::new(image::RgbaImage::new(32, 64)).unwrap(),
            direction: Direction::default(),
            recovery: recovery::Recovery::default(),
            message: String::new(),
            preview: Arc::from(b"trusted".to_vec()),
            status: "finishing",
            last_growth: Instant::now(),
            background: ExportBackground::White,
            confirmed_only: false,
            obscuring_windows: Vec::new(),
            archive: None,
        }
    }

    #[test]
    fn failed_output_freezes_trusted_canvas_and_keeps_preview_for_retry() {
        let mut session = session(7);
        let pixels = session.stitch.covered_pixels();
        let result = failed_output(&mut session, "磁盘不可写").unwrap();
        assert_eq!(result.status, "failed");
        assert_eq!(result.revision, 5);
        assert!(!result.collecting);
        assert_eq!(&*result.preview, b"trusted");
        assert!(result.message.contains("磁盘不可写"));
        assert!(session.confirmed_only);
        assert_eq!(session.stitch.covered_pixels(), pixels);
        // IPC 仍是 data URL。
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(
            json["preview"],
            format!("data:image/png;base64,{}", STANDARD.encode(b"trusted"))
        );
    }

    #[test]
    fn output_wait_observes_retry_cancel_and_replaced_sessions() {
        let state = ScrollState::default();
        *state.session.lock().unwrap() = Some(session(7));
        state.stop.store(true, Ordering::SeqCst);
        assert!(wait_for_output_action(&state, 7));
        state.stop.store(false, Ordering::SeqCst);
        state.cancel.store(true, Ordering::SeqCst);
        assert!(wait_for_output_action(&state, 7));
        // 即便上一会话的标志还未复位，也不得重试或清理替换后的会话。
        *state.session.lock().unwrap() = Some(session(8));
        assert!(!wait_for_output_action(&state, 7));
        *state.session.lock().unwrap() = None;
        assert!(!wait_for_output_action(&state, 7));
    }

    #[test]
    fn written_results_never_overwrite_each_other() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("scroll-results");
        let first = write_png(&root, b"one").unwrap();
        let second = write_png(&root, b"two").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        // 临时文件不留在结果目录。
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    }

    #[test]
    fn finish_and_stop_respect_cancel_and_collecting() {
        let state = ScrollState::default();
        assert!(!stop(&state));
        *state.session.lock().unwrap() = Some(session(3));
        state.collecting.store(true, Ordering::SeqCst);
        assert!(stop(&state));
        assert!(finish(&state, 4, None).is_err());
        finish(&state, 3, Some(true)).unwrap();
        assert!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .confirmed_only
        );
        state.cancel.store(true, Ordering::SeqCst);
        assert!(finish(&state, 3, None).is_err());
    }
}
