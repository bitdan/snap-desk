//! 录制中的标注：桌面浮层只负责交互，编码帧使用各自持有的不可变标注快照。
//! 绘制时仅选区和浮动工具栏接收输入，展示时整窗穿透；标注快照由编码线程写入 MP4/GIF。
#[path = "live_annotation_regions.rs"]
mod input_regions;
use super::annotation_track::TimedAnnotation;
use super::{annotation_layer::Layer, Phase, RecordingState};
use crate::{
    capture::{scroll::Screen, Rect},
    shared::annotations::{self, Annotation, Point},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

const LABEL: &str = "capture";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationView {
    id: u64,
    revision: u64,
    width: u32,
    height: u32,
    viewport_width: u32,
    viewport_height: u32,
    x: i32,
    y: i32,
    editing: bool,
    media_time_ms: u64,
    tools: AnnotationTools,
    editor: AnnotationEditorStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tracks: Vec<TimedAnnotation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shapes: Vec<Annotation>,
}
impl AnnotationView {
    fn status(&self, shapes: bool) -> Self {
        Self {
            id: self.id,
            revision: self.revision,
            width: self.width,
            height: self.height,
            viewport_width: self.viewport_width,
            viewport_height: self.viewport_height,
            x: self.x,
            y: self.y,
            editing: self.editing,
            media_time_ms: self.media_time_ms,
            tools: self.tools.clone(),
            editor: self.editor.clone(),
            tracks: if shapes {
                self.tracks.clone()
            } else {
                Vec::new()
            },
            shapes: if shapes {
                self.shapes.clone()
            } else {
                Vec::new()
            },
        }
    }
}
struct Session {
    view: AnnotationView,
    screen: Screen,
    region: Rect,
    layer: Arc<Layer>,
    history: Vec<TimedAnnotation>,
    current_ids: Vec<String>,
}
/// 共享截图工具栏的只读状态；撤销栈和选中图形仍只由画布中的 useAnnotations 持有。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationEditorStatus {
    ready: bool,
    selected: i32,
    pending: bool,
    interacting: bool,
    can_undo: bool,
    can_redo: bool,
    text: String,
    error: String,
}
impl Default for AnnotationEditorStatus {
    fn default() -> Self {
        Self {
            ready: false,
            selected: -1,
            pending: false,
            interacting: false,
            can_undo: false,
            can_redo: false,
            text: String::new(),
            error: String::new(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AnnotationEditorAction {
    Tool { tool: String },
    Style { style: AnnotationStyle },
    Undo,
    Redo,
    Remove,
    Clear,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    magnification: Option<f64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnnotationEditorRequest {
    id: u64,
    request_id: u64,
    action: AnnotationEditorAction,
}

/// 小控制窗持有绘图偏好；画布通过快照同步，工具切换不依赖窗口裁剪。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationTools {
    pub tool: String,
    pub color: String,
    pub width: f64,
    pub opacity: f64,
    pub font_size: f64,
    pub font_family: String,
    pub magnification: f64,
    pub lifetime_ms: u64,
    pub fade_out_ms: u64,
}
impl Default for AnnotationTools {
    fn default() -> Self {
        Self {
            tool: "pen".into(),
            color: "#ef4444".into(),
            width: 3.0,
            opacity: 1.0,
            font_size: 24.0,
            font_family: "sans".into(),
            magnification: 2.0,
            lifetime_ms: 0,
            fade_out_ms: 500,
        }
    }
}
impl AnnotationTools {
    fn validate(&self) -> Result<(), String> {
        if ![
            "edit",
            "eraser",
            "arrow-text",
            "rectangle",
            "arrow",
            "ellipse",
            "line",
            "polyline",
            "pen",
            "highlighter",
            "text",
            "number",
            "mosaic",
            "blur",
            "spotlight",
            "magnify",
            "erase",
        ]
        .contains(&self.tool.as_str())
            || self.color.len() != 7
            || !self.color.starts_with('#')
            || !self.color[1..].bytes().all(|b| b.is_ascii_hexdigit())
            || !self.width.is_finite()
            || !(0.5..=64.0).contains(&self.width)
            || !self.opacity.is_finite()
            || !(0.05..=1.0).contains(&self.opacity)
            || !self.font_size.is_finite()
            || !(8.0..=160.0).contains(&self.font_size)
            || !["sans", "serif", "mono"].contains(&self.font_family.as_str())
            || !self.magnification.is_finite()
            || !(2.0..=4.0).contains(&self.magnification)
            || ![0, 3000, 5000].contains(&self.lifetime_ms)
            || self.fade_out_ms > 1000
        {
            return Err("标注工具参数无效。".into());
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct LiveAnnotationState {
    editor_actions:
        Mutex<std::collections::HashMap<u64, (u64, std::sync::mpsc::Sender<Result<(), String>>)>>,
    next_editor_action: std::sync::atomic::AtomicU64,
    session: Mutex<Option<Session>>,
    write: Mutex<()>,
    /// 只保存外部窗口句柄值，避免 HWND 的线程约束传播到全局状态。
    desktop_focus: Mutex<Option<isize>>,
}

/// 启动时将冻结桌面中的标注转换为视频输出局部坐标；之后窗口移动也不会改变标注的归属。
pub(super) fn initialize(
    app: &AppHandle,
    id: u64,
    screen: &Screen,
    region: Rect,
    origin: (i32, i32),
    mut shapes: Vec<Annotation>,
    annotation_origin: Point,
) -> Result<(), String> {
    for shape in &mut shapes {
        for point in std::iter::once(&mut shape.start)
            .chain(std::iter::once(&mut shape.end))
            .chain(shape.points.iter_mut())
        {
            point.x = (point.x - annotation_origin.x).clamp(0.0, f64::from(region.width));
            point.y = (point.y - annotation_origin.y).clamp(0.0, f64::from(region.height));
        }
    }
    // 完全在录制区域外的面状标注没有可绘制像素。
    shapes.retain(|s| {
        !matches!(
            s.kind,
            annotations::Kind::Text
                | annotations::Kind::Number
                | annotations::Kind::Mosaic
                | annotations::Kind::Blur
                | annotations::Kind::Magnify
                | annotations::Kind::Spotlight
        ) || ((s.end.x - s.start.x).abs() >= 1.0 && (s.end.y - s.start.y).abs() >= 1.0)
    });
    annotations::validate_live(&shapes, region.width, region.height)?;
    if shapes
        .iter()
        .filter(|s| annotations::dynamic_filter(s))
        .count()
        > 8
    {
        return Err("录制中最多支持 8 个实时滤镜。".into());
    }
    let mut history = Vec::new();
    let current_ids = super::annotation_track::reconcile(&mut history, &[], &shapes, 0, 0, 0)?;
    let layer = Arc::new(Layer::timed(history.clone(), region.width, region.height)?);
    let state = app.state::<LiveAnnotationState>();
    *state.session.lock().map_err(|e| e.to_string())? = Some(Session {
        view: AnnotationView {
            id,
            revision: 0,
            width: region.width,
            height: region.height,
            viewport_width: screen.width,
            viewport_height: screen.height,
            x: origin.0 + region.x as i32 - screen.x,
            y: origin.1 + region.y as i32 - screen.y,
            editing: false,
            media_time_ms: 0,
            tools: AnnotationTools::default(),
            editor: AnnotationEditorStatus::default(),
            tracks: history.clone(),
            shapes,
        },
        screen: screen.clone(),
        region,
        layer,
        history,
        current_ids,
    });
    Ok(())
}
pub(super) fn has_timed_annotations(app: &AppHandle, id: u64) -> bool {
    let ms = media_time(app).unwrap_or(0);
    app.state::<LiveAnnotationState>()
        .session
        .lock()
        .ok()
        .is_some_and(|current| {
            current.as_ref().is_some_and(|s| {
                s.view.id == id
                    && s.view
                        .tracks
                        .iter()
                        .any(|t| t.end_ms.is_some_and(|end| ms <= end.saturating_add(100)))
            })
        })
}
pub(super) fn snapshot(app: &AppHandle, id: u64) -> (u64, Option<Arc<Layer>>) {
    app.state::<LiveAnnotationState>()
        .session
        .lock()
        .ok()
        .and_then(|s| {
            s.as_ref()
                .filter(|s| s.view.id == id)
                .map(|s| (s.view.revision, Some(s.layer.clone())))
        })
        .unwrap_or((0, None))
}
/// 编辑工具栏的按键、点击不产生录像中的输入特效，避免输入文字时泄露无关按键提示。
pub(super) fn editing(app: &AppHandle, id: u64) -> bool {
    app.state::<LiveAnnotationState>()
        .session
        .lock()
        .ok()
        .is_some_and(|s| {
            s.as_ref()
                .is_some_and(|s| s.view.id == id && s.view.editing)
        })
}
pub(super) fn update_origin(app: &AppHandle, id: u64, origin: (i32, i32)) {
    if let Ok(mut session) = app.state::<LiveAnnotationState>().session.lock() {
        if let Some(s) = session.as_mut().filter(|s| s.view.id == id) {
            s.view.x = origin.0 + s.region.x as i32 - s.screen.x;
            s.view.y = origin.1 + s.region.y as i32 - s.screen.y;
        }
    }
}
pub(super) fn clear(app: &AppHandle, id: u64) {
    let state = app.state::<LiveAnnotationState>();
    let Ok(_write) = state.write.lock() else {
        return;
    };
    let matches = state
        .session
        .lock()
        .map(|mut session| {
            if session.as_ref().is_some_and(|s| s.view.id == id) {
                *session = None;
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if !matches {
        return;
    }
    hide_interaction(app);
    let _ = app.emit_to(LABEL, "recording-annotations-closed", id);
}
/// 停止时立即收起交互层，封装队列仍保留各帧自己的标注快照。
pub(crate) fn hide_interaction(app: &AppHandle) {
    if let Ok(mut actions) = app.state::<LiveAnnotationState>().editor_actions.lock() {
        for (_, (_, sender)) in actions.drain() {
            let _ = sender.send(Err("标注交互已结束。".into()));
        }
    }
    if let Ok(mut current) = app.state::<LiveAnnotationState>().session.lock() {
        if let Some(session) = current.as_mut() {
            session.view.editing = false;
        }
    }
    restore_desktop_focus(app);
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Err(error) = window.set_ignore_cursor_events(true) {
            log::warn!("event=recording.overlay_passthrough_failed module=recording error={error}");
        }
        if let Err(error) = window.hide() {
            log::warn!("event=recording.overlay_hide_failed module=recording error={error}");
        }
        if let Err(error) = input_regions::reset(&window) {
            log::warn!(
                "event=recording.overlay_region_reset_failed module=recording error={error}"
            );
        }
        // 同步查询确认隐藏已经处理；停止路径尽力收尾，异常留给诊断日志。
        match window.is_visible() {
            Ok(false) => {}
            status => log::warn!(
                "event=recording.overlay_still_visible module=recording status={status:?}"
            ),
        }
    }
}
fn require_editing(app: &AppHandle, id: u64) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    let session = state.0.lock().map_err(|e| e.to_string())?;
    if session.as_ref().is_some_and(|s| {
        s.view.id == id
            && !s.stop
            && !s.cancel
            && matches!(
                s.view.phase,
                Phase::Ready | Phase::Countdown | Phase::Recording | Phase::Paused
            )
    }) {
        Ok(())
    } else {
        Err("录制已结束或尚未开始，无法修改录制标注。".into())
    }
}

/// 前端按实际视口比例转为物理像素；首区域为标注栏，录制控制紧邻其上方，组成框下的两行。
/// 原生补上选区，拒绝越界和旧会话的迟到布局。
#[tauri::command]
pub async fn set_recording_annotation_regions(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    regions: Vec<Rect>,
) -> Result<(), String> {
    crate::capture::require_window(&window, LABEL)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LiveAnnotationState>();
        let _write = state.write.lock().map_err(|e| e.to_string())?;
        require_editing(&app, id)?;
        let current = state.session.lock().map_err(|e| e.to_string())?;
        let session = current
            .as_ref()
            .filter(|s| s.view.id == id)
            .ok_or("录屏标注会话已过期。")?;
        // 完成标注后不允许迟到的 ResizeObserver 重新扩展窗口。
        if !session.view.editing {
            return Ok(());
        }
        if regions.len() > 12 {
            return Err("录屏标注交互区域过多。".into());
        }
        let mut rectangles = vec![Rect {
            x: session.view.x.max(0) as u32,
            y: session.view.y.max(0) as u32,
            width: session.view.width,
            height: session.view.height,
        }];
        let toolbar = regions.first().copied();
        for region in regions {
            rectangles.push(region.validate(session.screen.width, session.screen.height)?);
        }
        let screen = session.screen.clone();
        drop(current);
        input_regions::set(&window, &rectangles)?;
        if let Some(toolbar) = toolbar {
            crate::capture::scroll::chrome::place_recording_controls_above_toolbar(
                &app, &screen, toolbar,
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn current_recording_annotations(
    app: AppHandle,
    window: WebviewWindow,
    known_revision: Option<u64>,
) -> Result<Option<AnnotationView>, String> {
    if !matches!(window.label(), "capture" | "recording-controls") {
        return Err("当前窗口无权读取录制标注。".into());
    }
    let ms = media_time(&app)?;
    let state = app.state::<LiveAnnotationState>();
    let view = state
        .session
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|s| s.view.status(known_revision != Some(s.view.revision)));
    Ok(view
        .filter(|view| require_editing(&app, view.id).is_ok())
        .map(|mut view| {
            view.media_time_ms = ms;
            view
        }))
}

/// 绘制时画布和浮动工具栏接收输入；完成后整窗穿透，录制控制仍由独立小窗口承担。
#[tauri::command]
pub async fn set_recording_annotation_mode(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    editing: bool,
) -> Result<(), String> {
    if !matches!(window.label(), "capture" | "recording-controls") {
        return Err("当前窗口无权修改录制标注。".into());
    }
    tauri::async_runtime::spawn_blocking(move || apply_mode(&app, id, editing))
        .await
        .map_err(|e| e.to_string())?
}

/// 使用录屏 Timeline 的媒体 PTS；Ready/暂停期间时间不推进。
fn media_time(app: &AppHandle) -> Result<u64, String> {
    let state = app.state::<RecordingState>();
    let current = state.0.lock().map_err(|e| e.to_string())?;
    Ok(current.as_ref().map_or(0, |s| {
        s.timeline.elapsed(super::capture::now()).max(0) as u64 / 10_000
    }))
}

#[tauri::command]
pub async fn set_recording_annotation_tools(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    tools: AnnotationTools,
) -> Result<(), String> {
    if !matches!(window.label(), "capture" | "recording-controls") {
        return Err("当前窗口无权修改录制标注工具。".into());
    }
    tools.validate()?;
    tauri::async_runtime::spawn_blocking(move || {
        require_editing(&app, id)?;
        let state = app.state::<LiveAnnotationState>();
        let _write = state.write.lock().map_err(|e| e.to_string())?;
        let mut current = state.session.lock().map_err(|e| e.to_string())?;
        let session = current
            .as_mut()
            .filter(|s| s.view.id == id)
            .ok_or("录屏标注会话已过期。")?;
        session.view.tools = tools;
        drop(current);
        super::notify_changed(&app);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 控制窗不再维护第二个编辑器，只把截图工具栏的动作交给原画布执行并等待确认。
#[tauri::command]
pub async fn recording_annotation_editor_action(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    action: AnnotationEditorAction,
) -> Result<(), String> {
    crate::capture::require_window(&window, "recording-controls")?;
    tauri::async_runtime::spawn_blocking(move || {
        require_editing(&app, id)?;
        let state = app.state::<LiveAnnotationState>();
        {
            let current = state.session.lock().map_err(|e| e.to_string())?;
            let session = current
                .as_ref()
                .filter(|s| s.view.id == id)
                .ok_or("录屏标注会话已过期。")?;
            if !session.view.editor.ready {
                return Err("标注画布尚未就绪。".into());
            }
            let mut tools = session.view.tools.clone();
            match &action {
                AnnotationEditorAction::Tool { tool } => tools.tool = tool.clone(),
                AnnotationEditorAction::Style { style } => {
                    if style
                        .text
                        .as_ref()
                        .is_some_and(|text| text.chars().count() > 500)
                    {
                        return Err("标注文字过长。".into());
                    }
                    if let Some(value) = &style.color {
                        tools.color = value.clone();
                    }
                    if let Some(value) = style.width {
                        tools.width = value;
                    }
                    if let Some(value) = style.opacity {
                        tools.opacity = value;
                    }
                    if let Some(value) = style.font_size {
                        tools.font_size = value;
                    }
                    if let Some(value) = &style.font_family {
                        tools.font_family = value.clone();
                    }
                    if let Some(value) = style.magnification {
                        tools.magnification = value;
                    }
                }
                _ => {}
            }
            tools.validate()?;
        }
        let request_id = state
            .next_editor_action
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let (sender, receiver) = std::sync::mpsc::channel();
        state
            .editor_actions
            .lock()
            .map_err(|e| e.to_string())?
            .insert(request_id, (id, sender));
        let sent = app.emit_to(
            LABEL,
            "recording-annotation-editor-action",
            AnnotationEditorRequest {
                id,
                request_id,
                action,
            },
        );
        let result = match sent {
            Ok(()) => receiver
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap_or_else(|_| Err("标注画布未响应，请重新进入标注后重试。".into())),
            Err(e) => Err(e.to_string()),
        };
        state
            .editor_actions
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&request_id);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
/// 只有原画布可以确认编辑动作；会话编号阻止旧请求影响下一次录屏。
#[tauri::command]
pub fn finish_recording_annotation_editor_action(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    request_id: u64,
    error: Option<String>,
) -> Result<(), String> {
    crate::capture::require_window(&window, LABEL)?;
    let state = app.state::<LiveAnnotationState>();
    let mut actions = state.editor_actions.lock().map_err(|e| e.to_string())?;
    if actions
        .get(&request_id)
        .is_none_or(|(session, _)| *session != id)
    {
        return Err("标注工具请求已过期。".into());
    }
    if let Some((_, sender)) = actions.remove(&request_id) {
        let _ = sender.send(error.map_or(Ok(()), |error| Err(error.chars().take(2000).collect())));
    }
    Ok(())
}
/// 发布共享编辑器状态，保留原生持有的持续时间偏好，避免样式同步覆盖新选的时效。
#[tauri::command]
pub async fn publish_recording_annotation_editor(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    editor: AnnotationEditorStatus,
    mut tools: AnnotationTools,
) -> Result<(), String> {
    crate::capture::require_window(&window, LABEL)?;
    if editor.selected < -1
        || editor.selected > 99
        || editor.text.chars().count() > 500
        || editor.error.len() > 8000
    {
        return Err("标注编辑器状态无效。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        require_editing(&app, id)?;
        let state = app.state::<LiveAnnotationState>();
        let _write = state.write.lock().map_err(|e| e.to_string())?;
        let mut current = state.session.lock().map_err(|e| e.to_string())?;
        let session = current
            .as_mut()
            .filter(|s| s.view.id == id)
            .ok_or("录屏标注会话已过期。")?;
        tools.lifetime_ms = session.view.tools.lifetime_ms;
        tools.fade_out_ms = session.view.tools.fade_out_ms;
        tools.validate()?;
        session.view.tools = tools;
        session.view.editor = editor;
        drop(current);
        super::notify_changed(&app);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 记住前台或位于本程序后面的外部应用，退出标注时将键盘输入交还它。
pub(in crate::capture) fn remember_desktop_focus(app: &AppHandle) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
            GW_HWNDNEXT,
        };
        let mut hwnd = GetForegroundWindow();
        // 工具条已获得焦点时，沿 Z 序寻找它后面的外部应用；跳过本程序和隐藏窗口。
        // 不沿用很早以前的目标，兼容用户录制中途切换应用。
        for _ in 0..256 {
            if hwnd.0.is_null() {
                break;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != 0
                && pid != std::process::id()
                && IsWindowVisible(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool()
            {
                if let Ok(mut focus) = app.state::<LiveAnnotationState>().desktop_focus.lock() {
                    *focus = Some(hwnd.0 as isize);
                }
                return;
            }
            let Ok(next) = GetWindow(hwnd, GW_HWNDNEXT) else {
                break;
            };
            hwnd = next;
        }
        if let Ok(mut focus) = app.state::<LiveAnnotationState>().desktop_focus.lock() {
            *focus = None;
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// 仅在本程序仍占有前台时恢复，避免迟到的停止清理抢走用户新切换的窗口。
fn restore_desktop_focus(app: &AppHandle) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{
                GetForegroundWindow, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
                SetForegroundWindow,
            },
        };
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        if pid != std::process::id() {
            return;
        }
        let target = app
            .state::<LiveAnnotationState>()
            .desktop_focus
            .lock()
            .ok()
            .and_then(|focus| *focus);
        if let Some(target) = target {
            let hwnd = HWND(target as *mut std::ffi::c_void);
            let mut target_pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut target_pid));
            if IsWindow(Some(hwnd)).as_bool()
                && IsWindowVisible(hwnd).as_bool()
                && target_pid != 0
                && target_pid != std::process::id()
                && !SetForegroundWindow(hwnd).as_bool()
            {
                log::warn!("event=recording.focus_restore_failed module=recording");
            }
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

fn apply_mode(app: &AppHandle, id: u64, editing: bool) -> Result<(), String> {
    let state = app.state::<LiveAnnotationState>();
    let _write = state.write.lock().map_err(|e| e.to_string())?;
    require_editing(app, id)?;
    let (screen, has_shapes, region, drawing_region) = {
        let current = state.session.lock().map_err(|e| e.to_string())?;
        let session = current
            .as_ref()
            .filter(|s| s.view.id == id)
            .ok_or("录屏标注会话已过期。")?;
        (
            session.screen.clone(),
            !session.view.shapes.is_empty(),
            session.region,
            Rect {
                x: session.view.x.max(0) as u32,
                y: session.view.y.max(0) as u32,
                width: session.view.width,
                height: session.view.height,
            },
        )
    };
    remember_desktop_focus(app);
    let result = (|| -> Result<(), String> {
        let overlay = app
            .get_webview_window(LABEL)
            .ok_or("截图选区窗口不可用。")?;
        // 首帧仅开放选区；前端量取工具栏和 Portal 菜单后扩展区域，不能短暂拦截整屏。
        if editing {
            input_regions::set(&overlay, &[drawing_region])?;
        }
        overlay
            .set_ignore_cursor_events(!editing)
            .map_err(|e| e.to_string())?;
        if !editing {
            input_regions::reset(&overlay)?;
        }
        if editing || has_shapes {
            overlay
                .set_position(PhysicalPosition::new(screen.x, screen.y))
                .map_err(|e| e.to_string())?;
            overlay
                .set_size(PhysicalSize::new(screen.width, screen.height))
                .map_err(|e| e.to_string())?;
            #[cfg(windows)]
            crate::capture::scroll::chrome::protect_recording_window(&overlay)?;
            #[cfg(not(windows))]
            overlay
                .set_content_protected(true)
                .map_err(|e| e.to_string())?;
        } else {
            overlay.hide().map_err(|e| e.to_string())?;
            if overlay.is_visible().map_err(|e| e.to_string())? {
                return Err("录屏标注窗口未能隐藏。".into());
            }
        }
        {
            let mut current = state.session.lock().map_err(|e| e.to_string())?;
            let session = current
                .as_mut()
                .filter(|s| s.view.id == id)
                .ok_or("录屏标注会话已过期。")?;
            session.view.editing = editing;
        }
        // 先发布模式，再显示窗口，隐藏期间的前端轮询也能读到最新状态。
        super::notify_changed(app);
        if editing || has_shapes {
            overlay.show().map_err(|e| e.to_string())?;
        }
        if editing {
            overlay.set_focus().map_err(|e| e.to_string())?;
        }
        // 标注不替代录屏控制；显示顺序保证小控制条始终位于画布上方。
        crate::capture::scroll::chrome::place_recording_controls(app, &screen, region)?;
        crate::capture::scroll::chrome::show_recording_controls(app)?;
        if !editing {
            restore_desktop_focus(app);
        }
        crate::capture::scroll::chrome::show_recording_edges(app)?;
        // 全局停止可在窗口操作期间到达，迟到请求不得再次显示任何录制交互层。
        if require_editing(app, id).is_err() {
            hide_interaction(app);
        }
        Ok(())
    })();
    if result.is_err() {
        hide_interaction(app);
        // 显示标注失败仍保留小控制条，用户能重试或结束录制。
        if require_editing(app, id).is_ok() {
            if let Err(error) = crate::capture::scroll::chrome::show_recording_controls(app) {
                log::warn!(
                    "event=recording.controls_restore_failed module=recording error={error}"
                );
            }
        }
    }
    super::notify_changed(app);
    result
}

/// 渲染与提交串行，查询/采集只短暂取得 Arc；旧会话的慢请求不能写入下一次录制。
#[tauri::command]
pub async fn update_recording_annotations(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    shapes: Vec<Annotation>,
    known_revision: Option<u64>,
) -> Result<AnnotationView, String> {
    if !matches!(window.label(), "capture" | "recording-controls") {
        return Err("当前窗口无权修改录制标注。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LiveAnnotationState>();
        let _write = state.write.lock().map_err(|e| e.to_string())?;
        require_editing(&app, id)?;
        let (width, height) = {
            let current = state.session.lock().map_err(|e| e.to_string())?;
            let s = current
                .as_ref()
                .filter(|s| s.view.id == id)
                .ok_or("录屏标注会话已过期。")?;
            (s.view.width, s.view.height)
        };
        annotations::validate_live(&shapes, width, height)?;
        if shapes
            .iter()
            .filter(|s| annotations::dynamic_filter(s))
            .count()
            > 8
        {
            return Err("录制中最多支持 8 个实时滤镜。".into());
        }
        let ms = media_time(&app)?;
        require_editing(&app, id)?;
        let mut current = state.session.lock().map_err(|e| e.to_string())?;
        let s = current
            .as_mut()
            .filter(|s| s.view.id == id)
            .ok_or("录屏标注会话已过期。")?;
        if known_revision.is_some_and(|revision| revision != s.view.revision) {
            return Err("标注已在另一窗口更新，请等待同步后重试。".into());
        }
        let ids = super::annotation_track::reconcile(
            &mut s.history,
            &s.current_ids,
            &shapes,
            ms,
            s.view.tools.lifetime_ms,
            s.view.tools.fade_out_ms,
        )?;
        s.current_ids = ids;
        s.view.tracks = s
            .history
            .iter()
            .filter(|t| s.current_ids.contains(&t.id))
            .cloned()
            .collect();
        s.view.tracks.sort_by_key(|track| track.order);
        s.view.shapes = shapes;
        s.view.media_time_ms = ms;
        s.view.revision += 1;
        s.layer = Arc::new(Layer::timed(s.view.tracks.clone(), width, height)?);
        let mut view = s.view.status(false);
        view.tracks = s.view.tracks.clone();
        drop(current);
        super::notify_changed(&app);
        Ok(view)
    })
    .await
    .map_err(|e| e.to_string())?
}
