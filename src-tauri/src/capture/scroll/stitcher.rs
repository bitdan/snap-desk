//! 单轴滚动的分块画布：256² 图块、覆盖位、负坐标和有界历史锚点。
//! 快照共享 Arc，只有新增像素所在图块执行写时复制；预览直接取样图块。
use super::matcher::{
    alignment_in, stationary_overlay_conflict, verify_at, Alignment, Bounds, Content, Match,
    ScrollMode,
};
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
#[path = "benchmarks.rs"]
mod benchmarks;
#[cfg(test)]
#[path = "regression_tests.rs"]
mod regression_tests;
#[cfg(test)]
#[path = "mosaic_tests.rs"]
mod tests;

#[derive(Clone)]
struct Tile {
    pixels: Vec<[u8; 4]>,
    covered: Vec<u64>,
    filled: usize,
}
impl Tile {
    fn new() -> Self {
        Self {
            pixels: vec![[0, 0, 0, 0]; TILE_PIXELS],
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
    // 只记录判定阶段，不记录截图内容；失败时可区分定位、固定栏、遮挡和接缝。
    rejection: &'static str,
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
    /// 只允许从自动状态进入单轴，之后反向滚动可回访，但不能换轴产生混合画布。
    pub(super) fn lock_axis(&mut self, mode: ScrollMode) {
        if self.mode == ScrollMode::Auto && mode != ScrollMode::Auto {
            self.mode = mode;
            self.match_revision = self.match_revision.wrapping_add(1);
        }
    }
    /// 测试使用自动方向的简便入口；实际会话由首次滚轮输入锁定方向。
    #[cfg(test)]
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
            rejection: "none",
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
    /// 每轮进入恢复状态时由 Recovery 调用一次，避免同一失败帧刷屏。
    pub(super) fn log_rejection(&self) {
        log::info!(
            "event=scroll.rejected module=scroll reason={} axis={:?} frames={}",
            self.rejection,
            self.mode,
            self.frames
        );
    }
    pub(super) fn rejection(&self) -> &'static str {
        self.rejection
    }
    /// 可信像素数用于区分定位成功、回访和画布增长，不能用匹配次数代替。
    pub(super) fn covered_pixels(&self) -> u64 {
        self.covered
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
        // 补采的最后一帧也是可信端点。若直接覆盖 latest，它既不在缓存中，
        // 也从未作为上一帧入锚；随后只与该端点重叠的新画面就无法定位。
        // 无补采时无需留存当前帧，避免静止取样额外挤占历史预算。
        if self.latest.x != observed.latest.x || self.latest.y != observed.latest.y {
            self.remember_latest();
        }
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
            _ => "none",
        }
    }

    /// 测试只关心匹配结果；实际恢复流程还需 consider_recoverable 返回的缓存许可。
    #[cfg(test)]
    pub fn consider(&mut self, image: impl Into<Arc<RgbaImage>>) -> Result<Match, String> {
        self.consider_recoverable(image).map(|(result, _)| result)
    }

    /// 恢复层区分“暂时缺少重叠”与已检测到的布局、遮挡或画布冲突。
    /// 后者不能缓存重放，否则新视口可能失去原有冲突证据，把问题帧补进画布。
    pub(super) fn consider_recoverable(
        &mut self,
        image: impl Into<Arc<RgbaImage>>,
    ) -> Result<(Match, bool), String> {
        let image = image.into();
        let mut cacheable = true;
        let before = self.covered;
        let result = self.consider_impl(image, &mut cacheable)?;
        let (dx, dy, confidence) = match result {
            Match::Aligned(a) => (a.dx, a.dy, a.confidence),
            _ => (0, 0, if result == Match::Unchanged { 1.0 } else { 0.0 }),
        };
        log::debug!("event=scroll.alignment module=scroll result={:?} reason={} dx={} dy={} confidence={:.4} growth_pixels={} cacheable={}",
            result, self.rejection, dx, dy, confidence, self.covered.saturating_sub(before), cacheable);
        Ok((result, cacheable))
    }

    fn consider_impl(
        &mut self,
        image: Arc<RgbaImage>,
        cacheable: &mut bool,
    ) -> Result<Match, String> {
        self.rejection = "none";
        // 各个早退路径使用稳定原因码；对外结果和可信画布的处理保持一致。
        macro_rules! reject {
            ($reason:literal) => {{
                self.rejection = $reason;
                return Ok(Match::Uncertain);
            }};
        }
        if image.dimensions() != self.last().dimensions() {
            return Err("采集区域尺寸发生变化，请重新开始滚动截图。".into());
        }
        let direct = alignment_in(self.last(), &image, self.mode, self.content);
        if direct == Match::Unchanged {
            return Ok(direct);
        }
        // 新发现的固定栏是布局冲突，不能当作单纯缺少重叠缓存起来。
        // 必须在历史锚点复核之前拒绝，否则旧锚点可能用较宽的正文区覆盖此证据。
        if self.frames > 1 && matches!(direct, Match::Aligned(a) if a.content != self.content) {
            *cacheable = false;
            reject!("fixed_bounds_changed");
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
        // 已直接定位时先验证已知位移；验证失败才重新搜索，保留位置冲突检测。
        // 必须复核每个重叠锚点：较新锚点通过不能消除较早锚点的坐标反证。
        // 历史帧已受六帧/像素预算限制，不引入无界搜索。
        for anchor in self.anchors.iter().rev() {
            if let Some((x, y, _)) = located {
                // 完全相同的历史帧仍按原坐标重访处理，不能让周期纹理的偏移复核掩盖冲突。
                // 相同画面本身就提供坐标反证，必须先于几何重叠过滤；否则候选
                // 错移整屏后，恰好会因“不重叠”跳过最直接的重复画面证据。
                if anchor.image.as_ref() == image.as_ref() {
                    if x != anchor.x || y != anchor.y {
                        // 同一画面出现两个文档坐标，属于定位歧义而非缺少重叠。
                        // 锚点淘汰后重放会丢失反证，因此恢复层必须清空候选。
                        *cacheable = false;
                        reject!("anchor_position_conflict");
                    }
                    continue;
                }
                if (x - anchor.x).abs() >= i64::from(image.width())
                    || (y - anchor.y).abs() >= i64::from(image.height())
                {
                    continue;
                }
                if let Some(verified) = verify_at(
                    &anchor.image,
                    &image,
                    (x - anchor.x) as i32,
                    (y - anchor.y) as i32,
                    self.content,
                ) {
                    // 快速复核也会检测固定栏，不能只取“匹配成功”而丢掉边界。
                    if self.frames > 1 && verified.content != self.content {
                        *cacheable = false;
                        reject!("anchor_fixed_bounds_changed");
                    }
                    continue;
                }
            }
            let candidate = match alignment_in(&anchor.image, &image, self.mode, self.content) {
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
                    if self.frames > 1 && a.content != self.content {
                        *cacheable = false;
                        reject!("anchor_fixed_bounds_changed");
                    }
                    Some((anchor.x + i64::from(a.dx), anchor.y + i64::from(a.dy), a))
                }
                _ => None,
            };
            if let Some(next) = candidate {
                if located.is_some_and(|p| p.0 != next.0 || p.1 != next.1) {
                    // 搜索得到的历史位置也必须遵守相同的禁止缓存规则。
                    *cacheable = false;
                    reject!("anchor_position_conflict");
                }
                located = Some(next);
            }
        }
        let Some((x, y, mut result)) = located else {
            reject!("alignment_missing");
        };
        // 后续固定边界发生变化时不回溯篡改已提交内容，等待用户返回相同布局。
        if self.frames > 1 && result.content != self.content {
            *cacheable = false;
            reject!("fixed_bounds_changed");
        }
        result.dx = i32::try_from(x - self.latest.x).map_err(|_| "滚动位移超限。")?;
        result.dy = i32::try_from(y - self.latest.y).map_err(|_| "滚动位移超限。")?;
        if (self.mode == ScrollMode::Vertical && result.dx != 0)
            || (self.mode == ScrollMode::Horizontal && result.dy != 0)
        {
            reject!("axis_changed");
        }
        // 最终位移必须同时满足最新视口的屏幕固定纹理检查。放在统一提交入口，
        // 避免历史锚点搜索或已知位移快速复核绕过它；拒绝时尚未修改任何状态。
        if stationary_overlay_conflict(self.last(), &image, result.content, result.dx, result.dy) {
            *cacheable = false;
            reject!("stationary_overlay");
        }
        let next = Anchor { image, x, y };
        // 定位正确不等于新增像素可信：跨接缝的动画可能只占整帧 1%，
        // 通过全局九成一致门槛，却把另一半动画永久写入未覆盖区。
        if self.seam_conflict(&next, result.content) {
            *cacheable = false;
            reject!("seam_conflict");
        }
        if !self.consistent(&next, result.content) {
            // 已定位却与旧画布冲突的帧不是可信的桥接材料；禁止在后续
            // 覆盖率变化后借全局多数票重新放行，避免把问题帧的新增区写入成图。
            *cacheable = false;
            reject!("canvas_conflict");
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
        self.remember_latest();
        self.latest = next;
        if result.dx != 0 || result.dy != 0 {
            self.lock_axis(if result.dx != 0 {
                ScrollMode::Horizontal
            } else {
                ScrollMode::Vertical
            });
        }
        self.dx = result.dx;
        self.dy = result.dy;
        self.match_revision = self.match_revision.wrapping_add(1);
        Ok(Match::Aligned(result))
    }

    /// 提交新帧和恢复真实视口共用同一锚点预算，避免补采终点失去定位证据。
    /// 只共享图像 Arc；相同文档坐标不重复保留，仍由原有空间冗余策略淘汰。
    fn remember_latest(&mut self) {
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
    }

    /// 在覆盖区与新增区交界处复核局部一致性，同时适用于负坐标和二维空洞。
    /// 16×16 窗口每 8 像素滑动、内部每 4 像素取样；窗口相互重叠，
    /// 避免接缝恰好落在窗口边界而完全不被检查。只拒绝有多个旧像素
    /// 冲突且确实将写入新像素的窗口，不声称检测完全位于新增区的动画。
    fn seam_conflict(&self, next: &Anchor, content: Content) -> bool {
        let first = Self::frame_bounds(&self.latest, content);
        for y in (content.top..content.bottom).step_by(8) {
            for x in (content.left..content.right).step_by(8) {
                let mut known = 0;
                let mut novel = 0;
                let mut bad = 0;
                for py in (y..(y + 16).min(content.bottom)).step_by(4) {
                    for px in (x..(x + 16).min(content.right)).step_by(4) {
                        let gx = next.x + i64::from(px);
                        let gy = next.y + i64::from(py);
                        // 首次裁去固定栏时，只把即将保留的旧正文视作证据。
                        let retained = self.frames > 1
                            || (gx >= first.x
                                && gy >= first.y
                                && gx < first.x + i64::from(first.width)
                                && gy < first.y + i64::from(first.height));
                        let (key, index) = Self::address(gx, gy);
                        if let Some(tile) =
                            self.tiles.get(&key).filter(|t| retained && t.has(index))
                        {
                            known += 1;
                            let pixel = next.image.get_pixel(px, py);
                            let diff: u32 = (0..3)
                                .map(|c| u32::from(tile.pixels[index][c].abs_diff(pixel[c])))
                                .sum();
                            bad += u32::from(diff > 36);
                        } else {
                            novel += 1;
                        }
                    }
                }
                if known >= 4 && novel >= 4 && bad * 100 >= known * 75 {
                    return true;
                }
            }
        }
        false
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
                            tile.pixels[i] = anchor
                                .image
                                .get_pixel((x - anchor.x) as u32, (y - anchor.y) as u32)
                                .0;
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
            .map_or(Rgba([0, 0, 0, 0]), |t| Rgba(t.pixels[i]))
    }
    /// 空洞透明保留；导出背景选项在会话层统一应用，不污染占用掩码。
    pub fn compose(&self) -> RgbaImage {
        // 每个图块只查一次坐标，整行复制；避免导出时对每个像素做哈希查找。
        let mut image = RgbaImage::new(self.width, self.height);
        let output: &mut [u8] = &mut image;
        for (&(tx, ty), tile) in &self.tiles {
            let left = (tx * TILE).max(self.bounds.x);
            let top = (ty * TILE).max(self.bounds.y);
            let right = ((tx + 1) * TILE).min(self.bounds.x + i64::from(self.width));
            let bottom = ((ty + 1) * TILE).min(self.bounds.y + i64::from(self.height));
            for y in top..bottom {
                let source = ((y - ty * TILE) * TILE + left - tx * TILE) as usize;
                let target = ((y - self.bounds.y) * i64::from(self.width) + left - self.bounds.x)
                    as usize
                    * 4;
                let bytes = (right - left) as usize * 4;
                let row = &tile.pixels[source..source + bytes / 4];
                output[target..target + bytes].copy_from_slice(row.as_flattened());
                // 部分图块以覆盖位为准；按 64 位跳过完整区，仅清空实际缺失像素。
                // 正常新增条带大多整字覆盖，不会退回逐像素查找整张图。
                if tile.filled < TILE_PIXELS {
                    let mut index = source;
                    let end = source + row.len();
                    while index < end {
                        let count = (64 - index % 64).min(end - index);
                        let mask = u64::MAX >> (64 - count);
                        let mut missing = !(tile.covered[index / 64] >> (index % 64)) & mask;
                        while missing != 0 {
                            let offset = index - source + missing.trailing_zeros() as usize;
                            output[target + offset * 4..target + offset * 4 + 4].fill(0);
                            missing &= missing - 1;
                        }
                        index += count;
                    }
                }
            }
        }
        image
    }
    /// 检查半开文档矩形是否全部采集过；使用覆盖位而非像素透明度。
    /// 按图块及每行的 64 位掩码检查，避免为缩略图合成整张原尺寸画布。
    fn rectangle_covered(&self, left: i64, top: i64, right: i64, bottom: i64) -> bool {
        for ty in top.div_euclid(TILE)..=(bottom - 1).div_euclid(TILE) {
            for tx in left.div_euclid(TILE)..=(right - 1).div_euclid(TILE) {
                let Some(tile) = self.tiles.get(&(tx, ty)) else {
                    return false;
                };
                if tile.filled == TILE_PIXELS {
                    continue;
                }
                let start_x = left.max(tx * TILE) - tx * TILE;
                let end_x = right.min((tx + 1) * TILE) - tx * TILE;
                for y in top.max(ty * TILE)..bottom.min((ty + 1) * TILE) {
                    let row = ((y - ty * TILE) * TILE) as usize;
                    let mut i = row + start_x as usize;
                    let end = row + end_x as usize;
                    while i < end {
                        let count = (64 - i % 64).min(end - i);
                        // 右移构造掩码可覆盖完整 64 位，避免 1 << 64 溢出。
                        let mask = (u64::MAX >> (64 - count)) << (i % 64);
                        if tile.covered[i / 64] & mask != mask {
                            return false;
                        }
                        i += count;
                    }
                }
            }
        }
        true
    }

    /// 一个缩略像素对应的原图区域只要包含空洞，就透明展示棋盘格。
    /// 这会放大细小空洞以提示补采；不修改导出像素或真实覆盖率。
    pub fn preview(&self, max_width: u32, max_height: u32) -> RgbaImage {
        let scale = (f64::from(max_width) / f64::from(self.width))
            .min(f64::from(max_height) / f64::from(self.height))
            .min(1.0);
        let width = (f64::from(self.width) * scale).floor().max(1.0) as u32;
        let height = (f64::from(self.height) * scale).floor().max(1.0) as u32;
        let holes = self.has_holes();
        let source_x =
            |x| self.bounds.x + (u64::from(x) * u64::from(self.width) / u64::from(width)) as i64;
        let source_y =
            |y| self.bounds.y + (u64::from(y) * u64::from(self.height) / u64::from(height)) as i64;
        RgbaImage::from_fn(width, height, |x, y| {
            let left = source_x(x);
            let top = source_y(y);
            if holes && !self.rectangle_covered(left, top, source_x(x + 1), source_y(y + 1)) {
                Rgba([0, 0, 0, 0])
            } else {
                self.pixel(left, top)
            }
        })
    }
}
