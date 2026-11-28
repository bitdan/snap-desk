//! 不规则选区：自由截图与折线截图的多边形、多窗口截图的矩形并集。
//!
//! 选区本身仍是外接矩形（裁剪、标注、尺寸都按矩形）；导出时把形状以外的像素设为透明。
//! 多边形按奇偶规则填充（自交的部分与界面遮罩一致），矩形取并集。坐标均为截图物理像素。
use super::Rect;
use image::RgbaImage;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// 闭合多边形的顶点（首尾自动相连）。
    Polygon(Vec<(f64, f64)>),
    /// 多个矩形的并集。
    Rects(Vec<Rect>),
}

impl Shape {
    /// 形状的外接矩形，限制在 `area` 内；面积为零或完全在外时为 None。
    pub fn bounds(&self, area: Rect) -> Option<Rect> {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let mut extend = |x: f64, y: f64| {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        };
        match self {
            Shape::Polygon(points) => {
                if points.len() < 3 {
                    return None;
                }
                points.iter().for_each(|&(x, y)| extend(x, y));
            }
            Shape::Rects(rects) => {
                if rects.is_empty() {
                    return None;
                }
                for r in rects {
                    extend(f64::from(r.x), f64::from(r.y));
                    extend(f64::from(r.x) + f64::from(r.width), f64::from(r.y) + f64::from(r.height));
                }
            }
        }
        let ax1 = f64::from(area.x) + f64::from(area.width);
        let ay1 = f64::from(area.y) + f64::from(area.height);
        let left = x0.floor().max(f64::from(area.x));
        let top = y0.floor().max(f64::from(area.y));
        let right = x1.ceil().min(ax1);
        let bottom = y1.ceil().min(ay1);
        (right - left >= 1.0 && bottom - top >= 1.0).then(|| Rect {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }

    /// 像素中心 `(x + 0.5, y + 0.5)` 是否在形状内。
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match self {
            Shape::Polygon(points) => {
                let mut inside = false;
                let n = points.len();
                for i in 0..n {
                    let (ax, ay) = points[i];
                    let (bx, by) = points[(i + n - 1) % n];
                    if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
                        inside = !inside;
                    }
                }
                inside
            }
            Shape::Rects(rects) => rects.iter().any(|r| {
                x >= f64::from(r.x)
                    && y >= f64::from(r.y)
                    && x < f64::from(r.x) + f64::from(r.width)
                    && y < f64::from(r.y) + f64::from(r.height)
            }),
        }
    }

    /// 把 `image`（左上角位于截图 `origin`）中形状以外的像素设为全透明。
    pub fn apply(&self, image: &mut RgbaImage, origin: (u32, u32)) {
        let (ox, oy) = (f64::from(origin.0), f64::from(origin.1));
        let width = image.width();
        for row in 0..image.height() {
            let y = oy + f64::from(row) + 0.5;
            // 本行在形状内的区间（图片内 x 范围，半开）。
            let spans = self.spans(y, ox, width);
            let mut covered = 0u32;
            for (start, end) in spans.iter().copied().chain(std::iter::once((width, width))) {
                for x in covered..start.min(width) {
                    image.get_pixel_mut(x, row).0[3] = 0;
                }
                covered = covered.max(end);
            }
        }
    }

    /// 扫描线 `y` 上位于形状内的像素区间（相对 `ox`，限制在 `0..width`，已排序、不重叠）。
    fn spans(&self, y: f64, ox: f64, width: u32) -> Vec<(u32, u32)> {
        let clamp = |v: f64| (v - ox).round().clamp(0.0, f64::from(width)) as u32;
        let mut spans: Vec<(u32, u32)> = match self {
            Shape::Polygon(points) => {
                let n = points.len();
                let mut xs: Vec<f64> = (0..n)
                    .filter_map(|i| {
                        let (ax, ay) = points[i];
                        let (bx, by) = points[(i + 1) % n];
                        ((ay > y) != (by > y)).then(|| ax + (y - ay) * (bx - ax) / (by - ay))
                    })
                    .collect();
                xs.sort_by(f64::total_cmp);
                // 像素中心落在 [a, b) 内视为在内：从 ceil(a - 0.5) 到 ceil(b - 0.5)。
                xs.chunks_exact(2)
                    .map(|pair| (clamp((pair[0] - 0.5).ceil()), clamp((pair[1] - 0.5).ceil())))
                    .collect()
            }
            Shape::Rects(rects) => rects
                .iter()
                .filter(|r| y >= f64::from(r.y) && y < f64::from(r.y) + f64::from(r.height))
                .map(|r| (clamp(f64::from(r.x)), clamp(f64::from(r.x) + f64::from(r.width))))
                .collect(),
        };
        spans.retain(|(a, b)| b > a);
        spans.sort_unstable();
        spans
    }
}

/// 去掉与上一个点距离小于 `min` 的点（自由截图的鼠标轨迹采样）。
pub fn simplify(points: &[(f64, f64)], min: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|&(x, y)| (p.0 - x).hypot(p.1 - y) >= min) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect { x: 0, y: 0, width: 100, height: 100 }
    }

    #[test]
    fn polygon_bounds_and_mask_keep_only_the_inside() {
        let triangle = Shape::Polygon(vec![(10.0, 10.0), (30.0, 10.0), (10.0, 30.0)]);
        let rect = triangle.bounds(area()).unwrap();
        assert_eq!(rect, Rect { x: 10, y: 10, width: 20, height: 20 });
        let mut image = RgbaImage::from_pixel(20, 20, image::Rgba([1, 2, 3, 255]));
        triangle.apply(&mut image, (10, 10));
        assert_eq!(image.get_pixel(1, 1).0[3], 255);
        assert_eq!(image.get_pixel(18, 18).0[3], 0);
        assert_eq!(image.get_pixel(18, 0).0[3], 255);
        assert_eq!(image.get_pixel(0, 18).0[3], 255);
    }

    #[test]
    fn rect_union_masks_the_gap_between_windows() {
        let shape = Shape::Rects(vec![
            Rect { x: 0, y: 0, width: 10, height: 10 },
            Rect { x: 5, y: 5, width: 10, height: 10 },
            Rect { x: 30, y: 0, width: 5, height: 5 },
        ]);
        let rect = shape.bounds(area()).unwrap();
        assert_eq!(rect, Rect { x: 0, y: 0, width: 35, height: 15 });
        let mut image = RgbaImage::from_pixel(35, 15, image::Rgba([0, 0, 0, 255]));
        shape.apply(&mut image, (0, 0));
        assert_eq!(image.get_pixel(7, 7).0[3], 255, "overlap stays opaque");
        assert_eq!(image.get_pixel(20, 2).0[3], 0, "gap is transparent");
        assert_eq!(image.get_pixel(32, 2).0[3], 255);
        assert_eq!(image.get_pixel(1, 12).0[3], 0);
        assert!(shape.contains(12.5, 12.5) && !shape.contains(20.5, 2.5));
    }

    #[test]
    fn degenerate_shapes_have_no_bounds_and_points_are_thinned() {
        assert!(Shape::Polygon(vec![(1.0, 1.0), (2.0, 2.0)]).bounds(area()).is_none());
        assert!(Shape::Rects(Vec::new()).bounds(area()).is_none());
        let thinned = simplify(&[(0.0, 0.0), (0.5, 0.0), (3.0, 0.0), (3.5, 0.5)], 2.0);
        assert_eq!(thinned, vec![(0.0, 0.0), (3.0, 0.0)]);
    }
}
