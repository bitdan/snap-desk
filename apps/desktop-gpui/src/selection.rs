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
