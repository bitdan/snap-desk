//! 录制中的实时标注（对照 Tauri 版 `capture/recorder/live_annotations.rs` 与 Vue 录屏标注画布）。
//!
//! 控制条“标注”打开一个覆盖录制显示器的透明窗口（置顶、不进入录像），复用截图编辑器（`annotate.rs`）：
//! - 标注坐标为录制区域的物理像素；每次编辑器历史变化即把整份标注提交给 core `recording::live::Live`，
//!   由它对账出媒体时间轨并重建编码用的标注层（未改动的标注保留原起始时间）。
//! - 文字、序号在后台栅格化；马赛克、模糊、放大镜是实时滤镜，不栅格化，由编码线程逐帧从干净画面取样，
//!   画布上只画虚线框（浮层看不到自己下面的画面，也不应遮住被录内容）。
//! - 工具栏右侧可选标注时效（常驻 / 3 秒 / 5 秒），到期的标注从画布移除（录像中按媒体时间淡出）。
//! - “完成”/Esc/右键结束编辑：有标注时窗口改为鼠标穿透继续显示标注，没有标注时关闭窗口。
//!
//! 与 Tauri 版的差异：编辑时整个显示器都接收输入（Tauri 只开放选区和工具栏区域）；结束编辑后不把
//! 键盘焦点交还给之前的应用；窗口录制不支持实时标注（浮层无法跟随窗口移动）。
use crate::annotate::{self, Action, Editor, Job, Layers, Tool, ToolbarProps};
use gpui_kit::{
    component::input::{Textarea, TextareaState},
    *,
};
use image::RgbaImage;
use snap_core::{
    annotations::{self, edit::Bounds as RegionBounds, Annotation, Point as RegionPoint},
    recording::{
        live::{Live, DEFAULT_FADE_MS, LIFETIMES},
        RecordingState,
    },
    settings::Language,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// 时效检查间隔。
const POLL: Duration = Duration::from_millis(200);
/// 工具栏（工具行 + 样式行）预留高度，用于判断能否放在选区上方。
const TOOLBAR_HEIGHT: f32 = 84.0;
/// 工具行的大致宽度：选区左边起放不下时改为与选区右边对齐，避免“完成”被挤出屏幕。
const TOOLBAR_WIDTH: f32 = 760.0;
const GAP: f32 = 8.0;
const FRAME: u32 = 0x2f80ff;

/// 打开实时标注窗口所需的会话信息（坐标均为 GPUI 全局逻辑像素）。
#[derive(Clone)]
pub struct Target {
    pub id: u64,
    pub live: Arc<Mutex<Live>>,
    pub state: Arc<RecordingState>,
    pub language: Language,
    /// 录制显示器（GPUI 显示器编号与逻辑边界）。
    pub display_id: Option<DisplayId>,
    pub screen: Bounds<Pixels>,
    /// 录制区域。
    pub region: Bounds<Pixels>,
    /// 录制控制条（工具栏避开它）。
    pub bar: Bounds<Pixels>,
    /// 显示器缩放（物理像素 / 逻辑像素）。
    pub scale: f32,
}

impl Target {
    /// 录制开始前已有的标注（截图中画好的）。
    pub fn has_shapes(&self) -> bool {
        self.live.lock().is_ok_and(|live| !live.shapes.is_empty())
    }
}

/// 打开覆盖录制显示器的标注窗口；`editing` 为假时只显示标注（鼠标穿透）。
pub fn open(cx: &mut App, target: Target, editing: bool) -> Option<WindowHandle<Annotator>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(target.screen)),
        display_id: target.display_id,
        titlebar: None,
        focus: editing,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let result = cx.open_window(options, |window, cx| {
        crate::recording::protect(window);
        cx.new(|cx| Annotator::new(target, editing, window, cx))
    });
    match result {
        Ok(handle) => {
            if editing {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
            Some(handle)
        }
        Err(error) => {
            log::error!("event=gpui.live_annotations_failed message={error}");
            None
        }
    }
}

pub struct Annotator {
    target: Target,
    /// 录制区域在窗口内的逻辑坐标。
    region: Bounds<Pixels>,
    /// 工具栏位置（窗口内逻辑坐标）。
    toolbar: Anchor,
    editor: Editor,
    layers: Layers,
    text_input: Option<Entity<TextareaState>>,
    focus: FocusHandle,
    pub editing: bool,
    /// 已提交给 core 的编辑器历史版本。
    committed: u64,
    /// 新标注的时效（毫秒，0 为常驻）。
    lifetime: u64,
    error: Option<String>,
    /// 文字、序号栅格化不读取底图；滤镜在录制中不栅格化。
    blank: Arc<RgbaImage>,
    handle: AnyWindowHandle,
}

impl Annotator {
    fn new(target: Target, editing: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 以窗口实际位置换算选区（系统可能调整弹出窗口的位置），保证画布与录像中的标注重合。
        let actual = window.bounds();
        if actual.origin != target.screen.origin {
            log::warn!(
                "event=gpui.live_annotations_window_moved dx={} dy={}",
                f32::from(actual.origin.x - target.screen.origin.x),
                f32::from(actual.origin.y - target.screen.origin.y)
            );
        }
        let screen = Bounds::new(actual.origin, target.screen.size);
        let region = Bounds::new(
            point(
                target.region.origin.x - screen.origin.x,
                target.region.origin.y - screen.origin.y,
            ),
            target.region.size,
        );
        let bar = Bounds::new(
            point(
                target.bar.origin.x - screen.origin.x,
                target.bar.origin.y - screen.origin.y,
            ),
            target.bar.size,
        );
        let toolbar = toolbar_anchor(screen.size, region, bar);
        let (shapes, width, height) = target
            .live
            .lock()
            .map(|live| (live.shapes.clone(), live.width, live.height))
            .unwrap_or_default();
        let crop = RegionBounds {
            x: 0.0,
            y: 0.0,
            width: f64::from(width),
            height: f64::from(height),
        };
        let mut editor = Editor::new(crop, shapes);
        editor.set_tool(Tool::Pen);
        let mut layers = Layers::default();
        layers.update(&editor);
        let committed = editor.version;
        cx.spawn(
            async move |this: WeakEntity<Annotator>, cx: &mut AsyncApp| loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |view, cx| view.expire(cx)).is_err() {
                    break;
                }
            },
        )
        .detach();
        let mut view = Self {
            target,
            region,
            toolbar,
            editor,
            layers,
            text_input: None,
            focus: cx.focus_handle(),
            editing: false,
            committed,
            lifetime: 0,
            error: None,
            blank: Arc::new(RgbaImage::new(1, 1)),
            handle: window.window_handle(),
        };
        view.set_editing(editing, window, cx);
        view
    }

    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.target.language, key).to_owned()
    }

    /// 进入或结束编辑。结束时没有标注则关闭窗口，否则改为鼠标穿透继续显示。
    pub fn set_editing(&mut self, editing: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !editing && self.editor.text.is_some() {
            self.commit_text(window, cx);
        }
        if !editing {
            self.editor.abort();
            self.editor.select(None);
        }
        self.editing = editing;
        if let Ok(mut live) = self.target.live.lock() {
            live.editing = editing;
        }
        if let Err(error) = crate::native::set_click_through(window, !editing) {
            log::warn!("event=gpui.live_annotations_click_through_failed message={error}");
        }
        if editing {
            window.activate_window();
            window.focus(&self.focus, cx);
        } else if self.editor.shapes().is_empty() && !self.editor.pending {
            crate::recording::annotator_closed(cx, self.target.id);
            window.remove_window();
            return;
        }
        self.after_edit(window, cx);
    }

    /// 窗口内逻辑坐标 → 录制区域物理像素。
    fn to_point(&self, position: Point<Pixels>) -> RegionPoint {
        let scale = f64::from(self.target.scale);
        RegionPoint {
            x: f64::from(f32::from(position.x - self.region.origin.x)) * scale,
            y: f64::from(f32::from(position.y - self.region.origin.y)) * scale,
        }
    }

    /// 录制区域物理像素 → 窗口内逻辑坐标。
    fn map(&self, p: RegionPoint) -> Point<Pixels> {
        let scale = self.target.scale;
        point(
            self.region.origin.x + px(p.x as f32 / scale),
            self.region.origin.y + px(p.y as f32 / scale),
        )
    }

    /// 编辑器历史变化后提交给 core；失败时恢复为上次成功提交的标注并显示原因。
    fn sync(&mut self) {
        if self.editor.pending || self.editor.version == self.committed {
            return;
        }
        let ms = self.target.state.media_time_ms().unwrap_or(0);
        let shapes = self.editor.shapes().to_vec();
        let fade = if self.lifetime > 0 {
            DEFAULT_FADE_MS
        } else {
            0
        };
        let result = self
            .target
            .live
            .lock()
            .map_err(|e| e.to_string())
            .and_then(|mut live| {
                live.commit(shapes, ms, self.lifetime, fade)
                    .map_err(|error| {
                        let previous = live.shapes.clone();
                        self.editor.reset(previous);
                        error
                    })
            });
        if let Err(error) = result {
            log::warn!("event=gpui.live_annotations_rejected message={error}");
            self.error = Some(error);
            self.layers.update(&self.editor);
        }
        self.committed = self.editor.version;
    }

    /// 编辑器状态变化后：提交、刷新图层、释放旧纹理、按需创建或移除文字输入框。
    fn after_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync();
        self.layers.update(&self.editor);
        for image in self.layers.take_retired() {
            cx.drop_image(image, Some(window));
        }
        if let Some(error) = self.editor.error.take() {
            self.error = Some(error);
        }
        match (self.editor.text.as_ref(), self.text_input.is_some()) {
            (Some(text), false) => {
                let value = text.shape.text.clone().unwrap_or_default();
                let state = cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx);
                    state.set_auto_grow(1, 20, cx);
                    state.set_value(value, window, cx);
                    state
                });
                state.update(cx, |state, cx| state.focus(window, cx));
                self.text_input = Some(state);
            }
            (None, true) => {
                self.text_input = None;
                window.focus(&self.focus, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 文字、序号在后台栅格化；实时滤镜直接提交（不带像素，由编码线程逐帧取样）。
    fn run(&mut self, job: Option<Job>, cx: &mut Context<Self>) {
        let Some(job) = job else {
            return;
        };
        if annotations::dynamic_filter(&job.shape) {
            let mut shape = job.shape.clone();
            shape.raster = None;
            self.editor.complete(job, Ok(shape));
            return;
        }
        let image = self.blank.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let job = job.clone();
                    async move { job.run(&image, annotate::user_font()) }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.editor.complete(job, result);
                view.sync();
                view.layers.update(&view.editor);
                for image in view.layers.take_retired() {
                    cx.drop_image(image, None);
                }
                if let Some(error) = view.editor.error.take() {
                    view.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 到达时效的标注从画布移除（录像中已按媒体时间结束）；不在编辑且没有标注时关闭窗口。
    fn expire(&mut self, cx: &mut Context<Self>) {
        if self.editor.pending || self.editor.interacting() || self.editor.text.is_some() {
            return;
        }
        let ms = self.target.state.media_time_ms().unwrap_or(0);
        let expired = self
            .target
            .live
            .lock()
            .map(|live| live.expired(ms))
            .unwrap_or_default();
        if expired.is_empty() {
            return;
        }
        let remaining: Vec<Annotation> = self
            .editor
            .shapes()
            .iter()
            .enumerate()
            .filter(|(index, _)| !expired.contains(index))
            .map(|(_, shape)| shape.clone())
            .collect();
        self.editor.reset(remaining);
        self.sync();
        self.layers.update(&self.editor);
        for image in self.layers.take_retired() {
            cx.drop_image(image, None);
        }
        if !self.editing && self.editor.shapes().is_empty() {
            crate::recording::annotator_closed(cx, self.target.id);
            let handle = self.handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
            return;
        }
        cx.notify();
    }

    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .text_input
            .as_ref()
            .map(|state| state.read(cx).value().to_string())
            .unwrap_or_default();
        let job = self.editor.finish_text(&value);
        self.run(job, cx);
        self.after_edit(window, cx);
    }

    fn action(this: &mut Self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !this.editing {
            return;
        }
        this.error = None;
        let e = &mut this.editor;
        let job = match action {
            Action::Tool(tool) => {
                e.set_tool(tool);
                None
            }
            Action::Ui(ui) => {
                e.ui(ui);
                None
            }
            Action::Undo => {
                e.undo();
                None
            }
            Action::Redo => {
                e.redo();
                None
            }
            Action::Delete => {
                e.remove();
                None
            }
            Action::Clear => {
                e.clear();
                None
            }
            Action::Style(patch) => e.style(patch),
        };
        this.run(job, cx);
        this.after_edit(window, cx);
    }

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing {
            return;
        }
        if self.editor.text.is_some() {
            self.commit_text(window, cx);
            return;
        }
        self.error = None;
        let p = self.to_point(event.position);
        let tolerance = 6.0 * f64::from(self.target.scale);
        let handle = self.editor.handle_at(p, tolerance);
        if !self.editor.crop.contains(p) && handle.is_none() {
            // 选区外的按下只取消选中；不能开始绘制。
            self.editor.select(None);
            self.after_edit(window, cx);
            return;
        }
        if event.click_count >= 2 {
            if let Some(i) = self
                .editor
                .hit(p, tolerance)
                .filter(|&i| self.editor.shapes()[i].kind == annotations::Kind::Text)
            {
                self.editor.edit_text(i);
                self.after_edit(window, cx);
                return;
            }
        }
        self.editor.begin(p, tolerance, handle, event.modifiers.alt);
        self.after_edit(window, cx);
    }

    fn motion(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left)
            || self.editor.draft.is_none()
            || self.editor.text.is_some()
        {
            return;
        }
        let p = self.to_point(event.position);
        self.editor.drag(p);
        self.layers.update(&self.editor);
        for image in self.layers.take_retired() {
            cx.drop_image(image, Some(window));
        }
        cx.notify();
    }

    fn release(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.draft.is_none() {
            return;
        }
        let job = self.editor.finish(false);
        self.run(job, cx);
        self.after_edit(window, cx);
    }

    /// 右键先放弃未完成的笔画或输入；没有进行中的操作时结束编辑。
    fn right_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.text.is_some() {
            self.editor.cancel_text();
            self.after_edit(window, cx);
        } else if self.editor.interacting() {
            self.editor.abort();
            self.after_edit(window, cx);
        } else {
            self.set_editing(false, window, cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing {
            return;
        }
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;
        let key = keystroke.key.as_str();
        if self.editor.text.is_some() {
            match key {
                "escape" => {
                    self.editor.cancel_text();
                    self.after_edit(window, cx);
                    cx.stop_propagation();
                }
                "enter" if command => {
                    self.commit_text(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        match key {
            "escape" if self.editor.interacting() => {
                self.editor.abort();
                self.after_edit(window, cx);
            }
            "escape" => self.set_editing(false, window, cx),
            "z" if command => {
                if keystroke.modifiers.shift {
                    self.editor.redo();
                } else {
                    self.editor.undo();
                }
                self.after_edit(window, cx);
            }
            "y" if command => {
                self.editor.redo();
                self.after_edit(window, cx);
            }
            "delete" | "backspace" => {
                self.editor.remove();
                self.after_edit(window, cx);
            }
            "enter" if self.editor.tool == Tool::Polyline && self.editor.draft.is_some() => {
                let job = self.editor.finish(true);
                self.run(job, cx);
                self.after_edit(window, cx);
            }
            "left" | "right" | "up" | "down" if self.editor.selected.is_some() => {
                let step = if keystroke.modifiers.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    "left" => (-step, 0.0),
                    "right" => (step, 0.0),
                    "up" => (0.0, -step),
                    _ => (0.0, step),
                };
                let job = self.editor.nudge(dx, dy).1;
                self.run(job, cx);
                self.after_edit(window, cx);
            }
            _ => {}
        }
    }

    fn lifetime_chip(&self, index: usize, value: u64, cx: &mut Context<Self>) -> AnyElement {
        let on = self.lifetime == value;
        let label = match value {
            0 => self.t("常驻"),
            3000 => self.t("3 秒后消失"),
            _ => self.t("5 秒后消失"),
        };
        div()
            .id(("live-lifetime", index))
            .h(px(30.0))
            .px(px(8.0))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(6.0))
            .text_xs()
            .cursor_pointer()
            .text_color(rgb(if on {
                annotate::active_fg()
            } else {
                annotate::muted()
            }))
            .bg(rgb(if on {
                annotate::active_bg()
            } else {
                annotate::surface()
            }))
            .hover(|s| s.text_color(rgb(annotate::text())))
            .child(label)
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.lifetime = value;
                cx.notify();
            }))
            .into_any_element()
    }

    fn toolbar(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let view = cx.entity();
        let text = |key: &'static str| self.t(key);
        let props = ToolbarProps {
            editor: &self.editor,
            active: true,
            disabled: false,
            compact: false,
            grouped: true,
            collapsible: false,
            popover_up: false,
            text: &text,
        };
        let host = annotate::Host {
            view: &view,
            window,
            dispatch: Self::action,
        };
        let mut trailing: Vec<AnyElement> = LIFETIMES
            .iter()
            .enumerate()
            .map(|(index, &value)| self.lifetime_chip(index, value, cx))
            .collect();
        trailing.push(annotate::divider().into_any_element());
        trailing.push(
            div()
                .id("live-done")
                .h(px(30.0))
                .px(px(12.0))
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(6.0))
                .text_xs()
                .cursor_pointer()
                .text_color(rgb(annotate::text()))
                .bg(rgb(0x2563eb))
                .hover(|s| s.opacity(0.85))
                .child(self.t("完成"))
                .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                    view.set_editing(false, window, cx)
                }))
                .into_any_element(),
        );
        let anchor = self.toolbar;
        let bar = div().id("live-toolbar").absolute().top(anchor.top);
        let bar = match anchor.right {
            Some(right) => bar.right(right).flex().flex_col().items_end(),
            None => bar.left(anchor.left).flex().flex_col().items_start(),
        };
        bar.gap(px(6.0))
            .cursor_default()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(annotate::tool_row(&props, Vec::new(), trailing, &host))
            .children(annotate::options_row(&props, &host))
            .children(self.error.clone().map(annotate::error_row))
    }
}

/// 工具栏位置：`right` 为 Some 时与选区右边对齐（距窗口右边的距离），否则从 `left` 起排。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    top: Pixels,
    left: Pixels,
    right: Option<Pixels>,
    /// 放在选区内顶部（上方放不下或控制条在上方）。
    inside: bool,
}

/// 控制条不在选区上方且上方放得下时放在选区上方，否则放在选区内顶部；
/// 从选区左边起放不下时改为与选区右边对齐。
fn toolbar_anchor(screen: Size<Pixels>, region: Bounds<Pixels>, bar: Bounds<Pixels>) -> Anchor {
    let above = region.origin.y - px(GAP + TOOLBAR_HEIGHT);
    let bar_above = bar.origin.y + bar.size.height <= region.origin.y;
    let inside = above < px(0.0) || bar_above;
    let indent = if inside { px(GAP) } else { px(0.0) };
    let left = (region.origin.x + indent).max(px(GAP));
    let right = (left + px(TOOLBAR_WIDTH) > screen.width - px(GAP))
        .then(|| (screen.width - (region.origin.x + region.size.width) + indent).max(px(GAP)));
    Anchor {
        top: if inside {
            region.origin.y + px(GAP)
        } else {
            above
        },
        left,
        right,
        inside,
    }
}

impl Render for Annotator {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.target.scale;
        let mut root = div()
            .id("live-annotations")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_move(cx.listener(Self::motion))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::release))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_click));
        if self.editing {
            // 编辑时标出可绘制范围（窗口不进入录像）；整窗加一层几乎透明的底色，保证空白处也能收到鼠标。
            root = root.bg(rgba(0x00000001)).cursor_crosshair().child(
                div()
                    .absolute()
                    .left(self.region.origin.x - px(1.0))
                    .top(self.region.origin.y - px(1.0))
                    .w(self.region.size.width + px(2.0))
                    .h(self.region.size.height + px(2.0))
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(FRAME)),
            );
        }
        for layer in [self.layers.base(), self.layers.draft()]
            .into_iter()
            .flatten()
        {
            root = root.child(
                img(layer.image.clone())
                    .absolute()
                    .left(self.region.origin.x + px(layer.x as f32 / scale))
                    .top(self.region.origin.y + px(layer.y as f32 / scale))
                    .w(px(layer.width as f32 / scale))
                    .h(px(layer.height as f32 / scale))
                    .object_fit(ObjectFit::Fill),
            );
        }
        // 实时滤镜只画范围框：真实效果由编码线程从干净画面逐帧计算。
        for shape in self
            .editor
            .shapes()
            .iter()
            .filter(|s| annotations::dynamic_filter(s))
        {
            let (a, b) = (self.map(shape.start), self.map(shape.end));
            root = root.child(
                div()
                    .absolute()
                    .left(a.x)
                    .top(a.y)
                    .w(b.x - a.x)
                    .h(b.y - a.y)
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(annotate::muted())),
            );
        }
        let map = |p: RegionPoint| self.map(p);
        root = root.children(annotate::selection_overlay(&self.editor, &map));
        if let (Some(text), Some(state)) = (self.editor.text.as_ref(), self.text_input.as_ref()) {
            let at = self.map(text.shape.start);
            let width = ((self.editor.crop.width - text.shape.start.x).min(2048.0) as f32 / scale)
                .max(40.0);
            let font = (text.shape.font_size.unwrap_or(24.0) as f32 / scale).max(8.0);
            let color = u32::from_str_radix(text.shape.color.trim_start_matches('#'), 16)
                .unwrap_or(0xef4444);
            root = root.child(
                div()
                    .absolute()
                    .left(at.x)
                    .top(at.y)
                    .w(px(width))
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(FRAME))
                    .bg(rgba(0xffffff18))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Textarea::new(state)
                            .appearance(false)
                            .text_size(px(font))
                            .text_color(rgb(color)),
                    ),
            );
        }
        if self.editing {
            root = root.child(self.toolbar(window, cx));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::{toolbar_anchor, GAP, TOOLBAR_HEIGHT};
    use gpui_kit::{point, px, size, Bounds};

    #[test]
    fn toolbar_goes_above_unless_bar_or_screen_edge_is_there() {
        let screen = size(px(1920.0), px(1080.0));
        let region = Bounds::new(point(px(400.0), px(300.0)), size(px(600.0), px(400.0)));
        let below = Bounds::new(point(px(470.0), px(708.0)), size(px(460.0), px(44.0)));
        let at = toolbar_anchor(screen, region, below);
        assert_eq!(
            (at.top, at.left, at.right, at.inside),
            (px(300.0 - GAP - TOOLBAR_HEIGHT), px(400.0), None, false)
        );
        let above = Bounds::new(point(px(470.0), px(248.0)), size(px(460.0), px(44.0)));
        assert!(toolbar_anchor(screen, region, above).inside);
        let top = Bounds::new(point(px(0.0), px(20.0)), region.size);
        let at = toolbar_anchor(screen, top, below);
        assert_eq!(
            (at.top, at.left, at.inside),
            (px(20.0 + GAP), px(GAP), true)
        );
    }

    #[test]
    fn toolbar_aligns_to_region_right_edge_near_the_screen_edge() {
        let screen = size(px(1920.0), px(1080.0));
        let region = Bounds::new(point(px(1380.0), px(280.0)), size(px(460.0), px(300.0)));
        let bar = Bounds::new(point(px(1340.0), px(588.0)), size(px(540.0), px(44.0)));
        // 选区右边距屏幕右边 80。
        assert_eq!(toolbar_anchor(screen, region, bar).right, Some(px(80.0)));
    }
}
