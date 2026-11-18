//! X11 没有“窗口不参与录屏”的属性：根窗口画面里总带着本应用的录制控制条、选区边框和标注画布。
//! Windows / macOS 由系统排除这些窗口，标注再由录制管线自行合成；这里在每帧取回后把它们去掉。
//!
//! 只在合成管理器运行时生效：此时每个顶层窗口都有独立的离屏内容，可以单独读取。
//! 1. 半透明像素（标注画布的空白、抗锯齿边缘）按预乘 alpha 反解出下方画面；
//! 2. 近乎不透明的像素用下方其他应用的窗口按叠放顺序重新合成；没有窗口覆盖的部分
//!    取根窗口壁纸像素图，桌面环境不提供时为黑色。
//!
//! 合成器自行绘制的阴影、动画和桌面背景不在任何窗口里，被遮住的那一小块无法还原。
//! 没有合成管理器时透明窗口本身也无法正常显示，此时保留原画面。
use super::{super::super::frame::BgraImage, Segment};
use crate::shared::x11::X11;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use xcb::{shm, x, XidNew};

/// 根窗口坐标下的矩形：左、上、宽、高。
pub(super) type Area = (i32, i32, u32, u32);

/// alpha 达到该值即视为不透明：反解时误差会被放大 255 / (255 − α) 倍，超过 8 倍就不可信。
const OPAQUE: u8 = 224;

#[derive(Clone, Copy)]
struct Toplevel {
    id: x::Window,
    rect: Area,
    /// 32 位深的窗口带 alpha（ARGB 视觉），24 位深的窗口不透明。
    alpha: bool,
    own: bool,
}

pub(super) struct Exclusion {
    pid: u32,
    wallpaper: Option<x::Atom>,
    composited: bool,
    checked: Option<Instant>,
    /// 顶层窗口是否属于本进程；窗口销毁后剔除，避免 ID 复用时沿用旧结论。
    own: HashMap<x::Window, bool>,
    segment: Option<Segment>,
}

impl Exclusion {
    pub(super) fn new(x11: &X11, len: usize) -> Self {
        Self {
            pid: std::process::id(),
            wallpaper: x11.existing("_XROOTPMAP_ID"),
            composited: false,
            checked: None,
            own: HashMap::new(),
            segment: Segment::attach(x11, len),
        }
    }

    pub(super) fn release(&mut self, x11: &X11) {
        if let Some(segment) = self.segment.take() {
            segment.detach(x11);
        }
    }

    /// 合成管理器可在录制中途启停，每秒确认一次。
    fn composited(&mut self, x11: &X11) -> bool {
        if self
            .checked
            .map_or(true, |at| at.elapsed() >= Duration::from_secs(1))
        {
            self.checked = Some(Instant::now());
            self.composited = x11.composited();
        }
        self.composited
    }

    /// 根窗口直接子窗口按叠放顺序自下而上排列，即窗口管理器的外框或未装饰的客户窗口。
    fn stack(&mut self, x11: &X11) -> Option<Vec<Toplevel>> {
        let tree = x11
            .conn
            .wait_for_reply(x11.conn.send_request(&x::QueryTree { window: x11.root }))
            .ok()?;
        let children = tree.children().to_vec();
        // 先发出全部请求再逐个等待，几十个顶层窗口也只占一次往返的延迟。
        let cookies: Vec<_> = children
            .iter()
            .map(|&window| {
                (
                    x11.conn.send_request(&x::GetWindowAttributes { window }),
                    x11.conn.send_request(&x::GetGeometry {
                        drawable: x::Drawable::Window(window),
                    }),
                )
            })
            .collect();
        let mut stack = Vec::new();
        for (&id, (attributes, geometry)) in children.iter().zip(cookies) {
            let (Ok(attributes), Ok(geometry)) = (
                x11.conn.wait_for_reply(attributes),
                x11.conn.wait_for_reply(geometry),
            ) else {
                continue;
            };
            if attributes.map_state() != x::MapState::Viewable
                || attributes.class() != x::WindowClass::InputOutput
            {
                continue;
            }
            // 读取窗口内容的坐标从边框内侧开始。
            let border = i32::from(geometry.border_width());
            stack.push(Toplevel {
                id,
                rect: (
                    i32::from(geometry.x()) + border,
                    i32::from(geometry.y()) + border,
                    u32::from(geometry.width()),
                    u32::from(geometry.height()),
                ),
                alpha: geometry.depth() == 32,
                own: false,
            });
        }
        self.own
            .retain(|id, _| stack.iter().any(|window| window.id == *id));
        for window in &mut stack {
            window.own = match self.own.get(&window.id) {
                Some(&own) => own,
                None => {
                    let own = owned(x11, window.id, self.pid, 2);
                    self.own.insert(window.id, own);
                    own
                }
            };
        }
        Some(stack)
    }

    /// `image` 覆盖根窗口区域 (origin, 图像尺寸)，`part` 为其中实际取到画面的部分。
    pub(super) fn apply(
        &mut self,
        x11: &X11,
        msb: bool,
        image: &mut BgraImage,
        origin: (i32, i32),
        part: Area,
    ) {
        if !self.composited(x11) {
            return;
        }
        let Some(stack) = self.stack(x11) else {
            return;
        };
        if !stack
            .iter()
            .any(|window| window.own && intersect(window.rect, part).is_some())
        {
            return;
        }
        let width = image.width as usize;
        // 已由更上层的本应用窗口还原为最终画面的像素，下层窗口不再处理。
        let mut done = vec![false; part.2 as usize * part.3 as usize];
        // 自上而下逐层剥离：画面 = 上层 ⊕（下层 ⊕ 桌面），先去掉上层才能再去掉下层。
        for index in (0..stack.len()).rev() {
            let window = stack[index];
            if !window.own {
                continue;
            }
            let Some(clip) = intersect(window.rect, part) else {
                continue;
            };
            let Some(content) = fetch(
                x11,
                self.segment.as_ref(),
                x::Drawable::Window(window.id),
                (
                    clip.0 - window.rect.0,
                    clip.1 - window.rect.1,
                    clip.2,
                    clip.3,
                ),
            ) else {
                continue;
            };
            // 盖在本应用窗口上的其他不透明窗口处，画面本来就是那个窗口，不需要还原。
            let above: Vec<Area> = stack[index + 1..]
                .iter()
                .filter(|other| !other.own && !other.alpha)
                .filter_map(|other| intersect(other.rect, clip))
                .collect();
            let mut opaque = Vec::new();
            for row in 0..clip.3 {
                for column in 0..clip.2 {
                    let (x, y) = (clip.0 + column as i32, clip.1 + row as i32);
                    let local = (y - part.1) as usize * part.2 as usize + (x - part.0) as usize;
                    if done[local] || above.iter().any(|area| contains(*area, x, y)) {
                        continue;
                    }
                    let offset = (row as usize * clip.2 as usize + column as usize) * 4;
                    let mut pixel = argb(&content[offset..offset + 4], msb);
                    if !window.alpha {
                        pixel[3] = 255;
                    }
                    if pixel[3] == 0 {
                        continue;
                    }
                    let at = ((y - origin.1) as usize * width + (x - origin.0) as usize) * 4;
                    if pixel[3] < OPAQUE {
                        unblend(&mut image.data[at..at + 3], pixel);
                    } else {
                        opaque.push((local, at, x, y));
                    }
                }
            }
            let Some(extent) = bounds(opaque.iter().map(|&(_, _, x, y)| (x, y))) else {
                continue;
            };
            let under = self.compose(x11, msb, &stack[..index], extent);
            for (local, at, x, y) in opaque {
                let i = ((y - extent.1) as usize * extent.2 as usize + (x - extent.0) as usize) * 3;
                image.data[at..at + 3].copy_from_slice(&under[i..i + 3]);
                done[local] = true;
            }
        }
    }

    /// 把 `below` 中其他应用的窗口按叠放顺序合成到 `area`，返回 BGR 像素。
    fn compose(&self, x11: &X11, msb: bool, below: &[Toplevel], area: Area) -> Vec<u8> {
        let mut output = vec![0u8; area.2 as usize * area.3 as usize * 3];
        // 完整盖住目标区域的最上层不透明窗口以下都看不见，从它开始合成。
        let start = below
            .iter()
            .rposition(|window| !window.own && !window.alpha && covers(window.rect, area));
        if start.is_none() {
            self.wallpaper(x11, msb, area, &mut output);
        }
        for window in &below[start.unwrap_or(0)..] {
            if window.own {
                continue;
            }
            let Some(clip) = intersect(window.rect, area) else {
                continue;
            };
            let Some(content) = fetch(
                x11,
                self.segment.as_ref(),
                x::Drawable::Window(window.id),
                (
                    clip.0 - window.rect.0,
                    clip.1 - window.rect.1,
                    clip.2,
                    clip.3,
                ),
            ) else {
                continue;
            };
            paint(&mut output, area, &content, clip, msb, window.alpha);
        }
        output
    }

    /// 壁纸程序按惯例把根窗口背景像素图写入 `_XROOTPMAP_ID`；尺寸不足或读取失败时保留黑色。
    fn wallpaper(&self, x11: &X11, msb: bool, area: Area, output: &mut [u8]) {
        let Some(property) = self.wallpaper else {
            return;
        };
        let Some(id) = x11.first_u32(x11.root, property, x::ATOM_PIXMAP) else {
            return;
        };
        let drawable = x::Drawable::Pixmap(x::Pixmap::new(id));
        let Ok(geometry) = x11
            .conn
            .wait_for_reply(x11.conn.send_request(&x::GetGeometry { drawable }))
        else {
            return;
        };
        let size = (0, 0, u32::from(geometry.width()), u32::from(geometry.height()));
        if geometry.depth() == 32 || !covers(size, area) {
            return;
        }
        if let Some(content) = fetch(x11, self.segment.as_ref(), drawable, area) {
            paint(output, area, &content, area, msb, false);
        }
    }
}

/// `_NET_WM_PID` 设在客户窗口上；装饰窗口由窗口管理器创建，需要向下查找被它包住的客户窗口。
fn owned(x11: &X11, window: x::Window, pid: u32, depth: u8) -> bool {
    if x11.window_pid(window) == Some(pid) {
        return true;
    }
    depth > 0
        && x11
            .conn
            .wait_for_reply(x11.conn.send_request(&x::QueryTree { window }))
            .is_ok_and(|tree| {
                tree.children()
                    .iter()
                    .any(|&child| owned(x11, child, pid, depth - 1))
            })
}

/// 读取可绘制对象内一块 32 位 ZPixmap 像素；窗口须完全在屏幕内，调用方已按根窗口裁剪。
fn fetch(
    x11: &X11,
    segment: Option<&Segment>,
    drawable: x::Drawable,
    (x, y, width, height): Area,
) -> Option<Vec<u8>> {
    let bytes = width as usize * height as usize * 4;
    if bytes == 0 {
        return None;
    }
    let (x, y, width, height) = (x as i16, y as i16, width as u16, height as u16);
    if let Some(segment) = segment.filter(|segment| segment.len >= bytes) {
        let reply = x11.conn.wait_for_reply(x11.conn.send_request(&shm::GetImage {
            drawable,
            x,
            y,
            width,
            height,
            plane_mask: u32::MAX,
            format: x::ImageFormat::ZPixmap as u8,
            shmseg: segment.seg,
            offset: 0,
        }));
        if reply.is_ok() {
            // 回复到达时服务器已写完整块像素；同一线程内下一次请求前复制出来。
            return Some(unsafe { std::slice::from_raw_parts(segment.address, bytes) }.to_vec());
        }
    }
    let reply = x11
        .conn
        .wait_for_reply(x11.conn.send_request(&x::GetImage {
            format: x::ImageFormat::ZPixmap,
            drawable,
            x,
            y,
            width,
            height,
            plane_mask: u32::MAX,
        }))
        .ok()?;
    reply.data().get(..bytes).map(<[u8]>::to_vec)
}

/// 32 位 ZPixmap 像素（LSB 为 BGRA，MSB 为 ARGB）转为 [B, G, R, A]。
fn argb(pixel: &[u8], msb: bool) -> [u8; 4] {
    if msb {
        [pixel[3], pixel[2], pixel[1], pixel[0]]
    } else {
        [pixel[0], pixel[1], pixel[2], pixel[3]]
    }
}

/// 画面 = 窗口（预乘）+ 下方 ×（1 − α），反解出下方像素。
fn unblend(target: &mut [u8], pixel: [u8; 4]) {
    let rest = 255 - u32::from(pixel[3]);
    for (channel, own) in target.iter_mut().zip(pixel) {
        let value = u32::from(channel.saturating_sub(own)) * 255 / rest;
        *channel = value.min(255) as u8;
    }
}

/// 预乘 alpha 叠加：目标 = 源 + 目标 ×（1 − α）。
fn over(target: &mut [u8], pixel: [u8; 4]) {
    let rest = 255 - u32::from(pixel[3]);
    for (channel, source) in target.iter_mut().zip(pixel) {
        let value = u32::from(source) + u32::from(*channel) * rest / 255;
        *channel = value.min(255) as u8;
    }
}

/// 把一块窗口像素（根坐标 `clip`）画进 `area` 对应的 BGR 缓冲。
fn paint(output: &mut [u8], area: Area, content: &[u8], clip: Area, msb: bool, alpha: bool) {
    for row in 0..clip.3 as usize {
        for column in 0..clip.2 as usize {
            let offset = (row * clip.2 as usize + column) * 4;
            let pixel = argb(&content[offset..offset + 4], msb);
            let x = (clip.0 - area.0) as usize + column;
            let y = (clip.1 - area.1) as usize + row;
            let i = (y * area.2 as usize + x) * 3;
            if alpha {
                over(&mut output[i..i + 3], pixel);
            } else {
                output[i..i + 3].copy_from_slice(&pixel[..3]);
            }
        }
    }
}

fn intersect(a: Area, b: Area) -> Option<Area> {
    let left = a.0.max(b.0);
    let top = a.1.max(b.1);
    let right = (i64::from(a.0) + i64::from(a.2)).min(i64::from(b.0) + i64::from(b.2));
    let bottom = (i64::from(a.1) + i64::from(a.3)).min(i64::from(b.1) + i64::from(b.3));
    (right > i64::from(left) && bottom > i64::from(top)).then(|| {
        (
            left,
            top,
            (right - i64::from(left)) as u32,
            (bottom - i64::from(top)) as u32,
        )
    })
}

fn covers(outer: Area, inner: Area) -> bool {
    intersect(outer, inner) == Some(inner)
}

fn contains(area: Area, x: i32, y: i32) -> bool {
    covers(area, (x, y, 1, 1))
}

fn bounds(points: impl Iterator<Item = (i32, i32)>) -> Option<Area> {
    let mut result: Option<(i32, i32, i32, i32)> = None;
    for (x, y) in points {
        result = Some(match result {
            None => (x, y, x, y),
            Some((left, top, right, bottom)) => {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }
        });
    }
    result.map(|(left, top, right, bottom)| {
        (
            left,
            top,
            (right - left + 1) as u32,
            (bottom - top + 1) as u32,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unblend_reverses_premultiplied_over() {
        for alpha in [1u8, 64, 128, 200, OPAQUE - 1] {
            let under = [10u8, 120, 240];
            // 半透明红色，预乘后 R = alpha。
            let pixel = [0, 0, alpha, alpha];
            let mut screen = under;
            over(&mut screen, pixel);
            unblend(&mut screen, pixel);
            for (restored, original) in screen.iter().zip(under) {
                // 8 位量化下，反解误差不超过放大倍数。
                let tolerance = 255 / (255 - u32::from(alpha)) + 1;
                assert!(u32::from(restored.abs_diff(original)) <= tolerance, "{alpha}");
            }
        }
    }

    #[test]
    fn both_byte_orders_read_alpha() {
        assert_eq!(argb(&[1, 2, 3, 4], false), [1, 2, 3, 4]);
        assert_eq!(argb(&[4, 3, 2, 1], true), [1, 2, 3, 4]);
    }

    #[test]
    fn rectangles_intersect_cover_and_bound() {
        assert_eq!(
            intersect((0, 0, 10, 10), (5, -5, 10, 10)),
            Some((5, 0, 5, 5))
        );
        assert_eq!(intersect((0, 0, 10, 10), (10, 0, 5, 5)), None);
        assert!(covers((0, 0, 10, 10), (2, 2, 8, 8)));
        assert!(!covers((0, 0, 10, 10), (2, 2, 9, 8)));
        assert!(contains((0, 0, 1, 1), 0, 0) && !contains((0, 0, 1, 1), 1, 0));
        assert_eq!(
            bounds([(3, 4), (1, 9), (5, 6)].into_iter()),
            Some((1, 4, 5, 6))
        );
        assert_eq!(bounds(std::iter::empty()), None);
    }

    #[test]
    fn paint_copies_opaque_and_blends_alpha_windows() {
        let area = (10, 10, 2, 1);
        let mut output = vec![100u8; 6];
        // 不透明窗口只覆盖右侧像素，alpha 字节无意义也不参与。
        paint(&mut output, area, &[1, 2, 3, 0], (11, 10, 1, 1), false, false);
        assert_eq!(output, [100, 100, 100, 1, 2, 3]);
        // ARGB 窗口：半透明黑叠在左侧。
        paint(&mut output, area, &[0, 0, 0, 128], (10, 10, 1, 1), false, true);
        assert_eq!(&output[..3], &[49, 49, 49]);
    }
}
