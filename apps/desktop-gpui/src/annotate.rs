//! 截图与贴图共用的标注编辑器：工具状态、绘制/选择/移动/缩放手势、撤销重做、样式与预览图层。
//!
//! 行为：
//! - 标注始终使用原图物理像素坐标（截图为整张虚拟桌面截图，贴图为贴图图像），`crop` 为可编辑范围；
//! - 一次拖动只提交一次历史；文字、序号与滤镜在完成时栅格化（`snap_core::annotations::raster`），
//!   栅格化在后台线程进行，期间 `pending` 为真，完成后由宿主调用 [`Editor::complete`]；
//! - 预览图层直接用 `snap_core::annotations::render` 绘制，与导出（`capture::compose`、贴图合成）像素一致。
//!
//! 本模块不持有窗口：宿主（截图浮层、贴图窗口）负责坐标换算、事件分发、文字输入框与后台任务。
use gpui_kit::{component::Icon, *};
use image::RgbaImage;
use snap_core::annotations::{
    self, Annotation, ArrowHeads, Fill, Kind, LineStyle, Point,
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
    /// 工具栏顺序。
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

/// 折叠模式按用途合并工具；每组显示最近使用的工具，
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

/// 截图工具栏是否展开全部工具；进程内各次截图共享，不落盘。
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
    icon!(MOVE, "move");
    icon!(SLIDERS, "sliders-horizontal");
    icon!(PIN, "pin");
    icon!(PIN_OFF, "pin-off");
    icon!(SAVE, "save");
    icon!(CLOSE, "x");
    icon!(COPY, "copy");
    icon!(SCAN_TEXT, "scan-text");
    icon!(LANGUAGES, "languages");
    icon!(QR_CODE, "qr-code");
    icon!(VIDEO, "video");
    icon!(PAUSE, "pause");
    icon!(PLAY, "play");
    icon!(FOLDER_OPEN, "folder-open");
    icon!(SCROLL, "scroll-capture");
    icon!(CHEVRON_DOWN, "chevron-down");
    icon!(CHEVRON_UP, "chevron-up");
    icon!(CHEVRONS_RIGHT, "chevrons-right");
    icon!(CHEVRONS_LEFT, "chevrons-left");
    icon!(MONITOR, "monitor");
    icon!(RESELECT, "rotate-ccw");
    icon!(GRIP, "grip-vertical");
    icon!(APP_WINDOW, "app-window");
    icon!(CHECK, "check");
    icon!(INFO, "info");
    icon!(ARROW_END, "move-right");
    icon!(ARROW_BOTH, "move-horizontal");
    icon!(ARROW_START, "move-left");
    icon!(ARROW_DIMENSION, "dimension");
}

/// 样式条“圆角”按钮使用的半径（原图物理像素）。
pub const ROUNDED_RADIUS: f64 = 12.0;

/// 当前样式；选中图形时同步为该图形的样式。
#[derive(Clone, Debug)]
pub struct Style {
    /// 颜色板上的颜色。文字开启背景时它是背景框颜色，字色由它推导（见 [`paint_text`]）。
    pub color: String,
    /// 线宽；文字为描边宽度。
    pub width: f64,
    pub opacity: f64,
    pub magnification: f64,
    pub font_size: f64,
    pub font_family: String,
    /// 线型、箭头、填充、圆角在切换工具后保留，只作用于支持它们的图形。
    pub line_style: LineStyle,
    pub arrow_heads: ArrowHeads,
    pub fill: Fill,
    pub corner_radius: f64,
    pub text_background: bool,
    pub text_outline: bool,
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
            line_style: LineStyle::Solid,
            arrow_heads: ArrowHeads::End,
            fill: Fill::None,
            corner_radius: 0.0,
            text_background: false,
            text_outline: false,
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
    LineStyle(LineStyle),
    ArrowHeads(ArrowHeads),
    Fill(Fill),
    CornerRadius(f64),
    TextBackground(bool),
    TextOutline(bool),
}

/// 文字的三种颜色都由颜色板上的一种颜色推导，样式条只需一组色块：
/// 开背景时它填充背景框，字色取黑白中对比更强者（与序号一致），否则它就是字色；描边取字色的对比色。
/// 结果写成显式的字色、背景色、描边色，渲染与导出不依赖这条规则。
pub fn paint_text(shape: &mut Annotation, color: &str, background: bool, outline: bool) {
    if background {
        shape.background = Some(color.to_owned());
        shape.color = annotations::contrast(color);
    } else {
        shape.background = None;
        shape.color = color.to_owned();
    }
    shape.outline = outline.then(|| annotations::contrast(&shape.color));
}

/// 文字输入框的字色与底色（`rgb`、`rgba`）：有背景框时用背景色垫底，输入时就能看清对比色的字；
/// 否则沿用半透明白底。三个宿主（截图、贴图、录屏标注）共用。
pub fn text_input_colors(shape: &Annotation) -> (u32, u32) {
    let parse = |color: &str| u32::from_str_radix(color.trim_start_matches('#'), 16).ok();
    let text = parse(&shape.color).unwrap_or(0xef4444);
    let background = shape.background.as_deref().and_then(parse).map_or(0xffffff18, |c| (c << 8) | 0xff);
    (text, background)
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
    /// 每次开始输入递增；界面据此判断现有输入框是否属于当前草稿，避免沿用上一段文字的输入框。
    pub id: u64,
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
    /// 最近一次文字草稿的编号（见 TextDraft::id）。
    text_serial: u64,
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
            text_serial: 0,
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
            // 有背景框的文字，颜色板上选中的是背景色（字色由它推导）。
            self.style.color = s.background.clone().unwrap_or_else(|| s.color.clone());
            self.style.width = s.width;
            self.style.opacity = s.opacity;
            self.style.magnification = s.magnification.unwrap_or(2.0);
            self.style.font_size = s.font_size.unwrap_or(24.0);
            self.style.font_family = s.font_family.clone().unwrap_or_else(|| "sans".into());
            // 线型、箭头、填充等只从带这些属性的图形同步：选中一段文字不会把上次选的虚线清回实线。
            if s.kind.has_line_style() {
                self.style.line_style = s.line_style;
            }
            if s.kind == Kind::Arrow {
                self.style.arrow_heads = s.arrow_heads;
            }
            if s.kind.has_fill() {
                self.style.fill = s.fill;
            }
            if s.kind == Kind::Rectangle {
                self.style.corner_radius = s.corner_radius;
            }
            if s.kind == Kind::Text {
                self.style.text_background = s.background.is_some();
                self.style.text_outline = s.outline.is_some();
            }
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
            line_style: if kind.has_line_style() { self.style.line_style } else { LineStyle::Solid },
            arrow_heads: if kind == Kind::Arrow { self.style.arrow_heads } else { ArrowHeads::End },
            fill: if kind.has_fill() { self.style.fill } else { Fill::None },
            corner_radius: if kind == Kind::Rectangle { self.style.corner_radius } else { 0.0 },
            background: None,
            outline: None,
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
        self.text_serial += 1;
        let color = arrow.as_ref().map_or(self.style.color.clone(), |a| a.color.clone());
        let mut shape = Annotation {
            kind: Kind::Text,
            start,
            end: start,
            color: color.clone(),
            width: self.style.width,
            opacity: self.style.opacity,
            magnification: None,
            points: vec![],
            text: Some(String::new()),
            font_size: Some(self.style.font_size),
            font_family: Some(self.style.font_family.clone()),
            raster: None,
            line_style: LineStyle::Solid,
            arrow_heads: ArrowHeads::End,
            fill: Fill::None,
            corner_radius: 0.0,
            background: None,
            outline: None,
        };
        paint_text(&mut shape, &color, self.style.text_background, self.style.text_outline);
        self.text = Some(TextDraft {
            id: self.text_serial,
            shape,
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
        self.text_serial += 1;
        // 输入框按无装饰文字摆放：起点先移到字形处，提交时 finish_text 再退回同样距离，
        // 有背景框或描边的文字反复编辑也不会逐次向右下漂移。
        let (dx, dy) = raster::decoration_offset(&shape);
        let start = Point { x: shape.start.x + dx, y: shape.start.y + dy };
        self.text = Some(TextDraft {
            id: self.text_serial,
            shape: Annotation { raster: None, start, end: start, ..shape },
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
        // 背景框与描边加大了四周留白：起点向左上退回同样距离，字形落在输入框里的位置。
        let c = self.crop;
        let (dx, dy) = raster::decoration_offset(&draft.shape);
        let start = Point {
            x: (draft.shape.start.x - dx).max(c.x),
            y: (draft.shape.start.y - dy).max(c.y),
        };
        let shape = Annotation {
            text: Some(value.to_owned()),
            start,
            end: start,
            ..draft.shape.clone()
        };
        // 文字从输入框左上角起排，可用宽度为到选区右边缘的距离（最多 2048）。
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
            StylePatch::LineStyle(value) => s.line_style = *value,
            StylePatch::ArrowHeads(value) => s.arrow_heads = *value,
            StylePatch::Fill(value) => s.fill = *value,
            StylePatch::CornerRadius(radius) => s.corner_radius = radius.clamp(0.0, annotations::MAX_CORNER_RADIUS),
            StylePatch::TextBackground(on) => s.text_background = *on,
            StylePatch::TextOutline(on) => s.text_outline = *on,
        }
        let style = self.style.clone();
        // 只改图形支持的属性；不支持时图形不变，随后的 same_shape 判断会直接返回，不产生空的历史步骤。
        let restyle = |shape: &mut Annotation| match &patch {
            StylePatch::Color(_) | StylePatch::TextBackground(_) | StylePatch::TextOutline(_)
                if shape.kind == Kind::Text =>
            {
                paint_text(shape, &style.color, style.text_background, style.text_outline)
            }
            StylePatch::Color(_) => shape.color = style.color.clone(),
            StylePatch::Width(_) => shape.width = style.width,
            StylePatch::Opacity(_) => shape.opacity = style.opacity,
            StylePatch::Magnification(_) => shape.magnification = Some(style.magnification),
            StylePatch::FontSize(_) => shape.font_size = Some(style.font_size),
            StylePatch::FontFamily(_) => shape.font_family = Some(style.font_family.clone()),
            StylePatch::LineStyle(_) if shape.kind.has_line_style() => shape.line_style = style.line_style,
            StylePatch::ArrowHeads(_) if shape.kind == Kind::Arrow => shape.arrow_heads = style.arrow_heads,
            StylePatch::Fill(_) if shape.kind.has_fill() => shape.fill = style.fill,
            StylePatch::CornerRadius(_) if shape.kind == Kind::Rectangle => shape.corner_radius = style.corner_radius,
            StylePatch::LineStyle(_)
            | StylePatch::ArrowHeads(_)
            | StylePatch::Fill(_)
            | StylePatch::CornerRadius(_)
            | StylePatch::TextBackground(_)
            | StylePatch::TextOutline(_) => {}
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
        let font = matches!(patch, StylePatch::FontSize(_) | StylePatch::FontFamily(_));
        // 有装饰的文字三层颜色烤在同一张图里，开关装饰或改色都要重新排版（见 raster::rasterize）。
        let decorated = shape.kind == Kind::Text && (raster::decorated(&original) || raster::decorated(&shape));
        let mut crop = self.crop;
        if shape.kind == Kind::Text {
            // 字形停在原处：装饰或字号改变了四周留白时，起点反向移动同样距离，框和描边向四周展开或收回。
            let (ox, oy) = raster::decoration_offset(&original);
            let (nx, ny) = raster::decoration_offset(&shape);
            shape = edit::map_shape(&shape, |p| Point { x: p.x + ox - nx, y: p.y + oy - ny });
            if decorated && !font {
                // 只改颜色或装饰时从原起点换行：多行文字不会因为可用宽度变大而重排、整体左移。
                let x = shape.start.x.max(crop.x);
                crop = Bounds { x, width: crop.right() - x, ..crop };
            }
        }
        let needs = match shape.kind {
            // 不透明度在合成时才乘上，文字图片不必重画。
            Kind::Text => !matches!(patch, StylePatch::Opacity(_)),
            Kind::Number => matches!(patch, StylePatch::Color(_)),
            Kind::Mosaic | Kind::Blur | Kind::Magnify => true,
            _ => false,
        };
        if needs {
            return Some(self.job(shape, crop, !(font || decorated), Target::Restyle(index)));
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

    /// 追加一组已校验的标注，作为一步撤销（贴图把选中的文字转成荧光笔时使用）；返回是否已提交。
    pub fn append(&mut self, shapes: Vec<Annotation>) -> bool {
        if self.locked() || self.interacting() || shapes.is_empty() {
            return false;
        }
        let mut next = self.shapes().to_vec();
        next.extend(shapes);
        let version = self.version;
        self.commit(next);
        self.selected = None;
        self.version != version
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
                // 录制中的实时滤镜不带像素（编码线程逐帧取样），预览跳过，由宿主画范围框。
                .filter(|(i, s)| Some(*i) != excluded && drawable(s))
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

/// 预设颜色：GPUI 没有原生取色控件，提供常用色板。
pub const COLORS: [&str; 9] = [
    "#ef4444", "#f97316", "#facc15", "#22c55e", "#3b82f6", "#a855f7", "#ec4899", "#000000", "#ffffff",
];
const OPACITIES: [f64; 4] = [1.0, 0.7, 0.35, 0.15];

/// 界面色随主题变化（见 `theme.rs`）。
pub fn surface() -> u32 {
    crate::theme::c().surface
}
pub fn border() -> u32 {
    crate::theme::c().border
}
pub fn text() -> u32 {
    crate::theme::c().text
}
pub fn muted() -> u32 {
    crate::theme::c().muted
}
pub fn hover() -> u32 {
    crate::theme::c().hover
}
pub fn active_bg() -> u32 {
    crate::theme::c().active_bg
}
pub fn active_fg() -> u32 {
    crate::theme::c().active_fg
}
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
    let color = if pressed { active_fg() } else { text() };
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
        button = button.bg(rgb(active_bg()));
    }
    if disabled {
        button = button.opacity(0.4);
    } else {
        button = button.cursor_pointer().hover(|s| s.bg(rgb(hover())));
    }
    button
}

/// 极简悬停提示视图。
pub struct Tooltip(pub SharedString);
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // 提示框：浅色主题深底浅字，深色主题反过来。
        let (bg, fg) = if crate::theme::dark() { (0xf6f8faf2, 0x1f2328) } else { (0x1f2328f2, 0xf6f8fa) };
        div()
            .px(px(8.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .bg(rgba(bg))
            .shadow_md()
            .text_size(px(12.0))
            .line_height(relative(1.45))
            .text_color(rgb(fg))
            .child(self.0.clone())
    }
}

pub fn divider() -> Div {
    div().flex_none().w(px(1.0)).h(px(18.0)).mx(px(5.0)).bg(rgb(border()))
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
        .text_color(rgb(if pressed { active_fg() } else { text() }))
        .hover(|s| s.bg(rgb(hover())))
        .child(label);
    if pressed {
        b = b.bg(rgb(active_bg()));
    }
    b
}

/// 样张按钮：用小块示意图表示线型、箭头、填充与圆角，名称放在悬停提示里，样式条不必逐项加文字标签。
fn sample_button(id: impl Into<ElementId>, tooltip: String, pressed: bool, sample: impl IntoElement) -> Stateful<Div> {
    let mut button = div()
        .id(id)
        .size(px(24.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(hover())))
        .child(sample)
        .tooltip(move |_, cx| cx.new(|_| Tooltip(tooltip.clone().into())).into());
    if pressed {
        button = button.bg(rgb(active_bg()));
    }
    button
}

/// 线型样张：一条实线、三段短划、四个圆点或“划 点 划”，宽度相近，按钮排在一起时对齐。
fn line_sample(style: LineStyle, color: u32) -> Div {
    let lengths: &[f32] = match style {
        LineStyle::Solid => &[14.0],
        LineStyle::Dashed => &[4.0, 4.0, 4.0],
        LineStyle::Dotted => &[2.0, 2.0, 2.0, 2.0],
        LineStyle::DashDot => &[5.0, 2.0, 5.0],
    };
    div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .children(lengths.iter().map(move |&length| div().w(px(length)).h(px(2.0)).rounded(px(1.0)).bg(rgb(color))))
}

/// 填充与圆角样张：12 像素小方框，填充用底色深浅表示，圆角用方框圆角表示。
fn box_sample(fill: Fill, radius: f32, color: u32) -> Div {
    let square = div().size(px(12.0)).rounded(px(radius)).border_1().border_color(rgb(color));
    match fill {
        Fill::None => square,
        Fill::Translucent => square.bg(rgba((color << 8) | 0x55)),
        Fill::Solid => square.bg(rgb(color)),
    }
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
        .border_color(rgb(border()))
        .bg(rgb(surface()))
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
                    .child(icon(if props.popover_up { icons::CHEVRON_UP } else { icons::CHEVRON_DOWN }, muted()).size(px(12.0)))
                    .tooltip({
                        let name = (props.text)(name);
                        move |_, cx| cx.new(|_| Tooltip(name.clone().into())).into()
                    });
                let trigger = if locked { trigger } else { trigger.cursor_pointer().hover(|s| s.bg(rgb(hover()))) };
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
                        .border_color(rgb(border()))
                        .bg(rgb(surface()))
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
                            .hover(|s| s.bg(rgb(hover())))
                            .bg(rgb(if props.active && e.tool == tool { active_bg() } else { surface() }))
                            .child(icon(tool.icon(), if props.active && e.tool == tool { active_fg() } else { text() }))
                            .child(div().text_xs().text_color(rgb(text())).whitespace_nowrap().child((props.text)(tool.label())));
                        menu = menu.child(on(item, Action::Tool(tool), true, host));
                    }
                    // 下拉向下展开时会盖住紧随其后绘制的样式栏：延后绘制并遮挡下层，保证菜单在最上层且能点到。
                    // 菜单项在松开时触发；按下必须在菜单内截住，否则落到截图浮层上，在选区外会开始重新框选。
                    let menu = menu.occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                    group = group.child(deferred(menu).with_priority(1));
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
            .w(px(30.0))
            .h(px(30.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(hover())))
            // 双箭头横向指示整行变长/变短，与各工具旁的单箭头下拉区分。
            .child(icon(if props.grouped { icons::CHEVRONS_RIGHT } else { icons::CHEVRONS_LEFT }, text()))
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
    let clear = icon_button("annotation-clear", icons::ERASER, (props.text)("清空标注，可撤销恢复"), false, !clear_ok)
        .w_auto()
        .px(px(6.0))
        .gap(px(4.0))
        .child(div().text_size(px(13.0)).text_color(rgb(text())).child((props.text)("清空")));
    row = row.child(on(clear, Action::Clear, clear_ok, host));
    }
    if !trailing.is_empty() {
        row = row.child(divider()).children(trailing);
    }
    row
}

/// 样式条：颜色、粗细/直径/边框、线型、箭头、填充与圆角、倍率、不透明度、字号、字体与文字装饰，
/// 末尾的 info 图标悬停显示当前工具的操作提示；选择工具且未选中图形时返回 None。
/// 只列出当前图形支持的项；贴图工具栏窗口按 [`options_width`] 的估算加宽。
pub fn options_row<T: 'static>(props: &ToolbarProps, host: &Host<T>) -> Option<Div> {
    let e = props.editor;
    if !props.active || (e.tool == Tool::Edit && e.selected.is_none()) {
        return None;
    }
    // 输入文字时样式只作用于文字草稿（箭头文字的箭头已经画好），样式条按文字显示。
    let kind = if e.text.is_some() {
        Some(Kind::Text)
    } else {
        e.selected.and_then(|i| e.shapes().get(i)).map(|s| s.kind).or(e.tool.kind())
    };
    let text_mode = matches!(kind, Some(Kind::Text));
    let filter = matches!(kind, Some(Kind::Mosaic | Kind::Blur | Kind::Erase));
    let number = matches!(kind, Some(Kind::Number));
    let spotlight = matches!(kind, Some(Kind::Spotlight));
    let magnify = matches!(kind, Some(Kind::Magnify));
    let enabled = !props.disabled && !e.pending && (!e.interacting() || e.text.is_some());
    let label = |text: String| div().flex_none().text_xs().text_color(rgb(muted())).child(text);
    let mut row = div()
        .h(px(OPTIONS_HEIGHT))
        .px(px(10.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(border()))
        .bg(rgb(surface()))
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
                .border_color(rgb(if current { active_fg() } else { border() }))
                .bg(rgb(u32::from_str_radix(&color[1..], 16).unwrap_or(0)))
                .cursor_pointer();
            swatches = swatches.child(on(swatch, Action::Style(StylePatch::Color(color.into())), enabled, host));
        }
        row = row.child(swatches);
    }
    let width_control = |name: &'static str| {
        let width = e.style.width;
        let step = if width >= 10.0 { 4.0 } else { 1.0 };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .child(label((props.text)(name)))
            .child(on(text_button("annotation-width-down", "−".into(), false), Action::Style(StylePatch::Width((width - step).max(1.0))), enabled, host))
            .child(div().w(px(22.0)).flex().justify_center().text_xs().text_color(rgb(text())).child(format!("{width}")))
            .child(on(text_button("annotation-width-up", "+".into(), false), Action::Style(StylePatch::Width((width + step).min(64.0))), enabled, host))
    };
    // 文字的粗细是描边宽度，放在“描边”开关旁边，只在开启描边时出现。
    if !number && !spotlight && !text_mode {
        let name = if magnify {
            "边框"
        } else if matches!(kind, Some(Kind::Erase)) {
            "直径"
        } else {
            "粗细"
        };
        row = row.child(width_control(name));
    }
    let sample = |pressed: bool| if pressed { active_fg() } else { text() };
    if let Some(kind) = kind.filter(|k| k.has_line_style()) {
        let mut group = div().flex().flex_none().items_center().gap(px(2.0));
        for (index, (value, name)) in [(LineStyle::Solid, "实线"), (LineStyle::Dashed, "虚线"), (LineStyle::Dotted, "点线"), (LineStyle::DashDot, "点划线")].into_iter().enumerate() {
            let pressed = e.style.line_style == value;
            let b = sample_button(("annotation-line-style", index), (props.text)(name), pressed, line_sample(value, sample(pressed)));
            group = group.child(on(b, Action::Style(StylePatch::LineStyle(value)), enabled, host));
        }
        if kind == Kind::Arrow {
            group = group.child(divider());
            for (index, (value, name, data)) in [
                (ArrowHeads::End, "单向箭头", icons::ARROW_END),
                (ArrowHeads::Start, "反向箭头", icons::ARROW_START),
                (ArrowHeads::Both, "双向箭头", icons::ARROW_BOTH),
                (ArrowHeads::Dimension, "尺寸标注线", icons::ARROW_DIMENSION),
            ].into_iter().enumerate() {
                let pressed = e.style.arrow_heads == value;
                let b = sample_button(("annotation-arrow-heads", index), (props.text)(name), pressed, icon(data, sample(pressed)));
                group = group.child(on(b, Action::Style(StylePatch::ArrowHeads(value)), enabled, host));
            }
        }
        row = row.child(group);
    }
    if let Some(kind) = kind.filter(|k| k.has_fill()) {
        let mut group = div().flex().flex_none().items_center().gap(px(2.0));
        for (index, (value, name)) in [(Fill::None, "无填充"), (Fill::Translucent, "半透明填充"), (Fill::Solid, "实心填充")].into_iter().enumerate() {
            let pressed = e.style.fill == value;
            let b = sample_button(("annotation-fill", index), (props.text)(name), pressed, box_sample(value, 2.0, sample(pressed)));
            group = group.child(on(b, Action::Style(StylePatch::Fill(value)), enabled, host));
        }
        if kind == Kind::Rectangle {
            group = group.child(divider());
            for (index, (radius, name)) in [(0.0, "直角矩形"), (ROUNDED_RADIUS, "圆角矩形")].into_iter().enumerate() {
                let pressed = (e.style.corner_radius > 0.0) == (radius > 0.0);
                let b = sample_button(("annotation-corner", index), (props.text)(name), pressed, box_sample(Fill::None, if radius > 0.0 { 5.0 } else { 0.0 }, sample(pressed)));
                group = group.child(on(b, Action::Style(StylePatch::CornerRadius(radius)), enabled, host));
            }
        }
        row = row.child(group);
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
                .child(div().w(px(24.0)).flex().justify_center().text_xs().text_color(rgb(text())).child(format!("{size}")))
                .child(on(text_button("annotation-font-up", "+".into(), false), Action::Style(StylePatch::FontSize((size + 2.0).min(128.0))), enabled, host)),
        );
        let mut family = div().flex().flex_none().items_center().gap(px(2.0));
        for (index, (value, name)) in [("sans", "无衬线"), ("serif", "衬线"), ("mono", "等宽")].into_iter().enumerate() {
            let b = text_button(("annotation-family", index), (props.text)(name), e.style.font_family == value);
            family = family.child(on(b, Action::Style(StylePatch::FontFamily(value.into())), enabled, host));
        }
        row = row.child(family);
        let background = e.style.text_background;
        let outline = e.style.text_outline;
        let mut decoration = div().flex().flex_none().items_center().gap(px(2.0));
        decoration = decoration.child(on(text_button("annotation-text-background", (props.text)("背景"), background), Action::Style(StylePatch::TextBackground(!background)), enabled, host));
        decoration = decoration.child(on(text_button("annotation-text-outline", (props.text)("描边"), outline), Action::Style(StylePatch::TextOutline(!outline)), enabled, host));
        row = row.child(decoration);
        if outline {
            row = row.child(width_control("粗细"));
        }
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
    // 操作提示收进 info 图标（AGENTS.md：次要说明用 info 图标与悬停提示，不常驻），给样式控件让出宽度。
    let hint = (props.text)(hint);
    let info = div()
        .id("annotation-hint")
        .size(px(24.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(|s| s.bg(rgb(hover())))
        .child(icon(icons::INFO, muted()))
        .tooltip(move |_, cx| cx.new(|_| Tooltip(hint.clone().into())).into());
    Some(row.child(info))
}

/// 样式条的估算宽度（逻辑像素），每组按英文文案留余量。贴图工具栏是贴在图片外的独立小窗口，
/// 按它加宽以免末尾控件被裁掉；截图与录屏标注的浮层足够宽，不需要。分组规则与 [`options_row`] 一致，增删控件时同步。
pub fn options_width(editor: &Editor) -> f32 {
    let kind = if editor.text.is_some() {
        Some(Kind::Text)
    } else {
        editor.selected.and_then(|i| editor.shapes().get(i)).map(|s| s.kind).or(editor.tool.kind())
    };
    // 提示图标总在。
    let mut groups = vec![26.0_f32];
    if !matches!(kind, Some(Kind::Mosaic | Kind::Blur | Kind::Erase)) {
        // 色块与不透明度。
        groups.extend([230.0, 200.0]);
    }
    match kind {
        Some(Kind::Text) => {
            // 字号、字体、背景与描边；开启描边时多一组粗细。
            groups.extend([120.0, 190.0, 140.0]);
            if editor.style.text_outline {
                groups.push(100.0);
            }
        }
        Some(Kind::Number | Kind::Spotlight) => {}
        Some(kind) => {
            groups.push(100.0);
            if kind.has_line_style() {
                groups.push(if kind == Kind::Arrow { 140.0 } else { 80.0 });
            }
            if kind.has_fill() {
                groups.push(if kind == Kind::Rectangle { 140.0 } else { 80.0 });
            }
            if kind == Kind::Magnify {
                groups.push(130.0);
            }
        }
        None => groups.push(100.0),
    }
    // 组间距 10，左右内边距与边框 22。
    groups.iter().sum::<f32>() + 10.0 * (groups.len() - 1) as f32 + 22.0
}

/// 错误提示条。
pub fn error_row(message: String) -> Div {
    div()
        .max_w(px(640.0))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(rgb(surface()))
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

/// “通用设置”中下载或选择的导出字体；文字标注与文字贴图共用。
pub fn user_font() -> Option<Vec<u8>> {
    let path = snap_core::paths::data_dir()?.join("resources").join("export-font.ttf");
    let bytes = std::fs::read(path).ok()?;
    ab_glyph::FontRef::try_from_slice(&bytes).is_ok().then_some(bytes)
}

#[cfg(test)]
mod tests {
    // 不能 `use super::*`：gpui 的 `test` 宏会遮蔽标准库 #[test]。
    use super::{
        ArrowHeads, Bounds, Editor, Fill, Handle, Kind, Layers, LineStyle, Point, ROUNDED_RADIUS, RgbaImage, StylePatch, Tool,
        UiAction, annotations, options_width, raster, text_input_colors,
    };

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
    fn pen_polyline_and_eraser_semantics() {
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

    #[test]
    fn spotlight_previews_while_dragging() {
        let mut e = Editor::new(Bounds { x: 0.0, y: 0.0, width: 200.0, height: 100.0 }, vec![]);
        let mut layers = Layers::default();
        e.set_tool(Tool::Spotlight);
        e.begin(p(20.0, 20.0), 6.0, None, false);
        e.drag(p(80.0, 60.0));
        layers.update(&e);
        let base = layers.base().expect("拖动中应有聚光灯预览");
        assert_eq!((base.width, base.height), (200, 100));
        // 继续拖动时预览跟着更新。
        let before = std::sync::Arc::as_ptr(&base.image);
        e.drag(p(120.0, 80.0));
        layers.update(&e);
        assert_ne!(std::sync::Arc::as_ptr(&layers.base().expect("预览").image), before);
    }

    #[test]
    fn each_text_draft_gets_a_new_id_even_before_the_previous_one_is_rasterized() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Text);
        e.begin(p(10.0, 10.0), 6.0, None, false);
        let first = e.text.as_ref().expect("text draft").id;
        // 提交后栅格化尚未完成，草稿仍在；完成后结束草稿。
        let job = e.finish_text("123").expect("raster job");
        assert_eq!(e.text.as_ref().map(|t| t.id), Some(first));
        let shape = job.shape.clone();
        e.complete(job, Ok(shape));
        assert!(e.text.is_none());        e.begin(p(40.0, 40.0), 6.0, None, false);
        let second = e.text.as_ref().expect("second text draft");
        assert_ne!(second.id, first, "新文字草稿必须换新输入框");
        assert_eq!(second.shape.text.as_deref(), Some(""));
    }

    #[test]
    fn line_style_fill_corners_and_arrow_heads_apply_where_supported_with_undo() {
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Rectangle);
        assert!(e.style(StylePatch::LineStyle(LineStyle::Dashed)).is_none());
        assert!(e.style(StylePatch::Fill(Fill::Translucent)).is_none());
        e.begin(p(10.0, 10.0), 6.0, None, false);
        e.drag(p(60.0, 50.0));
        e.finish(false);
        let rect = &e.shapes()[0];
        assert_eq!((rect.line_style, rect.fill, rect.corner_radius), (LineStyle::Dashed, Fill::Translucent, 0.0));
        // 选中后改圆角是一步历史，可撤销、重做。
        e.set_tool(Tool::Edit);
        e.select(Some(0));
        assert!(e.style(StylePatch::CornerRadius(ROUNDED_RADIUS)).is_none());
        assert_eq!(e.shapes()[0].corner_radius, ROUNDED_RADIUS);
        e.undo();
        assert_eq!(e.shapes()[0].corner_radius, 0.0);
        e.redo();
        assert_eq!(e.shapes()[0].corner_radius, ROUNDED_RADIUS);
        // 箭头沿用线型，填充与圆角不写入不支持的类型。
        e.set_tool(Tool::Arrow);
        e.style(StylePatch::ArrowHeads(ArrowHeads::Both));
        e.begin(p(100.0, 80.0), 6.0, None, false);
        e.drag(p(180.0, 80.0));
        e.finish(false);
        let arrow = &e.shapes()[1];
        assert_eq!((arrow.line_style, arrow.arrow_heads, arrow.fill, arrow.corner_radius), (LineStyle::Dashed, ArrowHeads::Both, Fill::None, 0.0));
        annotations::validate(e.shapes(), 200, 100).unwrap();
        // 选中矩形不清掉上次的箭头选择；对矩形改箭头不产生历史步骤。
        e.set_tool(Tool::Edit);
        e.select(Some(0));
        assert_eq!(e.style.arrow_heads, ArrowHeads::Both);
        let version = e.version;
        assert!(e.style(StylePatch::ArrowHeads(ArrowHeads::End)).is_none());
        assert_eq!(e.version, version);
        // 选中箭头改为实线只影响它。
        e.select(Some(1));
        e.style(StylePatch::LineStyle(LineStyle::Solid));
        assert_eq!((e.shapes()[0].line_style, e.shapes()[1].line_style), (LineStyle::Dashed, LineStyle::Solid));
        // 贴图工具栏按样式条估算宽度加宽：矩形比画笔宽。
        e.set_tool(Tool::Rectangle);
        let rectangle = options_width(&e);
        e.set_tool(Tool::Pen);
        assert!(rectangle > options_width(&e));
    }

    #[test]
    fn text_background_keeps_glyphs_in_place_and_restyles_by_relayout() {
        let near = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
        let mut e = Editor::new(CROP, vec![]);
        e.set_tool(Tool::Text);
        e.style(StylePatch::TextBackground(true));
        e.begin(p(40.0, 30.0), 6.0, None, false);
        let draft = e.text.as_ref().expect("text draft").shape.clone();
        // 颜色板上的颜色填充背景，字色取对比色；输入框用背景色垫底。
        assert_eq!((draft.background.as_deref(), draft.color.as_str()), (Some("#ef4444"), "#ffffff"));
        assert_eq!(text_input_colors(&draft), (0xffffff, 0xef4444ff));
        let (dx, dy) = raster::decoration_offset(&draft);
        assert!(dx > 0.0 && dy > 0.0);
        let job = e.finish_text("Hi").expect("raster job");
        // 起点向左上退回装饰留白，字形停在输入框里的位置。
        assert!(near(job.shape.start, p(draft.start.x - dx, draft.start.y - dy)));
        let mut done = job.shape.clone();
        done.end = Point { x: done.start.x + 60.0, y: done.start.y + 40.0 };
        done.raster = Some("x".into());
        e.complete(job, Ok(done));
        let before = e.shapes()[0].start;
        e.set_tool(Tool::Edit);
        e.select(Some(0));
        assert!(e.style.text_background && e.style.color == "#ef4444");
        // 关闭背景要重新排版（不能只换色），起点移回字形处。
        let job = e.style(StylePatch::TextBackground(false)).expect("relayout job");
        assert!(!job.preserve);
        assert!(job.shape.background.is_none() && job.shape.color == "#ef4444");
        assert!(near(job.shape.start, p(before.x + dx, before.y + dy)));
        e.complete(job, Err("stop".into()));
        // 不透明度在合成时生效，不重画文字图片。
        assert!(e.style(StylePatch::Opacity(0.7)).is_none());
        assert_eq!(e.shapes()[0].opacity, 0.7);
        // 开启描边：描边色为字色的对比色，同样重新排版。
        let job = e.style(StylePatch::TextOutline(true)).expect("outline job");
        assert_eq!(job.shape.outline.as_deref(), Some("#111827"));
        e.complete(job, Err("stop".into()));
        // 重新编辑：输入框回到字形处，提交时退回同样距离，不会逐次漂移。
        e.edit_text(0);
        let draft = e.text.as_ref().expect("edit draft").shape.clone();
        assert!(near(draft.start, p(before.x + dx, before.y + dy)));
        let job = e.finish_text("Hi there").expect("raster job");
        assert!(near(job.shape.start, before));
    }
}
