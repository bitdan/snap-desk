//! 贴图“选择文字”的纯逻辑：把 OCR 逐行外框整理成一层可选中的文字，提供命中测试、
//! 选区高亮几何、双击取词 / 三击取行、复制文本拼接（按阅读顺序，保留换行）与荧光笔标注。
//!
//! 坐标单位均为贴图图像像素（原点在图片左上角，与标注坐标相同）；界面层把鼠标位置按缩放与 DPI
//! 换算成图像像素后调用这里，绘制高亮时再按同一比例换算回窗口逻辑像素。
//!
//! 各引擎只返回逐行外框，没有可靠的逐字坐标：行内字符位置按估算字宽（中日韩全角约 1em、
//! 其余约 0.56em，见 `overlay::estimate_width`）在框内等比分配。整行、整词选择准确，
//! 行内拖选的边界可能偏差半个字左右。行框一律取外接矩形并按横排处理；明显倾斜的行高亮会比文字略大。
use super::{
    ocr::{OcrResult, cjk},
    overlay::{Rect, estimate_width},
};
use crate::annotations::{Annotation, Kind, Point};

/// 文字层不可用的原因；[`Unavailable::message`] 是词典键（中文原文）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// 没有识别到任何文字。
    NoText,
    /// 识别到文字但没有可用位置：在线 OCR 不返回定位框，或识别坐标与贴图尺寸对不上。
    NoPositions,
}

impl Unavailable {
    pub fn message(self) -> &'static str {
        match self {
            Self::NoText => "未识别到文字。",
            Self::NoPositions => "当前识别引擎不返回文字位置，无法在贴图上选择文字。",
        }
    }
}

/// 一行可选中的文字（对应一个 OCR 行框）。
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    chars: Vec<char>,
    /// 外接矩形（图像像素）。
    pub rect: Rect,
    /// 字符边界的横坐标（图像像素），比字符数多一个，首尾等于外框左右边。
    stops: Vec<f32>,
    /// 与上一行处在同一视觉行（并排的两个识别框）：复制时用空格而不是换行连接。
    pub same_row: bool,
}

impl Line {
    fn new(text: &str, rect: Rect) -> Self {
        // 识别文字里偶见制表符或换行，统一当空格，保证一个字符对应一段宽度。
        let chars: Vec<char> = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let mut buffer = [0u8; 4];
        let weights: Vec<f32> = chars.iter().map(|c| estimate_width(c.encode_utf8(&mut buffer))).collect();
        let total: f32 = weights.iter().sum::<f32>().max(f32::EPSILON);
        let mut stops = Vec::with_capacity(chars.len() + 1);
        let mut sum = 0.0f32;
        stops.push(rect.x);
        for weight in &weights {
            sum += *weight;
            stops.push(rect.x + rect.width * sum / total);
        }
        Self { chars, rect, stops, same_row: false }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// 离横坐标 `x` 最近的字符边界（0..=字符数）。
    fn nearest_offset(&self, x: f32) -> usize {
        let mut best = (0, f32::INFINITY);
        for (index, stop) in self.stops.iter().enumerate() {
            let distance = (x - stop).abs();
            if distance < best.1 {
                best = (index, distance);
            }
        }
        best.0
    }

    /// `x` 所在的字符下标；落在框外时取首字或末字。
    fn char_index(&self, x: f32) -> usize {
        let last = self.chars.len().saturating_sub(1);
        self.stops[1..].iter().position(|&stop| x < stop).unwrap_or(last).min(last)
    }

    /// 点到外框的（竖直，水平）距离，在框内为 0。
    fn distance(&self, x: f32, y: f32) -> (f32, f32) {
        let r = &self.rect;
        let dx = (r.x - x).max(x - (r.x + r.width)).max(0.0);
        let dy = (r.y - y).max(y - (r.y + r.height)).max(0.0);
        (dy, dx)
    }
}

/// 文字中的插入位置：第几行（阅读顺序）的第几个字符边界。先比行再比偏移，可直接比较先后。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Caret {
    pub line: usize,
    pub offset: usize,
}

/// 选区：按下处为锚点，拖动处为焦点；两者先后不定，使用前用 [`Selection::ordered`] 排序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Caret,
    pub focus: Caret,
}

impl Selection {
    pub fn at(caret: Caret) -> Self {
        Self { anchor: caret, focus: caret }
    }

    fn range(line: usize, start: usize, end: usize) -> Self {
        Self { anchor: Caret { line, offset: start }, focus: Caret { line, offset: end } }
    }

    pub fn ordered(self) -> (Caret, Caret) {
        if self.anchor <= self.focus { (self.anchor, self.focus) } else { (self.focus, self.anchor) }
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.focus
    }
}

/// 一张贴图图片上的全部可选文字，行已按阅读顺序排好。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayer {
    lines: Vec<Line>,
}

/// 识别坐标系与贴图宽高比允许的偏差；超出说明识别结果不属于这张图（被裁剪或变形）。
const ASPECT_TOLERANCE: f32 = 0.02;

impl TextLayer {
    /// 由识别结果建立文字层；`width`/`height` 为贴图图像尺寸。识别坐标系（`image_width/height`）
    /// 与贴图尺寸不同时按比例换算；没有外框或外框退化的行跳过，全部跳过时返回 `NoPositions`。
    pub fn new(result: &OcrResult, width: u32, height: u32) -> Result<Self, Unavailable> {
        if result.lines.iter().all(|line| line.text.trim().is_empty()) {
            return Err(Unavailable::NoText);
        }
        if result.image_width == 0 || result.image_height == 0 || width == 0 || height == 0 {
            return Err(Unavailable::NoPositions);
        }
        let sx = width as f32 / result.image_width as f32;
        let sy = height as f32 / result.image_height as f32;
        if (sx / sy - 1.0).abs() > ASPECT_TOLERANCE {
            return Err(Unavailable::NoPositions);
        }
        let (w, h) = (width as f32, height as f32);
        let lines: Vec<(usize, Line)> = result
            .lines
            .iter()
            .filter_map(|line| {
                let text = line.text.trim();
                let points = line.polygon.as_ref()?;
                if text.is_empty() || points.len() < 3 || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                    return None;
                }
                let x0 = points.iter().map(|p| p.x * sx).fold(f32::INFINITY, f32::min).clamp(0.0, w);
                let x1 = points.iter().map(|p| p.x * sx).fold(f32::NEG_INFINITY, f32::max).clamp(0.0, w);
                let y0 = points.iter().map(|p| p.y * sy).fold(f32::INFINITY, f32::min).clamp(0.0, h);
                let y1 = points.iter().map(|p| p.y * sy).fold(f32::NEG_INFINITY, f32::max).clamp(0.0, h);
                if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
                    return None;
                }
                Some((line.order, Line::new(text, Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })))
            })
            .collect();
        if lines.is_empty() {
            return Err(Unavailable::NoPositions);
        }
        Ok(Self { lines: reading_order(lines) })
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// 离点最近的行：先比竖直距离再比水平距离。指针在某一行的高度范围内时总是落到这一行，
    /// 拖到行尾右侧也停在这一行的末尾（与网页拖选一致）。
    fn nearest(&self, p: Point) -> Option<(usize, &Line)> {
        let (x, y) = (p.x as f32, p.y as f32);
        self.lines.iter().enumerate().min_by(|(_, a), (_, b)| {
            let (ady, adx) = a.distance(x, y);
            let (bdy, bdx) = b.distance(x, y);
            ady.total_cmp(&bdy).then(adx.total_cmp(&bdx))
        })
    }

    /// 指针是否落在某个文字框上（外扩 `tolerance` 图像像素）：按下时据此决定开始选择还是拖动贴图。
    pub fn hit(&self, p: Point, tolerance: f64) -> bool {
        let (x, y, tolerance) = (p.x as f32, p.y as f32, tolerance as f32);
        self.lines.iter().any(|line| {
            let (dy, dx) = line.distance(x, y);
            dy <= tolerance && dx <= tolerance
        })
    }

    /// 拖选时指针对应的插入位置；拖出文字范围时停在最近一行的行首或行尾。
    pub fn caret_at(&self, p: Point) -> Option<Caret> {
        let (line, nearest) = self.nearest(p)?;
        Some(Caret { line, offset: nearest.nearest_offset(p.x as f32) })
    }

    /// 双击取词：字母数字连成的单词、连续的中日韩文字、连续空白各算一段，其余标点单独成段。
    /// 没有分词词典，中文会选到前后标点之间的整段文字。
    pub fn word_at(&self, p: Point) -> Option<Selection> {
        let (index, line) = self.nearest(p)?;
        if line.is_empty() {
            return None;
        }
        let class = |c: char| -> u8 {
            if c.is_whitespace() {
                0
            } else if c.is_alphanumeric() && cjk(c) {
                1
            } else if c.is_alphanumeric() || c == '_' {
                2
            } else {
                3
            }
        };
        let at = line.char_index(p.x as f32);
        let kind = class(line.chars[at]);
        if kind == 3 {
            return Some(Selection::range(index, at, at + 1));
        }
        let mut start = at;
        while start > 0 && class(line.chars[start - 1]) == kind {
            start -= 1;
        }
        let mut end = at + 1;
        while end < line.len() && class(line.chars[end]) == kind {
            end += 1;
        }
        Some(Selection::range(index, start, end))
    }

    /// 三击选中整行（一个识别框）。
    pub fn line_at(&self, p: Point) -> Option<Selection> {
        let (index, line) = self.nearest(p)?;
        Some(Selection::range(index, 0, line.len()))
    }

    pub fn select_all(&self) -> Option<Selection> {
        let last = self.lines.len().checked_sub(1)?;
        Some(Selection { anchor: Caret { line: 0, offset: 0 }, focus: Caret { line: last, offset: self.lines[last].len() } })
    }

    /// 选区在各行上的片段：（行下标，起始字符，结束字符），已收进行长度并跳过空片段。
    fn spans(&self, selection: Selection) -> Vec<(usize, usize, usize)> {
        let (start, end) = selection.ordered();
        if self.lines.is_empty() || start.line >= self.lines.len() {
            return Vec::new();
        }
        let last = end.line.min(self.lines.len() - 1);
        (start.line..=last)
            .filter_map(|index| {
                let length = self.lines[index].len();
                let from = if index == start.line { start.offset.min(length) } else { 0 };
                let to = if index == end.line { end.offset.min(length) } else { length };
                (to > from).then_some((index, from, to))
            })
            .collect()
    }

    /// 复制的文字：按阅读顺序拼接，不同视觉行之间换行，同一视觉行并排的识别框之间加空格。
    pub fn text(&self, selection: Selection) -> String {
        let mut out = String::new();
        let mut previous: Option<usize> = None;
        for (index, from, to) in self.spans(selection) {
            if let Some(previous) = previous {
                // 片段按行连续，只有首尾行可能为空被跳过；同排标记只对紧邻的上一行有效。
                let same_row = index == previous + 1 && self.lines[index].same_row;
                out.push(if same_row { ' ' } else { '\n' });
            }
            out.extend(&self.lines[index].chars[from..to]);
            previous = Some(index);
        }
        out
    }

    /// 选区在各行上的高亮矩形（图像像素），用于绘制选中底色与生成荧光笔标注。
    pub fn highlights(&self, selection: Selection) -> Vec<Rect> {
        self.spans(selection)
            .into_iter()
            .map(|(index, from, to)| {
                let line = &self.lines[index];
                let (left, right) = (line.stops[from], line.stops[to]);
                Rect { x: left, y: line.rect.y, width: right - left, height: line.rect.height }
            })
            .collect()
    }
}

/// 视觉行分组：先按上边排序（同高时保持引擎顺序），与当前视觉行首个框在竖直方向重叠超过
/// 较矮者一半的框归入同一行；行内从左到右。多栏排版会按行交错，与多数 PDF 阅读器复制的结果相同。
fn reading_order(mut lines: Vec<(usize, Line)>) -> Vec<Line> {
    lines.sort_by(|(a_order, a), (b_order, b)| a.rect.y.total_cmp(&b.rect.y).then(a_order.cmp(b_order)));
    let mut rows: Vec<Vec<Line>> = Vec::new();
    for (_, line) in lines {
        if let Some(row) = rows.last_mut() {
            let first = row[0].rect;
            let overlap = (first.y + first.height).min(line.rect.y + line.rect.height) - first.y.max(line.rect.y);
            if overlap >= first.height.min(line.rect.height) * 0.5 {
                row.push(line);
                continue;
            }
        }
        rows.push(vec![line]);
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
            row.into_iter().enumerate().map(|(index, mut line)| {
                line.same_row = index > 0;
                line
            })
        })
        .collect()
}

/// 荧光笔最大笔宽，与标注校验的线宽上限一致。
const MAX_MARK_WIDTH: f64 = 64.0;

/// 把高亮矩形转成荧光笔标注（一段文字一笔）：沿文字中线画，笔宽取行高（最多 64 像素）。
/// 荧光笔是圆头笔画，两端各内缩半个笔宽，涂色范围才与选中的文字对齐；坐标收进图片范围内。
pub fn highlighter_marks(rects: &[Rect], color: &str, opacity: f64, width: u32, height: u32) -> Vec<Annotation> {
    let (w, h) = (f64::from(width), f64::from(height));
    rects
        .iter()
        .filter(|r| r.width > 0.0 && r.height > 0.0)
        .map(|r| {
            let stroke = f64::from(r.height).clamp(1.0, MAX_MARK_WIDTH);
            let y = (f64::from(r.y) + f64::from(r.height) / 2.0).clamp(0.0, h);
            let (left, right) = (f64::from(r.x), f64::from(r.x + r.width));
            let inset = (stroke / 2.0).min((right - left) / 2.0);
            let x0 = (left + inset).clamp(0.0, w);
            // 两点重合时栅格化只剩圆点，至少留半个像素的长度。
            let x1 = (right - inset).max(x0 + 0.5).clamp(0.0, w);
            let (a, b) = (Point { x: x0, y }, Point { x: x1, y });
            Annotation {
                kind: Kind::Highlighter,
                start: a,
                end: b,
                color: color.to_owned(),
                width: stroke,
                opacity,
                magnification: None,
                points: vec![a, b],
                text: None,
                font_size: None,
                font_family: None,
                raster: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Caret, Selection, TextLayer, Unavailable, highlighter_marks};
    use crate::annotations::{self, Kind, Point};
    use crate::recognition::ocr::{OcrLine, OcrResult, PolygonPoint};

    fn line(order: usize, text: &str, x: f32, y: f32, w: f32, h: f32) -> OcrLine {
        OcrLine {
            id: order.to_string(),
            text: text.into(),
            confidence: None,
            polygon: Some(vec![
                PolygonPoint { x, y },
                PolygonPoint { x: x + w, y },
                PolygonPoint { x: x + w, y: y + h },
                PolygonPoint { x, y: y + h },
            ]),
            language: None,
            order,
            paragraph_id: None,
        }
    }

    fn result(lines: Vec<OcrLine>, width: u32, height: u32) -> OcrResult {
        OcrResult {
            request_id: String::new(),
            provider: "test".into(),
            image_width: width,
            image_height: height,
            full_text: String::new(),
            lines,
            elapsed_ms: 0,
            warnings: Vec::new(),
            auto_copy: false,
            merge_lines: false,
        }
    }

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    /// 两行文字，第二行右侧并排一个识别框；引擎顺序故意打乱。
    fn sample() -> TextLayer {
        let lines = vec![
            line(0, "tail", 200.0, 52.0, 40.0, 20.0),
            line(1, "Hello world", 10.0, 10.0, 110.0, 20.0),
            line(2, "second", 10.0, 50.0, 60.0, 20.0),
        ];
        TextLayer::new(&result(lines, 400, 100), 400, 100).unwrap()
    }

    #[test]
    fn lines_follow_reading_order_and_mark_side_by_side_boxes() {
        let layer = sample();
        let texts: Vec<String> = layer.lines().iter().map(|l| l.text()).collect();
        assert_eq!(texts, ["Hello world", "second", "tail"]);
        assert!(!layer.lines()[1].same_row);
        assert!(layer.lines()[2].same_row);
    }

    #[test]
    fn select_all_keeps_line_breaks_and_joins_same_row_with_space() {
        let layer = sample();
        let all = layer.select_all().unwrap();
        assert_eq!(layer.text(all), "Hello world\nsecond tail");
    }

    #[test]
    fn drag_across_lines_copies_partial_first_and_last_line() {
        let layer = sample();
        // “Hello world” 在 10..120 内按字宽分配，空格与“w”的边界约在 67.8；
        // “second” 每字 10 像素，x=40 正好是第 3 个字符边界。从下往上反向拖选，结果相同。
        let anchor = layer.caret_at(at(70.0, 20.0)).unwrap();
        let focus = layer.caret_at(at(40.0, 60.0)).unwrap();
        assert_eq!(anchor, Caret { line: 0, offset: 6 });
        assert_eq!(focus, Caret { line: 1, offset: 3 });
        assert_eq!(layer.text(Selection { anchor: focus, focus: anchor }), "world\nsec");
        assert_eq!(layer.text(Selection { anchor, focus }), "world\nsec");
    }

    #[test]
    fn caret_clamps_to_line_ends_outside_text() {
        let layer = sample();
        // 第一行右侧空白处：停在第一行末尾，而不是跳到下一行。
        assert_eq!(layer.caret_at(at(180.0, 15.0)), Some(Caret { line: 0, offset: 11 }));
        // 图片顶部空白：落到第一行，按横坐标取行首。
        assert_eq!(layer.caret_at(at(0.0, 0.0)), Some(Caret { line: 0, offset: 0 }));
        // 两行之间、偏下：竖直距离更近的第二行。
        assert_eq!(layer.caret_at(at(5.0, 45.0)).map(|c| c.line), Some(1));
    }

    #[test]
    fn hit_uses_tolerance_around_boxes() {
        let layer = sample();
        assert!(layer.hit(at(15.0, 15.0), 0.0));
        assert!(layer.hit(at(5.0, 15.0), 6.0));
        assert!(!layer.hit(at(300.0, 90.0), 6.0));
    }

    #[test]
    fn highlights_follow_character_stops_within_line_box() {
        let layer = sample();
        let all = layer.select_all().unwrap();
        let rects = layer.highlights(all);
        assert_eq!(rects.len(), 3);
        assert_eq!((rects[0].x, rects[0].width, rects[0].y, rects[0].height), (10.0, 110.0, 10.0, 20.0));
        // 部分选择：前 5 个字符（“Hello”）在 10..120 内，按字宽占比应短于整行一半。
        let hello = Selection { anchor: Caret { line: 0, offset: 0 }, focus: Caret { line: 0, offset: 5 } };
        let part = layer.highlights(hello);
        assert_eq!(part.len(), 1);
        assert!(part[0].width > 40.0 && part[0].width < 55.0, "{:?}", part[0]);
        assert_eq!(layer.text(hello), "Hello");
        // 选区止于下一行行首：下一行没有片段，也不多出换行。
        let to_next = Selection { anchor: Caret { line: 0, offset: 6 }, focus: Caret { line: 1, offset: 0 } };
        assert_eq!(layer.highlights(to_next).len(), 1);
        assert_eq!(layer.text(to_next), "world");
    }

    #[test]
    fn word_and_line_selection() {
        let layer = sample();
        let word = layer.word_at(at(100.0, 20.0)).unwrap();
        assert_eq!(layer.text(word), "world");
        let line = layer.line_at(at(30.0, 60.0)).unwrap();
        assert_eq!(layer.text(line), "second");
        let cjk = TextLayer::new(&result(vec![line_cjk()], 200, 40), 200, 40).unwrap();
        let word = cjk.word_at(at(15.0, 10.0)).unwrap();
        assert_eq!(cjk.text(word), "你好");
        let comma = cjk.word_at(at(50.0, 10.0)).unwrap();
        assert_eq!(cjk.text(comma), "，");
    }

    fn line_cjk() -> OcrLine {
        // 每个全角字符宽 20 像素：“你好，世界”占 0..100。
        line(0, "你好，世界", 0.0, 0.0, 100.0, 20.0)
    }

    #[test]
    fn rejects_results_without_text_or_positions() {
        assert_eq!(TextLayer::new(&result(vec![], 10, 10), 10, 10), Err(Unavailable::NoText));
        let mut online = line(0, "text", 0.0, 0.0, 5.0, 5.0);
        online.polygon = None;
        assert_eq!(TextLayer::new(&result(vec![online], 10, 10), 10, 10), Err(Unavailable::NoPositions));
        // 识别坐标宽高比与贴图不一致：识别结果不属于这张图。
        let skewed = result(vec![line(0, "text", 0.0, 0.0, 5.0, 5.0)], 10, 10);
        assert_eq!(TextLayer::new(&skewed, 20, 10), Err(Unavailable::NoPositions));
    }

    #[test]
    fn scales_ocr_coordinates_to_pin_image() {
        // 引擎在半尺寸图上识别：坐标按 2 倍换算回贴图像素。
        let layer = TextLayer::new(&result(vec![line(0, "ab", 5.0, 5.0, 20.0, 10.0)], 100, 50), 200, 100).unwrap();
        let rect = layer.lines()[0].rect;
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (10.0, 10.0, 40.0, 20.0));
    }

    #[test]
    fn highlighter_marks_are_valid_annotations_inside_the_image() {
        let layer = sample();
        let rects = layer.highlights(layer.select_all().unwrap());
        let marks = highlighter_marks(&rects, "#facc15", 0.35, 400, 100);
        assert_eq!(marks.len(), 3);
        assert!(marks.iter().all(|m| m.kind == Kind::Highlighter && m.points.len() == 2));
        // 笔宽等于行高；圆头内缩半个笔宽后，涂色范围与文字框左右对齐。
        assert_eq!(marks[0].width, 20.0);
        assert_eq!((marks[0].points[0].x, marks[0].points[1].x, marks[0].points[0].y), (20.0, 110.0, 20.0));
        annotations::validate(&marks, 400, 100).unwrap();
        // 很高的文字：笔宽封顶 64；很窄的片段仍是两点不重合的一笔。
        let tall = [super::Rect { x: 0.0, y: 0.0, width: 4.0, height: 90.0 }];
        let marks = highlighter_marks(&tall, "#facc15", 0.35, 400, 100);
        assert_eq!(marks[0].width, 64.0);
        assert!(marks[0].points[1].x > marks[0].points[0].x);
        annotations::validate(&marks, 400, 100).unwrap();
    }
}
