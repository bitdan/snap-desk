//! 录制中的标注：桌面浮层只负责交互，编码帧使用各自持有的不可变标注快照。
//! 浮层排除于系统采集，避免把工具栏/选择手柄录进去；标注由编码线程真正写入 MP4/GIF。
use super::{frame, Phase, RecordingState};
use crate::{
    annotations::{self, Annotation, Point},
    capture::{scroll::Screen, Rect},
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

const LABEL: &str = "recording-annotations";
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
}
#[derive(Default)]
pub struct LiveAnnotationState {
    session: Mutex<Option<Session>>,
    write: Mutex<()>,
}

/// 普通图形只在修改时绘成透明层；动态滤镜才从每一帧的干净像素重新计算。
pub enum Layer {
    Static(Option<frame::AnnotationLayer>),
    Dynamic(Vec<Annotation>),
}
impl Layer {
    fn new(shapes: &[Annotation], width: u32, height: u32) -> Result<Self, String> {
        annotations::validate_live(shapes, width, height)?;
        if shapes.iter().any(annotations::dynamic_filter) {
            if shapes
                .iter()
                .filter(|s| annotations::dynamic_filter(s))
                .count()
                > 8
            {
                return Err("录制中最多支持 8 个实时滤镜。".into());
            }
            Ok(Self::Dynamic(shapes.to_vec()))
        } else {
            Ok(Self::Static(frame::AnnotationLayer::new(
                shapes,
                width,
                height,
                Point { x: 0.0, y: 0.0 },
            )?))
        }
    }
    pub fn apply(&self, frame: &mut frame::BgraImage) -> Result<(), String> {
        match self {
            Self::Static(Some(layer)) => layer.apply(frame),
            Self::Static(None) => Ok(()),
            Self::Dynamic(shapes) => {
                let mut image =
                    image::RgbaImage::from_raw(frame.width, frame.height, frame.opaque_rgba()?)
                        .ok_or("录屏帧尺寸无效。")?;
                annotations::render_live(&mut image, shapes)?;
                let mut pixels = image.into_raw();
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                frame.data = pixels;
                Ok(())
            }
        }
    }
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
    let layer = Arc::new(Layer::new(&shapes, region.width, region.height)?);
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
            shapes,
        },
        screen: screen.clone(),
        region,
        layer,
    });
    Ok(())
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
    app.state::<LiveAnnotationState>().session.lock().ok()
        .is_some_and(|s| s.as_ref().is_some_and(|s| s.view.id == id && s.view.editing))
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
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    let _ = app.emit_to(LABEL, "recording-annotations-closed", id);
}
/// 停止时立即收起交互层，封装队列仍保留各帧自己的标注快照。
pub(crate) fn hide_interaction(app: &AppHandle) {
    if let Ok(mut current) = app.state::<LiveAnnotationState>().session.lock() {
        if let Some(session) = current.as_mut() {
            session.view.editing = false;
        }
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}
fn require_editing(app: &AppHandle, id: u64) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    let session = state.0.lock().map_err(|e| e.to_string())?;
    if session.as_ref().is_some_and(|s| {
        s.view.id == id
            && !s.stop
            && !s.cancel
            && matches!(s.view.phase, Phase::Recording | Phase::Paused)
    }) {
        Ok(())
    } else {
        Err("录制已结束或尚未开始，无法修改录制标注。".into())
    }
}

#[tauri::command]
pub fn current_recording_annotations(
    app: AppHandle,
    window: WebviewWindow,
    known_revision: Option<u64>,
) -> Result<Option<AnnotationView>, String> {
    crate::capture::require_window(&window, LABEL)?;
    let state = app.state::<LiveAnnotationState>();
    let view = state
        .session
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|s| s.view.status(known_revision != Some(s.view.revision)));
    Ok(view.filter(|view| require_editing(&app, view.id).is_ok()))
}

/// 开启时接收选区绘制；完成后透明层保持可见但鼠标穿透，用户可继续操作原应用。
#[tauri::command]
pub async fn set_recording_annotation_mode(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    editing: bool,
) -> Result<(), String> {
    if !matches!(
        window.label(),
        "recording-controls" | "recording-annotations"
    ) {
        return Err("当前窗口无权修改录制标注。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        require_editing(&app, id)?;
        let (screen, empty) = {
            let state = app.state::<LiveAnnotationState>();
            let mut session = state.session.lock().map_err(|e| e.to_string())?;
            let s = session
                .as_mut()
                .filter(|s| s.view.id == id)
                .ok_or("录屏标注会话已过期。")?;
            s.view.editing = editing;
            (s.screen.clone(), s.view.shapes.is_empty())
        };
        let result = (|| -> Result<(), String> {
            let overlay = if let Some(value) = app.get_webview_window(LABEL) {
                value
            } else {
                WebviewWindowBuilder::new(
                    &app,
                    LABEL,
                    WebviewUrl::App("index.html?window=recording-annotations".into()),
                )
                .title("Snap Desk")
                .transparent(true)
                .decorations(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .content_protected(true)
                .visible(false)
                .build()
                .map_err(|e| e.to_string())?
            };
            overlay
                .set_position(PhysicalPosition::new(screen.x, screen.y))
                .map_err(|e| e.to_string())?;
            overlay
                .set_size(PhysicalSize::new(screen.width, screen.height))
                .map_err(|e| e.to_string())?;
            overlay
                .set_ignore_cursor_events(!editing)
                .map_err(|e| e.to_string())?;
            if editing || !empty {
                overlay.show().map_err(|e| e.to_string())?;
                #[cfg(windows)]
                crate::capture::scroll::chrome::protect_recording_window(&overlay)?;
                if editing {
                    overlay.set_focus().map_err(|e| e.to_string())?;
                }
            } else {
                overlay.hide().map_err(|e| e.to_string())?;
            }
            // 显示过程中可能已按全局快捷键停止，不能让迟到的窗口挡住回放。
            if require_editing(&app, id).is_err() {
                let _ = overlay.hide();
            }
            Ok(())
        })();
        if result.is_err() {
            let current = app
                .state::<LiveAnnotationState>()
                .session
                .lock()
                .ok()
                .is_some_and(|session| session.as_ref().is_some_and(|s| s.view.id == id));
            if current {
                hide_interaction(&app);
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 渲染与提交串行，查询/采集只短暂取得 Arc；旧会话的慢请求不能写入下一次录制。
#[tauri::command]
pub async fn update_recording_annotations(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    shapes: Vec<Annotation>,
) -> Result<AnnotationView, String> {
    crate::capture::require_window(&window, LABEL)?;
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
        let layer = Arc::new(Layer::new(&shapes, width, height)?);
        require_editing(&app, id)?;
        let mut current = state.session.lock().map_err(|e| e.to_string())?;
        let s = current
            .as_mut()
            .filter(|s| s.view.id == id)
            .ok_or("录屏标注会话已过期。")?;
        s.view.shapes = shapes;
        s.view.revision += 1;
        s.layer = layer;
        Ok(s.view.status(false))
    })
    .await
    .map_err(|e| e.to_string())?
}
