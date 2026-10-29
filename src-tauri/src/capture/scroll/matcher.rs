//! 单轴平移匹配：只扫描固定的 20 条采样线，分区投票后复核重叠像素。
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

/// 检查正文中是否存在随屏幕静止、却不服从文档位移的纹理小块。
/// 只在真实重叠区检查反证：新增条带没有旧文档像素可比较，不能仅凭屏幕同位置
/// 恰好出现相同小块，就把 IDEA 中重复的括号、缩进或关键字认作悬浮控件。
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
                    // 旧帧未观察到该文档坐标时没有反证，不能把“未知”当作像素冲突。
                    // 新增条带中的重复代码会与旧屏幕同位置相同，但这是合法文档内容。
                    if ax < i64::from(roi.left)
                        || ax >= i64::from(roi.right)
                        || ay < i64::from(roi.top)
                        || ay >= i64::from(roi.bottom)
                    {
                        continue;
                    }
                    if difference(a, b, ax as u32, ay as u32, x + ox, y + oy) > 36 {
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

/// 已由最新帧确定位置时，历史锚点只需复核这个位移，不再全图搜索一次。
/// None 表示缺少可信证据；调用方保留完整搜索作为失败回退。
pub(super) fn verify_at(
    a: &RgbaImage,
    b: &RgbaImage,
    dx: i32,
    dy: i32,
    known: Content,
) -> Option<Alignment> {
    if (dx != 0 && dy != 0) || a.dimensions() != b.dimensions() || a.width() < 32 || a.height() < 64
    {
        return None;
    }
    verify(a, b, content(a, b).intersect(known), dx, dy)
}

/// 每个交集分成 4×4 块，按多数纹理提名位移；少量差异仍可能通过。
/// 这里只负责定位，实际写入前还须由 Stitch 复核画布与接缝冲突。
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

#[cfg(test)]
pub fn alignment(a: &RgbaImage, b: &RgbaImage, mode: ScrollMode) -> Match {
    alignment_in(a, b, mode, Content::full(a))
}

/// 自动状态只用于首次识别：分别试两个单轴，不能接受斜向或有歧义的位移。
/// Windows 会话在调用前由滚轮输入锁轴；其他平台由首个可信单轴画面锁轴。
pub(super) fn alignment_in(
    a: &RgbaImage,
    b: &RgbaImage,
    mode: ScrollMode,
    known: Content,
) -> Match {
    if a.dimensions() != b.dimensions() || a.width() < 32 || a.height() < 64 {
        return Match::Uncertain;
    }
    if a == b {
        return Match::Unchanged;
    }
    let roi = content(a, b).intersect(known);
    if roi.right.saturating_sub(roi.left) < MIN_OVERLAP
        || roi.bottom.saturating_sub(roi.top) < MIN_OVERLAP
    {
        return Match::Uncertain;
    }
    // 返回原位置后可能发生少量文字重绘；原位置也必须作为候选参加歧义检查，
    // 不能把一次全局稳定判断直接当作 Unchanged，更不能覆盖已知重复纹理歧义。
    let stationary = roi == known && super::stability::stable(a, b);
    match mode {
        ScrollMode::Vertical => align_axis(a, b, roi, true, stationary),
        ScrollMode::Horizontal => align_axis(a, b, roi, false, stationary),
        ScrollMode::Auto => match (
            align_axis(a, b, roi, true, stationary),
            align_axis(a, b, roi, false, stationary),
        ) {
            (Match::Aligned(a), Match::Aligned(b))
                if a.dx == 0 && a.dy == 0 && b.dx == 0 && b.dy == 0 =>
            {
                Match::Aligned(a)
            }
            (result @ Match::Aligned(_), Match::Uncertain)
            | (Match::Uncertain, result @ Match::Aligned(_)) => result,
            _ => Match::Uncertain,
        },
    }
}

/// 单轴移动时横截坐标保持不变，因此只需要扫描 20 条线，而无需遍历整帧。
/// 指纹仅提名位移；重复纹理、有多个近似解、无重叠仍然拒绝，最终检查原始像素。
fn align_axis(
    a: &RgbaImage,
    b: &RgbaImage,
    roi: Content,
    vertical: bool,
    stationary: bool,
) -> Match {
    match align_axis_exact(a, b, roi, vertical, stationary) {
        Ok(result) => return Match::Aligned(result),
        // 已经复核出多个近似位移时保留拒绝，不能让回退丢掉已知歧义。
        Err(true) => return Match::Uncertain,
        Err(false) => {}
    }
    // 浏览器滚动后可能重新栅格化文字。精确指纹没有通过时，再用有容差的小块提名；
    // 两条路径最终共用 verify，不能为了恢复候选而降低整幅重叠区的验收标准。
    align_axis_tolerant(a, b, roi, vertical)
}

// Err(true) 表示像素复核已确认歧义；Err(false) 表示证据不足，可继续容差搜索。
fn align_axis_exact(
    a: &RgbaImage,
    b: &RgbaImage,
    roi: Content,
    vertical: bool,
    stationary: bool,
) -> Result<Alignment, bool> {
    let (start, end, cross_start, cross_end) = if vertical {
        (roi.top, roi.bottom, roi.left, roi.right)
    } else {
        (roi.left, roi.right, roi.top, roi.bottom)
    };
    let point = |cross, along| {
        if vertical {
            (cross, along)
        } else {
            (along, cross)
        }
    };
    let mut candidates: HashMap<i32, u16> = HashMap::new();
    for line in 0..20 {
        let cross = cross_start + (cross_end - cross_start - 3) * line / 19;
        let mut patches: HashMap<u64, Vec<(u32, usize)>> = HashMap::new();
        for sample in 0..20 {
            let along = start + (end - start - 3) * sample / 19;
            let (x, y) = point(cross, along);
            if texture(b, x, y) {
                patches
                    .entry(fingerprint(b, x, y))
                    .or_default()
                    .push((along, (line / 5 * 4 + sample / 5) as usize));
            }
        }
        if patches.is_empty() {
            continue;
        }
        let mut hits: HashMap<u64, Vec<u32>> = HashMap::new();
        for along in start..end - 2 {
            let (x, y) = point(cross, along);
            let hash = fingerprint(a, x, y);
            if patches.contains_key(&hash) {
                let points = hits.entry(hash).or_default();
                if points.len() < 9 {
                    points.push(along);
                }
            }
        }
        for (hash, points) in hits {
            if points.len() > 8 {
                continue;
            }
            for &(next, region) in &patches[&hash] {
                for &previous in &points {
                    let shift = previous as i32 - next as i32;
                    if shift != 0 {
                        *candidates.entry(shift).or_default() |= 1 << region;
                    }
                }
            }
        }
    }
    let ranked: Vec<_> = candidates
        .into_iter()
        .filter(|(_, votes)| votes.count_ones() >= 2)
        .collect();
    if ranked.len() > 64 {
        return Err(false);
    }
    let mut valid: Vec<_> = ranked
        .into_iter()
        .filter_map(|(shift, _)| {
            let (dx, dy) = if vertical { (0, shift) } else { (shift, 0) };
            verify(a, b, roi, dx, dy)
        })
        .collect();
    if stationary {
        if let Some(candidate) = verify(a, b, roi, 0, 0) {
            valid.push(candidate);
        }
    }
    valid.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let Some(best) = valid.first().copied() else {
        return Err(false);
    };
    if valid
        .iter()
        .skip(1)
        .any(|v| v.confidence >= best.confidence - 0.03)
    {
        return Err(true);
    }
    Ok(best)
}

/// 有界的容差回退：在 20 条线上自适应选取纹理，不要求文字恰好落在固定网格点。
/// 每个空间分区最多两个锚点，总数最多 32；只沿已锁定的轴搜索，不做二维搜索。
fn align_axis_tolerant(a: &RgbaImage, b: &RgbaImage, roi: Content, vertical: bool) -> Match {
    let (start, end, cross_start, cross_end) = if vertical {
        (roi.top, roi.bottom, roi.left, roi.right)
    } else {
        (roi.left, roi.right, roi.top, roi.bottom)
    };
    let point = |cross, along| {
        if vertical {
            (cross, along)
        } else {
            (along, cross)
        }
    };
    // (纹理强度, 横截坐标, 轴向坐标)，各区保留相隔至少 8 像素的两个小块。
    let mut anchors: [Vec<(u32, u32, u32)>; 16] = std::array::from_fn(|_| Vec::new());
    for line in 0..20 {
        let cross = cross_start + (cross_end - cross_start - 3) * line / 19;
        for along in start..end - 2 {
            let (x, y) = point(cross, along);
            let score = difference(b, b, x, y, x + 2, y)
                + difference(b, b, x, y, x, y + 2)
                + difference(b, b, x, y, x + 2, y + 2);
            if score <= 60 {
                continue;
            }
            let region = (line / 5 * 4 + ((along - start) * 4 / (end - start)).min(3)) as usize;
            let patches = &mut anchors[region];
            if let Some(close) = patches
                .iter_mut()
                .find(|(_, c, p)| cross.abs_diff(*c) + along.abs_diff(*p) < 8)
            {
                if score > close.0 {
                    *close = (score, cross, along);
                }
            } else if patches.len() < 2 {
                patches.push((score, cross, along));
            } else {
                let weakest = usize::from(patches[1].0 < patches[0].0);
                if score > patches[weakest].0 {
                    patches[weakest] = (score, cross, along);
                }
            }
        }
    }
    let length = (end - start) as i32;
    // 位移以 length 为零点索引，避免为每个命中分配 HashMap 节点。
    let mut votes = vec![0u16; (length * 2 + 1) as usize];
    for (region, patches) in anchors.iter().enumerate() {
        for &(_, cross, next) in patches {
            let (bx, by) = point(cross, next);
            for previous in start..end - 2 {
                let shift = previous as i32 - next as i32;
                if shift == 0 || length - shift.abs() < MIN_OVERLAP as i32 {
                    continue;
                }
                let (ax, ay) = point(cross, previous);
                if difference(a, b, ax, ay, bx, by) > 36 {
                    continue;
                }
                let mut bad = 0;
                'patch: for oy in 0..3 {
                    for ox in 0..3 {
                        bad += u32::from(difference(a, b, ax + ox, ay + oy, bx + ox, by + oy) > 36);
                        if bad > 1 {
                            break 'patch;
                        }
                    }
                }
                if bad <= 1 {
                    votes[(shift + length) as usize] |= 1 << region;
                }
            }
        }
    }
    let mut best: Option<Alignment> = None;
    let mut runner_up = 0.0f32;
    let mut candidates = 0;
    for (index, regions) in votes.into_iter().enumerate() {
        if regions.count_ones() < 2 {
            continue;
        }
        candidates += 1;
        let shift = index as i32 - length;
        let (dx, dy) = if vertical { (0, shift) } else { (shift, 0) };
        let Some(result) = verify(a, b, roi, dx, dy) else {
            continue;
        };
        if best.is_none_or(|current| result.confidence > current.confidence) {
            runner_up = best.map_or(runner_up, |current| runner_up.max(current.confidence));
            best = Some(result);
        } else {
            runner_up = runner_up.max(result.confidence);
        }
        // 次优解已接近满分时，任何后续候选都无法拉开安全差距；重复表格尽早拒绝。
        if runner_up >= 0.97 {
            break;
        }
    }
    if log::log_enabled!(log::Level::Debug) && crate::diagnostics::runtime::sample_debug() {
        log::debug!(
            "event=scroll.match_tolerant module=scroll candidates={} matched={}",
            candidates,
            best.is_some()
        );
    }
    match best {
        Some(result) if runner_up < result.confidence - 0.03 => Match::Aligned(result),
        _ => Match::Uncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 白底、窄文字区、重复行距；每行的笔画不同，文档位置有唯一真值。
    fn text_document() -> RgbaImage {
        RgbaImage::from_fn(480, 1000, |x, y| {
            if !(80..360).contains(&x) || y % 27 >= 10 {
                return Rgba([255, 255, 255, 255]);
            }
            let mut value = ((x / 7).wrapping_mul(73856093)
                ^ (y / 27).wrapping_mul(19349663)
                ^ (x % 7).wrapping_mul(83492791)
                ^ (y % 27).wrapping_mul(2654435761))
            .wrapping_mul(2246822519);
            value ^= value >> 13;
            let shade = if value % 3 == 0 { 45 } else { 255 };
            Rgba([shade, shade, shade, 255])
        })
    }

    #[test]
    fn small_text_scroll_survives_minor_redraw_in_both_axes_and_directions() {
        let doc = text_document();
        let first = image::imageops::crop_imm(&doc, 0, 150, 480, 320).to_image();
        for shift in [-9, -3, -1, 1, 3, 9, 27] {
            let mut next =
                image::imageops::crop_imm(&doc, 0, (150 + shift) as u32, 480, 320).to_image();
            for pixel in next.pixels_mut() {
                if pixel[0] < 240 {
                    for channel in &mut pixel.0[..3] {
                        *channel += 2;
                    }
                }
            }
            // 先锁定旧实现的失败前提：微小重绘使精确哈希没有候选。
            assert_eq!(
                align_axis_exact(&first, &next, Content::full(&first), true, false),
                Err(false)
            );
            for horizontal in [false, true] {
                let (a, b, mode) = if horizontal {
                    (
                        image::imageops::rotate90(&first),
                        image::imageops::rotate90(&next),
                        ScrollMode::Horizontal,
                    )
                } else {
                    (first.clone(), next.clone(), ScrollMode::Vertical)
                };
                let Match::Aligned(result) = alignment(&a, &b, mode) else {
                    panic!("小幅文字滚动未匹配：{shift}, horizontal={horizontal}");
                };
                assert_eq!(
                    (result.dx, result.dy),
                    if horizontal { (-shift, 0) } else { (0, shift) }
                );
            }
        }
    }

    #[test]
    fn tolerant_search_rejects_repeated_textures_blank_pages_and_wrong_axes() {
        let repeat = RgbaImage::from_fn(160, 360, |x, y| {
            Rgba([((x % 20) * 12) as u8, ((y % 20) * 12) as u8, 0, 255])
        });
        let first = image::imageops::crop_imm(&repeat, 0, 40, 160, 160).to_image();
        let next = image::imageops::crop_imm(&repeat, 0, 45, 160, 160).to_image();
        assert_eq!(
            align_axis_tolerant(&first, &next, Content::full(&first), true),
            Match::Uncertain
        );
        assert_eq!(
            alignment(
                &first,
                &RgbaImage::from_pixel(160, 160, Rgba([255; 4])),
                ScrollMode::Vertical
            ),
            Match::Uncertain
        );
        let doc = text_document();
        let first = image::imageops::crop_imm(&doc, 0, 150, 480, 320).to_image();
        let next = image::imageops::crop_imm(&doc, 0, 159, 480, 320).to_image();
        assert_eq!(
            alignment(&first, &next, ScrollMode::Horizontal),
            Match::Uncertain
        );
    }
    #[test]
    fn single_axes_match_exactly_and_diagonal_motion_is_rejected() {
        let doc = RgbaImage::from_fn(600, 600, |x, y| {
            let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
            Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
        });
        let frame = |x, y| image::imageops::crop_imm(&doc, x, y, 160, 160).to_image();
        let first = frame(200, 200);
        for (dx, dy, mode) in [
            (1, 0, ScrollMode::Horizontal),
            (-128, 0, ScrollMode::Horizontal),
            (0, 1, ScrollMode::Vertical),
            (0, 128, ScrollMode::Vertical),
        ] {
            let next = frame((200 + dx) as u32, (200 + dy) as u32);
            let Match::Aligned(result) = alignment(&first, &next, mode) else {
                panic!("未识别单轴位移");
            };
            assert_eq!((result.dx, result.dy), (dx, dy));
            assert_eq!(
                alignment(&first, &next, ScrollMode::Auto),
                Match::Aligned(result)
            );
            assert!(verify_at(&first, &next, dx + 2, dy + 2, Content::full(&first)).is_none());
        }
        assert_eq!(
            alignment(&first, &frame(240, 240), ScrollMode::Auto),
            Match::Uncertain
        );
        assert_eq!(
            alignment(&first, &frame(240, 200), ScrollMode::Vertical),
            Match::Uncertain
        );
    }
}
