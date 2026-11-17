//! X11 录制来源：按帧率从根窗口抓取选区，优先经 MIT-SHM 共享内存取像素，服务器不支持时
//! （如经网络转发的显示）退回普通 GetImage；需要时用 XFixes 叠加当前光标。
//!
//! X11 没有“新画面到达”的通知，静态桌面也按帧率取样，由编码端延续画面。
//! 坐标约定与截图一致：显示器原点与选区都是 RandR 物理像素。窗口录制取窗口外框在根窗口上的
//! 当前画面，因此跟随窗口移动，但也会录到盖在它上面的其他窗口。
use super::{
    super::frame::{BgraImage, FramePool},
    CaptureBackend, NativeSource, RecorderConfig, VideoFrame,
};
use crate::{
    capture::Rect,
    shared::x11::{self, X11},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use xcb::{shm, x, xfixes, XidNew};

/// 录屏时间轴单位为 100 ns，取 CLOCK_MONOTONIC；GStreamer 系统时钟默认同源，音画对齐无需换算。
pub fn now() -> i64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // CLOCK_MONOTONIC 在 Linux 上总是可用，调用不会失败。
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) };
    time.tv_sec * 10_000_000 + time.tv_nsec / 100
}

pub fn supported() -> Result<(), String> {
    let x11 = connect()?;
    pixel_order(&x11).map(|_| ())
}

fn connect() -> Result<X11, String> {
    if x11::wayland() {
        return Err("暂不支持 Wayland 会话录屏，请在登录界面选择 X11（Xorg）会话。".into());
    }
    let (conn, index) = xcb::Connection::connect_with_extensions(
        None,
        &[],
        &[
            xcb::Extension::Shm,
            xcb::Extension::XFixes,
            xcb::Extension::RandR,
        ],
    )
    .map_err(|e| format!("无法连接 X 服务器：{e}"))?;
    let root = conn
        .get_setup()
        .roots()
        .nth(usize::try_from(index).unwrap_or(0))
        .ok_or("未找到 X 屏幕。")?
        .root();
    Ok(X11 { conn, root })
}

fn screen(x11: &X11) -> Option<&x::Screen> {
    x11.conn
        .get_setup()
        .roots()
        .find(|screen| screen.root() == x11.root)
}

/// 只接受 24/32 位深、每像素 32 位的 TrueColor 根窗口；返回服务器字节序是否为 MSB。
fn pixel_order(x11: &X11) -> Result<bool, String> {
    let setup = x11.conn.get_setup();
    let depth = screen(x11).ok_or("未找到 X 屏幕。")?.root_depth();
    let bpp = setup
        .pixmap_formats()
        .iter()
        .find(|f| f.depth() == depth)
        .map(|f| f.bits_per_pixel())
        .unwrap_or(0);
    if !matches!(depth, 24 | 32) || bpp != 32 {
        return Err(format!(
            "暂不支持 {depth} 位色深的 X11 显示录屏，请切换为 24 位色。"
        ));
    }
    Ok(setup.image_byte_order() == x::ImageOrder::MsbFirst)
}

/// 按物理原点找回 RandR 显示器；布局变化后找不到即视为来源失效。
fn monitor(x11: &X11, x: i32, y: i32) -> Result<(i32, i32, u32, u32), String> {
    x11.monitors()
        .into_iter()
        .find(|m| (m.0, m.1) == (x, y))
        .ok_or_else(|| "录制的显示器已断开或布局已改变，请重新选择。".into())
}

/// 窗口需仍存在、已映射（最小化的窗口会被取消映射）且属于原进程，返回其外框。
fn window(x11: &X11, id: u32, pid: u32) -> Result<(i32, i32, u32, u32), String> {
    let window = unsafe { x::Window::new(id) };
    let viewable = x11
        .conn
        .wait_for_reply(x11.conn.send_request(&x::GetWindowAttributes { window }))
        .is_ok_and(|attributes| attributes.map_state() == x::MapState::Viewable);
    if !viewable || x11.window_pid(window).is_some_and(|current| current != pid) {
        return Err("录制窗口已关闭、最小化或被替换，录制已结束。".into());
    }
    crate::capture::desktop::linux_window_frame(x11, window)
        .ok_or_else(|| "读取窗口位置失败。".into())
}

fn source_rect(x11: &X11, source: NativeSource) -> Result<(i32, i32, u32, u32), String> {
    match source {
        NativeSource::Monitor { x, y } => monitor(x11, x, y),
        NativeSource::Window { id, pid } => window(x11, id, pid),
    }
}

pub fn source_dimensions(source: NativeSource) -> Result<(u32, u32), String> {
    let x11 = connect()?;
    let (_, _, width, height) = source_rect(&x11, source)?;
    if width < 2 || height < 2 {
        return Err("录制来源尺寸无效。".into());
    }
    Ok((width, height))
}

pub fn prepare(
    config: &RecorderConfig,
    size: (u32, u32),
    source: NativeSource,
    pool: Arc<FramePool>,
) -> Result<Box<dyn CaptureBackend>, String> {
    Ok(Box::new(Capture::new(config, size, source, pool)?))
}

/// 服务器写入、本进程读取的共享内存段。创建后立即标记删除，双方都分离时才真正释放，
/// 进程异常退出也不会残留 System V 共享内存。
struct Segment {
    seg: shm::Seg,
    address: *mut u8,
    len: usize,
}

impl Segment {
    fn attach(x11: &X11, len: usize) -> Option<Self> {
        if !x11
            .conn
            .active_extensions()
            .any(|extension| extension == xcb::Extension::Shm)
        {
            return None;
        }
        x11.conn
            .wait_for_reply(x11.conn.send_request(&shm::QueryVersion {}))
            .ok()?;
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, len, libc::IPC_CREAT | 0o600) };
        if id < 0 {
            return None;
        }
        let address = unsafe { libc::shmat(id, std::ptr::null(), 0) };
        if address as isize == -1 {
            unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
            return None;
        }
        let seg: shm::Seg = x11.conn.generate_id();
        // 本机 X 服务器才能挂接同一段内存；远程显示返回错误，调用方回退普通 GetImage。
        let attached = x11
            .conn
            .send_and_check_request(&shm::Attach {
                shmseg: seg,
                shmid: id as u32,
                read_only: false,
            })
            .is_ok();
        unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
        if !attached {
            unsafe { libc::shmdt(address) };
            return None;
        }
        Some(Self {
            seg,
            address: address.cast(),
            len,
        })
    }

    fn detach(self, x11: &X11) {
        let _ = x11
            .conn
            .send_and_check_request(&shm::Detach { shmseg: self.seg });
        unsafe { libc::shmdt(self.address.cast()) };
    }
}

/// 与选区相交的根窗口部分；超出屏幕的部分（窗口拖出屏幕边缘）保持黑色。
fn visible(
    (left, top, width, height): (i32, i32, u32, u32),
    (root_width, root_height): (u32, u32),
) -> Option<(i32, i32, u32, u32)> {
    let right = (i64::from(left) + i64::from(width)).min(i64::from(root_width));
    let bottom = (i64::from(top) + i64::from(height)).min(i64::from(root_height));
    let (x, y) = (i64::from(left).max(0), i64::from(top).max(0));
    (right > x && bottom > y).then(|| {
        (
            x as i32,
            y as i32,
            (right - x) as u32,
            (bottom - y) as u32,
        )
    })
}

/// 把 ZPixmap 32 位像素（LSB 为 BGRX，MSB 为 XRGB）复制到目标 BGRA 的指定位置，alpha 置 255。
fn copy_pixels(
    source: &[u8],
    (width, height): (u32, u32),
    msb: bool,
    target: &mut BgraImage,
    (dx, dy): (u32, u32),
) {
    let (width, height) = (width as usize, height as usize);
    let stride = target.width as usize * 4;
    for row in 0..height {
        let from = &source[row * width * 4..(row + 1) * width * 4];
        let start = (dy as usize + row) * stride + dx as usize * 4;
        let to = &mut target.data[start..start + width * 4];
        for (output, input) in to.chunks_exact_mut(4).zip(from.chunks_exact(4)) {
            if msb {
                output.copy_from_slice(&[input[3], input[2], input[1], 255]);
            } else {
                output.copy_from_slice(&[input[0], input[1], input[2], 255]);
            }
        }
    }
}

/// XFixes 光标为预乘 alpha 的 ARGB32，按“源 + 目标 ×（1 − α）”叠加到 BGRA 帧上。
fn blend_cursor(
    image: &mut BgraImage,
    cursor: &[u32],
    (cursor_width, cursor_height): (u32, u32),
    (left, top): (i64, i64),
) {
    let (width, height) = (i64::from(image.width), i64::from(image.height));
    for row in 0..i64::from(cursor_height) {
        let y = top + row;
        if !(0..height).contains(&y) {
            continue;
        }
        for column in 0..i64::from(cursor_width) {
            let x = left + column;
            if !(0..width).contains(&x) {
                continue;
            }
            let Some(&pixel) = cursor.get((row * i64::from(cursor_width) + column) as usize)
            else {
                return;
            };
            let alpha = pixel >> 24;
            if alpha == 0 {
                continue;
            }
            let index = ((y * width + x) * 4) as usize;
            let target = &mut image.data[index..index + 3];
            for (channel, shift) in target.iter_mut().zip([0u32, 8, 16]) {
                let source = (pixel >> shift) & 0xff;
                let value = source + u32::from(*channel) * (255 - alpha) / 255;
                *channel = value.min(255) as u8;
            }
        }
    }
}

pub struct Capture {
    x11: X11,
    segment: Option<Segment>,
    msb: bool,
    cursor: bool,
    region: Rect,
    source: NativeSource,
    size: (u32, u32),
    pool: Arc<FramePool>,
    interval: Duration,
    due: Option<Instant>,
}

impl Capture {
    pub fn new(
        config: &RecorderConfig,
        size: (u32, u32),
        source: NativeSource,
        pool: Arc<FramePool>,
    ) -> Result<Self, String> {
        let x11 = connect()?;
        let msb = pixel_order(&x11)?;
        let (_, _, width, height) = source_rect(&x11, source)?;
        if (width, height) != size {
            return Err("录制来源尺寸已变化，请重新选择录屏区域。".into());
        }
        let cursor = config.capture_cursor
            && x11.conn
                .wait_for_reply(x11.conn.send_request(&xfixes::QueryVersion {
                    client_major_version: 4,
                    client_minor_version: 0,
                }))
                .is_ok();
        let region = config.region;
        let len = region.width as usize * region.height as usize * 4;
        let segment = Segment::attach(&x11, len);
        Ok(Self {
            x11,
            segment,
            msb,
            cursor,
            region,
            source,
            size,
            pool,
            interval: Duration::from_nanos(1_000_000_000 / u64::from(config.fps.max(1))),
            due: None,
        })
    }

    /// 来源当前的全局外框；窗口移动后原点随之变化，尺寸变化则结束录制。
    fn current(&self) -> Result<(i32, i32, u32, u32), String> {
        let rect = source_rect(&self.x11, self.source)?;
        if (rect.2, rect.3) != self.size {
            return Err("录制来源尺寸已改变，录制已结束；请重新选择来源。".into());
        }
        Ok(rect)
    }

    fn grab(&mut self) -> Result<BgraImage, String> {
        let (origin_x, origin_y, _, _) = self.current()?;
        let r = self.region;
        let left = origin_x + r.x as i32;
        let top = origin_y + r.y as i32;
        let mut image = self.pool.acquire(r.width, r.height);
        let root = screen(&self.x11)
            .map(|s| (u32::from(s.width_in_pixels()), u32::from(s.height_in_pixels())))
            .unwrap_or_default();
        let Some(part) = visible((left, top, r.width, r.height), root) else {
            image.data.fill(0);
            return Ok(image);
        };
        if part != (left, top, r.width, r.height) {
            image.data.fill(0);
        }
        let (x, y, width, height) = part;
        let offset = ((x - left) as u32, (y - top) as u32);
        let bytes = width as usize * height as usize * 4;
        let (px, py, pw, ph) = (x as i16, y as i16, width as u16, height as u16);
        let shared = self.segment.as_ref().filter(|s| s.len >= bytes).and_then(|segment| {
            self.x11
                .conn
                .wait_for_reply(self.x11.conn.send_request(&shm::GetImage {
                    drawable: x::Drawable::Window(self.x11.root),
                    x: px,
                    y: py,
                    width: pw,
                    height: ph,
                    plane_mask: u32::MAX,
                    format: x::ImageFormat::ZPixmap as u8,
                    shmseg: segment.seg,
                    offset: 0,
                }))
                .ok()
                .map(|_| segment.address)
        });
        if let Some(address) = shared {
            // 回复到达时服务器已写完整块像素；段在 self 存活期间保持挂接。
            let source = unsafe { std::slice::from_raw_parts(address, bytes) };
            copy_pixels(source, (width, height), self.msb, &mut image, offset);
        } else {
            let reply = self
                .x11
                .conn
                .wait_for_reply(self.x11.conn.send_request(&x::GetImage {
                    format: x::ImageFormat::ZPixmap,
                    drawable: x::Drawable::Window(self.x11.root),
                    x: px,
                    y: py,
                    width: pw,
                    height: ph,
                    plane_mask: u32::MAX,
                }))
                .map_err(|e| format!("X11 屏幕采集失败：{e}"))?;
            if reply.data().len() < bytes {
                return Err("X11 返回的录屏数据不完整。".into());
            }
            copy_pixels(reply.data(), (width, height), self.msb, &mut image, offset);
        }
        if self.cursor {
            if let Ok(cursor) = self
                .x11
                .conn
                .wait_for_reply(self.x11.conn.send_request(&xfixes::GetCursorImage {}))
            {
                blend_cursor(
                    &mut image,
                    cursor.cursor_image(),
                    (u32::from(cursor.width()), u32::from(cursor.height())),
                    (
                        i64::from(cursor.x()) - i64::from(cursor.xhot()) - i64::from(left),
                        i64::from(cursor.y()) - i64::from(cursor.yhot()) - i64::from(top),
                    ),
                );
            }
        }
        Ok(image)
    }
}

impl CaptureBackend for Capture {
    fn desktop_origin(&self) -> Result<(i32, i32), String> {
        self.current().map(|(x, y, _, _)| (x, y))
    }
    fn check_source(&self) -> Result<(), String> {
        self.current().map(|_| ())
    }
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String> {
        let now = Instant::now();
        if self.due.is_some_and(|due| now < due) {
            return Ok(None);
        }
        // 落后超过一帧时从当前时间重新计时，不连续补抓追赶。
        self.due = Some(match self.due {
            Some(due) if now < due + self.interval => due + self.interval,
            _ => now + self.interval,
        });
        let image = self.grab()?;
        Ok(Some(VideoFrame {
            source_time: now(),
            image,
        }))
    }
    fn wait_for_frame(&self, timeout: Duration) {
        let wait = self
            .due
            .map_or(Duration::ZERO, |due| due.saturating_duration_since(Instant::now()));
        std::thread::sleep(wait.min(timeout));
    }
    fn refresh(&mut self) -> Result<(), String> {
        // 每帧都重新抓取，恢复后立即取一张新画面即可。
        self.due = None;
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(segment) = self.segment.take() {
            segment.detach(&self.x11);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_part_is_clipped_to_the_root_window() {
        assert_eq!(visible((10, 10, 20, 20), (100, 100)), Some((10, 10, 20, 20)));
        assert_eq!(visible((-5, 90, 20, 20), (100, 100)), Some((0, 90, 15, 10)));
        assert_eq!(visible((120, 0, 20, 20), (100, 100)), None);
    }

    #[test]
    fn copies_both_byte_orders_into_an_offset_bgra_frame() {
        let mut image = BgraImage::new(3, 2);
        // 一个像素：B=1 G=2 R=3。
        copy_pixels(&[1, 2, 3, 0], (1, 1), false, &mut image, (1, 1));
        assert_eq!(&image.data[16..20], &[1, 2, 3, 255]);
        copy_pixels(&[0, 3, 2, 1], (1, 1), true, &mut image, (0, 0));
        assert_eq!(&image.data[0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn premultiplied_cursor_blends_and_clips_at_the_edges() {
        let mut image = BgraImage::new(2, 1);
        image.data.fill(200);
        // 半透明白（预乘 128）与完全不透明红；第二个像素超出画面右侧。
        let cursor = [0x8080_8080, 0xffff_0000, 0xffff_0000];
        blend_cursor(&mut image, &cursor, (3, 1), (0, 0));
        assert_eq!(&image.data[0..3], &[227, 227, 227]);
        assert_eq!(&image.data[4..7], &[0, 0, 255]);
        let mut image = BgraImage::new(1, 1);
        blend_cursor(&mut image, &cursor, (3, 1), (-1, 0));
        assert_eq!(&image.data[0..3], &[0, 0, 255]);
    }

    #[test]
    fn monotonic_ticks_advance() {
        let first = now();
        std::thread::sleep(Duration::from_millis(2));
        assert!(now() > first);
    }
}
