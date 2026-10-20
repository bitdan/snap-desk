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
    let band = |vertical: bool, reverse: bool| {
        let length = if vertical { a.width() } else { a.height() };
        let cross = if vertical { a.height() } else { a.width() };
        let mut textured = 0;
        let mut count = 0;
        for i in 0..length / 4 {
            let pos = if reverse { length - 1 - i } else { i };
            let mut same = 0;
            let mut row_texture = 0;
            for n in 1..=48 {
                let q = n * (cross - 1) / 49;
                let (x, y) = if vertical { (pos, q) } else { (q, pos) };
                same += u32::from(difference(a, b, x, y, x, y) <= 6);
                // 沿边界切向检查纹理，不能借用边界外正文的纹理把空白边距当固定栏。
                let (nx, ny) = if vertical {
                    (x, (y + 2).min(a.height() - 1))
                } else {
                    ((x + 2).min(a.width() - 1), y)
                };
                row_texture += u32::from(difference(a, a, x, y, nx, ny) > 20);
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
    Content {
        left: band(true, false),
        right: a.width() - band(true, true),
        top: band(false, false),
        bottom: a.height() - band(false, true),
    }
}

fn texture(image: &RgbaImage, x: u32, y: u32) -> bool {
    let p = image.get_pixel(x, y);
    [(2, 0), (0, 2), (2, 2)]
        .iter()
        .any(|&(dx, dy)| (0..3).any(|c| p[c].abs_diff(image.get_pixel(x + dx, y + dy)[c]) > 20))
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

/// 每个交集分成 4×4 块；独立纹理块须一致，动画块不参与写入决策。
fn verify(a: &RgbaImage, b: &RgbaImage, roi: Content, dx: i32, dy: i32) -> Option<Alignment> {
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

pub fn alignment(a: &RgbaImage, b: &RgbaImage, mode: ScrollMode) -> Match {
    if a.dimensions() != b.dimensions() || a.width() < 32 || a.height() < 64 {
        return Match::Uncertain;
    }
    // 相同画面无需再扫描全帧。精确相同不会把重复页面误认成移动。
    if a == b {
        return Match::Unchanged;
    }
    let roi = content(a, b);
    let mut patches: HashMap<u64, Vec<(u32, u32, usize)>> = HashMap::new();
    for row in 0..20 {
        for col in 0..20 {
            let x = roi.left + (roi.right - roi.left - 3) * col / 19;
            let y = roi.top + (roi.bottom - roi.top - 3) * row / 19;
            if texture(b, x, y) {
                patches.entry(fingerprint(b, x, y)).or_default().push((
                    x,
                    y,
                    (row / 5 * 4 + col / 5) as usize,
                ));
            }
        }
    }
    // 每块只保留少量命中；高度重复图案不能让候选数量或 CPU 无界增长。
    let mut hits: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
    for y in roi.top..roi.bottom - 2 {
        for x in roi.left..roi.right - 2 {
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
