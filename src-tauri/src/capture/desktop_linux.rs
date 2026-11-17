//! X11 截图：RandR 读取各屏物理像素边界并逐屏拼接，EWMH 枚举窗口吸附候选，XFixes 叠加光标。
//! xcap 在 X11 下会把显示器边界除以 Xft.dpi 缩放，与 GetImage 返回的像素不一致，
//! 因此显示器几何与抓屏都直接走 xcb；窗口内容抓取仍复用 xcap。
use super::{clipped, Desktop, Display, NativeWindow, Target};
use image::RgbaImage;
use xcb::{randr, x, xfixes, XidNew};

pub(super) struct X11 {
    conn: xcb::Connection,
    root: x::Window,
}

impl X11 {
    pub fn connect() -> Result<Self, String> {
        if wayland() {
            return Err("暂不支持 Wayland 会话截图，请在登录界面选择 X11（Xorg）会话。".into());
        }
        let (conn, index) = xcb::Connection::connect_with_extensions(
            None,
            &[xcb::Extension::RandR],
            &[xcb::Extension::XFixes],
        )
        .map_err(|e| format!("无法连接 X 服务器：{e}"))?;
        let root = conn
            .get_setup()
            .roots()
            .nth(usize::try_from(index).unwrap_or(0))
            .ok_or("未找到 X 屏幕。")?
            .root();
        Ok(Self { conn, root })
    }

    /// RandR 1.5 的活动显示器（物理像素）。没有设置主屏时，第一块屏视为主屏。
    pub fn displays(&self, scale: f64) -> Result<Vec<Display>, String> {
        let reply = self
            .conn
            .wait_for_reply(self.conn.send_request(&randr::GetMonitors {
                window: self.root,
                get_active: true,
            }))
            .map_err(|e| format!("读取显示器失败：{e}"))?;
        let mut displays: Vec<Display> = reply
            .monitors()
            .filter(|m| m.width() > 0 && m.height() > 0)
            .map(|m| Display {
                x: i32::from(m.x()),
                y: i32::from(m.y()),
                width: u32::from(m.width()),
                height: u32::from(m.height()),
                primary: m.primary(),
                scale,
            })
            .collect();
        if displays.is_empty() {
            return Err("未找到可用显示器，请检查显示器连接。".into());
        }
        if !displays.iter().any(|d| d.primary) {
            displays[0].primary = true;
        }
        displays.sort_by_key(|d| (d.x, d.y, d.width, d.height));
        Ok(displays)
    }

    /// 从根窗口抓取一块全局像素区域；只接受 24/32 位深、32 位像素的常见 TrueColor 格式。
    pub fn grab(&self, left: i32, top: i32, width: u32, height: u32) -> Result<RgbaImage, String> {
        let (x, y, w, h) = (
            i16::try_from(left).map_err(|_| "截图区域超出 X11 坐标范围。")?,
            i16::try_from(top).map_err(|_| "截图区域超出 X11 坐标范围。")?,
            u16::try_from(width).map_err(|_| "截图区域超出 X11 坐标范围。")?,
            u16::try_from(height).map_err(|_| "截图区域超出 X11 坐标范围。")?,
        );
        let reply = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetImage {
                format: x::ImageFormat::ZPixmap,
                drawable: x::Drawable::Window(self.root),
                x,
                y,
                width: w,
                height: h,
                plane_mask: u32::MAX,
            }))
            .map_err(|e| format!("截图失败：{e}"))?;
        let setup = self.conn.get_setup();
        let depth = reply.depth();
        let bpp = setup
            .pixmap_formats()
            .iter()
            .find(|f| f.depth() == depth)
            .map(|f| f.bits_per_pixel())
            .unwrap_or(0);
        if !matches!(depth, 24 | 32) || bpp != 32 {
            return Err(format!(
                "暂不支持 {depth} 位色深的 X11 显示，请切换为 24 位色。"
            ));
        }
        let msb = setup.image_byte_order() == x::ImageOrder::MsbFirst;
        to_rgba(reply.data(), width, height, msb)
    }

    /// 读取当前光标（全局像素坐标的左上角与预乘 ARGB 位图）；XFixes 不可用时返回 None。
    pub fn cursor(&self) -> Option<(i32, i32, RgbaImage)> {
        self.conn
            .wait_for_reply(self.conn.send_request(&xfixes::QueryVersion {
                client_major_version: 4,
                client_minor_version: 0,
            }))
            .ok()?;
        let reply = self
            .conn
            .wait_for_reply(self.conn.send_request(&xfixes::GetCursorImage {}))
            .ok()?;
        let image = cursor_rgba(
            reply.cursor_image(),
            u32::from(reply.width()),
            u32::from(reply.height()),
        )?;
        Some((
            i32::from(reply.x()) - i32::from(reply.xhot()),
            i32::from(reply.y()) - i32::from(reply.yhot()),
            image,
        ))
    }

    fn atom(&self, name: &str) -> Option<x::Atom> {
        let atom = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::InternAtom {
                only_if_exists: true,
                name: name.as_bytes(),
            }))
            .ok()?
            .atom();
        (atom != x::ATOM_NONE).then_some(atom)
    }

    fn cardinals(&self, window: x::Window, property: Option<x::Atom>, len: u32) -> Vec<u32> {
        let Some(property) = property else {
            return Vec::new();
        };
        self.conn
            .wait_for_reply(self.conn.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type: x::ATOM_CARDINAL,
                long_offset: 0,
                long_length: len,
            }))
            .map(|reply| reply.value::<u32>().to_vec())
            .unwrap_or_default()
    }

    fn atoms(&self, window: x::Window, property: Option<x::Atom>) -> Vec<x::Atom> {
        let Some(property) = property else {
            return Vec::new();
        };
        self.conn
            .wait_for_reply(self.conn.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type: x::ATOM_ATOM,
                long_offset: 0,
                long_length: 32,
            }))
            .map(|reply| reply.value::<x::Atom>().to_vec())
            .unwrap_or_default()
    }
}

pub(super) fn wayland() -> bool {
    crate::shared::x11::wayland()
}

/// ZPixmap 32 位像素按服务器字节序为 BGRX（LSB）或 XRGB（MSB），扫描线无额外填充。
fn to_rgba(data: &[u8], width: u32, height: u32, msb: bool) -> Result<RgbaImage, String> {
    let pixels = usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| "截图过大。")?;
    if data.len() < pixels * 4 {
        return Err("X11 返回的图像数据不完整，请重新截图。".into());
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    for p in data[..pixels * 4].chunks_exact(4) {
        if msb {
            rgba.extend_from_slice(&[p[1], p[2], p[3], 255]);
        } else {
            rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "截图数据无效。".into())
}

/// XFixes 光标为预乘 alpha 的 ARGB32；还原为直通 alpha 以便 overlay 正确混合。
fn cursor_rgba(argb: &[u32], width: u32, height: u32) -> Option<RgbaImage> {
    let count = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
    if width == 0 || height == 0 || argb.len() < count {
        return None;
    }
    let mut rgba = Vec::with_capacity(count * 4);
    for &pixel in &argb[..count] {
        let a = (pixel >> 24) as u8;
        let channel = |shift: u32| {
            let c = ((pixel >> shift) & 0xff) as u16;
            if a == 0 {
                0
            } else {
                (c * 255 / u16::from(a)).min(255) as u8
            }
        };
        rgba.extend_from_slice(&[channel(16), channel(8), channel(0), a]);
    }
    RgbaImage::from_raw(width, height, rgba)
}

fn draw_cursor(x11: &X11, image: &mut RgbaImage, origin: (i32, i32)) {
    if let Some((left, top, cursor)) = x11.cursor() {
        image::imageops::overlay(
            image,
            &cursor,
            i64::from(left) - i64::from(origin.0),
            i64::from(top) - i64::from(origin.1),
        );
    }
}

/// 外框在客户区基础上加窗口管理器装饰（_NET_FRAME_EXTENTS），
/// 再减去 GTK 客户端装饰的阴影（_GTK_FRAME_EXTENTS）；均为 左、右、上、下。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Extents {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Extents {
    fn from(values: &[u32]) -> Self {
        match values {
            [left, right, top, bottom, ..] => Self {
                left: *left,
                right: *right,
                top: *top,
                bottom: *bottom,
            },
            _ => Self::default(),
        }
    }
}

/// 返回窗口可见外框的全局像素矩形；阴影大于窗口时视为无效。
pub(super) fn frame_rect(
    client: (i32, i32, u32, u32),
    decoration: Extents,
    shadow: Extents,
) -> Option<(i32, i32, u32, u32)> {
    let (x, y, w, h) = client;
    let left = i64::from(x) - i64::from(decoration.left) + i64::from(shadow.left);
    let top = i64::from(y) - i64::from(decoration.top) + i64::from(shadow.top);
    let width = i64::from(w) + i64::from(decoration.left) + i64::from(decoration.right)
        - i64::from(shadow.left)
        - i64::from(shadow.right);
    let height = i64::from(h) + i64::from(decoration.top) + i64::from(decoration.bottom)
        - i64::from(shadow.top)
        - i64::from(shadow.bottom);
    Some((
        i32::try_from(left).ok()?,
        i32::try_from(top).ok()?,
        u32::try_from(width).ok().filter(|v| *v > 0)?,
        u32::try_from(height).ok().filter(|v| *v > 0)?,
    ))
}

/// 录屏按窗口 ID 读取外框，与吸附候选同样计入装饰并去掉 GTK 阴影，两处换算保持一致。
pub(crate) fn window_frame_of(
    x11: &crate::shared::x11::X11,
    window: x::Window,
) -> Option<(i32, i32, u32, u32)> {
    let extents = |name: &str| {
        let Some(property) = x11.existing(name) else {
            return Extents::default();
        };
        x11.conn
            .wait_for_reply(x11.conn.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type: x::ATOM_CARDINAL,
                long_offset: 0,
                long_length: 4,
            }))
            .ok()
            .filter(|reply| reply.format() == 32)
            .map(|reply| Extents::from(reply.value::<u32>()))
            .unwrap_or_default()
    };
    frame_rect(
        x11.window_rect(window)?,
        extents("_NET_FRAME_EXTENTS"),
        extents("_GTK_FRAME_EXTENTS"),
    )
}

struct WindowFrame {
    rect: (i32, i32, u32, u32),
    decoration: Extents,
    shadow: Extents,
}

fn window_frame(x11: &X11, window: &NativeWindow) -> Option<WindowFrame> {
    let id = unsafe { x::Window::new(window.id().ok()?) };
    let client = (
        window.x().ok()?,
        window.y().ok()?,
        window.width().ok()?,
        window.height().ok()?,
    );
    let decoration = Extents::from(&x11.cardinals(id, x11.atom("_NET_FRAME_EXTENTS"), 4));
    let shadow = Extents::from(&x11.cardinals(id, x11.atom("_GTK_FRAME_EXTENTS"), 4));
    Some(WindowFrame {
        rect: frame_rect(client, decoration, shadow)?,
        decoration,
        shadow,
    })
}

pub fn capture(primary_only: bool, include_cursor: bool, snap: bool) -> Result<Desktop, String> {
    let x11 = X11::connect()?;
    // Xft.dpi 是 X11 的全局缩放，仅用于浮窗布局；截图坐标始终是物理像素。
    let scale = xcap::Monitor::all()
        .ok()
        .and_then(|m| m.first().and_then(|m| m.scale_factor().ok()))
        .map(f64::from)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    let mut displays = x11.displays(scale)?;
    if primary_only {
        displays.retain(|d| d.primary);
    }
    let x = displays
        .iter()
        .map(|d| d.x)
        .min()
        .ok_or("没有可用显示器。")?;
    let y = displays
        .iter()
        .map(|d| d.y)
        .min()
        .ok_or("没有可用显示器。")?;
    let right = displays
        .iter()
        .map(|d| i64::from(d.x) + i64::from(d.width))
        .max()
        .unwrap_or_default();
    let bottom = displays
        .iter()
        .map(|d| i64::from(d.y) + i64::from(d.height))
        .max()
        .unwrap_or_default();
    let width = u32::try_from(right - i64::from(x)).map_err(|_| "桌面宽度超出限制。")?;
    let height = u32::try_from(bottom - i64::from(y)).map_err(|_| "桌面高度超出限制。")?;
    if u64::from(width) * u64::from(height) > 64_000_000 || width == 0 || height == 0 {
        return Err("虚拟桌面超过 6400 万像素，请降低屏幕分辨率或减少显示器。".into());
    }
    // 屏幕之间的空隙保持黑色，与 Windows 拼接一致；镜像屏只会重复覆盖同一块像素。
    let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for d in &displays {
        let shot = x11.grab(d.x, d.y, d.width, d.height)?;
        image::imageops::replace(
            &mut image,
            &shot,
            i64::from(d.x) - i64::from(x),
            i64::from(d.y) - i64::from(y),
        );
    }
    if include_cursor {
        draw_cursor(&x11, &mut image, (x, y));
    }
    let mut desktop = Desktop {
        image,
        x,
        y,
        displays,
        targets: Vec::new(),
        windows: Vec::new(),
        include_cursor,
    };
    if snap && !primary_only {
        snap_windows(&x11, &mut desktop, (width, height));
    }
    Ok(desktop)
}

/// 只取当前工作区内、未最小化的普通窗口；xcap 已按层级排序（前者在上）。
fn snap_windows(x11: &X11, desktop: &mut Desktop, size: (u32, u32)) {
    let Ok(windows) = xcap::Window::all() else {
        return;
    };
    let own = std::process::id();
    let wm_desktop = x11.atom("_NET_WM_DESKTOP");
    let current = x11
        .cardinals(x11.root, x11.atom("_NET_CURRENT_DESKTOP"), 1)
        .first()
        .copied();
    let window_type = x11.atom("_NET_WM_WINDOW_TYPE");
    // 桌面图标层、面板与停靠栏会覆盖整屏或屏幕边缘，吸附它们只会干扰框选。
    let skipped: Vec<x::Atom> = [
        "_NET_WM_WINDOW_TYPE_DESKTOP",
        "_NET_WM_WINDOW_TYPE_DOCK",
        "_NET_WM_WINDOW_TYPE_TOOLBAR",
        "_NET_WM_WINDOW_TYPE_NOTIFICATION",
    ]
    .iter()
    .filter_map(|name| x11.atom(name))
    .collect();
    for window in windows.into_iter().take(256) {
        let Ok(id) = window.id() else {
            continue;
        };
        let xid = unsafe { x::Window::new(id) };
        let on_current = match (x11.cardinals(xid, wm_desktop, 1).first().copied(), current) {
            // 0xFFFFFFFF 表示“所有工作区”。
            (Some(desk), Some(current)) => desk == current || desk == u32::MAX,
            _ => true,
        };
        let eligible = on_current
            && window.pid().map_or(true, |pid| pid != own)
            && !window.is_minimized().unwrap_or(true)
            && !x11
                .atoms(xid, window_type)
                .iter()
                .any(|kind| skipped.contains(kind));
        // 小于 40 像素的窗口多为工具提示或浮标，吸附反而干扰框选。
        let Some(rect) = eligible
            .then(|| window_frame(x11, &window))
            .flatten()
            .map(|frame| frame.rect)
            .filter(|(_, _, w, h)| *w >= 40 && *h >= 40)
            .and_then(|(wx, wy, ww, wh)| clipped(wx, wy, ww, wh, (desktop.x, desktop.y), size))
        else {
            continue;
        };
        let index = desktop.windows.len();
        desktop.targets.push(Target {
            rect,
            kind: "window",
            window_index: index,
            recording_only: false,
            icon: None,
            title: window
                .title()
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect(),
            app_name: window
                .app_name()
                .unwrap_or_default()
                .chars()
                .take(80)
                .collect(),
        });
        desktop.windows.push(window);
    }
}

/// 原生窗口截图：客户区取自窗口自身（合成器开启时被遮挡部分也完整），
/// 去掉 GTK 阴影后放回外框；服务器端标题栏等装饰从根窗口的当前画面补齐。
/// 返回的位置与尺寸和吸附候选使用同一外框，截图会话据此校验窗口是否移动。
pub fn native_window(
    window: &NativeWindow,
    include_cursor: bool,
) -> Result<(RgbaImage, i32, i32), String> {
    let x11 = X11::connect()?;
    let frame = window_frame(&x11, window).ok_or("读取窗口位置失败。")?;
    let (fx, fy, fw, fh) = frame.rect;
    if u64::from(fw) * u64::from(fh) > 64_000_000 {
        return Err("窗口超过 6400 万像素，请缩小窗口后重试。".into());
    }
    let client = window
        .capture_image()
        .map_err(|e| format!("原生窗口截图失败：{e}"))?;
    let (cw, ch) = (
        window.width().map_err(|e| e.to_string())?,
        window.height().map_err(|e| e.to_string())?,
    );
    if client.dimensions() != (cw, ch) {
        return Err("原生窗口图片与窗口尺寸不一致，请使用区域截图。".into());
    }
    let Extents { left, top, .. } = frame.shadow;
    let inner = image::imageops::crop_imm(
        &client,
        left,
        top,
        cw.saturating_sub(left + frame.shadow.right),
        ch.saturating_sub(top + frame.shadow.bottom),
    )
    .to_image();
    let mut image = if frame.decoration == Extents::default() {
        inner
    } else {
        let mut base = x11.grab(fx, fy, fw, fh)?;
        image::imageops::replace(
            &mut base,
            &inner,
            i64::from(frame.decoration.left),
            i64::from(frame.decoration.top),
        );
        base
    };
    if image.dimensions() != (fw, fh) {
        return Err("原生窗口图片与窗口尺寸不一致，请使用区域截图。".into());
    }
    if include_cursor {
        draw_cursor(&x11, &mut image, (fx, fy));
    }
    Ok((image, fx, fy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_adds_decorations_and_removes_csd_shadow() {
        let ssd = Extents::from(&[2, 2, 30, 2]);
        assert_eq!(
            frame_rect((100, 130, 800, 600), ssd, Extents::default()),
            Some((98, 100, 804, 632))
        );
        let shadow = Extents::from(&[24, 24, 18, 30]);
        assert_eq!(
            frame_rect((100, 100, 848, 648), Extents::default(), shadow),
            Some((124, 118, 800, 600))
        );
        assert_eq!(frame_rect((0, 0, 20, 20), Extents::default(), shadow), None);
        assert_eq!(Extents::from(&[1, 2]), Extents::default());
    }

    #[test]
    fn converts_both_byte_orders_to_opaque_rgba() {
        let lsb = to_rgba(&[10, 20, 30, 0, 1, 2, 3, 0], 2, 1, false).unwrap();
        assert_eq!(lsb.as_raw(), &[30, 20, 10, 255, 3, 2, 1, 255]);
        let msb = to_rgba(&[0, 30, 20, 10], 1, 1, true).unwrap();
        assert_eq!(msb.as_raw(), &[30, 20, 10, 255]);
        assert!(to_rgba(&[0; 4], 2, 1, false).is_err());
    }

    #[test]
    fn unpremultiplies_cursor_pixels() {
        let image = cursor_rgba(&[0x8040_2000, 0, 0xffff_ffff, 0x80ff_0000], 2, 2).unwrap();
        assert_eq!(&image.as_raw()[..4], &[127, 63, 0, 128]);
        assert_eq!(&image.as_raw()[4..8], &[0, 0, 0, 0]);
        assert_eq!(&image.as_raw()[8..12], &[255, 255, 255, 255]);
        // 损坏的预乘值（通道大于 alpha）被钳到 255。
        assert_eq!(&image.as_raw()[12..16], &[255, 0, 0, 128]);
        assert!(cursor_rgba(&[0; 3], 2, 2).is_none());
    }
}
