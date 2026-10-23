//! 二维纯平移匹配：纹理指纹提出候选，空间分块与原始像素复核后才提交。
//! 不缩放或拉伸文字；重复纹理、运动冲突和无重叠均保守拒绝。
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MIN_OVERLAP: u32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollMode {
    #[default]
    Auto,
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Bounds {
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}

/// 排除固定栏后的帧内有效矩形；坐标仍以原始选区左上角为原点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Content {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
impl Content {
    pub fn full(image: &RgbaImage) -> Self {
        Self {
            left: 0,
            top: 0,
            right: image.width(),
            bottom: image.height(),
        }
    }

    /// 已确认排除的固定栏不因滚动方向改变而重新进入正文。
    /// 新检测到更窄的区域仍交给 Stitch 的布局检查拒绝，不能静默改写旧画布。
    fn intersect(self, known: Self) -> Self {
        Self {
            left: self.left.max(known.left),
            top: self.top.max(known.top),
            right: self.right.min(known.right),
            bottom: self.bottom.min(known.bottom),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// 文档坐标位移：向右、向下滚动分别为正数。
    pub dx: i32,
    pub dy: i32,
    /// 在新帧坐标中的真实交集。
    pub overlap: Bounds,
    pub confidence: f32,
    pub content: Content,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Match {
    Unchanged,
    Aligned(Alignment),
    Uncertain,
}

fn difference(a: &RgbaImage, b: &RgbaImage, ax: u32, ay: u32, bx: u32, by: u32) -> u32 {
    (0..3)
        .map(|c| u32::from(a.get_pixel(ax, ay)[c].abs_diff(b.get_pixel(bx, by)[c])))
        .sum()
}

/// 静止边界必须连续、包含纹理且在四分之一尺寸内终止；空白背景不当成固定栏。
fn content(a: &RgbaImage, b: &RgbaImage) -> Content {
    content_with_motion(a, b, 0, 0)
}

/// 首次斜向滚动时，固定表头只跟随横向分量，冻结列只跟随纵向分量。
/// 候选位移只是检测线索：仍要求连续、带纹理的边界和后续正文像素复核。
/// 纯单轴滚动沿用同位置检测，避免把整个正常正文误识别为固定区。
fn content_with_motion(a: &RgbaImage, b: &RgbaImage, dx: i32, dy: i32) -> Content {
    let band = |vertical: bool, reverse: bool, central: bool| {
        let length = if vertical { a.width() } else { a.height() };
        let cross = if vertical { a.height() } else { a.width() };
        let shift = if dx != 0 && dy != 0 {
            if vertical {
                dy
            } else {
                dx
            }
        } else {
            0
        };
        // q 属于新帧，q + shift 属于旧帧；两点都必须位于切向真实重叠。
        // 斜向移动时，只在两帧中央半区取样：可识别固定栏本就小于尺寸的
        // 四分之一，交叉冻结角因而不会混入另一条固定栏的切向复核。
        // 例如表头随 x 移动，但左上冻结角不动；把它们混采会错误否决表头。
        // 先尝试完整重叠，仅失败时使用中央区；不能让较大位移下中央重叠
        // 不足的新限制破坏原本可识别的单条固定栏。证据不足仍不猜边界。
        let inset = if central { i64::from(cross / 4) } else { 0 };
        let cross_end = i64::from(cross) - inset;
        let start = inset.max(inset - i64::from(shift));
        let end = cross_end.min(cross_end - i64::from(shift));
        if end - start < i64::from(MIN_OVERLAP) {
            return 0;
        }
        let mut textured = 0;
        let mut count = 0;
        for i in 0..length / 4 {
            let pos = if reverse { length - 1 - i } else { i };
            let mut same = 0;
            let mut row_texture = 0;
            for n in 1..=48 {
                let q = (start + i64::from(n) * (end - start - 1) / 49) as u32;
                let old_q = (i64::from(q) + i64::from(shift)) as u32;
                let (x, y) = if vertical { (pos, q) } else { (q, pos) };
                let (ax, ay) = if vertical { (pos, old_q) } else { (old_q, pos) };
                same += u32::from(difference(a, b, ax, ay, x, y) <= 6);
                // 沿边界切向检查纹理，不能借用边界外正文的纹理把空白边距当固定栏。
                let (nx, ny) = if vertical {
                    (x, (y + 2).min(a.height() - 1))
                } else {
                    ((x + 2).min(a.width() - 1), y)
                };
                row_texture += u32::from(difference(b, b, x, y, nx, ny) > 20);
            }
            if same < 46 {
                break;
            }
            textured += row_texture;
            count += 1;
        }
        if count >= 4 && count < length / 4 && textured >= 12 {
            count
        } else {
            0
        }
    };
    let detect = |vertical, reverse| {
        let full = band(vertical, reverse, false);
        if full == 0 && dx != 0 && dy != 0 {
            band(vertical, reverse, true)
        } else {
            full
        }
    };
    Content {
        left: detect(true, false),
        right: a.width() - detect(true, true),
        top: detect(false, false),
        bottom: a.height() - detect(false, true),
    }
}

fn texture(image: &RgbaImage, x: u32, y: u32) -> bool {
    let p = image.get_pixel(x, y);
    [(2, 0), (0, 2), (2, 2)]
        .iter()
        .any(|&(dx, dy)| (0..3).any(|c| p[c].abs_diff(image.get_pixel(x + dx, y + dy)[c]) > 20))
}

/// 检查正文中是否存在随屏幕静止、却不服从文档位移的纹理小块。
/// 仅凭大多数重叠像素一致不能安全写入整帧：悬浮按钮可能位于新增条带，
/// 根本不参加重叠复核。这里扫描整个有效内容区，发现冲突就保留可信画布。
/// 这不是控件识别器；不尝试猜测遮挡下的正文，也不放宽动画的匹配阈值。
pub(super) fn stationary_overlay_conflict(
    a: &RgbaImage,
    b: &RgbaImage,
    roi: Content,
    dx: i32,
    dy: i32,
) -> bool {
    if dx == 0 && dy == 0 {
        return false;
    }
    // 每隔 8 像素检查一个 5×5 小块的九个点。先比较屏幕同位置，通常首点
    // 就能排除正常正文；只有九点均静止且有纹理时才检查文档坐标。
    for y in (roi.top..roi.bottom.saturating_sub(4)).step_by(8) {
        for x in (roi.left..roi.right.saturating_sub(4)).step_by(8) {
            let stationary = (0..=4).step_by(2).all(|oy| {
                (0..=4)
                    .step_by(2)
                    .all(|ox| difference(a, b, x + ox, y + oy, x + ox, y + oy) <= 6)
            });
            if !stationary || !texture(b, x, y) {
                continue;
            }
            let mut conflicts = 0;
            for oy in (0..=4).step_by(2) {
                for ox in (0..=4).step_by(2) {
                    let ax = i64::from(x + ox) + i64::from(dx);
                    let ay = i64::from(y + oy) + i64::from(dy);
                    // 位移后的点在旧内容区之外时也算冲突：否则新增条带内的
                    // 悬浮按钮会绕过检查。空白块没有纹理，不会走到这里。
                    if ax < i64::from(roi.left)
                        || ax >= i64::from(roi.right)
                        || ay < i64::from(roi.top)
                        || ay >= i64::from(roi.bottom)
                        || difference(a, b, ax as u32, ay as u32, x + ox, y + oy) > 36
                    {
                        conflicts += 1;
                    }
                }
            }
            if conflicts >= 5 {
                return true;
            }
        }
    }
    false
}

/// 小纹理块哈希只用于提名，不作为匹配结论。索引最多 400 个采样块，避免按屏幕像素建表。
fn fingerprint(image: &RgbaImage, x: u32, y: u32) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for (dx, dy) in [(0, 0), (2, 0), (0, 2), (2, 2)] {
        for c in 0..3 {
            hash =
                (hash ^ u64::from(image.get_pixel(x + dx, y + dy)[c])).wrapping_mul(0x100000001b3);
        }
    }
    hash
}

/// 先用两个像素筛掉不可能命中的位置，再计算完整指纹。
/// 位表只会多放行、不会漏掉相同纹理；最终仍使用原来的完整指纹与像素复核。
const PREFIX_WORDS: usize = 1024;
fn prefix(image: &RgbaImage, x: u32, y: u32) -> usize {
    let a = image.get_pixel(x, y);
    let b = image.get_pixel(x + 2, y + 2);
    let rgb =
        |p: &image::Rgba<u8>| u32::from(p[0]) | (u32::from(p[1]) << 8) | (u32::from(p[2]) << 16);
    let mixed = rgb(a).wrapping_mul(0x9e3779b1) ^ rgb(b).wrapping_mul(0x85ebca6b);
    ((mixed ^ (mixed >> 16)) & 0xffff) as usize
}

/// 已由最新帧确定位置时，历史锚点只需复核这个位移，不再全图搜索一次。
/// None 表示缺少可信证据；调用方保留完整搜索作为失败回退。
pub(super) fn verify_at(
    a: &RgbaImage,
    b: &RgbaImage,
    dx: i32,
    dy: i32,
    known: Content,
) -> Option<Alignment> {
    if a.dimensions() != b.dimensions() || a.width() < 32 || a.height() < 64 {
        return None;
    }
    verify(a, b, content(a, b).intersect(known), dx, dy)
}

/// 每个交集分成 4×4 块，按多数纹理提名位移；少量差异仍可能通过。
/// 这里只负责定位，实际写入前还须由 Stitch 复核画布与接缝冲突。
fn verify(a: &RgbaImage, b: &RgbaImage, roi: Content, dx: i32, dy: i32) -> Option<Alignment> {
    // 全搜索和历史位移快速复核共用此入口，不能让其中一路丢掉新边界证据。
    let roi = if dx != 0 && dy != 0 {
        content_with_motion(a, b, dx, dy).intersect(roi)
    } else {
        roi
    };
    let left = (roi.left as i32).max(roi.left as i32 - dx);
    let top = (roi.top as i32).max(roi.top as i32 - dy);
    let right = (roi.right as i32).min(roi.right as i32 - dx);
    let bottom = (roi.bottom as i32).min(roi.bottom as i32 - dy);
    if right - left < MIN_OVERLAP as i32 || bottom - top < MIN_OVERLAP as i32 {
        return None;
    }
    let mut votes = 0;
    let mut active = 0;
    let mut good_total = 0;
    let mut total = 0;
    for row in 0..4 {
        for col in 0..4 {
            let x0 = left + (right - left) * col / 4;
            let x1 = left + (right - left) * (col + 1) / 4;
            let y0 = top + (bottom - top) * row / 4;
            let y1 = top + (bottom - top) * (row + 1) / 4;
            let mut edges = 0;
            let mut good_edges = 0;
            for sy in 0..8 {
                for sx in 0..8 {
                    let x = x0 + (x1 - x0 - 1) * sx / 7;
                    let y = y0 + (y1 - y0 - 1) * sy / 7;
                    let good =
                        difference(a, b, (x + dx) as u32, (y + dy) as u32, x as u32, y as u32)
                            <= 36;
                    good_total += u32::from(good);
                    total += 1;
                    if x + 2 < right && y + 2 < bottom && texture(b, x as u32, y as u32) {
                        edges += 1;
                        good_edges += u32::from(good);
                    }
                }
            }
            if edges >= 4 {
                active += 1;
                votes += u32::from(good_edges * 100 >= edges * 90);
            }
        }
    }
    if votes < 2 || votes * 100 < active * 85 || good_total * 100 < total * 90 {
        return None;
    }
    Some(Alignment {
        dx,
        dy,
        overlap: Bounds {
            x: left as i64,
            y: top as i64,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        },
        confidence: good_total as f32 / total as f32,
        content: roi,
    })
}

#[cfg(test)]
pub fn alignment(a: &RgbaImage, b: &RgbaImage, mode: ScrollMode) -> Match {
    align_impl(a, b, mode, true)
}

/// 直接匹配与历史锚点匹配共用内容区，保证候选提名、像素复核和写入边界一致。
pub(super) fn alignment_in(
    a: &RgbaImage,
    b: &RgbaImage,
    mode: ScrollMode,
    known: Content,
) -> Match {
    align_content(a, b, mode, true, known)
}

// 测试可关闭前置过滤，与原扫描路径对照；生产路径始终启用。
#[cfg(test)]
fn align_impl(a: &RgbaImage, b: &RgbaImage, mode: ScrollMode, filter_prefix: bool) -> Match {
    align_content(a, b, mode, filter_prefix, Content::full(a))
}

fn align_content(
    a: &RgbaImage,
    b: &RgbaImage,
    mode: ScrollMode,
    filter_prefix: bool,
    known: Content,
) -> Match {
    if a.dimensions() != b.dimensions() || a.width() < 32 || a.height() < 64 {
        return Match::Uncertain;
    }
    // 相同画面无需再扫描全帧。精确相同不会把重复页面误认成移动。
    if a == b {
        return Match::Unchanged;
    }
    let roi = content(a, b).intersect(known);
    // 防止无效内容区在下面的无符号采样坐标计算中下溢。
    if roi.right.saturating_sub(roi.left) < MIN_OVERLAP
        || roi.bottom.saturating_sub(roi.top) < MIN_OVERLAP
    {
        return Match::Uncertain;
    }
    let mut patches: HashMap<u64, Vec<(u32, u32, usize)>> = HashMap::new();
    let mut prefixes = [0u64; PREFIX_WORDS];
    for row in 0..20 {
        for col in 0..20 {
            let x = roi.left + (roi.right - roi.left - 3) * col / 19;
            let y = roi.top + (roi.bottom - roi.top - 3) * row / 19;
            if texture(b, x, y) {
                let key = prefix(b, x, y);
                prefixes[key / 64] |= 1u64 << (key % 64);
                patches.entry(fingerprint(b, x, y)).or_default().push((
                    x,
                    y,
                    (row / 5 * 4 + col / 5) as usize,
                ));
            }
        }
    }
    if patches.is_empty() {
        return Match::Uncertain;
    }
    // 每块只保留少量命中；高度重复图案不能让候选数量或 CPU 无界增长。
    let mut hits: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
    for y in roi.top..roi.bottom - 2 {
        for x in roi.left..roi.right - 2 {
            if filter_prefix {
                let key = prefix(a, x, y);
                if prefixes[key / 64] & (1u64 << (key % 64)) == 0 {
                    continue;
                }
            }
            if !texture(a, x, y) {
                continue;
            }
            let hash = fingerprint(a, x, y);
            if patches.contains_key(&hash) {
                let points = hits.entry(hash).or_default();
                if points.len() < 9 {
                    points.push((x, y));
                }
            }
        }
    }
    let mut candidates: HashMap<(i32, i32), u16> = HashMap::new();
    for (hash, points) in hits {
        // 重复块只允许有限候选参与消歧；九次以上视为无定位价值。
        if points.len() > 8 {
            continue;
        }
        for &(bx, by, region) in &patches[&hash] {
            for &(ax, ay) in &points {
                let (dx, dy) = (ax as i32 - bx as i32, ay as i32 - by as i32);
                // 先收集各方向冲突，模式只限制最终接受的位移，不能掩盖独立面板。
                if dx == 0 && dy == 0 {
                    continue;
                }
                *candidates.entry((dx, dy)).or_default() |= 1 << region;
            }
        }
    }
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter(|(_, regions)| regions.count_ones() >= 2)
        .collect();
    ranked.sort_by_key(|(_, regions)| std::cmp::Reverse(regions.count_ones()));
    // 两个明显不同的运动都获得多区域支持时，不能猜测主容器。
    if ranked.len() > 64 {
        return Match::Uncertain;
    }
    let mut valid = Vec::new();
    for ((dx, dy), _) in ranked {
        if let Some(result) = verify(a, b, roi, dx, dy) {
            valid.push(result);
        }
    }
    valid.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let Some(best) = valid.first().copied() else {
        return Match::Uncertain;
    };
    if valid
        .iter()
        .skip(1)
        .any(|v| v.confidence >= best.confidence - 0.03)
    {
        return Match::Uncertain;
    }
    if (mode == ScrollMode::Vertical && best.dx != 0)
        || (mode == ScrollMode::Horizontal && best.dy != 0)
    {
        return Match::Uncertain;
    }
    Match::Aligned(best)
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    use image::Rgba;
    use std::{hint::black_box, time::Instant};

    fn document(width: u32, height: u32, sparse: bool) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            if sparse && (y % 30 > 10 || x % 50 > 24) {
                return Rgba([250, 250, 250, 255]);
            }
            let value =
                (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
        })
    }

    #[test]
    fn prefix_filter_preserves_full_scan_results() {
        for sparse in [false, true] {
            let doc = document(560, 560, sparse);
            let frame = |x, y| image::imageops::crop_imm(&doc, x, y, 160, 160).to_image();
            let a = frame(160, 160);
            for (x, y) in [
                (160, 160),
                (161, 160),
                (160, 288),
                (32, 160),
                (205, 107),
                (380, 380),
            ] {
                let b = frame(x, y);
                for mode in [
                    ScrollMode::Auto,
                    ScrollMode::Vertical,
                    ScrollMode::Horizontal,
                ] {
                    assert_eq!(alignment(&a, &b, mode), align_impl(&a, &b, mode, false));
                }
            }
        }
        let repeat = RgbaImage::from_fn(160, 320, |x, y| {
            Rgba([((x % 20) * 12) as u8, ((y % 20) * 12) as u8, 0, 255])
        });
        let a = image::imageops::crop_imm(&repeat, 0, 0, 160, 160).to_image();
        let b = image::imageops::crop_imm(&repeat, 0, 5, 160, 160).to_image();
        assert_eq!(alignment(&a, &b, ScrollMode::Auto), Match::Uncertain);
        assert_eq!(
            alignment(&a, &b, ScrollMode::Auto),
            align_impl(&a, &b, ScrollMode::Auto, false)
        );
    }

    #[test]
    fn diagonal_known_offset_preserves_new_fixed_band_evidence() {
        let doc = document(600, 600, false);
        for columns in [false, true] {
            let frame = |x, y| {
                let mut image = image::imageops::crop_imm(&doc, x, y, 160, 160).to_image();
                for py in 0..160 {
                    for px in 0..160 {
                        if columns && (px < 12 || px >= 148) {
                            image.put_pixel(px, py, *doc.get_pixel(400 + px, y + py));
                        } else if !columns && (py < 12 || py >= 148) {
                            image.put_pixel(px, py, *doc.get_pixel(x + px, 400 + py));
                        }
                    }
                }
                image
            };
            let a = frame(100, 100);
            let b = frame(140, 60);
            assert_eq!(content(&a, &b), Content::full(&a));
            let verified = verify_at(&a, &b, 40, -40, Content::full(&a)).unwrap();
            assert_eq!(
                verified.content,
                if columns {
                    Content {
                        left: 12,
                        top: 0,
                        right: 148,
                        bottom: 160,
                    }
                } else {
                    Content {
                        left: 0,
                        top: 12,
                        right: 160,
                        bottom: 148,
                    }
                }
            );
            assert_eq!(
                alignment(&a, &b, ScrollMode::Auto),
                Match::Aligned(verified)
            );
            assert!(verify_at(&a, &b, 41, -40, Content::full(&a)).is_none());
        }
    }

    #[test]
    fn known_offset_still_requires_pixel_agreement() {
        let doc = document(400, 400, false);
        let a = image::imageops::crop_imm(&doc, 80, 80, 160, 160).to_image();
        let b = image::imageops::crop_imm(&doc, 120, 50, 160, 160).to_image();
        let expected = verify_at(&a, &b, 40, -30, Content::full(&a)).unwrap();
        assert_eq!(
            alignment(&a, &b, ScrollMode::Auto),
            Match::Aligned(expected)
        );
        assert!(verify_at(&a, &b, 41, -30, Content::full(&a)).is_none());
        assert!(verify_at(&a, &b, 150, 0, Content::full(&a)).is_none());
    }

    /// 同一进程交替执行优化前后扫描，输出中位数；不把机器相关耗时写成测试阈值。
    #[test]
    #[ignore = "滚动识别定向性能基准；需用户允许编译后以 release 手动运行"]
    fn compare_matcher_scan_cost() {
        for (width, height) in [(640, 480), (1280, 720), (2560, 1440)] {
            for sparse in [false, true] {
                let doc = document(width + 200, height + 200, sparse);
                let a = image::imageops::crop_imm(&doc, 0, 0, width, height).to_image();
                let b = image::imageops::crop_imm(&doc, 61, 97, width, height).to_image();
                let expected = align_impl(&a, &b, ScrollMode::Auto, false);
                assert!(matches!(expected, Match::Aligned(_)));
                let mut baseline = Vec::new();
                let mut optimized = Vec::new();
                for iteration in 0..6 {
                    for filter in if iteration % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        let started = Instant::now();
                        let result =
                            align_impl(black_box(&a), black_box(&b), ScrollMode::Auto, filter);
                        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                        assert_eq!(result, expected);
                        if filter {
                            optimized.push(elapsed);
                        } else {
                            baseline.push(elapsed);
                        }
                    }
                }
                baseline.sort_by(f64::total_cmp);
                optimized.sort_by(f64::total_cmp);
                let before = (baseline[2] + baseline[3]) / 2.0;
                let after = (optimized[2] + optimized[3]) / 2.0;
                eprintln!("scroll.matcher {width}x{height} sparse={sparse} baseline_median_ms={before:.2} optimized_median_ms={after:.2} ratio={:.2}",before/after);
            }
        }
    }
}
