//! 截图选区几何：新建、移动、八向缩放、手柄命中与窗口吸附层级。
//!
//! 全部使用虚拟桌面截图内的物理像素（与 `snap_core::capture::Rect` 同一坐标系），
//! 中间计算用 i64，避免负坐标与 u32 溢出。界面的逻辑像素换算由浮层负责，这里不涉及 DPI。
use snap_core::capture::Rect;

/// 选区的八个缩放手柄；名称按方位（N 上、S 下、W 左、E 右）。
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
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    /// 手柄在选区上的相对位置：0 为左/上边，1 为中点，2 为右/下边。
    pub fn anchor(self) -> (u8, u8) {
        match self {
            Self::NW => (0, 0),
            Self::N => (1, 0),
            Self::NE => (2, 0),
            Self::E => (2, 1),
            Self::SE => (2, 2),
            Self::S => (1, 2),
            Self::SW => (0, 2),
            Self::W => (0, 1),
        }
    }
}

pub type Point = (i64, i64);

fn edges(rect: Rect) -> (i64, i64, i64, i64) {
    let (x, y) = (i64::from(rect.x), i64::from(rect.y));
    (x, y, x + i64::from(rect.width), y + i64::from(rect.height))
}

/// 由左、上、右、下边构造选区并限制在 `area` 内；交换了方向的边会先归一，空选区返回 None。
pub fn from_edges(left: i64, top: i64, right: i64, bottom: i64, area: Rect) -> Option<Rect> {
    let (al, at, ar, ab) = edges(area);
    let (l, r) = (left.min(right).clamp(al, ar), left.max(right).clamp(al, ar));
    let (t, b) = (top.min(bottom).clamp(at, ab), top.max(bottom).clamp(at, ab));
    (r > l && b > t).then(|| Rect {
        x: l as u32,
        y: t as u32,
        width: (r - l) as u32,
        height: (b - t) as u32,
    })
}

/// 拖动新建：两个角点之间的矩形，限制在按下时所在显示器的区域内。
pub fn from_points(a: Point, b: Point, area: Rect) -> Option<Rect> {
    from_edges(a.0, a.1, b.0, b.1, area)
}

/// 整体平移，尺寸不变；碰到 `area` 边缘时停在边上。
pub fn moved(start: Rect, dx: i64, dy: i64, area: Rect) -> Rect {
    let (al, at, ar, ab) = edges(area);
    let (l, t, r, b) = edges(start);
    let x = (l + dx).clamp(al, (ar - (r - l)).max(al));
    let y = (t + dy).clamp(at, (ab - (b - t)).max(at));
    Rect { x: x as u32, y: y as u32, ..start }
}

/// 拖动手柄缩放：只移动手柄所在的边；越过对边时自动翻转。
pub fn resized(start: Rect, handle: Handle, dx: i64, dy: i64, area: Rect) -> Option<Rect> {
    let (mut l, mut t, mut r, mut b) = edges(start);
    let (hx, hy) = handle.anchor();
    match hx {
        0 => l += dx,
        2 => r += dx,
        _ => {}
    }
    match hy {
        0 => t += dy,
        2 => b += dy,
        _ => {}
    }
    from_edges(l, t, r, b, area)
}

/// 宽高比（宽:高）；两项都不为 0。
pub type Ratio = (u32, u32);

fn ratio_parts(ratio: Ratio) -> (i64, i64) {
    (i64::from(ratio.0.max(1)), i64::from(ratio.1.max(1)))
}

/// 按比例拖出选区：`anchor` 是固定角，取拖动范围里按比例折算后更长的一边推出另一边，方向跟随拖动；
/// 朝拖动方向放不下时整体等比缩小，比例不变。整数像素向下取整，比例最多差 1 像素。
pub fn from_points_ratio(anchor: Point, to: Point, ratio: Ratio, area: Rect) -> Option<Rect> {
    let (rw, rh) = ratio_parts(ratio);
    let (al, at, ar, ab) = edges(area);
    let anchor = (anchor.0.clamp(al, ar), anchor.1.clamp(at, ab));
    let (sx, sy) = (if to.0 < anchor.0 { -1 } else { 1 }, if to.1 < anchor.1 { -1 } else { 1 });
    let (dw, dh) = ((to.0 - anchor.0).abs(), (to.1 - anchor.1).abs());
    let (mut w, mut h) = if dw * rh >= dh * rw { (dw, dw * rh / rw) } else { (dh * rw / rh, dh) };
    let room_w = if sx > 0 { ar - anchor.0 } else { anchor.0 - al };
    let room_h = if sy > 0 { ab - anchor.1 } else { anchor.1 - at };
    if w > room_w {
        w = room_w;
        h = w * rh / rw;
    }
    if h > room_h {
        h = room_h;
        w = h * rw / rh;
    }
    from_edges(anchor.0, anchor.1, anchor.0 + sx * w, anchor.1 + sy * h, area)
}

/// 按比例拖动手柄缩放：角手柄以对角为固定点；边手柄以对边为固定边，另一方向从上边或左边延伸。
pub fn resized_ratio(start: Rect, handle: Handle, dx: i64, dy: i64, ratio: Ratio, area: Rect) -> Option<Rect> {
    let (l, t, r, b) = edges(start);
    // 边手柄只拖动一个方向：另一方向只给 1 像素，让拖动方向占优，再由比例推出另一边。
    let (anchor, to) = match handle {
        Handle::NW => ((r, b), (l + dx, t + dy)),
        Handle::NE => ((l, b), (r + dx, t + dy)),
        Handle::SE => ((l, t), (r + dx, b + dy)),
        Handle::SW => ((r, t), (l + dx, b + dy)),
        Handle::E => ((l, t), (r + dx, t + 1)),
        Handle::W => ((r, t), (l + dx, t + 1)),
        Handle::S => ((l, t), (l + 1, b + dy)),
        Handle::N => ((l, b), (l + 1, t + dy)),
    };
    from_points_ratio(anchor, to, ratio, area)
}

/// 按输入的位置与尺寸放置选区：尺寸限制在 `area` 内（`keep_ratio` 时等比缩小），位置再向内推，
/// 保证整个选区都在截图范围里。尺寸不足 1 像素返回 None。
pub fn placed(x: i64, y: i64, width: i64, height: i64, keep_ratio: bool, area: Rect) -> Option<Rect> {
    if width < 1 || height < 1 {
        return None;
    }
    let (aw, ah) = (i64::from(area.width), i64::from(area.height));
    let (mut w, mut h) = (width, height);
    if keep_ratio && (w > aw || h > ah) {
        // 整数比较哪一边先碰到范围，避免浮点误差少算 1 像素。
        if w * ah >= h * aw {
            (w, h) = (aw, (h * aw / w).max(1));
        } else {
            (w, h) = ((w * ah / h).max(1), ah);
        }
    }
    let (w, h) = (w.min(aw), h.min(ah));
    let (al, at, ar, ab) = edges(area);
    let (x, y) = (x.clamp(al, ar - w), y.clamp(at, ab - h));
    from_edges(x, y, x + w, y + h, area)
}

/// 把选区改成给定比例：保持左上角与宽度，由比例推出高度；放不下时等比缩小。
pub fn with_ratio(rect: Rect, ratio: Ratio, area: Rect) -> Rect {
    let (rw, rh) = ratio_parts(ratio);
    let width = i64::from(rect.width);
    let height = (width * rh / rw).max(1);
    placed(i64::from(rect.x), i64::from(rect.y), width, height, true, area).unwrap_or(rect)
}

/// 方向键对应的选区边。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// 方向键调整选区的方式：平移，或改变方向键所指那条边。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    Move,
    /// 把这条边向外推（选区变大）。
    Grow,
    /// 把这条边向内收（选区变小）。
    Shrink,
}

/// 键盘微调选区 `step` 像素。`symmetric` 时同一轴上的对边同步变化，以中心对称缩放；
/// 扩展碰到 `area` 边缘时停在边上，收缩到不足 1 像素时保持原样（不像拖动手柄那样翻转）。
pub fn nudged(rect: Rect, side: Side, nudge: Nudge, step: i64, symmetric: bool, area: Rect) -> Rect {
    let (dx, dy) = match side {
        Side::Left => (-step, 0),
        Side::Right => (step, 0),
        Side::Up => (0, -step),
        Side::Down => (0, step),
    };
    let grow = match nudge {
        Nudge::Move => return moved(rect, dx, dy, area),
        Nudge::Grow => step,
        Nudge::Shrink => -step,
    };
    let (mut l, mut t, mut r, mut b) = edges(rect);
    // 先改方向键所指的边，对称时再反向改对边。
    match side {
        Side::Left => l -= grow,
        Side::Right => r += grow,
        Side::Up => t -= grow,
        Side::Down => b += grow,
    }
    if symmetric {
        match side {
            Side::Left => r += grow,
            Side::Right => l -= grow,
            Side::Up => b += grow,
            Side::Down => t -= grow,
        }
    }
    if r - l < 1 || b - t < 1 {
        return rect;
    }
    from_edges(l, t, r, b, area).unwrap_or(rect)
}

/// 手柄中心点（物理像素）。
pub fn handle_point(rect: Rect, handle: Handle) -> Point {
    let (l, t, r, b) = edges(rect);
    let (hx, hy) = handle.anchor();
    let pick = |a: i64, z: i64, k: u8| match k {
        0 => a,
        1 => (a + z) / 2,
        _ => z,
    };
    (pick(l, r, hx), pick(t, b, hy))
}

/// 距离某个手柄中心不超过 `tolerance` 像素时命中；角手柄优先于边中点。
pub fn handle_at(rect: Rect, p: Point, tolerance: i64) -> Option<Handle> {
    Handle::ALL.into_iter().find(|&handle| {
        let (x, y) = handle_point(rect, handle);
        (p.0 - x).abs() <= tolerance && (p.1 - y).abs() <= tolerance
    })
}

pub fn contains(rect: Rect, p: Point) -> bool {
    let (l, t, r, b) = edges(rect);
    p.0 >= l && p.0 < r && p.1 >= t && p.1 < b
}

/// 两个矩形的交集；不相交返回 None。
pub fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (al, at, ar, ab) = edges(a);
    let (bl, bt, br, bb) = edges(b);
    from_edges(al.max(bl), at.max(bt), ar.min(br), ab.min(bb), a)
}

/// 吸附候选的种类；决定层级标签上的文字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// 系统辅助功能提供的控件。
    Element,
    /// 按画面识别出的界面区域（应用不提供控件信息时的补充）。
    Region,
    Window,
    Screen,
}

/// 一个吸附候选。`window` 是所属顶层窗口在系统前后层次中的序号（越小越靠上），
/// `name` 只在本机浮层的层级标签上显示，不写日志。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snap {
    pub rect: Rect,
    pub kind: SnapKind,
    pub window: usize,
    pub name: Option<std::sync::Arc<str>>,
}

fn area(rect: Rect) -> u64 {
    u64::from(rect.width) * u64::from(rect.height)
}

/// 鼠标下可切换的吸附层级，从内到外（面积从小到大）：只取包含该点的最上层窗口
/// 及其控件，被它挡住的下层窗口与控件不参与；默认层级是第一项（最内层控件）。
pub fn snap_levels(targets: &[Snap], p: Point) -> Vec<Snap> {
    let Some(top) = targets.iter().filter(|t| contains(t.rect, p)).map(|t| t.window).min() else {
        return Vec::new();
    };
    let mut levels: Vec<Snap> =
        targets.iter().filter(|t| t.window == top && contains(t.rect, p)).cloned().collect();
    // 同面积时控件在前、窗口在后，保证窗口始终是最外层。
    levels.sort_by_key(|t| (area(t.rect), t.kind == SnapKind::Window));
    levels.dedup_by(|a, b| a.rect == b.rect);
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0, y: 0, width: 100, height: 80 };

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn creating_normalizes_direction_and_clamps_to_display() {
        assert_eq!(from_points((30, 40), (10, 20), AREA), Some(rect(10, 20, 20, 20)));
        assert_eq!(from_points((90, 70), (150, -5), AREA), Some(rect(90, 0, 10, 70)));
        assert_eq!(from_points((5, 5), (5, 30), AREA), None);
        // 第二块屏从 x=100 开始：拖到它外面仍限制在本屏。
        let second = rect(100, 0, 50, 80);
        assert_eq!(from_points((120, 10), (60, 30), second), Some(rect(100, 10, 20, 20)));
    }

    #[test]
    fn moving_keeps_size_and_stops_at_edges() {
        let start = rect(10, 10, 30, 20);
        assert_eq!(moved(start, 5, -3, AREA), rect(15, 7, 30, 20));
        assert_eq!(moved(start, -50, 200, AREA), rect(0, 60, 30, 20));
    }

    #[test]
    fn resizing_moves_only_the_dragged_edges_and_flips_when_crossing() {
        let start = rect(10, 10, 30, 20);
        assert_eq!(resized(start, Handle::E, 5, 99, AREA), Some(rect(10, 10, 35, 20)));
        assert_eq!(resized(start, Handle::NW, -4, -6, AREA), Some(rect(6, 4, 34, 26)));
        // 左边拖过右边：选区翻到右侧。
        assert_eq!(resized(start, Handle::W, 40, 0, AREA), Some(rect(40, 10, 10, 20)));
        assert_eq!(resized(start, Handle::S, 0, -20, AREA), None);
    }

    #[test]
    fn keyboard_grows_shrinks_and_mirrors_the_pointed_edge() {
        let start = rect(10, 10, 30, 20);
        assert_eq!(nudged(start, Side::Right, Nudge::Move, 10, false, AREA), rect(20, 10, 30, 20));
        assert_eq!(nudged(start, Side::Left, Nudge::Grow, 1, false, AREA), rect(9, 10, 31, 20));
        assert_eq!(nudged(start, Side::Left, Nudge::Shrink, 1, false, AREA), rect(11, 10, 29, 20));
        assert_eq!(nudged(start, Side::Down, Nudge::Grow, 1, false, AREA), rect(10, 10, 30, 21));
        assert_eq!(nudged(start, Side::Up, Nudge::Shrink, 1, false, AREA), rect(10, 11, 30, 19));
        // 对称：两条边一起动，中心不变。
        assert_eq!(nudged(start, Side::Right, Nudge::Grow, 1, true, AREA), rect(9, 10, 32, 20));
        assert_eq!(nudged(start, Side::Up, Nudge::Shrink, 1, true, AREA), rect(10, 11, 30, 18));
        // 扩展停在显示区域边上；收缩不能把选区收没或翻转。
        assert_eq!(nudged(rect(0, 0, 10, 10), Side::Left, Nudge::Grow, 1, false, AREA), rect(0, 0, 10, 10));
        assert_eq!(nudged(rect(5, 5, 1, 4), Side::Right, Nudge::Shrink, 1, false, AREA), rect(5, 5, 1, 4));
        assert_eq!(nudged(rect(5, 5, 2, 4), Side::Left, Nudge::Shrink, 1, true, AREA), rect(5, 5, 2, 4));
    }

    #[test]
    fn locked_ratio_follows_the_longer_drag_and_shrinks_at_edges() {
        // 4:3：横向拖得更远时由宽推出高；向左上拖动时方向跟随。
        assert_eq!(from_points_ratio((10, 10), (50, 20), (4, 3), AREA), Some(rect(10, 10, 40, 30)));
        // 纵向更远：由高推出宽（53），左侧只剩 50 像素，等比缩成 50×37。
        assert_eq!(from_points_ratio((50, 50), (40, 10), (4, 3), AREA), Some(rect(0, 13, 50, 37)));
        assert_eq!(from_points_ratio((60, 60), (30, 20), (1, 1), AREA), Some(rect(20, 20, 40, 40)));
        // 往右下放不下：等比缩到碰边为止。
        assert_eq!(from_points_ratio((80, 10), (200, 20), (2, 1), AREA), Some(rect(80, 10, 20, 10)));
        assert_eq!(from_points_ratio((10, 70), (11, 200), (1, 2), AREA), Some(rect(10, 70, 5, 10)));
    }

    #[test]
    fn locked_ratio_resizes_from_the_opposite_corner_or_edge() {
        let start = rect(10, 10, 40, 30);
        assert_eq!(resized_ratio(start, Handle::SE, 20, 0, (4, 3), AREA), Some(rect(10, 10, 60, 45)));
        assert_eq!(resized_ratio(start, Handle::NW, -4, 0, (4, 3), AREA), Some(rect(6, 7, 44, 33)));
        assert_eq!(resized_ratio(start, Handle::E, 8, 50, (4, 3), AREA), Some(rect(10, 10, 48, 36)));
        assert_eq!(resized_ratio(start, Handle::S, 50, 6, (4, 3), AREA), Some(rect(10, 10, 48, 36)));
    }

    #[test]
    fn typed_size_is_clamped_inside_the_area() {
        assert_eq!(placed(90, 70, 30, 20, false, AREA), Some(rect(70, 60, 30, 20)));
        assert_eq!(placed(-5, 5, 300, 20, false, AREA), Some(rect(0, 5, 100, 20)));
        // 保持比例：200×100 缩成 100×50。
        assert_eq!(placed(0, 0, 200, 100, true, AREA), Some(rect(0, 0, 100, 50)));
        assert_eq!(placed(0, 0, 0, 10, false, AREA), None);
        assert_eq!(with_ratio(rect(10, 10, 40, 10), (4, 3), AREA), rect(10, 10, 40, 30));
        // 3:4 推出的高度超出：等比缩小后再向内推。
        assert_eq!(with_ratio(rect(0, 0, 90, 10), (3, 4), AREA), rect(0, 0, 60, 80));
    }

    fn snap(rect: Rect, kind: SnapKind, window: usize) -> Snap {
        Snap { rect, kind, window, name: None }
    }

    #[test]
    fn snap_levels_go_from_inner_control_to_window_and_skip_covered_windows() {
        let front = snap(rect(0, 0, 100, 80), SnapKind::Window, 0);
        let panel = snap(rect(10, 10, 60, 50), SnapKind::Element, 0);
        let button = snap(rect(20, 20, 10, 10), SnapKind::Element, 0);
        // 被前景窗口挡住的小窗口与它的控件：面积更小也不能被选中。
        let behind = snap(rect(15, 15, 30, 30), SnapKind::Window, 1);
        let hidden = snap(rect(22, 22, 5, 5), SnapKind::Element, 1);
        let targets = [front.clone(), behind.clone(), panel.clone(), button.clone(), hidden];
        assert_eq!(snap_levels(&targets, (24, 24)), vec![button, panel.clone(), front.clone()]);
        assert_eq!(snap_levels(&targets, (50, 50)), vec![panel, front]);
        // 前景窗口之外，下层窗口重新可选。
        let outside = snap(rect(200, 0, 50, 50), SnapKind::Window, 1);
        assert_eq!(snap_levels(std::slice::from_ref(&outside), (210, 10)), vec![outside]);
        assert!(snap_levels(&targets, (150, 150)).is_empty());
    }

    #[test]
    fn handles_hit_within_tolerance_and_rects_intersect() {
        let selection = rect(10, 10, 30, 20);
        assert_eq!(handle_at(selection, (11, 9), 3), Some(Handle::NW));
        assert_eq!(handle_at(selection, (25, 30), 3), Some(Handle::S));
        assert_eq!(handle_at(selection, (25, 20), 3), None);
        let window = rect(0, 0, 60, 60);
        assert_eq!(intersect(window, rect(50, 50, 30, 30)), Some(rect(50, 50, 10, 10)));
        assert_eq!(intersect(window, rect(70, 0, 5, 5)), None);
    }
}
