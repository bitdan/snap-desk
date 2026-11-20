//! ext-image-copy-capture-v1（COSMIC、较新的 niri / Sway 等）：标准协议，无需门户授权。
//! 共享内存缓冲由 memfd 提供；协商、拷贝与读取都在调用线程上完成，最多等待 5 秒。
use super::{outputs, to_rgba, Output, Shot};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::protocol::{wl_buffer, wl_output, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};

#[derive(Default)]
struct State {
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    constraints: bool,
    transform: Option<wl_output::Transform>,
    result: Option<Result<(), String>>,
}

wayland_client::delegate_noop!(State: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State: wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(State: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(State: ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(State: ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(State: ExtImageCopyCaptureManagerV1);

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.size = Some((width, height)),
            Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.formats.push(format),
            Event::Done => state.constraints = true,
            Event::Stopped => {
                state
                    .result
                    .get_or_insert(Err("合成器停止了截图会话。".into()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Transform {
                transform: WEnum::Value(transform),
            } => state.transform = Some(transform),
            Event::Ready => state.result = Some(Ok(())),
            Event::Failed { reason } => state.result = Some(Err(format!("{reason:?}"))),
            _ => {}
        }
    }
}

/// 只接受 32 位格式；返回内存是否为 BGRA 顺序以及是否保留 alpha。
pub(super) fn layout_of(format: wl_shm::Format) -> Option<(bool, bool)> {
    match format {
        wl_shm::Format::Xrgb8888 => Some((true, false)),
        wl_shm::Format::Argb8888 => Some((true, false)),
        wl_shm::Format::Xbgr8888 => Some((false, false)),
        wl_shm::Format::Abgr8888 => Some((false, false)),
        _ => None,
    }
}

/// 映射到进程内存的 memfd；析构时解除映射。
struct Mapping {
    fd: OwnedFd,
    ptr: *mut libc::c_void,
    len: usize,
}

impl Mapping {
    fn new(len: usize) -> Result<Self, String> {
        let name = c"snap-desk-capture";
        // SAFETY: memfd_create 返回新描述符，立即交给 OwnedFd；随后的 ftruncate / mmap 都检查返回值。
        let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let size = libc::off_t::try_from(len).map_err(|_| "截图过大。")?;
        if unsafe { libc::ftruncate(raw, size) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                raw,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self { fd, ptr, len })
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: 映射在 Mapping 存活期间有效，长度与 mmap 时一致。
        unsafe { std::slice::from_raw_parts(self.ptr.cast::<u8>(), self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: ptr / len 来自成功的 mmap。
        unsafe {
            libc::munmap(self.ptr, self.len);
        }
    }
}

fn dispatch_until(
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    deadline: Instant,
    done: impl Fn(&State) -> bool,
) -> Result<(), String> {
    while !done(state) {
        if Instant::now() > deadline {
            return Err("合成器没有及时返回画面。".into());
        }
        queue.flush().map_err(|e| e.to_string())?;
        queue.dispatch_pending(state).map_err(|e| e.to_string())?;
        if done(state) {
            break;
        }
        // 只等待有限时间，避免合成器不回应时卡住截图线程。
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut poll = libc::pollfd {
                fd: std::os::fd::AsRawFd::as_raw_fd(&fd),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: pollfd 指向栈上有效结构，数量为 1。
            let ready = unsafe { libc::poll(&mut poll, 1, 100) };
            if ready > 0 {
                guard.read().map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub(super) fn capture(layout: &[Output], target: usize, cursor: bool) -> Result<Shot, String> {
    let wanted = layout.get(target).ok_or("未找到目标显示器。")?;
    let wayshot = outputs::connect()?;
    let info = wayshot
        .get_all_outputs()
        .iter()
        .find(|info| info.name == wanted.name)
        .ok_or("目标显示器已断开。")?;
    let mut queue = wayshot.conn.new_event_queue::<State>();
    let qh = queue.handle();
    let bind_error = |e: wayland_client::globals::BindError| e.to_string();
    let shm: wl_shm::WlShm = wayshot.globals.bind(&qh, 1..=1, ()).map_err(bind_error)?;
    let sources: ExtOutputImageCaptureSourceManagerV1 =
        wayshot.globals.bind(&qh, 1..=1, ()).map_err(bind_error)?;
    let copier: ExtImageCopyCaptureManagerV1 =
        wayshot.globals.bind(&qh, 1..=1, ()).map_err(bind_error)?;
    let source = sources.create_source(&info.wl_output, &qh, ());
    let options = if cursor {
        Options::PaintCursors
    } else {
        Options::empty()
    };
    let session = copier.create_session(&source, options, &qh, ());
    let mut state = State::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = (|| {
        dispatch_until(&mut queue, &mut state, deadline, |s| {
            s.constraints || s.result.is_some()
        })?;
        if let Some(Err(error)) = state.result.take() {
            return Err(error);
        }
        let (width, height) = state.size.ok_or("合成器没有提供画面尺寸。")?;
        let (format, (bgra, keep_alpha)) = state
            .formats
            .iter()
            .find_map(|f| layout_of(*f).map(|layout| (*f, layout)))
            .ok_or("合成器没有提供可用的像素格式。")?;
        if u64::from(width) * u64::from(height) > 64_000_000 {
            return Err("显示器超过 6400 万像素，暂不支持截图。".to_string());
        }
        let stride = width * 4;
        let len = stride as usize * height as usize;
        let mapping = Mapping::new(len)?;
        let pool = shm.create_pool(mapping.fd.as_fd(), len as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            &qh,
            (),
        );
        let frame = session.create_frame(&qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        let copied = dispatch_until(&mut queue, &mut state, deadline, |s| s.result.is_some())
            .and_then(|()| state.result.take().unwrap_or(Ok(())));
        frame.destroy();
        buffer.destroy();
        pool.destroy();
        copied?;
        let image = to_rgba(
            mapping.bytes(),
            width,
            height,
            stride as usize,
            bgra,
            keep_alpha,
        )?;
        Ok(rotate(image, state.transform))
    })();
    session.destroy();
    source.destroy();
    copier.destroy();
    sources.destroy();
    let _ = queue.flush();
    Ok(Shot {
        output: outputs::from_wayshot(info),
        image: result?,
    })
}

/// 缓冲区按显示器原始方向填充；旋转显示器转回用户看到的方向（翻转变换较少见，按对应旋转处理）。
fn rotate(image: image::RgbaImage, transform: Option<wl_output::Transform>) -> image::RgbaImage {
    use wl_output::Transform;
    match transform {
        Some(Transform::_90 | Transform::Flipped90) => image::imageops::rotate270(&image),
        Some(Transform::_180 | Transform::Flipped180) => image::imageops::rotate180(&image),
        Some(Transform::_270 | Transform::Flipped270) => image::imageops::rotate90(&image),
        _ => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_32_bit_shm_formats() {
        assert_eq!(layout_of(wl_shm::Format::Xrgb8888), Some((true, false)));
        assert_eq!(layout_of(wl_shm::Format::Abgr8888), Some((false, false)));
        assert_eq!(layout_of(wl_shm::Format::Rgb565), None);
    }
}
