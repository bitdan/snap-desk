//! 标注编辑几何与撤销历史：外接矩形、移动、缩放、命中测试、放大镜镜框、序号与历史。
//!
//! 供截图与贴图共用。
//! 坐标全部是原图物理像素；`crop` 是允许编辑的范围（截图选区或贴图整图）。
use super::{Annotation, Kind, Point};
use std::collections::HashSet;

/// 原图像素中的浮点矩形；宽高非负。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
    /// 把点限制在矩形内。
    pub fn clamp(&self, p: Point) -> Point {
        Point {
            x: clamp(p.x, self.x, self.right()),
            y: clamp(p.y, self.y, self.bottom()),
        }
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x <= self.right() && p.y <= self.bottom()
    }
}

/// 与 JS `Math.min(max, Math.max(min, value))` 一致：min > max 时取 max，不会 panic。
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    max.min(min.max(value))
}

/// 缩放手柄：外接矩形八个方位，直线与箭头改用起点/终点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    Start,
    End,
}

impl Handle {
    pub const BOX: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    /// 手柄在外接矩形上的位置；起点/终点不在此列。
    pub fn point(self, b: Bounds) -> Point {
        let (fx, fy) = match self {
            Self::NW => (0.0, 0.0),
            Self::N => (0.5, 0.0),
            Self::NE => (1.0, 0.0),
            Self::E => (1.0, 0.5),
            Self::SE => (1.0, 1.0),
            Self::S => (0.5, 1.0),
            Self::SW => (0.0, 1.0),
            Self::W => (0.0, 0.5),
            Self::Start | Self::End => (0.0, 0.0),
        };
        Point {
            x: b.x + b.width * fx,
            y: b.y + b.height * fy,
        }
    }

    fn west(self) -> bool {
        matches!(self, Self::NW | Self::W | Self::SW)
    }
    fn east(self) -> bool {
        matches!(self, Self::NE | Self::E | Self::SE)
    }
    fn north(self) -> bool {
        matches!(self, Self::NW | Self::N | Self::NE)
    }
    fn south(self) -> bool {
        matches!(self, Self::SW | Self::S | Self::SE)
    }
}

/// 直线与箭头用两个端点编辑，其余图形用外接矩形八向手柄。
pub fn uses_endpoints(kind: Kind) -> bool {
    matches!(kind, Kind::Line | Kind::Arrow)
}

/// 由两点得到归一化矩形。
pub fn normalize(a: Point, b: Point) -> Bounds {
    Bounds {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        width: (a.x - b.x).abs(),
        height: (a.y - b.y).abs(),
    }
}

/// 拖动手柄后的矩形：手柄所在的边移到 `p`，越过对边时自动翻转。
pub fn resize_bounds(b: Bounds, handle: Handle, p: Point) -> Bounds {
    let mut a = Point { x: b.x, y: b.y };
    let mut c = Point {
        x: b.right(),
        y: b.bottom(),
    };
    if handle.west() {
        a.x = p.x;
    }
    if handle.east() {
        c.x = p.x;
    }
    if handle.north() {
        a.y = p.y;
    }
    if handle.south() {
        c.y = p.y;
    }
    normalize(a, c)
}

/// 路径类标注取全部路径点，其他取起止点。
pub fn bounds(shape: &Annotation) -> Bounds {
    let pair = [shape.start, shape.end];
    let points: &[Point] = if shape.points.is_empty() {
        &pair
    } else {
        &shape.points
    };
    let (mut left, mut top) = (f64::INFINITY, f64::INFINITY);
    let (mut right, mut bottom) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        left = left.min(p.x);
        top = top.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    Bounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

/// 用新的外接矩形替换起止点，其余样式不变。
pub fn with_bounds(shape: &Annotation, b: Bounds) -> Annotation {
    Annotation {
        start: Point { x: b.x, y: b.y },
        end: Point {
            x: b.right(),
            y: b.bottom(),
        },
        ..shape.clone()
    }
}

pub fn map_shape(shape: &Annotation, map: impl Fn(Point) -> Point) -> Annotation {
    Annotation {
        start: map(shape.start),
        end: map(shape.end),
        points: shape.points.iter().copied().map(&map).collect(),
        ..shape.clone()
    }
}

/// 整体平移；位移先限制在 `crop` 内，图形不会被拖出可编辑范围。
pub fn move_shape(shape: &Annotation, dx: f64, dy: f64, crop: Bounds) -> Annotation {
    let b = bounds(shape);
    let dx = clamp(dx, crop.x - b.x, crop.right() - b.right());
    let dy = clamp(dy, crop.y - b.y, crop.bottom() - b.bottom());
    map_shape(shape, |p| Point {
        x: p.x + dx,
        y: p.y + dy,
    })
}

/// 仿射缩放全部路径点；外接矩形某一维为 0（水平/垂直线）时落在新矩形中线，避免除以零。
pub fn resize_shape(shape: &Annotation, handle: Handle, p: Point, crop: Bounds) -> Annotation {
    let p = crop.clamp(p);
    match handle {
        Handle::Start => Annotation {
            start: p,
            ..shape.clone()
        },
        Handle::End => Annotation {
            end: p,
            ..shape.clone()
        },
        _ => {
            let b = bounds(shape);
            let next = resize_bounds(b, handle, p);
            map_shape(shape, |v| Point {
                x: next.x
                    + if b.width != 0.0 {
                        (v.x - b.x) / b.width
                    } else {
                        0.5
                    } * next.width,
                y: next.y
                    + if b.height != 0.0 {
                        (v.y - b.y) / b.height
                    } else {
                        0.5
                    } * next.height,
            })
        }
    }
}

/// 箭头两翼端点；尺寸公式与 `render` 的箭头一致。
pub fn arrow_wings(shape: &Annotation) -> [Point; 2] {
    let (a, b) = (shape.start, shape.end);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return [b, b];
    }
    let size = (shape.width * 4.0).max(10.0).min(length * 0.45);
    let (ux, uy) = (dx / length, dy / length);
    [
        Point {
            x: b.x - ux * size + uy * size * 0.5,
            y: b.y - uy * size - ux * size * 0.5,
        },
        Point {
            x: b.x - ux * size - uy * size * 0.5,
            y: b.y - uy * size + ux * size * 0.5,
        },
    ]
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let squared = dx * dx + dy * dy;
    let t = clamp(
        ((p.x - a.x) * dx + (p.y - a.y) * dy) / if squared == 0.0 { 1.0 } else { squared },
        0.0,
        1.0,
    );
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}

/// 从最后绘制的图形开始命中。`interiors` 为真时封闭图形内部也算命中（选择/移动），
/// 删除整项时只按实际笔迹命中；`skip_erase` 跳过局部擦除笔迹。
pub fn hit_test(
    shapes: &[Annotation],
    p: Point,
    tolerance: f64,
    skip_erase: bool,
    interiors: bool,
) -> Option<usize> {
    let hit = |s: &Annotation| {
        let b = bounds(s);
        let t = tolerance + s.width / 2.0;
        if matches!(s.kind, Kind::Erase) && s.points.len() == 1 {
            return (p.x - s.start.x).hypot(p.y - s.start.y) <= t;
        }
        if !s.points.is_empty() {
            return s
                .points
                .windows(2)
                .any(|w| segment_distance(p, w[0], w[1]) <= t);
        }
        if matches!(s.kind, Kind::Line | Kind::Arrow) {
            return segment_distance(p, s.start, s.end) <= t
                || (matches!(s.kind, Kind::Arrow)
                    && arrow_wings(s)
                        .iter()
                        .any(|w| segment_distance(p, *w, s.end) <= t));
        }
        if matches!(s.kind, Kind::Ellipse) {
            let (rx, ry) = (b.width / 2.0, b.height / 2.0);
            if rx <= 0.0 || ry <= 0.0 {
                return false;
            }
            let radius = ((p.x - b.x - rx) / rx).hypot((p.y - b.y - ry) / ry);
            return if interiors {
                radius <= 1.0 + t / rx.min(ry)
            } else {
                (radius - 1.0).abs() <= t / rx.min(ry)
            };
        }
        let inside =
            p.x >= b.x - t && p.x <= b.right() + t && p.y >= b.y - t && p.y <= b.bottom() + t;
        inside
            && (!matches!(s.kind, Kind::Rectangle)
                || interiors
                || (p.x - b.x)
                    .abs()
                    .min((p.x - b.right()).abs())
                    .min((p.y - b.y).abs())
                    .min((p.y - b.bottom()).abs())
                    <= t)
    };
    (0..shapes.len())
        .rev()
        .find(|&i| !(skip_erase && matches!(shapes[i].kind, Kind::Erase)) && hit(&shapes[i]))
}

/// 文字、序号与局部滤镜携带受限 PNG（见 `raster` 模块）。
pub fn is_raster(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Text | Kind::Number | Kind::Mosaic | Kind::Blur | Kind::Magnify
    )
}

fn valid_zoom(zoom: f64) -> Result<f64, String> {
    if zoom.is_finite() && (2.0..=4.0).contains(&zoom) {
        Ok(zoom)
    } else {
        Err("放大倍数须为 2～4。".into())
    }
}

/// 放大镜采样区域：镜框中心按倍数缩小的部分，始终在镜框内部。
pub fn magnifier_source(shape: &Annotation) -> Result<Bounds, String> {
    let b = bounds(shape);
    let zoom = valid_zoom(shape.magnification.unwrap_or(2.0))?;
    let (width, height) = (b.width / zoom, b.height / zoom);
    Ok(Bounds {
        x: b.x + (b.width - width) / 2.0,
        y: b.y + (b.height - height) / 2.0,
        width,
        height,
    })
}

/// 由框出的内容生成镜框：以内容为中心按倍数展开；放不下时先等比缩小，再平移回 `crop` 内。
pub fn magnifier_lens(source: Bounds, zoom: f64, crop: Bounds) -> Result<Bounds, String> {
    let zoom = valid_zoom(zoom)?;
    let fit = 1.0_f64
        .min(crop.width / (source.width * zoom))
        .min(crop.height / (source.height * zoom));
    let (width, height) = (source.width * zoom * fit, source.height * zoom * fit);
    Ok(Bounds {
        x: clamp(
            source.x + (source.width - width) / 2.0,
            crop.x,
            crop.right() - width,
        ),
        y: clamp(
            source.y + (source.height - height) / 2.0,
            crop.y,
            crop.bottom() - height,
        ),
        width,
        height,
    })
}

/// 序号由当前图层推导：撤销/删除末项后可复用，删除中间项不重排已有序号。
pub fn next_number(shapes: &[Annotation]) -> u32 {
    shapes
        .iter()
        .filter(|s| matches!(s.kind, Kind::Number))
        .filter_map(|s| s.text.as_deref()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// 比较可见几何与样式；跳过 raster，避免逐字符比较 base64 PNG。
pub fn same_shape(a: &Annotation, b: &Annotation) -> bool {
    let point = |p: Point, q: Point| p.x == q.x && p.y == q.y;
    std::mem::discriminant(&a.kind) == std::mem::discriminant(&b.kind)
        && a.color == b.color
        && a.width == b.width
        && a.opacity == b.opacity
        && a.magnification.unwrap_or(2.0) == b.magnification.unwrap_or(2.0)
        && point(a.start, b.start)
        && point(a.end, b.end)
        && a.text == b.text
        && a.font_size == b.font_size
        && a.font_family == b.font_family
        && a.points.len() == b.points.len()
        && a.points.iter().zip(&b.points).all(|(p, q)| point(*p, *q))
}

/// 撤销历史最多 50 步；唯一栅格字符串总量（按每字符两字节保守估算，与前端一致）不超过 32 MiB。
pub const HISTORY_STEPS: usize = 50;
pub const HISTORY_RASTER_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct History {
    past: Vec<Vec<Annotation>>,
    present: Vec<Annotation>,
    future: Vec<Vec<Annotation>>,
}

impl History {
    pub fn new(present: Vec<Annotation>) -> Self {
        Self {
            past: Vec::new(),
            present,
            future: Vec::new(),
        }
    }
    pub fn present(&self) -> &[Annotation] {
        &self.present
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// 提交新结果并清空重做栈；始终保留当前结果，超出预算时优先淘汰最旧的步骤。
    pub fn commit(&mut self, shapes: Vec<Annotation>) {
        let mut seen = HashSet::new();
        let mut bytes = 0usize;
        let mut include = |snapshot: &[Annotation], seen: &mut HashSet<String>| {
            for raster in snapshot.iter().filter_map(|s| s.raster.as_ref()) {
                if seen.insert(raster.clone()) {
                    bytes += raster.len() * 2;
                }
            }
            bytes
        };
        include(&shapes, &mut seen);
        let mut candidates = std::mem::take(&mut self.past);
        candidates.push(std::mem::replace(&mut self.present, shapes));
        let skip = candidates.len().saturating_sub(HISTORY_STEPS);
        let candidates: Vec<_> = candidates.into_iter().skip(skip).collect();
        let mut start = candidates.len();
        for index in (0..candidates.len()).rev() {
            if include(&candidates[index], &mut seen) > HISTORY_RASTER_BYTES {
                break;
            }
            start = index;
        }
        self.past = candidates.into_iter().skip(start).collect();
        self.future.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(last) = self.past.pop() else {
            return false;
        };
        self.future.push(std::mem::replace(&mut self.present, last));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(last) = self.future.pop() else {
            return false;
        };
        self.past.push(std::mem::replace(&mut self.present, last));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kind: Kind, a: (f64, f64), b: (f64, f64)) -> Annotation {
        Annotation {
            kind,
            start: Point { x: a.0, y: a.1 },
            end: Point { x: b.0, y: b.1 },
            color: "#ff0000".into(),
            width: 2.0,
            opacity: 1.0,
            magnification: None,
            points: vec![],
            text: None,
            font_size: None,
            font_family: None,
            raster: None,
        }
    }
    const CROP: Bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };

    #[test]
    fn move_is_clamped_to_crop_and_resize_flips_past_opposite_edge() {
        let s = shape(Kind::Rectangle, (10.0, 10.0), (30.0, 20.0));
        let moved = move_shape(&s, 200.0, -50.0, CROP);
        assert_eq!(
            bounds(&moved),
            Bounds {
                x: 80.0,
                y: 0.0,
                width: 20.0,
                height: 10.0
            }
        );
        let resized = resize_shape(&s, Handle::E, Point { x: 0.0, y: 15.0 }, CROP);
        assert_eq!(
            bounds(&resized),
            Bounds {
                x: 0.0,
                y: 10.0,
                width: 10.0,
                height: 10.0
            }
        );
        let line = shape(Kind::Line, (10.0, 10.0), (50.0, 10.0));
        let end = resize_shape(&line, Handle::End, Point { x: 150.0, y: 40.0 }, CROP);
        assert_eq!((end.end.x, end.end.y), (100.0, 40.0));
        // 水平线按外接矩形缩放时 y 落在新矩形中线，不出现 NaN。
        let scaled = resize_shape(&line, Handle::S, Point { x: 0.0, y: 30.0 }, CROP);
        assert!(scaled.start.y.is_finite() && scaled.start.y == 20.0);
    }

    #[test]
    fn hit_test_prefers_topmost_and_respects_interiors() {
        let a = shape(Kind::Rectangle, (10.0, 10.0), (50.0, 50.0));
        let b = shape(Kind::Ellipse, (20.0, 20.0), (40.0, 40.0));
        let shapes = vec![a, b];
        assert_eq!(
            hit_test(&shapes, Point { x: 30.0, y: 30.0 }, 2.0, true, true),
            Some(1)
        );
        // 不含内部时，椭圆中心与矩形中心都不算命中。
        assert_eq!(
            hit_test(&shapes, Point { x: 30.0, y: 30.0 }, 2.0, true, false),
            None
        );
        assert_eq!(
            hit_test(&shapes, Point { x: 10.0, y: 30.0 }, 2.0, true, false),
            Some(0)
        );
        let mut erase = shape(Kind::Erase, (70.0, 70.0), (70.0, 70.0));
        erase.points = vec![erase.start];
        let shapes = vec![erase];
        assert_eq!(
            hit_test(&shapes, Point { x: 70.0, y: 70.0 }, 2.0, true, true),
            None
        );
        assert_eq!(
            hit_test(&shapes, Point { x: 70.0, y: 70.0 }, 2.0, false, false),
            Some(0)
        );
    }

    #[test]
    fn magnifier_lens_expands_around_source_and_fits_crop() {
        let source = Bounds {
            x: 40.0,
            y: 40.0,
            width: 10.0,
            height: 10.0,
        };
        let lens = magnifier_lens(source, 2.0, CROP).unwrap();
        assert_eq!(
            lens,
            Bounds {
                x: 35.0,
                y: 35.0,
                width: 20.0,
                height: 20.0
            }
        );
        let big = Bounds {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 40.0,
        };
        let lens = magnifier_lens(big, 4.0, CROP).unwrap();
        assert!(lens.width <= 100.0 && lens.x >= 0.0 && lens.right() <= 100.0);
        assert!(magnifier_lens(source, 5.0, CROP).is_err());
        let mut s = with_bounds(&shape(Kind::Magnify, (0.0, 0.0), (0.0, 0.0)), lens);
        s.magnification = Some(4.0);
        let back = magnifier_source(&s).unwrap();
        assert!((back.width - lens.width / 4.0).abs() < 1e-9);
    }

    #[test]
    fn numbers_continue_from_maximum_and_history_undoes_and_redoes() {
        let mut one = shape(Kind::Number, (0.0, 0.0), (10.0, 10.0));
        one.text = Some("1".into());
        let mut five = one.clone();
        five.text = Some("5".into());
        assert_eq!(next_number(&[one.clone(), five]), 6);
        assert_eq!(next_number(&[]), 1);
        let mut history = History::default();
        history.commit(vec![one.clone()]);
        history.commit(vec![]);
        assert!(history.undo());
        assert_eq!(history.present().len(), 1);
        assert!(history.redo());
        assert!(history.present().is_empty());
        assert!(!history.redo());
        for _ in 0..80 {
            history.commit(vec![one.clone()]);
        }
        let mut steps = 0;
        while history.undo() {
            steps += 1;
        }
        assert_eq!(steps, HISTORY_STEPS);
    }

    #[test]
    fn history_drops_oldest_steps_when_raster_budget_is_exceeded() {
        let mut history = History::default();
        for index in 0..6 {
            let mut s = shape(Kind::Mosaic, (0.0, 0.0), (10.0, 10.0));
            // 每步一个 8 MiB（按两字节/字符计 16 MiB）的不同栅格。
            s.raster = Some(format!("{index}{}", "a".repeat(4 * 1024 * 1024)));
            history.commit(vec![s]);
        }
        let mut steps = 0;
        while history.undo() {
            steps += 1;
        }
        assert!(steps <= 2, "steps={steps}");
    }

    #[test]
    fn same_shape_ignores_raster_but_not_geometry() {
        let a = shape(Kind::Blur, (0.0, 0.0), (10.0, 10.0));
        let mut b = a.clone();
        b.raster = Some("x".into());
        assert!(same_shape(&a, &b));
        b.end.x = 11.0;
        assert!(!same_shape(&a, &b));
    }
}
