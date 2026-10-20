//! 稀疏二维画布：256² 图块、逐像素覆盖位、负坐标和有界空间锚点。
//! 快照共享 Arc，只有新增像素所在图块执行写时复制；预览直接取样图块。
use super::matcher::{alignment, verify_at, Alignment, Bounds, Content, Match, ScrollMode};
use image::{Rgba, RgbaImage};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

pub const MAX_PIXELS: u64 = 32_000_000;
pub const MAX_HEIGHT: u32 = 60_000;
const TILE: i64 = 256;
const TILE_PIXELS: usize = 256 * 256;
const MAX_TILES: usize = 512;
const ANCHOR_PIXELS: u64 = 8_000_000;

#[cfg(test)]
#[path = "regression_tests.rs"]
mod regression_tests;
#[cfg(test)]
#[path = "mosaic_tests.rs"]
mod tests;

#[derive(Clone)]
struct Tile {
    pixels: Vec<Rgba<u8>>,
    covered: Vec<u64>,
    filled: usize,
}
impl Tile {
    fn new() -> Self {
        Self {
            pixels: vec![Rgba([0, 0, 0, 0]); TILE_PIXELS],
            covered: vec![0; TILE_PIXELS / 64],
            filled: 0,
        }
    }
    fn has(&self, i: usize) -> bool {
        self.covered[i / 64] & (1u64 << (i % 64)) != 0
    }
}
#[derive(Clone)]
struct Anchor {
    image: Arc<RgbaImage>,
    x: i64,
    y: i64,
}

#[derive(Clone)]
pub struct Stitch {
    tiles: HashMap<(i64, i64), Arc<Tile>>,
    anchors: VecDeque<Anchor>,
    latest: Anchor,
    content: Content,
    mode: ScrollMode,
    frames: usize,
    covered: u64,
    // 可信位置或画布变化时递增，用于使失败帧匹配缓存失效。
    match_revision: u64,
    pub bounds: Bounds,
    pub width: u32,
    pub height: u32,
    pub frame_height: u32,
    pub dx: i32,
    pub dy: i32,
}

impl Stitch {
    pub fn new(image: RgbaImage) -> Result<Self, String> {
        Self::with_mode(image, ScrollMode::Auto)
    }
    pub fn with_mode(image: RgbaImage, mode: ScrollMode) -> Result<Self, String> {
        let (width, height) = image.dimensions();
        if width < 32
            || height < 64
            || width > 8192
            || height > 8192
            || u64::from(width) * u64::from(height) > 8_000_000
        {
            return Err(
                "滚动截图选区须至少 32 × 64 像素，宽高不超过 8192，单帧不超过 800 万像素。".into(),
            );
        }
        let content = Content::full(&image);
        let mut stitch = Self {
            tiles: HashMap::new(),
            anchors: VecDeque::new(),
            latest: Anchor {
                image: Arc::new(image),
                x: 0,
                y: 0,
            },
            content,
            mode,
            frames: 1,
            covered: 0,
            match_revision: 0,
            bounds: Bounds {
                x: 0,
                y: 0,
                width,
                height,
            },
            width,
            height,
            frame_height: height,
            dx: 0,
            dy: 0,
        };
        stitch.write(stitch.latest.clone(), content)?;
        Ok(stitch)
    }
    pub fn count(&self) -> usize {
        self.frames
    }
    pub(super) fn match_revision(&self) -> u64 {
        self.match_revision
    }
    pub fn last(&self) -> &RgbaImage {
        &self.latest.image
    }
    pub fn last_shared(&self) -> Arc<RgbaImage> {
        self.latest.image.clone()
    }
    /// 恢复缓存可以补图，但不能把历史帧误显示为用户当前看到的视口。
    pub(super) fn restore_viewport(&mut self, observed: &Self) {
        self.latest = observed.latest.clone();
        self.dx = observed.dx;
        self.dy = observed.dy;
        self.match_revision = self.match_revision.wrapping_add(1);
    }
    /// 固定栏不作为文档内容导出，视口也仅标注实际参与拼接的内容区。
    pub fn viewport(&self) -> Bounds {
        Bounds {
            x: self.latest.x + i64::from(self.content.left),
            y: self.latest.y + i64::from(self.content.top),
            width: self.content.right - self.content.left,
            height: self.content.bottom - self.content.top,
        }
    }
    pub fn coverage(&self) -> f64 {
        self.covered as f64 / (u64::from(self.width) * u64::from(self.height)) as f64 * 100.0
    }
    pub fn has_holes(&self) -> bool {
        self.covered < u64::from(self.width) * u64::from(self.height)
    }
    pub fn direction(&self) -> &'static str {
        match (self.dx.signum(), self.dy.signum()) {
            (0, 0) => "none",
            (0, -1) => "up",
            (0, 1) => "down",
            (-1, 0) => "left",
            (1, 0) => "right",
            _ => "diagonal",
        }
    }

    pub fn consider(&mut self, image: impl Into<Arc<RgbaImage>>) -> Result<Match, String> {
        let image = image.into();
        if image.dimensions() != self.last().dimensions() {
            return Err("采集区域尺寸发生变化，请重新开始滚动截图。".into());
        }
        let direct = alignment(self.last(), &image, self.mode);
        if direct == Match::Unchanged {
            return Ok(direct);
        }
        let mut located = match direct {
            Match::Aligned(a) => Some((
                self.latest.x + i64::from(a.dx),
                self.latest.y + i64::from(a.dy),
                a,
            )),
            _ => None,
        };
        // 历史锚点带文档坐标。重访只移动视口，不按时间顺序追加旧候选。
        // 已直接定位时只验证已知位移；验证失败才重新搜索，保留位置冲突检测。
        let mut checked = false;
        for anchor in self.anchors.iter().rev() {
            if let Some((x, y, _)) = located {
                if checked
                    || (x - anchor.x).abs() >= i64::from(image.width())
                    || (y - anchor.y).abs() >= i64::from(image.height())
                {
                    continue;
                }
                // 完全相同的历史帧仍按原坐标重访处理，不能让周期纹理的偏移复核掩盖冲突。
                if anchor.image.as_ref() == image.as_ref() {
                    if x != anchor.x || y != anchor.y {
                        return Ok(Match::Uncertain);
                    }
                    checked = true;
                    continue;
                }
                if verify_at(
                    &anchor.image,
                    &image,
                    (x - anchor.x) as i32,
                    (y - anchor.y) as i32,
                )
                .is_some()
                {
                    checked = true;
                    continue;
                }
            }
            let candidate = match alignment(&anchor.image, &image, self.mode) {
                Match::Unchanged => Some((
                    anchor.x,
                    anchor.y,
                    Alignment {
                        dx: 0,
                        dy: 0,
                        overlap: Bounds {
                            x: 0,
                            y: 0,
                            width: image.width(),
                            height: image.height(),
                        },
                        confidence: 1.0,
                        content: self.content,
                    },
                )),
                Match::Aligned(a) => {
                    Some((anchor.x + i64::from(a.dx), anchor.y + i64::from(a.dy), a))
                }
                _ => None,
            };
            checked = true;
            if let Some(next) = candidate {
                if located.is_some_and(|p| p.0 != next.0 || p.1 != next.1) {
                    return Ok(Match::Uncertain);
                }
                located = Some(next);
            }
        }
        let Some((x, y, mut result)) = located else {
            return Ok(Match::Uncertain);
        };
        // 后续固定边界发生变化时不回溯篡改已提交内容，等待用户返回相同布局。
        if self.frames > 1 && result.content != self.content {
            return Ok(Match::Uncertain);
        }
        result.dx = i32::try_from(x - self.latest.x).map_err(|_| "滚动位移超限。")?;
        result.dy = i32::try_from(y - self.latest.y).map_err(|_| "滚动位移超限。")?;
        if (self.mode == ScrollMode::Vertical && result.dx != 0)
            || (self.mode == ScrollMode::Horizontal && result.dy != 0)
        {
            return Ok(Match::Uncertain);
        }
        let next = Anchor { image, x, y };
        if !self.consistent(&next, result.content) {
            return Ok(Match::Uncertain);
        }
        // 所有可能失败的容量检查先完成；失败不改变画布、锚点或视口。
        self.check_capacity(&next, result.content)?;
        if self.frames == 1 && result.content != self.content {
            self.tiles.clear();
            self.covered = 0;
            self.content = result.content;
            self.bounds = Self::frame_bounds(&self.latest, self.content);
            self.width = self.bounds.width;
            self.height = self.bounds.height;
            self.write(self.latest.clone(), self.content)?;
        }
        let before = self.covered;
        self.write(next.clone(), self.content)?;
        if self.covered > before {
            self.frames += 1;
        }
        // 除最新帧外最多保留六个锚点且总计不超过八百万像素。
        if !self
            .anchors
            .iter()
            .any(|a| a.x == self.latest.x && a.y == self.latest.y)
        {
            self.anchors.push_back(self.latest.clone());
        }
        while self.anchors.len() > 6
            || self
                .anchors
                .iter()
                .map(|a| u64::from(a.image.width()) * u64::from(a.image.height()))
                .sum::<u64>()
                > ANCHOR_PIXELS
        {
            // 优先淘汰空间上最冗余的中间锚点，保留起点和最近端点；
            // 大图仅容纳一个历史帧时优先保留最近帧，仍遵守像素预算。
            let remove = if self.anchors.len() <= 2 {
                0
            } else {
                (1..self.anchors.len() - 1)
                    .min_by_key(|&i| {
                        let a = &self.anchors[i];
                        self.anchors
                            .iter()
                            .enumerate()
                            .filter(|(j, _)| *j != i)
                            .map(|(_, b)| (a.x - b.x).abs() + (a.y - b.y).abs())
                            .min()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0)
            };
            self.anchors.remove(remove);
        }
        self.latest = next;
        self.dx = result.dx;
        self.dy = result.dy;
        self.match_revision = self.match_revision.wrapping_add(1);
        Ok(Match::Aligned(result))
    }

    /// 与已提交覆盖区再做空间一致性检查，防止候选只对最新帧成立却冲突于旧画布。
    fn consistent(&self, next: &Anchor, content: Content) -> bool {
        let mut total = 0u32;
        let mut good = 0u32;
        let step_x = ((content.right - content.left) / 64).max(1) as usize;
        let step_y = ((content.bottom - content.top) / 64).max(1) as usize;
        for y in (content.top..content.bottom).step_by(step_y) {
            for x in (content.left..content.right).step_by(step_x) {
                let gx = next.x + i64::from(x);
                let gy = next.y + i64::from(y);
                // 首次识别到固定栏时，这些旧像素即将被剔除，不能投反对票。
                if self.frames == 1 {
                    let first = Self::frame_bounds(&self.latest, content);
                    if gx < first.x
                        || gy < first.y
                        || gx >= first.x + i64::from(first.width)
                        || gy >= first.y + i64::from(first.height)
                    {
                        continue;
                    }
                }
                let (key, index) = Self::address(gx, gy);
                if let Some(tile) = self.tiles.get(&key).filter(|t| t.has(index)) {
                    let pixel = next.image.get_pixel(x, y);
                    let diff: u32 = (0..3)
                        .map(|c| u32::from(tile.pixels[index][c].abs_diff(pixel[c])))
                        .sum();
                    total += 1;
                    good += u32::from(diff <= 36);
                }
            }
        }
        total == 0 || good * 100 >= total * 90
    }

    fn frame_bounds(anchor: &Anchor, content: Content) -> Bounds {
        Bounds {
            x: anchor.x + i64::from(content.left),
            y: anchor.y + i64::from(content.top),
            width: content.right - content.left,
            height: content.bottom - content.top,
        }
    }
    fn union(&self, next: Bounds) -> Bounds {
        let x = self.bounds.x.min(next.x);
        let y = self.bounds.y.min(next.y);
        Bounds {
            x,
            y,
            width: ((self.bounds.x + i64::from(self.width)).max(next.x + i64::from(next.width)) - x)
                as u32,
            height: ((self.bounds.y + i64::from(self.height)).max(next.y + i64::from(next.height))
                - y) as u32,
        }
    }
    fn check_capacity(&self, anchor: &Anchor, content: Content) -> Result<Bounds, String> {
        let frame = Self::frame_bounds(anchor, content);
        let bounds = self.union(frame);
        let mut missing = 0;
        for ty in
            frame.y.div_euclid(TILE)..=(frame.y + i64::from(frame.height) - 1).div_euclid(TILE)
        {
            for tx in
                frame.x.div_euclid(TILE)..=(frame.x + i64::from(frame.width) - 1).div_euclid(TILE)
            {
                missing += usize::from(!self.tiles.contains_key(&(tx, ty)));
            }
        }
        if bounds.width > MAX_HEIGHT
            || bounds.height > MAX_HEIGHT
            || u64::from(bounds.width) * u64::from(bounds.height) > MAX_PIXELS
            || self.tiles.len() + missing > MAX_TILES
        {
            return Err("已达到滚动截图容量限制（边长 60000 / 包围盒 3200 万像素 / 512 图块），仅输出已确认部分。".into());
        }
        Ok(bounds)
    }
    fn address(x: i64, y: i64) -> ((i64, i64), usize) {
        (
            (x.div_euclid(TILE), y.div_euclid(TILE)),
            (y.rem_euclid(TILE) * TILE + x.rem_euclid(TILE)) as usize,
        )
    }
    fn write(&mut self, anchor: Anchor, content: Content) -> Result<(), String> {
        let bounds = self.check_capacity(&anchor, content)?;
        let frame = Self::frame_bounds(&anchor, content);
        let right = frame.x + i64::from(frame.width);
        let bottom = frame.y + i64::from(frame.height);
        // 一次查找一个图块，而非每像素查 HashMap；回滚经过已填满图块时直接跳过。
        for ty in frame.y.div_euclid(TILE)..=(bottom - 1).div_euclid(TILE) {
            for tx in frame.x.div_euclid(TILE)..=(right - 1).div_euclid(TILE) {
                let tile = self
                    .tiles
                    .entry((tx, ty))
                    .or_insert_with(|| Arc::new(Tile::new()));
                if tile.filled == TILE_PIXELS {
                    continue;
                }
                let left = frame.x.max(tx * TILE);
                let top = frame.y.max(ty * TILE);
                let end_x = right.min((tx + 1) * TILE);
                let end_y = bottom.min((ty + 1) * TILE);
                let index = |x: i64, y: i64| ((y - ty * TILE) * TILE + x - tx * TILE) as usize;
                // 部分图块也可能全是重访区域；有新像素才触发 Arc 写时复制。
                if !(top..end_y).any(|y| (left..end_x).any(|x| !tile.has(index(x, y)))) {
                    continue;
                }
                let tile = Arc::make_mut(tile);
                for y in top..end_y {
                    for x in left..end_x {
                        let i = index(x, y);
                        if !tile.has(i) {
                            tile.pixels[i] = *anchor
                                .image
                                .get_pixel((x - anchor.x) as u32, (y - anchor.y) as u32);
                            tile.covered[i / 64] |= 1u64 << (i % 64);
                            tile.filled += 1;
                            self.covered += 1;
                        }
                    }
                }
            }
        }
        self.bounds = bounds;
        self.width = bounds.width;
        self.height = bounds.height;
        Ok(())
    }
    fn pixel(&self, x: i64, y: i64) -> Rgba<u8> {
        let (key, i) = Self::address(x, y);
        self.tiles
            .get(&key)
            .filter(|t| t.has(i))
            .map_or(Rgba([0, 0, 0, 0]), |t| t.pixels[i])
    }
    /// 空洞透明保留；导出背景选项在会话层统一应用，不污染占用掩码。
    pub fn compose(&self) -> RgbaImage {
        RgbaImage::from_fn(self.width, self.height, |x, y| {
            self.pixel(self.bounds.x + i64::from(x), self.bounds.y + i64::from(y))
        })
    }
    pub fn preview(&self, max_width: u32, max_height: u32) -> RgbaImage {
        let scale = (f64::from(max_width) / f64::from(self.width))
            .min(f64::from(max_height) / f64::from(self.height))
            .min(1.0);
        let width = (f64::from(self.width) * scale).floor().max(1.0) as u32;
        let height = (f64::from(self.height) * scale).floor().max(1.0) as u32;
        RgbaImage::from_fn(width, height, |x, y| {
            self.pixel(
                self.bounds.x + (u64::from(x) * u64::from(self.width) / u64::from(width)) as i64,
                self.bounds.y + (u64::from(y) * u64::from(self.height) / u64::from(height)) as i64,
            )
        })
    }
}
