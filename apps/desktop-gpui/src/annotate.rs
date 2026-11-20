//! 截图与贴图共用的标注编辑器：工具状态、绘制/选择/移动/缩放手势、撤销重做、样式与预览图层。
//!
//! 行为对照 Vue 版 `use-annotations.ts`：
//! - 标注始终使用原图物理像素坐标（截图为整张虚拟桌面截图，贴图为贴图图像），`crop` 为可编辑范围；
//! - 一次拖动只提交一次历史；文字、序号与滤镜在完成时栅格化（`snap_core::annotations::raster`），
//!   栅格化在后台线程进行，期间 `pending` 为真，完成后由宿主调用 [`Editor::complete`]；
//! - 预览图层直接用 `snap_core::annotations::render` 绘制，与导出（`capture::compose`、贴图合成）像素一致。
//!
//! 本模块不持有窗口：宿主（截图浮层、贴图窗口）负责坐标换算、事件分发、文字输入框与后台任务。
use gpui_kit::{component::Icon, *};
use image::RgbaImage;
use snap_core::annotations::{
    self, Annotation, Kind, Point,
    edit::{self, Bounds, Handle, History},
    raster,
};
use std::sync::Arc;

/// 标注数量上限，与 `annotations::validate` 一致。
const MAX_SHAPES: usize = 100;
/// 路径点上限。
const MAX_POINTS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Edit,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    ArrowText,
    Polyline,
    Pen,
    Highlighter,
    Text,
    Number,
    Spotlight,
    Magnify,
    Mosaic,
    Blur,
    Erase,
    Eraser,
}

impl Tool {
    /// 工具栏顺序，与 Vue 版默认顺序一致。
    pub const ALL: [Self; 17] = [
        Self::Edit,
        Self::Rectangle,
        Self::Ellipse,
        Self::Line,
        Self::Arrow,
        Self::ArrowText,
        Self::Polyline,
        Self::Pen,
        Self::Highlighter,
        Self::Text,
        Self::Number,
        Self::Spotlight,
        Self::Magnify,
        Self::Mosaic,
        Self::Blur,
        Self::Erase,
        Self::Eraser,
    ];

    /// 新建图形的种类；选择与整项删除没有。
    pub fn kind(self) -> Option<Kind> {
        Some(match self {
            Self::Edit | Self::Eraser => return None,
            Self::Rectangle => Kind::Rectangle,
            Self::Ellipse => Kind::Ellipse,
            Self::Line => Kind::Line,
            Self::Arrow | Self::ArrowText => Kind::Arrow,
            Self::Polyline => Kind::Polyline,
            Self::Pen => Kind::Pen,
            Self::Highlighter => Kind::Highlighter,
            Self::Text => Kind::Text,
            Self::Number => Kind::Number,
            Self::Spotlight => Kind::Spotlight,
            Self::Magnify => Kind::Magnify,
            Self::Mosaic => Kind::Mosaic,
            Self::Blur => Kind::Blur,
            Self::Erase => Kind::Erase,
        })
    }

    /// 词典键（中文原文），用于悬停提示。
    pub fn label(self) -> &'static str {
        match self {
            Self::Edit => "选择 / 编辑",
            Self::Rectangle => "矩形",
            Self::Ellipse => "椭圆",
            Self::Line => "直线",
            Self::Arrow => "箭头",
            Self::ArrowText => "箭头文字",
            Self::Polyline => "折线",
            Self::Pen => "画笔",
            Self::Highlighter => "荧光笔",
            Self::Text => "文字",
            Self::Number => "序号",
            Self::Spotlight => "聚光灯",
            Self::Magnify => "放大镜标注",
            Self::Mosaic => "马赛克",
            Self::Blur => "模糊",
            Self::Erase => "局部像素橡皮擦",
            Self::Eraser => "删除整项标注",
        }
    }

    pub fn icon(self) -> &'static [u8] {
        match self {
            Self::Edit => icons::POINTER,
            Self::Rectangle => icons::SQUARE,
            Self::Ellipse => icons::CIRCLE,
            Self::Line => icons::MINUS,
            Self::Arrow => icons::ARROW,
            Self::ArrowText => icons::ARROW_TEXT,
            Self::Polyline => icons::SPLINE,
            Self::Pen => icons::PENCIL,
            Self::Highlighter => icons::HIGHLIGHTER,
            Self::Text => icons::TYPE,
            Self::Number => icons::NUMBER,
            Self::Spotlight => icons::FOCUS,
            Self::Magnify => icons::ZOOM,
            Self::Mosaic => icons::GRID,
            Self::Blur => icons::DROPLETS,
            Self::Erase => icons::ERASER,
            Self::Eraser => icons::SQUARE_X,
        }
    }

    /// 笔刷、折线、序号与聚光灯保持连续绘制，不优先选中已有图形。
    fn draws_over(self) -> bool {
        matches!(
            self,
            Self::Pen | Self::Highlighter | Self::Erase | Self::Polyline | Self::Number | Self::Spotlight
        )
    }
}

/// 折叠模式按用途合并工具（与 Vue 版 `annotationToolGroups` 一致）；每组显示最近使用的工具，
/// 旁边的小箭头展开该组全部工具。
pub const GROUPS: [(&str, &[Tool]); 7] = [
    ("选择 / 编辑", &[Tool::Edit]),
    ("形状", &[Tool::Rectangle, Tool::Ellipse]),
    ("线条", &[Tool::Arrow, Tool::ArrowText, Tool::Line, Tool::Polyline]),
    ("画笔", &[Tool::Pen, Tool::Highlighter]),
    ("文字与序号", &[Tool::Text, Tool::Number]),
    ("遮挡与擦除", &[Tool::Mosaic, Tool::Blur, Tool::Erase, Tool::Eraser]),
    ("强调", &[Tool::Spotlight, Tool::Magnify]),
];

fn group_of(tool: Tool) -> usize {
    GROUPS.iter().position(|(_, tools)| tools.contains(&tool)).unwrap_or(0)
}

/// 截图工具栏是否展开全部工具；进程内各次截图共享（Vue 版存于 localStorage）。
static EXPANDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn expanded() -> bool {
    EXPANDED.load(std::sync::atomic::Ordering::Relaxed)
}

/// 工具栏图标（Lucide，ISC 许可，见 assets/icons/LICENSE-LUCIDE）；以 SVG 数据嵌入，不注册资源路径。
pub mod icons {
    macro_rules! icon {
        ($name:ident, $file:literal) => {
            pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));
        };
    }
    icon!(POINTER, "mouse-pointer-2");
    icon!(SQUARE, "square");
    icon!(CIRCLE, "circle");
    icon!(MINUS, "minus");
    icon!(ARROW, "arrow-up-right");
    icon!(ARROW_TEXT, "arrow-text");
    icon!(SPLINE, "spline");
    icon!(PENCIL, "pencil");
    icon!(HIGHLIGHTER, "highlighter");
    icon!(TYPE, "type");
    icon!(NUMBER, "list-ordered");
    icon!(FOCUS, "focus");
    icon!(ZOOM, "zoom-in");
    icon!(GRID, "grid-2x2");
    icon!(DROPLETS, "droplets");
    icon!(ERASER, "eraser");
    icon!(SQUARE_X, "square-x");
    icon!(UNDO, "undo-2");
    icon!(REDO, "redo-2");
    icon!(TRASH, "trash");
    icon!(CLEAR, "brush-cleaning");
    icon!(MOVE, "move");
    icon!(SLIDERS, "sliders-horizontal");
    icon!(PIN, "pin");
    icon!(PIN_OFF, "pin-off");
    icon!(SAVE, "save");
    icon!(CLOSE, "x");
    icon!(COPY, "copy");
}

/// 当前样式；选中图形时同步为该图形的样式。
#[derive(Clone, Debug)]
pub struct Style {
    pub color: String,
    pub width: f64,
    pub opacity: f64,
    pub magnification: f64,
    pub font_size: f64,
    pub font_family: String,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: "#ef4444".into(),
            width: 3.0,
            opacity: 1.0,
            magnification: 2.0,
            font_size: 24.0,
            font_family: "sans".into(),
        }
    }
}

/// 样式修改；一次只改一项。
#[derive(Clone, Debug, PartialEq)]
pub enum StylePatch {
    Color(String),
    Width(f64),
    Opacity(f64),
    Magnification(f64),
    FontSize(f64),
    FontFamily(String),
}

#[derive(Clone, Copy)]
enum Mode {
    Draw,
    Move,
    Resize(Handle),
}

struct Gesture {
    start: Point,
    shape: Annotation,
    mode: Mode,
    /// 移动前允许的抖动，避免单击选中时产生 1 像素位移。
    threshold: f64,
}

/// 正在输入的文字。`shape.start` 为输入框左上角；`index` 为重新编辑的已有文字。
#[derive(Clone)]
pub struct TextDraft {
    pub shape: Annotation,
    pub index: Option<usize>,
    /// 箭头文字：先画箭头，再输入文字，两层一起提交。
    pub arrow: Option<Annotation>,
}

/// 栅格化完成后如何写回。
#[derive(Clone)]
enum Target {
    Append,
    Replace(usize),
    /// 箭头文字：箭头与文字一起追加，文字按箭头中点水平居中。
    ArrowText(Annotation),
    /// 修改样式；聚光灯颜色/透明度同步到全部亮区。
    Restyle(usize),
}

/// 交给宿主在后台执行的栅格化任务。
#[derive(Clone)]
pub struct Job {
    generation: u64,
    target: Target,
    pub shape: Annotation,
    pub crop: Bounds,
    pub preserve: bool,
}

impl Job {
    /// 需要读取的字体族（文字）与是否需要粗体（序号）；滤镜不需要字体。
    pub fn fonts(&self) -> Option<(Option<String>, bool)> {
        match self.shape.kind {
            Kind::Text => Some((self.shape.font_family.clone(), false)),
            Kind::Number => Some((None, true)),
            _ => None,
        }
    }

    /// 在后台线程执行：读取字体（需要时）并栅格化。
    pub fn run(&self, source: &RgbaImage, user_font: Option<Vec<u8>>) -> Result<Annotation, String> {
        let fonts = match self.fonts() {
            Some((family, bold)) => raster::Fonts::system(user_font, family.as_deref(), bold)?,
            None => raster::Fonts {
                sans: Vec::new(),
                serif: None,
                mono: None,
                bold: None,
            },
        };
        raster::rasterize(&self.shape, source, self.crop, &fonts, self.preserve)
    }
}

pub struct Editor {
    history: History,
    pub tool: Tool,
    pub selected: Option<usize>,
    pub style: Style,
    /// 正在绘制或编辑的图形（预览用）。
    pub draft: Option<Annotation>,
    pub text: Option<TextDraft>,
    gesture: Option<Gesture>,
    pub crop: Bounds,
    pub pending: bool,
    pub error: Option<String>,
    /// 撤销历史每次变化加一；预览图层据此判断是否需要重绘。
    pub version: u64,
    /// 草稿每次变化加一。
    pub draft_version: u64,
    /// 丢弃过期的栅格化结果（撤销、取消或重置之后）。
    generation: u64,
    /// 折叠工具栏中展开下拉的分组。
    pub open_group: Option<usize>,
    /// 各分组最近使用的工具。
    group_tools: [Tool; 7],
}

impl Editor {
    pub fn new(crop: Bounds, shapes: Vec<Annotation>) -> Self {
        Self {
            history: History::new(shapes),
            tool: Tool::Edit,
            selected: None,
            style: Style::default(),
            draft: None,
            text: None,
            gesture: None,
            crop,
            pending: false,
            error: None,
            version: 0,
            draft_version: 0,
            generation: 0,
            open_group: None,
            group_tools: GROUPS.map(|(_, tools)| tools[0]),
        }
    }

    pub fn shapes(&self) -> &[Annotation] {
        self.history.present()
    }
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    /// 正在绘制、拖动或输入文字。
    pub fn interacting(&self) -> bool {
        self.draft.is_some() || self.text.is_some()
    }
    /// 正在拖动选中的图形（预览时从底层图层中排除它）。
    pub fn editing_index(&self) -> Option<usize> {
        self.gesture
            .as_ref()
            .filter(|g| !matches!(g.mode, Mode::Draw))
            .and(self.selected)
    }

    fn touch_draft(&mut self) {
        self.draft_version = self.draft_version.wrapping_add(1);
    }

    fn commit(&mut self, shapes: Vec<Annotation>) {
        if shapes.iter().filter_map(|s| s.raster.as_ref()).map(String::len).sum::<usize>() > 8_000_000 {
            self.error = Some("文字和滤镜数据总量过大，请删除部分标注。".into());
            return;
        }
        self.history.commit(shapes);
        self.version += 1;
    }

    /// 替换全部标注并清空历史（贴图变换后标注坐标已变，旧历史不再适用）。
    pub fn reset(&mut self, shapes: Vec<Annotation>) {
        self.generation += 1;
        self.history = History::new(shapes);
        self.selected = None;
        self.pending = false;
        self.abort();
        self.error = None;
        self.version += 1;
    }

    /// 放弃当前手势与草稿，保留已提交图层。
    pub fn abort(&mut self) {
        self.gesture = None;
        self.draft = None;
        self.text = None;
        self.touch_draft();
    }

    fn locked(&self) -> bool {
        self.pending
    }

    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&i| i < self.shapes().len());
        if let Some(s) = self.selected.map(|i| self.shapes()[i].clone()) {
            self.style.color = s.color.clone();
            self.style.width = s.width;
            self.style.opacity = s.opacity;
            self.style.magnification = s.magnification.unwrap_or(2.0);
            self.style.font_size = s.font_size.unwrap_or(24.0);
            self.style.font_family = s.font_family.clone().unwrap_or_else(|| "sans".into());
        }
    }

    /// 切换工具并套用该工具的默认样式（荧光笔宽而半透明、聚光灯沿用已有亮区等）。
    pub fn set_tool(&mut self, tool: Tool) {
        if self.locked() || self.interacting() {
            return;
        }
        self.tool = tool;
        self.selected = None;
        self.error = None;
        self.open_group = None;
        self.group_tools[group_of(tool)] = tool;
        match tool {
            Tool::Highlighter => {
                self.style.width = 20.0;
                self.style.opacity = 0.35;
            }
            Tool::Spotlight => {
                let existing = self
                    .shapes()
                    .iter()
                    .find(|s| matches!(s.kind, Kind::Spotlight))
                    .map(|s| (s.color.clone(), s.opacity));
                let (color, opacity) = existing.unwrap_or(("#000000".into(), 0.7));
                self.style.color = color;
                self.style.opacity = opacity;
                self.style.width = 3.0;
            }
            Tool::Erase => {
                self.style.width = 24.0;
                self.style.opacity = 1.0;
            }
            Tool::Mosaic | Tool::Blur => {
                self.style.width = 16.0;
                self.style.opacity = 1.0;
            }
            _ => {
                if matches!(self.style.color.as_str(), "#000000") {
                    self.style.color = Style::default().color;
                }
                self.style.width = 3.0;
                self.style.opacity = 1.0;
            }
        }
    }

    /// 选中图形上命中的缩放手柄。
    pub fn handle_at(&self, p: Point, tolerance: f64) -> Option<Handle> {
        let shape = self.shapes().get(self.selected?)?;
        let near = |q: Point| (p.x - q.x).abs() <= tolerance && (p.y - q.y).abs() <= tolerance;
        if edit::uses_endpoints(shape.kind) {
            return [(Handle::Start, shape.start), (Handle::End, shape.end)]
                .into_iter()
                .find(|(_, q)| near(*q))
                .map(|(h, _)| h);
        }
        let b = edit::bounds(shape);
        Handle::BOX.into_iter().find(|h| near(h.point(b)))
    }

    /// 某点是否落在已有图形上（宿主据此决定交给编辑器还是选区）。
    pub fn hit(&self, p: Point, tolerance: f64) -> Option<usize> {
        edit::hit_test(self.shapes(), p, tolerance, true, true)
    }

    /// 按下：命中手柄则缩放；选择工具或绘制几何图形时命中已有图形则选中并准备移动；否则开始绘制。
    /// `force_draw`（Alt）时总是绘制。返回整项删除等立即完成的操作不需要后台任务。
    pub fn begin(&mut self, p: Point, tolerance: f64, handle: Option<Handle>, force_draw: bool) {
        self.open_group = None;
        if self.locked() {
            return;
        }
        let p = self.crop.clamp(p);
        if self.tool == Tool::Polyline {
            if let Some(draft) = self.draft.as_mut() {
                if draft.points.len() < MAX_POINTS {
                    draft.points.push(p);
                    draft.end = p;
                }
                self.touch_draft();
                return;
            }
        }
        if self.interacting() {
            return;
        }
        let direct = !force_draw && !self.tool.draws_over();
        let index = if handle.is_some() {
            self.selected
        } else {
            edit::hit_test(
                self.shapes(),
                p,
                tolerance,
                self.tool != Tool::Eraser,
                self.tool != Tool::Eraser,
            )
        };
        if handle.is_some()
            || matches!(self.tool, Tool::Eraser | Tool::Edit)
            || (direct && index.is_some())
        {
            self.select(index);
            let Some(selected) = self.selected else {
                return;
            };
            if self.tool == Tool::Eraser {
                self.remove();
                return;
            }
            let shape = self.shapes()[selected].clone();
            self.gesture = Some(Gesture {
                start: p,
                shape: shape.clone(),
                mode: handle.map_or(Mode::Move, Mode::Resize),
                threshold: tolerance / 2.0,
            });
            self.draft = Some(shape);
            self.touch_draft();
            return;
        }
        let reserve = if self.tool == Tool::ArrowText { 2 } else { 1 };
        if self.shapes().len() + reserve > MAX_SHAPES {
            self.error = Some("最多支持 100 个标注。".into());
            return;
        }
        let Some(kind) = self.tool.kind() else {
            return;
        };
        self.selected = None;
        let mut shape = Annotation {
            kind,
            start: p,
            end: p,
            color: self.style.color.clone(),
            width: self.style.width,
            opacity: self.style.opacity,
            magnification: None,
            points: vec![],
            text: None,
            font_size: None,
            font_family: None,
            raster: None,
        };
        match kind {
            Kind::Magnify => shape.magnification = Some(self.style.magnification),
            Kind::Number => {
                let c = self.crop;
                let size = 36.0_f64.min(c.width).min(c.height);
                let value = edit::next_number(self.shapes());
                if value > 999 || size < 12.0 {
                    self.error = Some(
                        if value > 999 { "序号最多为 999。" } else { "选区太小，无法放置序号。" }.into(),
                    );
                    return;
                }
                let x = edit::clamp(p.x - size / 2.0, c.x, c.right() - size);
                let y = edit::clamp(p.y - size / 2.0, c.y, c.bottom() - size);
                shape.start = Point { x, y };
                shape.end = Point { x: x + size, y: y + size };
                shape.text = Some(value.to_string());
            }
            Kind::Pen | Kind::Highlighter | Kind::Polyline | Kind::Erase => shape.points = vec![p],
            Kind::Text => {
                self.start_text(p, None);
                return;
            }
            _ => {}
        }
        self.draft = Some(shape.clone());
        if kind != Kind::Polyline {
            self.gesture = Some(Gesture {
                start: p,
                shape,
                mode: Mode::Draw,
                threshold: 0.0,
            });
        }
        self.touch_draft();
    }

    /// 放置空输入框；靠近右/下边缘时让出可输入的空间。
    fn start_text(&mut self, p: Point, arrow: Option<Annotation>) {
        let c = self.crop;
        let line = (self.style.font_size * 1.3 + 4.0).ceil();
        let start = Point {
            x: p.x.min(c.x + (c.width - 120.0).max(0.0)),
            y: p.y.min(c.y + (c.height - line).max(0.0)),
        };
        self.text = Some(TextDraft {
            shape: Annotation {
                kind: Kind::Text,
                start,
                end: start,
                color: arrow.as_ref().map_or(self.style.color.clone(), |a| a.color.clone()),
                width: self.style.width,
                opacity: self.style.opacity,
                magnification: None,
                points: vec![],
                text: Some(String::new()),
                font_size: Some(self.style.font_size),
                font_family: Some(self.style.font_family.clone()),
                raster: None,
            },
            index: None,
            arrow,
        });
        self.error = None;
        self.touch_draft();
    }

    /// 重新编辑已有文字（双击）。取消或失败都不改原图层。
    pub fn edit_text(&mut self, index: usize) {
        if self.locked() || self.interacting() {
            return;
        }
        let Some(shape) = self.shapes().get(index).filter(|s| matches!(s.kind, Kind::Text)).cloned() else {
            return;
        };
        self.select(Some(index));
        self.text = Some(TextDraft {
            shape: Annotation { raster: None, ..shape },
            index: Some(index),
            arrow: None,
        });
        self.touch_draft();
    }

    pub fn drag(&mut self, p: Point) {
        if self.locked() {
            return;
        }
        let p = self.crop.clamp(p);
        let Some(gesture) = self.gesture.as_ref() else {
            return;
        };
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let next = match gesture.mode {
            Mode::Move => {
                if (p.x - gesture.start.x).hypot(p.y - gesture.start.y) < gesture.threshold {
                    return;
                }
                edit::move_shape(&gesture.shape, p.x - gesture.start.x, p.y - gesture.start.y, self.crop)
            }
            Mode::Resize(handle) => edit::resize_shape(&gesture.shape, handle, p, self.crop),
            Mode::Draw if !draft.points.is_empty() => {
                let last = *draft.points.last().unwrap_or(&p);
                if (p.x - last.x).hypot(p.y - last.y) < 1.0 || draft.points.len() >= MAX_POINTS {
                    return;
                }
                let mut next = draft.clone();
                next.points.push(p);
                next.end = p;
                next
            }
            Mode::Draw if matches!(draft.kind, Kind::Number | Kind::Text) => return,
            Mode::Draw => Annotation { end: p, ..draft.clone() },
        };
        self.draft = Some(next);
        self.touch_draft();
    }

    /// 松开（或 Enter 完成折线）。需要栅格化时返回任务，交给宿主在后台执行。
    pub fn finish(&mut self, polyline: bool) -> Option<Job> {
        if self.text.is_some() {
            self.gesture = None;
            return None;
        }
        if self.locked() || (self.tool == Tool::Polyline && !polyline && self.gesture.is_none()) {
            return None;
        }
        let mut shape = self.draft.clone()?;
        let editing = self.selected.is_some()
            && self.gesture.as_ref().is_some_and(|g| !matches!(g.mode, Mode::Draw));
        let index = self.selected;
        if editing {
            if let Some(original) = index.and_then(|i| self.shapes().get(i)) {
                if edit::same_shape(&shape, original) {
                    self.gesture = None;
                    self.draft = None;
                    self.touch_draft();
                    return None;
                }
            }
        }
        let dx = (shape.end.x - shape.start.x).abs();
        let dy = (shape.end.y - shape.start.y).abs();
        let area = matches!(
            shape.kind,
            Kind::Rectangle
                | Kind::Ellipse
                | Kind::Text
                | Kind::Number
                | Kind::Mosaic
                | Kind::Blur
                | Kind::Spotlight
                | Kind::Magnify
        );
        let too_small = if editing {
            area && (dx < 1.0 || dy < 1.0)
        } else if matches!(shape.kind, Kind::Erase) {
            false
        } else if !shape.points.is_empty() || shape.kind == Kind::Polyline {
            shape.points.len() < 2
        } else if matches!(shape.kind, Kind::Line | Kind::Arrow) {
            dx.hypot(dy) < 1.0
        } else {
            dx < 1.0 || dy < 1.0
        };
        if too_small {
            self.abort();
            return None;
        }
        if !editing && self.tool == Tool::ArrowText && shape.kind == Kind::Arrow {
            // 松开箭头后转为文字输入；两层输入完成才统一提交。
            self.gesture = None;
            self.draft = None;
            let mid = Point {
                x: (shape.start.x + shape.end.x) / 2.0 - 90.0_f64.min(self.crop.width / 2.0),
                y: (shape.start.y + shape.end.y) / 2.0 - (self.style.font_size * 1.3 + 4.0).ceil() / 2.0,
            };
            self.start_text(self.crop.clamp(mid), Some(shape));
            return None;
        }
        if !editing && shape.kind == Kind::Magnify {
            match edit::magnifier_lens(edit::bounds(&shape), shape.magnification.unwrap_or(2.0), self.crop) {
                Ok(lens) => shape = edit::with_bounds(&shape, lens),
                Err(error) => {
                    self.error = Some(error);
                    self.abort();
                    return None;
                }
            }
        }
        if !editing && edit::is_raster(shape.kind) {
            shape = edit::with_bounds(&shape, edit::bounds(&shape));
        }
        let target = match (editing, index) {
            (true, Some(i)) => Target::Replace(i),
            _ => Target::Append,
        };
        if raster::needs_raster(&shape, editing) {
            self.gesture = None;
            return Some(self.job(shape, self.crop, false, target));
        }
        self.apply(target, shape);
        self.abort();
        None
    }

    fn job(&mut self, shape: Annotation, crop: Bounds, preserve: bool, target: Target) -> Job {
        self.pending = true;
        self.error = None;
        Job {
            generation: self.generation,
            target,
            shape,
            crop,
            preserve,
        }
    }

    fn apply(&mut self, target: Target, shape: Annotation) {
        let mut next = self.shapes().to_vec();
        match target {
            Target::Append => next.push(shape),
            Target::Replace(i) if i < next.len() => next[i] = shape,
            Target::Replace(_) => return,
            Target::ArrowText(arrow) => {
                let b = edit::bounds(&shape);
                let c = self.crop;
                let x = edit::clamp((arrow.start.x + arrow.end.x) / 2.0 - b.width / 2.0, c.x, c.right() - b.width);
                next.push(arrow);
                next.push(edit::with_bounds(&shape, Bounds { x, ..b }));
            }
            Target::Restyle(i) if i < next.len() => {
                let spotlight = matches!(next[i].kind, Kind::Spotlight);
                for (j, s) in next.iter_mut().enumerate() {
                    if spotlight && matches!(s.kind, Kind::Spotlight) {
                        s.color = shape.color.clone();
                        s.opacity = shape.opacity;
                    } else if j == i {
                        *s = shape.clone();
                    }
                }
            }
            Target::Restyle(_) => return,
        }
        self.commit(next);
    }

    /// 后台栅格化完成。过期结果（期间撤销、取消或重置）丢弃；失败保留原图层并显示错误。
    pub fn complete(&mut self, job: Job, result: Result<Annotation, String>) {
        if job.generation != self.generation {
            return;
        }
        self.pending = false;
        match result {
            Ok(shape) => {
                let restyle = matches!(job.target, Target::Restyle(_));
                self.apply(job.target, shape);
                if restyle {
                    self.select(self.selected);
                } else {
                    self.abort();
                }
            }
            Err(error) => {
                self.error = Some(error);
                // 文字失败保留输入框，允许修改后重试；其他草稿放弃。
                if self.text.is_none() {
                    self.abort();
                }
            }
        }
    }

    /// 输入完成：空文字时只提交箭头文字的箭头；否则返回栅格化任务。
    pub fn finish_text(&mut self, value: &str) -> Option<Job> {
        if self.locked() {
            return None;
        }
        let draft = self.text.clone()?;
        if value.trim().is_empty() {
            if let Some(arrow) = draft.arrow {
                self.apply(Target::Append, arrow);
            }
            self.abort();
            return None;
        }
        if value.chars().count() > 500 {
            self.error = Some("文字最多 500 个字符。".into());
            return None;
        }
        let shape = Annotation {
            text: Some(value.to_owned()),
            ..draft.shape.clone()
        };
        // 文字从输入框左上角起排，可用宽度为到选区右边缘的距离（最多 2048）。
        let c = self.crop;
        let crop = Bounds {
            x: shape.start.x,
            y: shape.start.y,
            width: (c.right() - shape.start.x).min(2048.0),
            height: c.bottom() - shape.start.y,
        };
        let target = match (draft.index, draft.arrow) {
            (Some(i), _) => Target::Replace(i),
            (None, Some(arrow)) => Target::ArrowText(arrow),
            (None, None) => Target::Append,
        };
        Some(self.job(shape, crop, false, target))
    }

    /// Esc：放弃输入，保留已提交图层。
    pub fn cancel_text(&mut self) {
        if self.text.is_some() {
            self.generation += 1;
            self.pending = false;
            self.error = None;
            self.abort();
        }
    }

    /// 修改样式：输入中只改草稿；选中图形时改该图形（必要时重新栅格化）；聚光灯同步整组。
    pub fn style(&mut self, patch: StylePatch) -> Option<Job> {
        if self.locked() {
            return None;
        }
        if let StylePatch::Magnification(zoom) = patch {
            if !zoom.is_finite() || !(2.0..=4.0).contains(&zoom) {
                self.error = Some("放大倍数须为 2～4。".into());
                return None;
            }
        }
        let s = &mut self.style;
        match &patch {
            StylePatch::Color(color) => s.color = color.clone(),
            StylePatch::Width(width) => s.width = width.clamp(1.0, 64.0),
            StylePatch::Opacity(opacity) => s.opacity = *opacity,
            StylePatch::Magnification(zoom) => s.magnification = *zoom,
            StylePatch::FontSize(size) => s.font_size = size.clamp(8.0, 128.0),
            StylePatch::FontFamily(family) => s.font_family = family.clone(),
        }
        let style = self.style.clone();
        let restyle = |shape: &mut Annotation| match &patch {
            StylePatch::Color(_) => shape.color = style.color.clone(),
            StylePatch::Width(_) => shape.width = style.width,
            StylePatch::Opacity(_) => shape.opacity = style.opacity,
            StylePatch::Magnification(_) => shape.magnification = Some(style.magnification),
            StylePatch::FontSize(_) => shape.font_size = Some(style.font_size),
            StylePatch::FontFamily(_) => shape.font_family = Some(style.font_family.clone()),
        };
        if let Some(text) = self.text.as_mut() {
            if self.gesture.is_none() {
                restyle(&mut text.shape);
                self.error = None;
                self.touch_draft();
            }
            return None;
        }
        if self.interacting() {
            return None;
        }
        let Some(index) = self.selected.filter(|&i| i < self.shapes().len()) else {
            // 未选中时聚光灯工具的颜色/透明度直接作用于已有亮区（一步历史）。
            if self.tool == Tool::Spotlight
                && matches!(patch, StylePatch::Color(_) | StylePatch::Opacity(_))
                && self.shapes().iter().any(|s| matches!(s.kind, Kind::Spotlight))
            {
                let next = self
                    .shapes()
                    .iter()
                    .cloned()
                    .map(|mut s| {
                        if matches!(s.kind, Kind::Spotlight) {
                            s.color = style.color.clone();
                            s.opacity = style.opacity;
                        }
                        s
                    })
                    .collect();
                self.commit(next);
            }
            return None;
        };
        let original = self.shapes()[index].clone();
        // 遮挡滤镜固定不透明；序号/聚光灯没有线宽。
        if matches!(original.kind, Kind::Mosaic | Kind::Blur | Kind::Erase)
            && matches!(patch, StylePatch::Color(_) | StylePatch::Opacity(_))
        {
            return None;
        }
        let mut shape = original.clone();
        restyle(&mut shape);
        if original.kind != Kind::Magnify {
            shape.magnification = original.magnification;
        }
        if original.kind != Kind::Text {
            shape.font_size = original.font_size;
            shape.font_family = original.font_family.clone();
        }
        if edit::same_shape(&shape, &original) {
            return None;
        }
        if original.kind == Kind::Magnify {
            if let StylePatch::Magnification(zoom) = patch {
                // 改倍率时保持被放大的内容不变，由它重新展开镜框。
                match edit::magnifier_source(&original)
                    .and_then(|source| edit::magnifier_lens(source, zoom, self.crop))
                {
                    Ok(lens) => shape = edit::with_bounds(&shape, lens),
                    Err(error) => {
                        self.error = Some(error);
                        return None;
                    }
                }
            }
        }
        let relayout = matches!(patch, StylePatch::FontSize(_) | StylePatch::FontFamily(_));
        let needs = match shape.kind {
            Kind::Text => true,
            Kind::Number => matches!(patch, StylePatch::Color(_)),
            Kind::Mosaic | Kind::Blur | Kind::Magnify => true,
            _ => false,
        };
        if needs {
            return Some(self.job(shape, self.crop, !relayout, Target::Restyle(index)));
        }
        self.apply(Target::Restyle(index), shape);
        self.select(Some(index));
        None
    }

    /// 工具栏自身的界面状态（分组下拉、展开/折叠）。
    pub fn ui(&mut self, action: UiAction) {
        match action {
            UiAction::OpenGroup(index) => {
                self.open_group = if self.open_group == Some(index) { None } else { Some(index) };
            }
            UiAction::ToggleExpanded => {
                EXPANDED.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
                self.open_group = None;
            }
        }
    }

    pub fn remove(&mut self) {
        if self.locked() || self.interacting() {
            return;
        }
        let Some(index) = self.selected.take() else {
            return;
        };
        let mut next = self.shapes().to_vec();
        if index < next.len() {
            next.remove(index);
            self.commit(next);
        }
    }

    pub fn clear(&mut self) {
        if self.locked() || self.interacting() || self.shapes().is_empty() {
            return;
        }
        self.commit(Vec::new());
        self.selected = None;
    }

    pub fn undo(&mut self) {
        if !self.locked() && !self.interacting() && self.history.undo() {
            self.selected = None;
            self.version += 1;
        }
    }

    pub fn redo(&mut self) {
        if !self.locked() && !self.interacting() && self.history.redo() {
            self.selected = None;
            self.version += 1;
        }
    }

    /// 方向键平移选中图形 1 像素（Shift 10 像素）；返回是否处理（需要重新取样的滤镜返回任务）。
    pub fn nudge(&mut self, dx: f64, dy: f64) -> (bool, Option<Job>) {
        let Some(index) = self.selected.filter(|&i| i < self.shapes().len()) else {
            return (false, None);
        };
        if self.locked() || self.interacting() {
            return (true, None);
        }
        let shape = self.shapes()[index].clone();
        self.gesture = Some(Gesture {
            start: shape.start,
            shape: shape.clone(),
            mode: Mode::Move,
            threshold: 0.0,
        });
        self.draft = Some(edit::move_shape(&shape, dx, dy, self.crop));
        (true, self.finish(false))
    }
}

/// 预览图层：底层（已提交、未在拖动的图形）缓存到历史变化为止；草稿只在其外接范围内绘制。
/// 聚光灯与局部擦除影响整层，拖动时与底层合并重绘。被替换的图像放进 `retired`，
/// 由宿主调用 `cx.drop_image` 释放 GPU 纹理。
#[derive(Default)]
pub struct Layers {
    base: Option<(LayerKey, Layer)>,
    draft: Option<(u64, Layer)>,
    retired: Vec<Arc<RenderImage>>,
}

#[derive(Clone, PartialEq)]
struct LayerKey {
    version: u64,
    excluded: Option<usize>,
    crop: (i64, i64, u32, u32),
    /// 合并草稿时为草稿版本。
    merged: Option<u64>,
}

/// 一张预览图与其在原图中的位置（物理像素）。
#[derive(Clone)]
pub struct Layer {
    pub image: Arc<RenderImage>,
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}

fn pixel_rect(b: Bounds) -> (i64, i64, u32, u32) {
    let x = b.x.floor() as i64;
    let y = b.y.floor() as i64;
    let right = b.right().ceil() as i64;
    let bottom = b.bottom().ceil() as i64;
    (x, y, (right - x).max(0) as u32, (bottom - y).max(0) as u32)
}

fn to_layer(mut image: RgbaImage, rect: (i64, i64, u32, u32)) -> Layer {
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Layer {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(image)])),
        x: rect.0,
        y: rect.1,
        width: rect.2,
        height: rect.3,
    }
}

fn draw(shapes: &[Annotation], rect: (i64, i64, u32, u32)) -> Option<RgbaImage> {
    if rect.2 == 0 || rect.3 == 0 || shapes.is_empty() {
        return None;
    }
    let mut image = RgbaImage::new(rect.2, rect.3);
    let origin = Point { x: rect.0 as f64, y: rect.1 as f64 };
    match annotations::render(&mut image, shapes, origin) {
        Ok(()) => Some(image),
        Err(error) => {
            log::warn!("event=gpui.annotation_preview_failed message={error}");
            None
        }
    }
}

/// 栅格类草稿还没有像素数据（刚拖出的马赛克区域等）时不能交给 render，改画虚线框。
pub fn drawable(shape: &Annotation) -> bool {
    !edit::is_raster(shape.kind) || shape.raster.is_some()
}

impl Layers {
    pub fn base(&self) -> Option<&Layer> {
        self.base.as_ref().map(|(_, layer)| layer)
    }
    pub fn draft(&self) -> Option<&Layer> {
        self.draft.as_ref().map(|(_, layer)| layer)
    }
    pub fn take_retired(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.retired)
    }

    /// 按编辑器状态更新图层；未变化时不重绘。
    pub fn update(&mut self, editor: &Editor) {
        let crop = pixel_rect(editor.crop);
        let excluded = editor.editing_index();
        let draft = editor.draft.as_ref().filter(|d| drawable(d));
        let merged = draft.filter(|d| matches!(d.kind, Kind::Spotlight | Kind::Erase));
        let key = LayerKey {
            version: editor.version,
            excluded,
            crop,
            merged: merged.map(|_| editor.draft_version),
        };
        if self.base.as_ref().map(|(k, _)| k) != Some(&key) {
            let mut shapes: Vec<Annotation> = editor
                .shapes()
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != excluded)
                .map(|(_, s)| s.clone())
                .collect();
            if let Some(d) = merged {
                shapes.push(d.clone());
            }
            let layer = draw(&shapes, crop).map(|image| to_layer(image, crop));
            if let Some((_, old)) = self.base.take() {
                self.retired.push(old.image);
            }
            self.base = layer.map(|layer| (key, layer));
        }
        let wanted = draft.filter(|_| merged.is_none());
        if wanted.is_none() {
            if let Some((_, old)) = self.draft.take() {
                self.retired.push(old.image);
            }
            return;
        }
        if self.draft.as_ref().is_some_and(|(v, _)| *v == editor.draft_version) {
            return;
        }
        let shape = wanted.expect("checked above");
        // 线宽与箭头翼会超出外接矩形；按线宽外扩后裁到可编辑范围。
        let b = edit::bounds(shape);
        let pad = shape.width * 4.0 + 12.0;
        let grown = Bounds {
            x: (b.x - pad).max(editor.crop.x),
            y: (b.y - pad).max(editor.crop.y),
            width: 0.0,
            height: 0.0,
        };
        let grown = Bounds {
            width: ((b.right() + pad).min(editor.crop.right()) - grown.x).max(0.0),
            height: ((b.bottom() + pad).min(editor.crop.bottom()) - grown.y).max(0.0),
            ..grown
        };
        let rect = pixel_rect(grown);
        let layer = draw(std::slice::from_ref(shape), rect).map(|image| to_layer(image, rect));
        if let Some((_, old)) = self.draft.take() {
            self.retired.push(old.image);
        }
        self.draft = layer.map(|layer| (editor.draft_version, layer));
    }

    /// 宿主关闭时释放全部纹理。
    pub fn clear(&mut self) {
        if let Some((_, layer)) = self.base.take() {
            self.retired.push(layer.image);
        }
        if let Some((_, layer)) = self.draft.take() {
            self.retired.push(layer.image);
        }
    }
}

/// 工具栏与样式条发出的操作。
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Tool(Tool),
    Ui(UiAction),
    Undo,
    Redo,
    Delete,
    Clear,
    Style(StylePatch),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiAction {
    OpenGroup(usize),
    ToggleExpanded,
}

/// 预设颜色（Vue 版使用系统取色器；这里提供常用色板）。
pub const COLORS: [&str; 9] = [
    "#ef4444", "#f97316", "#facc15", "#22c55e", "#3b82f6", "#a855f7", "#ec4899", "#000000", "#ffffff",
];
const OPACITIES: [f64; 4] = [1.0, 0.7, 0.35, 0.15];

pub const SURFACE: u32 = 0x20252c;
pub const BORDER: u32 = 0x39424e;
pub const TEXT: u32 = 0xe6edf3;
pub const MUTED: u32 = 0x9aa7b4;
pub const HOVER: u32 = 0x2b323c;
pub const ACTIVE_BG: u32 = 0x263a57;
pub const ACTIVE_FG: u32 = 0x93c5fd;
/// 工具行与样式条高度（逻辑像素）。
pub const ROW_HEIGHT: f32 = 40.0;
pub const OPTIONS_HEIGHT: f32 = 36.0;

pub fn icon(data: &'static [u8], color: u32) -> Icon {
    Icon::default().data(data).size_4().text_color(rgb(color))
}

/// 图标按钮；`pressed` 为选中态，`disabled` 时不响应点击。
pub fn icon_button(
    id: impl Into<ElementId>,
    data: &'static [u8],
    tooltip: String,
    pressed: bool,
    disabled: bool,
) -> Stateful<Div> {
    let color = if disabled {
        0x5b6470
    } else if pressed {
        ACTIVE_FG
    } else {
        TEXT
    };
    let mut button = div()
        .id(id)
        .size(px(30.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .child(icon(data, color))
        .tooltip(move |_, cx| cx.new(|_| Tooltip(tooltip.clone().into())).into());
    if pressed {
        button = button.bg(rgb(ACTIVE_BG));
    }
    if !disabled {
        button = button.cursor_pointer().hover(|s| s.bg(rgb(HOVER)));
    }
    button
}

/// 极简悬停提示视图。
pub struct Tooltip(SharedString);
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(4.0))
            .bg(rgb(0x0b0d10))
            .text_xs()
            .text_color(rgb(TEXT))
            .child(self.0.clone())
    }
}

pub fn divider() -> Div {
    div().flex_none().w(px(1.0)).h(px(18.0)).mx(px(5.0)).bg(rgb(BORDER))
}

fn text_button(id: impl Into<ElementId>, label: String, pressed: bool) -> Stateful<Div> {
    let mut b = div()
        .id(id)
        .h(px(24.0))
        .px(px(6.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .text_xs()
        .text_color(rgb(if pressed { ACTIVE_FG } else { TEXT }))
        .hover(|s| s.bg(rgb(HOVER)))
        .child(label);
    if pressed {
        b = b.bg(rgb(ACTIVE_BG));
    }
    b
}

/// 工具栏所需的上下文：翻译函数与是否禁用。
pub struct ToolbarProps<'a> {
    pub editor: &'a Editor,
    /// `active` 为假时只高亮宿主的“移动/调整选区”按钮（截图的选区模式、贴图的移动模式）。
    pub active: bool,
    pub disabled: bool,
    /// 贴图单行工具栏省去删除/清空按钮（仍可用 Delete 与“图片处理”菜单）。
    pub compact: bool,
    /// 按用途分组显示；`collapsible` 时可切换为全部工具（截图），否则总是分组（贴图）。
    pub grouped: bool,
    pub collapsible: bool,
    /// 分组下拉向上弹出（工具栏在内容下方、下方空间不足时）。
    pub popover_up: bool,
    pub text: &'a dyn Fn(&'static str) -> String,
}

type Dispatch<T> = fn(&mut T, Action, &mut Window, &mut Context<T>);

/// 宿主视图与当前窗口：用 `Window::listener_for` 构造点击回调，宿主在渲染时可同时借用自身状态。
pub struct Host<'a, T: 'static> {
    pub view: &'a Entity<T>,
    pub window: &'a Window,
    pub dispatch: Dispatch<T>,
}

fn on<T: 'static>(element: Stateful<Div>, action: Action, enabled: bool, host: &Host<T>) -> Stateful<Div> {
    if !enabled {
        return element;
    }
    let dispatch = host.dispatch;
    element.on_click(host.window.listener_for(host.view, move |this, _: &ClickEvent, window, cx| {
        dispatch(this, action.clone(), window, cx);
    }))
}

/// 工具行：全部工具 + 撤销/重做/删除/清空；`leading`、`trailing` 由宿主追加（移动按钮、导出动作等）。
pub fn tool_row<T: 'static>(
    props: &ToolbarProps,
    leading: Vec<AnyElement>,
    trailing: Vec<AnyElement>,
    host: &Host<T>,
) -> Div {
    let e = props.editor;
    let locked = props.disabled || e.pending || e.interacting();
    let mut row = div()
        .h(px(ROW_HEIGHT))
        .px(px(5.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(SURFACE))
        .shadow_lg()
        .children(leading);
    if props.grouped {
        for (index, (name, tools)) in GROUPS.into_iter().enumerate() {
            let current = if tools.contains(&e.tool) { e.tool } else { e.group_tools[index] };
            let pressed = props.active && tools.contains(&e.tool);
            let button = icon_button(("annotation-group", index), current.icon(), (props.text)(current.label()), pressed, locked);
            let mut group = div().relative().flex().flex_none().items_center().child(on(button, Action::Tool(current), !locked, host));
            if tools.len() > 1 {
                let trigger = div()
                    .id(("annotation-group-trigger", index))
                    .w(px(14.0))
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(if props.popover_up { "▴" } else { "▾" })
                    .tooltip({
                        let name = (props.text)(name);
                        move |_, cx| cx.new(|_| Tooltip(name.clone().into())).into()
                    });
                let trigger = if locked { trigger } else { trigger.cursor_pointer().hover(|s| s.bg(rgb(HOVER))) };
                group = group.child(on(trigger, Action::Ui(UiAction::OpenGroup(index)), !locked, host));
                if e.open_group == Some(index) && !locked {
                    let mut menu = div()
                        .absolute()
                        .left(px(0.0))
                        .p(px(4.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(SURFACE))
                        .shadow_lg();
                    menu = if props.popover_up { menu.bottom(px(36.0)) } else { menu.top(px(36.0)) };
                    for (position, tool) in tools.iter().copied().enumerate() {
                        let item = div()
                            .id(("annotation-group-item", index * 8 + position))
                            .h(px(30.0))
                            .px(px(6.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(HOVER)))
                            .bg(rgb(if props.active && e.tool == tool { ACTIVE_BG } else { SURFACE }))
                            .child(icon(tool.icon(), if props.active && e.tool == tool { ACTIVE_FG } else { TEXT }))
                            .child(div().text_xs().text_color(rgb(TEXT)).whitespace_nowrap().child((props.text)(tool.label())));
                        menu = menu.child(on(item, Action::Tool(tool), true, host));
                    }
                    group = group.child(menu);
                }
            }
            row = row.child(group);
        }
    } else {
        for (index, tool) in Tool::ALL.into_iter().enumerate() {
            let pressed = props.active && e.tool == tool;
            let button = icon_button(("annotation-tool", index), tool.icon(), (props.text)(tool.label()), pressed, locked);
            row = row.child(on(button, Action::Tool(tool), !locked, host));
        }
    }
    if props.collapsible {
        let label = if props.grouped { "展开全部工具" } else { "折叠相似工具" };
        let toggle = div()
            .id("annotation-expand")
            .w(px(18.0))
            .h(px(30.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(HOVER)))
            .text_xs()
            .text_color(rgb(MUTED))
            .child(if props.grouped { "»" } else { "«" })
            .tooltip({
                let label = (props.text)(label);
                move |_, cx| cx.new(|_| Tooltip(label.clone().into())).into()
            });
        row = row.child(on(toggle, Action::Ui(UiAction::ToggleExpanded), !locked, host));
    }
    row = row.child(divider());
    let undo = icon_button("annotation-undo", icons::UNDO, (props.text)("撤销 Ctrl/⌘+Z"), false, locked || !e.can_undo());
    row = row.child(on(undo, Action::Undo, !locked && e.can_undo(), host));
    let redo = icon_button("annotation-redo", icons::REDO, (props.text)("重做 Ctrl/⌘+Shift+Z"), false, locked || !e.can_redo());
    row = row.child(on(redo, Action::Redo, !locked && e.can_redo(), host));
    if !props.compact {
    let delete_ok = !locked && e.selected.is_some();
    let delete = icon_button("annotation-delete", icons::TRASH, (props.text)("删除选中标注 Delete"), false, !delete_ok);
    row = row.child(on(delete, Action::Delete, delete_ok, host));
    let clear_ok = !locked && !e.shapes().is_empty();
    let clear = icon_button("annotation-clear", icons::CLEAR, (props.text)("清空标注，可撤销恢复"), false, !clear_ok);
    row = row.child(on(clear, Action::Clear, clear_ok, host));
    }
    if !trailing.is_empty() {
        row = row.child(divider()).children(trailing);
    }
    row
}

/// 样式条：颜色、粗细/直径/边框、倍率、不透明度、字号与字体；选择工具且未选中图形时返回 None。
pub fn options_row<T: 'static>(props: &ToolbarProps, host: &Host<T>) -> Option<Div> {
    let e = props.editor;
    if !props.active || (e.tool == Tool::Edit && e.selected.is_none()) {
        return None;
    }
    let kind = e
        .selected
        .and_then(|i| e.shapes().get(i))
        .map(|s| s.kind)
        .or(e.tool.kind());
    let text_mode = matches!(kind, Some(Kind::Text)) || e.tool == Tool::ArrowText || e.text.is_some();
    let filter = matches!(kind, Some(Kind::Mosaic | Kind::Blur | Kind::Erase));
    let number = matches!(kind, Some(Kind::Number));
    let spotlight = matches!(kind, Some(Kind::Spotlight));
    let magnify = matches!(kind, Some(Kind::Magnify));
    let enabled = !props.disabled && !e.pending && (!e.interacting() || e.text.is_some());
    let label = |text: String| div().flex_none().text_xs().text_color(rgb(MUTED)).child(text);
    let mut row = div()
        .h(px(OPTIONS_HEIGHT))
        .px(px(10.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(SURFACE))
        .shadow_lg();
    if !filter {
        let mut swatches = div().flex().flex_none().items_center().gap(px(4.0)).child(label((props.text)("颜色")));
        for (index, color) in COLORS.into_iter().enumerate() {
            let current = e.style.color.eq_ignore_ascii_case(color);
            let swatch = div()
                .id(("annotation-color", index))
                .size(px(18.0))
                .flex_none()
                .rounded_full()
                .border_2()
                .border_color(rgb(if current { ACTIVE_FG } else { BORDER }))
                .bg(rgb(u32::from_str_radix(&color[1..], 16).unwrap_or(0)))
                .cursor_pointer();
            swatches = swatches.child(on(swatch, Action::Style(StylePatch::Color(color.into())), enabled, host));
        }
        row = row.child(swatches);
    }
    if !number && !spotlight {
        let name = if magnify {
            "边框"
        } else if matches!(kind, Some(Kind::Erase)) {
            "直径"
        } else {
            "粗细"
        };
        let width = e.style.width;
        let step = if width >= 10.0 { 4.0 } else { 1.0 };
        row = row.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.0))
                .child(label((props.text)(name)))
                .child(on(text_button("annotation-width-down", "−".into(), false), Action::Style(StylePatch::Width((width - step).max(1.0))), enabled, host))
                .child(div().w(px(22.0)).flex().justify_center().text_xs().text_color(rgb(TEXT)).child(format!("{width}")))
                .child(on(text_button("annotation-width-up", "+".into(), false), Action::Style(StylePatch::Width((width + step).min(64.0))), enabled, host)),
        );
    }
    if magnify {
        let mut zoom = div().flex().flex_none().items_center().gap(px(2.0)).child(label((props.text)("倍率")));
        for value in [2.0, 3.0, 4.0] {
            let b = text_button(("annotation-zoom", value as usize), format!("{value}×"), e.style.magnification == value);
            zoom = zoom.child(on(b, Action::Style(StylePatch::Magnification(value)), enabled, host));
        }
        row = row.child(zoom);
    }
    if !filter {
        let mut opacity = div().flex().flex_none().items_center().gap(px(2.0)).child(label((props.text)("不透明度")));
        for (index, value) in OPACITIES.into_iter().enumerate() {
            let b = text_button(("annotation-opacity", index), format!("{}%", (value * 100.0).round()), (e.style.opacity - value).abs() < 0.001);
            opacity = opacity.child(on(b, Action::Style(StylePatch::Opacity(value)), enabled, host));
        }
        row = row.child(opacity);
    }
    if text_mode {
        let size = e.style.font_size;
        row = row.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.0))
                .child(label((props.text)("字号")))
                .child(on(text_button("annotation-font-down", "−".into(), false), Action::Style(StylePatch::FontSize((size - 2.0).max(8.0))), enabled, host))
                .child(div().w(px(24.0)).flex().justify_center().text_xs().text_color(rgb(TEXT)).child(format!("{size}")))
                .child(on(text_button("annotation-font-up", "+".into(), false), Action::Style(StylePatch::FontSize((size + 2.0).min(128.0))), enabled, host)),
        );
        let mut family = div().flex().flex_none().items_center().gap(px(2.0));
        for (index, (value, name)) in [("sans", "无衬线"), ("serif", "衬线"), ("mono", "等宽")].into_iter().enumerate() {
            let b = text_button(("annotation-family", index), (props.text)(name), e.style.font_family == value);
            family = family.child(on(b, Action::Style(StylePatch::FontFamily(value.into())), enabled, host));
        }
        row = row.child(family);
    }
    let hint = if text_mode {
        "Ctrl+Enter 或点击输入框外完成；Esc 取消；双击文字重新编辑"
    } else if e.tool == Tool::Erase {
        "拖动擦除已有标注并露出底图；可撤销"
    } else if spotlight {
        "拖框保留亮区，框外覆盖遮罩"
    } else if magnify {
        "拖框放大中心底图；移动后重新取样"
    } else if number {
        "点击连续放置序号；选择 / 编辑可移动和缩放"
    } else if e.tool == Tool::Polyline {
        "逐点点击，Enter 完成"
    } else {
        "单击标注拖动；Alt 拖动叠加绘制"
    };
    Some(row.child(label((props.text)(hint))))
}

/// 错误提示条。
pub fn error_row(message: String) -> Div {
    div()
        .max_w(px(640.0))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(rgb(SURFACE))
        .text_xs()
        .text_color(rgb(0xfb7185))
        .child(message)
}

/// 由宿主选中图形在屏幕上的外接框与手柄（逻辑像素）。`map` 把原图像素换算为窗口坐标。
pub fn selection_overlay(editor: &Editor, map: &dyn Fn(Point) -> gpui_kit::Point<Pixels>) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    let shape = match (editor.draft.as_ref(), editor.selected) {
        (Some(draft), Some(_)) if editor.editing_index().is_some() => draft,
        (None, Some(i)) => match editor.shapes().get(i) {
            Some(shape) => shape,
            None => return elements,
        },
        _ => {
            // 栅格类草稿尚无像素时画虚线框表示范围。
            if let Some(draft) = editor.draft.as_ref().filter(|d| !drawable(d)) {
                let b = edit::bounds(draft);
                let (a, c) = (map(Point { x: b.x, y: b.y }), map(Point { x: b.right(), y: b.bottom() }));
                elements.push(
                    div()
                        .absolute()
                        .left(a.x)
                        .top(a.y)
                        .w(c.x - a.x)
                        .h(c.y - a.y)
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(0x2f80ff))
                        .into_any_element(),
                );
            }
            return elements;
        }
    };
    let b = edit::bounds(shape);
    let (a, c) = (map(Point { x: b.x, y: b.y }), map(Point { x: b.right(), y: b.bottom() }));
    elements.push(
        div()
            .absolute()
            .left(a.x - px(2.0))
            .top(a.y - px(2.0))
            .w(c.x - a.x + px(4.0))
            .h(c.y - a.y + px(4.0))
            .border_1()
            .border_dashed()
            .border_color(rgb(0x2f80ff))
            .into_any_element(),
    );
    let handles: Vec<Point> = if edit::uses_endpoints(shape.kind) {
        vec![shape.start, shape.end]
    } else {
        Handle::BOX.iter().map(|h| h.point(b)).collect()
    };
    for p in handles {
        let q = map(p);
        elements.push(
            div()
                .absolute()
                .left(q.x - px(4.0))
                .top(q.y - px(4.0))
                .size(px(8.0))
                .rounded_full()
                .bg(rgb(0xffffff))
                .border_1()
                .border_color(rgb(0x2f80ff))
                .into_any_element(),
        );
    }
    elements
}

/// Tauri 版“通用设置”中下载或选择的导出字体；文字标注与文字贴图共用。
pub fn user_font() -> Option<Vec<u8>> {
    let path = snap_core::paths::data_dir()?.join("resources").join("export-font.ttf");
    let bytes = std::fs::read(path).ok()?;
    ab_glyph::FontRef::try_from_slice(&bytes).is_ok().then_some(bytes)
}

#[cfg(test)]
mod tests {
    // 不能 `use super::*`：gpui 的 `test` 宏会遮蔽标准库 #[test]。
    use super::{Bounds, Editor, Handle, Kind, Layers, Point, RgbaImage, StylePatch, Tool, UiAction, annotations};

    const CROP: Bounds = Bounds { x: 0.0, y: 0.0, width: 200.0, height: 100.0 };
    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn drawing_a_rectangle_commits_one_history_step() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Rectangle);
        e.begin(p(10.0, 10.0), 6.0, None, false);
        e.drag(p(30.0, 40.0));
        e.drag(p(50.0, 60.0));
        assert!(e.finish(false).is_none());
        assert_eq!(e.shapes().len(), 1);
        assert_eq!((e.shapes()[0].end.x, e.shapes()[0].end.y), (50.0, 60.0));
        assert!(e.can_undo() && !e.interacting());
        // 单击（无拖动）不产生图形。
        e.set_tool(Tool::Ellipse);
        e.begin(p(150.0, 80.0), 6.0, None, false);
        e.finish(false);
        assert_eq!(e.shapes().len(), 1);
        e.undo();
        assert!(e.shapes().is_empty());
        e.redo();
        assert_eq!(e.shapes().len(), 1);
    }

    #[test]
    fn geometry_tools_select_existing_shapes_and_move_them_within_crop() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Rectangle);
        e.begin(p(10.0, 10.0), 6.0, None, false);
        e.drag(p(50.0, 50.0));
        e.finish(false);
        // 矩形工具点在已有矩形上 → 选中并移动，而不是新画一个。
        e.begin(p(30.0, 30.0), 6.0, None, false);
        assert_eq!(e.selected, Some(0));
        e.drag(p(500.0, 30.0));
        e.finish(false);
        assert_eq!(e.shapes().len(), 1);
        assert_eq!(e.shapes()[0].end.x, 200.0);
        // Alt 叠加绘制。
        e.begin(p(170.0, 20.0), 6.0, None, true);
        e.drag(p(190.0, 40.0));
        e.finish(false);
        assert_eq!(e.shapes().len(), 2);
        // 手柄缩放与删除。
        e.select(Some(1));
        let handle = e.handle_at(p(190.0, 40.0), 4.0);
        assert_eq!(handle, Some(Handle::SE));
        e.begin(p(190.0, 40.0), 6.0, handle, false);
        e.drag(p(195.0, 90.0));
        e.finish(false);
        assert_eq!(e.shapes()[1].end.y, 90.0);
        e.select(Some(1));
        e.remove();
        assert_eq!(e.shapes().len(), 1);
    }

    #[test]
    fn pen_polyline_and_eraser_follow_vue_semantics() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Pen);
        e.begin(p(10.0, 10.0), 6.0, None, false);
        e.drag(p(10.2, 10.2));
        e.drag(p(20.0, 20.0));
        e.drag(p(30.0, 10.0));
        e.finish(false);
        assert_eq!(e.shapes()[0].points.len(), 3);
        e.set_tool(Tool::Polyline);
        e.begin(p(50.0, 50.0), 6.0, None, false);
        assert!(e.finish(false).is_none());
        assert!(e.interacting(), "折线松开后继续添加点");
        e.begin(p(80.0, 60.0), 6.0, None, false);
        e.begin(p(90.0, 90.0), 6.0, None, false);
        e.finish(true);
        assert_eq!(e.shapes()[1].points.len(), 3);
        e.set_tool(Tool::Eraser);
        e.begin(p(20.0, 20.0), 6.0, None, false);
        assert_eq!(e.shapes().len(), 1);
        assert!(matches!(e.shapes()[0].kind, Kind::Polyline));
    }

    #[test]
    fn filters_and_numbers_go_through_raster_jobs() {
        let source = RgbaImage::from_pixel(200, 100, image::Rgba([10, 200, 30, 255]));
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Mosaic);
        e.begin(p(10.0, 10.0), 6.0, None, false);
        e.drag(p(60.0, 50.0));
        let job = e.finish(false).expect("mosaic needs raster");
        assert!(e.pending && job.fonts().is_none());
        // 栅格化期间锁定，不响应新的绘制。
        e.begin(p(100.0, 10.0), 6.0, None, false);
        assert!(e.draft.as_ref().is_some_and(|d| matches!(d.kind, Kind::Mosaic)));
        let result = job.run(&source, None);
        e.complete(job, result);
        assert!(!e.pending && !e.interacting());
        assert!(e.shapes()[0].raster.is_some() && e.shapes()[0].opacity == 1.0);
        annotations::validate(e.shapes(), 200, 100).unwrap();
        // 序号从 1 起连续编号。
        e.set_tool(Tool::Number);
        e.begin(p(150.0, 50.0), 6.0, None, false);
        let job = e.finish(false).expect("number needs raster");
        assert_eq!(job.shape.text.as_deref(), Some("1"));
        assert_eq!(job.fonts(), Some((None, true)));
        // 撤销期间（generation 变化）完成的任务被丢弃。
        e.cancel_text();
        e.reset(e.shapes().to_vec());
        e.complete(job, Err("stale".into()));
        assert!(e.error.is_none());
    }

    #[test]
    fn arrow_text_commits_arrow_alone_when_text_is_empty() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::ArrowText);
        e.begin(p(10.0, 50.0), 6.0, None, false);
        e.drag(p(120.0, 50.0));
        assert!(e.finish(false).is_none());
        assert!(e.text.as_ref().is_some_and(|t| t.arrow.is_some()));
        assert!(e.finish_text("  ").is_none());
        assert_eq!(e.shapes().len(), 1);
        assert!(matches!(e.shapes()[0].kind, Kind::Arrow));
        // 有文字时返回栅格化任务，提交后两层一起追加。
        e.begin(p(10.0, 80.0), 6.0, None, true);
        e.drag(p(120.0, 80.0));
        e.finish(false);
        let job = e.finish_text("说明").expect("text needs raster");
        assert_eq!(job.fonts(), Some((Some("sans".into()), false)));
        let mut text = job.shape.clone();
        text.end = Point { x: text.start.x + 40.0, y: text.start.y + 20.0 };
        text.raster = Some("x".into());
        e.complete(job, Ok(text));
        assert_eq!(e.shapes().len(), 3);
        let t = &e.shapes()[2];
        assert!(((t.start.x + t.end.x) / 2.0 - 65.0).abs() < 1e-9, "文字按箭头中点居中");
    }

    #[test]
    fn styling_selected_shapes_and_spotlight_group() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Spotlight);
        for x in [10.0, 100.0] {
            e.begin(p(x, 10.0), 6.0, None, false);
            e.drag(p(x + 40.0, 50.0));
            e.finish(false);
        }
        assert_eq!(e.style.color, "#000000");
        assert!(e.style(StylePatch::Opacity(0.35)).is_none());
        assert!(e.shapes().iter().all(|s| s.opacity == 0.35));
        e.set_tool(Tool::Line);
        e.begin(p(10.0, 90.0), 6.0, None, false);
        e.drag(p(100.0, 90.0));
        e.finish(false);
        e.select(Some(2));
        assert!(e.style(StylePatch::Width(8.0)).is_none());
        assert_eq!(e.shapes()[2].width, 8.0);
        assert!(e.style(StylePatch::Magnification(5.0)).is_none());
        assert!(e.error.is_some());
    }

    #[test]
    fn layers_render_committed_and_draft_shapes_separately() {
        let mut e = Editor::new(Bounds { x: 100.0, y: 50.0, width: 200.0, height: 100.0 }, vec![]);
        let mut layers = Layers::default();
        layers.update(&e);
        assert!(layers.base().is_none() && layers.draft().is_none());
        e.set_tool(Tool::Line);
        e.begin(p(120.0, 60.0), 6.0, None, false);
        e.drag(p(180.0, 90.0));
        layers.update(&e);
        let draft = layers.draft().expect("draft layer");
        assert!(draft.x >= 100 && draft.width <= 200);
        e.finish(false);
        layers.update(&e);
        assert!(layers.draft().is_none());
        let base = layers.base().expect("base layer");
        assert_eq!((base.x, base.y, base.width, base.height), (100, 50, 200, 100));
        assert!(!layers.take_retired().is_empty());
        // 未变化时不重绘。
        layers.update(&e);
        assert!(layers.take_retired().is_empty());
    }

    #[test]
    fn groups_remember_last_tool_and_dropdown_closes_on_choice() {
        let mut e = Editor::new(CROP, vec![]);
        assert_eq!(e.group_tools[2], Tool::Arrow);
        e.ui(UiAction::OpenGroup(2));
        assert_eq!(e.open_group, Some(2));
        e.ui(UiAction::OpenGroup(2));
        assert_eq!(e.open_group, None);
        e.ui(UiAction::OpenGroup(5));
        e.set_tool(Tool::Blur);
        assert_eq!(e.open_group, None);
        assert_eq!(e.group_tools[5], Tool::Blur);
        e.ui(UiAction::OpenGroup(1));
        e.begin(p(5.0, 5.0), 6.0, None, false);
        assert_eq!(e.open_group, None, "在画布上按下关闭下拉");
    }
}
