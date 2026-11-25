//! 按画面识别界面区域：给不提供辅助功能控件的应用（Java / Compose、Qt、游戏、自绘界面）
//! 补上吸附层级。
//!
//! 做法：在窗口范围内把颜色几乎相同的相邻像素连成区域（扁平界面的面板、按钮底色、表头、
//! 选中行都是一块纯色），再从鼠标处的区域起，逐级找到包围它的外层区域，得到由内到外的一串矩形。
//! 只读冻结截图，不访问其他进程；所有坐标都是截图物理像素。
use super::Rect;
use image::RgbaImage;

/// 相邻像素各通道差都不超过该值时视为同一块底色；纯色界面差为 0，抗锯齿与阴影会被切开。
const TOLERANCE: u8 = 4;
/// 小于该尺寸的区域（文字笔画、图标碎片）不作为吸附层级，但仍用作向外查找的起点。
const MIN_SIDE: u32 = 12;
/// 每级向外查找时，沿外圈最多采样的点数。
const RING_SAMPLES: u32 = 96;
const MAX_LEVELS: usize = 8;
/// 一条横线（竖线）至少横跨外层区域这么大比例的宽度（高度），才算分隔条带的分界线。
const LINE_COVERAGE: u32 = 85;
/// 圆角卡片的分界线两端会短一截；内层超出线端不多于这么多像素时仍算在条带内。
const CORNER: u32 = 12;

/// 一个窗口范围内的颜色连通区域；构建一次后可反复按鼠标位置查询。
pub struct Regions {
    bounds: Rect,
    /// 每个像素所属区域的编号（按 `bounds` 内行优先）。
    labels: Vec<u32>,
    /// 每个区域的外接矩形（截图坐标）。
    boxes: Vec<Rect>,
    /// 未被遮住部分的外接矩形；接近它的区域等同于整个窗口。
    visible: Rect,
    /// 横向分界的前缀和：`rows[k * (w + 1) + x]` 是第 k 行与第 k+1 行之间、前 x 列中颜色突变的列数。
    rows: Vec<u16>,
    /// 纵向分界的前缀和：`columns[k * (h + 1) + y]` 是第 k 列与第 k+1 列之间、前 y 行中颜色突变的行数。
    columns: Vec<u16>,
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        let next = parent[parent[i as usize] as usize];
        parent[i as usize] = next;
        i = next;
    }
    i
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        let (low, high) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[high as usize] = low;
    }
}

fn similar(a: &image::Rgba<u8>, b: &image::Rgba<u8>, tolerance: u8) -> bool {
    a.0[..3].iter().zip(&b.0[..3]).all(|(x, y)| x.abs_diff(*y) <= tolerance)
}

/// 被上层窗口遮住的像素：不属于任何区域，也不参与连通。
const COVERED: u32 = u32::MAX;

fn area(r: Rect) -> u64 {
    u64::from(r.width) * u64::from(r.height)
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.width >= inner.x + inner.width
        && outer.y + outer.height >= inner.y + inner.height
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect { x, y, width: right - x, height: bottom - y }
}

/// `outer` 只是 `inner` 向外扩了一圈细边（面板描边、内边距）时视为同一层。
fn nearly_same(inner: Rect, outer: Rect) -> bool {
    const SLACK: u32 = 6;
    contains_rect(outer, inner)
        && inner.x - outer.x <= SLACK
        && inner.y - outer.y <= SLACK
        && (outer.x + outer.width) - (inner.x + inner.width) <= SLACK
        && (outer.y + outer.height) - (inner.y + inner.height) <= SLACK
}

impl Regions {
    /// 把 `bounds`（会裁到截图范围内）里的像素连成区域；`covered` 是挡在这个窗口上面的
    /// 其他窗口，那部分画面不属于这个窗口，不参与识别。百万像素级，应在后台线程调用。
    pub fn build(image: &RgbaImage, bounds: Rect, covered: &[Rect]) -> Self {
        Self::build_tuned(image, bounds, covered, TOLERANCE)
    }

    /// 指定颜色容差的 [`Regions::build`]，用于调参与测试。
    #[doc(hidden)]
    pub fn build_tuned(image: &RgbaImage, bounds: Rect, covered: &[Rect], tolerance: u8) -> Self {
        let right = (bounds.x + bounds.width).min(image.width());
        let bottom = (bounds.y + bounds.height).min(image.height());
        let bounds = Rect {
            x: bounds.x.min(right),
            y: bounds.y.min(bottom),
            width: right.saturating_sub(bounds.x),
            height: bottom.saturating_sub(bounds.y),
        };
        let (w, h) = (bounds.width, bounds.height);
        let mut blocked = vec![false; (w * h) as usize];
        for c in covered {
            let (left, top) = (c.x.max(bounds.x), c.y.max(bounds.y));
            let right = (c.x + c.width).min(bounds.x + w);
            let bottom = (c.y + c.height).min(bounds.y + h);
            for y in top..bottom.max(top) {
                let row = ((y - bounds.y) * w) as usize;
                blocked[row + (left - bounds.x) as usize..row + (right.max(left) - bounds.x) as usize].fill(true);
            }
        }
        // 宽或高超过 65535 的区域不会出现在屏幕上；前缀和用 u16 省内存。
        let (w, h) = (w.min(u32::from(u16::MAX)), h.min(u32::from(u16::MAX)));
        let mut rows = vec![0u16; (h * (w + 1)) as usize];
        let mut columns = vec![0u16; (w * (h + 1)) as usize];
        let edge = |a: (u32, u32), b: (u32, u32)| {
            let (ia, ib) = ((a.1 * w + a.0) as usize, (b.1 * w + b.0) as usize);
            !blocked[ia]
                && !blocked[ib]
                && !similar(
                    image.get_pixel(bounds.x + a.0, bounds.y + a.1),
                    image.get_pixel(bounds.x + b.0, bounds.y + b.1),
                    tolerance,
                )
        };
        for k in 0..h.saturating_sub(1) {
            for x in 0..w {
                let i = (k * (w + 1) + x) as usize;
                rows[i + 1] = rows[i] + u16::from(edge((x, k), (x, k + 1)));
            }
        }
        for k in 0..w.saturating_sub(1) {
            for y in 0..h {
                let i = (k * (h + 1) + y) as usize;
                columns[i + 1] = columns[i] + u16::from(edge((k, y), (k + 1, y)));
            }
        }
        let mut parent: Vec<u32> = (0..w * h).collect();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if blocked[i as usize] {
                    continue;
                }
                let here = image.get_pixel(bounds.x + x, bounds.y + y);
                if x + 1 < w
                    && !blocked[i as usize + 1]
                    && similar(here, image.get_pixel(bounds.x + x + 1, bounds.y + y), tolerance)
                {
                    union(&mut parent, i, i + 1);
                }
                if y + 1 < h
                    && !blocked[(i + w) as usize]
                    && similar(here, image.get_pixel(bounds.x + x, bounds.y + y + 1), tolerance)
                {
                    union(&mut parent, i, i + w);
                }
            }
        }
        // 根编号压缩成连续编号，同时累计每个区域的外接矩形。
        let mut compact = vec![u32::MAX; (w * h) as usize];
        let mut edges: Vec<(u32, u32, u32, u32)> = Vec::new();
        let mut labels = vec![0u32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if blocked[i as usize] {
                    labels[i as usize] = COVERED;
                    continue;
                }
                let root = find(&mut parent, i) as usize;
                if compact[root] == u32::MAX {
                    compact[root] = edges.len() as u32;
                    edges.push((x, y, x, y));
                }
                let label = compact[root];
                labels[i as usize] = label;
                let e = &mut edges[label as usize];
                e.0 = e.0.min(x);
                e.1 = e.1.min(y);
                e.2 = e.2.max(x);
                e.3 = e.3.max(y);
            }
        }
        let visible = edges
            .iter()
            .copied()
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));
        let to_rect = |(l, t, r, b): (u32, u32, u32, u32)| Rect {
            x: bounds.x + l,
            y: bounds.y + t,
            width: r - l + 1,
            height: b - t + 1,
        };
        let visible = visible.map_or(Rect { width: 0, height: 0, ..bounds }, to_rect);
        let boxes = edges.into_iter().map(to_rect).collect();
        Self { bounds, labels, boxes, visible, rows, columns }
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    fn label_at(&self, x: u32, y: u32) -> Option<u32> {
        let b = self.bounds;
        if x < b.x || y < b.y || x >= b.x + b.width || y >= b.y + b.height {
            return None;
        }
        self.labels.get(((y - b.y) * b.width + (x - b.x)) as usize).copied().filter(|&label| label != COVERED)
    }

    /// 紧贴 `r` 外侧一圈的采样点所属区域。
    fn ring(&self, r: Rect) -> Vec<u32> {
        let b = self.bounds;
        let perimeter = 2 * (r.width + r.height);
        let step = (perimeter / RING_SAMPLES).max(1);
        let mut found = Vec::new();
        let mut sample = |x: i64, y: i64| {
            if x >= i64::from(b.x) && y >= i64::from(b.y) {
                if let Some(label) = self.label_at(x as u32, y as u32) {
                    found.push(label);
                }
            }
        };
        let (left, top) = (i64::from(r.x) - 1, i64::from(r.y) - 1);
        let (right, bottom) = (i64::from(r.x + r.width), i64::from(r.y + r.height));
        let mut x = left;
        while x <= right {
            sample(x, top);
            sample(x, bottom);
            x += i64::from(step);
        }
        let mut y = top;
        while y <= bottom {
            sample(left, y);
            sample(right, y);
            y += i64::from(step);
        }
        found
    }

    /// `inner` 外、`outer` 内由横跨 `outer` 的分界线围出的条带（工具栏、表头、列表行、分栏）。
    /// 这类条带常常只有上下（左右）两条细线，没有完整边框，颜色连通看不出来。
    fn bands(&self, inner: Rect, outer: Rect) -> Vec<Rect> {
        let b = self.bounds;
        let (w, h) = (b.width.min(u32::from(u16::MAX)), b.height.min(u32::from(u16::MAX)));
        // 全部换成区域内的局部坐标，右、下边界不含。
        let (il, it) = (inner.x - b.x, inner.y - b.y);
        let (ir, ib) = (il + inner.width, it + inner.height);
        let (ol, ot) = (outer.x - b.x, outer.y - b.y);
        let (or, ob) = ((ol + outer.width).min(w), (ot + outer.height).min(h));
        // 第 k 行与 k+1 行之间在 [x0, x1) 内突变的列数；纵向同理。
        let row = |k: u32, x0: u32, x1: u32| {
            let base = (k * (w + 1)) as usize;
            u32::from(self.rows[base + x1 as usize] - self.rows[base + x0 as usize])
        };
        let column = |k: u32, y0: u32, y1: u32| {
            let base = (k * (h + 1)) as usize;
            u32::from(self.columns[base + y1 as usize] - self.columns[base + y0 as usize])
        };
        // 分界线所在的边界序号及其实际跨度（第一处到最后一处突变）；外层自身的边缘也可作边界，
        // 但上下（左右）至少要有一条真正的分界线，否则只是外层本身。
        let mut found = Vec::new();
        let extent = |count: &dyn Fn(u32, u32) -> u32, lo: u32, hi: u32| {
            let first = (lo..hi).find(|&i| count(i, i + 1) > 0)?;
            let last = (first..hi).rev().find(|&i| count(i, i + 1) > 0)?;
            Some((first, last + 1))
        };
        let span = or - ol;
        let line = |k: u32| (row(k, ol, or) * 100 >= span * LINE_COVERAGE).then(|| extent(&|a, b| row(k, a, b), ol, or)).flatten();
        let top = (ot..it).rev().find_map(|k| (k + 1 < h).then(|| line(k)).flatten().map(|e| (k + 1, e)));
        let bottom = (ib.saturating_sub(1)..ob.saturating_sub(1)).find_map(|k| line(k).map(|e| (k + 1, e)));
        if top.is_some() || bottom.is_some() {
            let (t, te) = top.unwrap_or((ot, (ol, or)));
            let (bt, be) = bottom.unwrap_or((ob, (ol, or)));
            let (l, r) = (te.0.max(be.0), te.1.min(be.1));
            if l <= il + CORNER && r + CORNER >= ir {
                let (l, r) = (l.min(il), r.max(ir));
                found.push(Rect { x: b.x + l, y: b.y + t, width: r - l, height: bt - t });
            }
        }
        let span = ob - ot;
        let line = |k: u32| (column(k, ot, ob) * 100 >= span * LINE_COVERAGE).then(|| extent(&|a, b| column(k, a, b), ot, ob)).flatten();
        let left = (ol..il).rev().find_map(|k| (k + 1 < w).then(|| line(k)).flatten().map(|e| (k + 1, e)));
        let right = (ir.saturating_sub(1)..or.saturating_sub(1)).find_map(|k| line(k).map(|e| (k + 1, e)));
        if left.is_some() || right.is_some() {
            let (l, le) = left.unwrap_or((ol, (ot, ob)));
            let (r, re) = right.unwrap_or((or, (ot, ob)));
            let (t, bt) = (le.0.max(re.0), le.1.min(re.1));
            if t <= it + CORNER && bt + CORNER >= ib {
                let (t, bt) = (t.min(it), bt.max(ib));
                found.push(Rect { x: b.x + l, y: b.y + t, width: r - l, height: bt - t });
            }
        }
        found.retain(|&band| {
            contains_rect(band, inner)
                && contains_rect(outer, band)
                && !nearly_same(inner, band)
                && !nearly_same(band, outer)
        });
        found.sort_by_key(|&band| area(band));
        found
    }

    /// 鼠标处由内到外的界面区域；不含接近整个窗口的区域（窗口本身已是一级）。
    pub fn levels(&self, point: (u32, u32)) -> Vec<Rect> {
        let chain = self.chain(point);
        // 相邻两级之间（以及最外一级与整个窗口之间）补上分界线围出的条带。
        let mut out: Vec<Rect> = Vec::new();
        let whole = area(self.visible);
        for (i, &level) in chain.iter().enumerate() {
            let outer = chain.get(i + 1).copied().unwrap_or(self.visible);
            for rect in std::iter::once(level).chain(self.bands(level, outer)) {
                // 层级必须由内到外逐层包含：横向、纵向条带互不包含时只留先出现（较小）的一个。
                let skip = out.last().is_some_and(|&last| !contains_rect(rect, last) || nearly_same(last, rect));
                if area(rect) * 100 >= whole * 92 || skip {
                    continue;
                }
                out.push(rect);
            }
        }
        out.truncate(MAX_LEVELS);
        out
    }

    /// 只按颜色连通得到的由内到外的区域链。
    fn chain(&self, point: (u32, u32)) -> Vec<Rect> {
        let Some(start) = self.label_at(point.0, point.1) else {
            return Vec::new();
        };
        let whole = area(self.visible);
        let mut current = self.boxes[start as usize];
        let mut out: Vec<Rect> = Vec::new();
        let push = |out: &mut Vec<Rect>, rect: Rect| {
            if rect.width < MIN_SIDE || rect.height < MIN_SIDE || area(rect) * 100 >= whole * 92 {
                return;
            }
            match out.last_mut() {
                Some(last) if nearly_same(*last, rect) => *last = rect,
                _ => out.push(rect),
            }
        };
        push(&mut out, current);
        for _ in 0..MAX_LEVELS * 4 {
            if out.len() >= MAX_LEVELS || area(current) * 100 >= whole * 92 {
                break;
            }
            let ring = self.ring(current);
            if ring.is_empty() {
                break;
            }
            // 优先选完整包住当前区域的最小外层（面板底色、选中行）；没有时取外圈最常见的区域合并。
            let enclosing = ring
                .iter()
                .map(|&label| self.boxes[label as usize])
                .filter(|&r| contains_rect(r, current) && r != current)
                .min_by_key(|&r| area(r));
            let next = match enclosing {
                Some(r) => r,
                None => {
                    let mut counts = std::collections::HashMap::new();
                    for label in ring {
                        *counts.entry(label).or_insert(0u32) += 1;
                    }
                    let (&label, _) = counts.iter().max_by_key(|(&label, &count)| (count, std::cmp::Reverse(label)))
                        .expect("ring 非空");
                    union_rect(current, self.boxes[label as usize])
                }
            };
            if next == current {
                break;
            }
            current = next;
            push(&mut out, current);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    fn fill(image: &mut RgbaImage, r: Rect, color: [u8; 3]) {
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                image.put_pixel(x, y, image::Rgba([color[0], color[1], color[2], 255]));
            }
        }
    }

    /// 白色窗口里有一块浅灰侧栏，侧栏里有一个深色按钮，按钮上有一个“文字”小块。
    fn window() -> RgbaImage {
        let mut image = RgbaImage::from_pixel(200, 120, image::Rgba([255, 255, 255, 255]));
        fill(&mut image, rect(10, 10, 60, 100), [235, 235, 235]);
        fill(&mut image, rect(20, 30, 40, 20), [90, 90, 200]);
        fill(&mut image, rect(30, 36, 6, 8), [255, 255, 255]);
        image
    }

    #[test]
    fn levels_go_from_button_to_sidebar_and_skip_whole_window() {
        let regions = Regions::build(&window(), rect(0, 0, 200, 120), &[]);
        let button = rect(20, 30, 40, 20);
        let sidebar = rect(10, 10, 60, 100);
        assert_eq!(regions.levels((25, 40)), vec![button, sidebar]);
        // 文字小块本身太小，不单独成一级，从它向外仍能找到按钮与侧栏。
        assert_eq!(regions.levels((32, 40)), vec![button, sidebar]);
        assert_eq!(regions.levels((15, 100)), vec![sidebar]);
        // 窗口空白处只剩整个窗口，不重复给出。
        assert!(regions.levels((150, 60)).is_empty());
        assert!(regions.levels((500, 60)).is_empty());
    }

    #[test]
    fn a_thin_border_merges_with_the_panel_it_outlines() {
        let mut image = RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255]));
        fill(&mut image, rect(10, 10, 50, 40), [120, 120, 120]);
        fill(&mut image, rect(11, 11, 48, 38), [240, 240, 240]);
        let regions = Regions::build(&image, rect(0, 0, 100, 100), &[]);
        assert_eq!(regions.levels((30, 30)), vec![rect(10, 10, 50, 40)]);
    }

    #[test]
    fn a_toolbar_drawn_only_with_top_and_bottom_lines_becomes_a_level() {
        // 白色面板里的工具栏：只有上下两条浅灰细线，没有左右边框；工具栏里有一个灰色按钮。
        let mut image = RgbaImage::from_pixel(300, 200, image::Rgba([255, 255, 255, 255]));
        fill(&mut image, rect(0, 0, 300, 200), [250, 250, 250]);
        fill(&mut image, rect(20, 20, 260, 160), [255, 255, 255]);
        fill(&mut image, rect(20, 40, 260, 1), [242, 242, 242]);
        fill(&mut image, rect(20, 80, 260, 1), [242, 242, 242]);
        fill(&mut image, rect(40, 50, 60, 20), [234, 234, 234]);
        let regions = Regions::build(&image, rect(0, 0, 300, 200), &[]);
        assert_eq!(
            regions.levels((60, 60)),
            vec![rect(40, 50, 60, 20), rect(20, 41, 260, 39)]
        );
    }

    #[test]
    fn crossing_row_and_column_bands_keep_levels_nested() {
        // 网格：鼠标所在格子同时被一行和一列的分隔线围住，两条条带互不包含。
        let mut image = RgbaImage::from_pixel(200, 200, image::Rgba([255, 255, 255, 255]));
        for i in [50u32, 100, 150] {
            fill(&mut image, rect(0, i, 200, 1), [200, 200, 200]);
            fill(&mut image, rect(i, 0, 1, 200), [200, 200, 200]);
        }
        fill(&mut image, rect(110, 110, 20, 20), [90, 90, 200]);
        let regions = Regions::build(&image, rect(0, 0, 200, 200), &[]);
        let levels = regions.levels((115, 115));
        assert_eq!(levels.first(), Some(&rect(110, 110, 20, 20)));
        assert!(levels.windows(2).all(|pair| contains_rect(pair[1], pair[0])), "{levels:?}");
    }

    #[test]
    fn pixels_under_a_covering_window_are_ignored() {
        // 上层窗口盖住侧栏右半边：侧栏只剩露出的部分，鼠标落在被盖住的地方没有区域。
        let regions = Regions::build(&window(), rect(0, 0, 200, 120), &[rect(40, 0, 160, 120)]);
        assert_eq!(regions.levels((15, 100)), vec![rect(10, 10, 30, 100)]);
        assert!(regions.levels((50, 40)).is_empty());
    }

    #[test]
    fn build_clips_bounds_to_the_image_and_offsets_boxes() {
        let regions = Regions::build(&window(), rect(5, 5, 400, 400), &[]);
        assert_eq!(regions.bounds(), rect(5, 5, 195, 115));
        assert_eq!(regions.levels((25, 40)).first(), Some(&rect(20, 30, 40, 20)));
    }
}
