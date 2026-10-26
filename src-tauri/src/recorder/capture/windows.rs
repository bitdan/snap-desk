//! WGC 自由线程帧池 + D3D11 区域复制。没有 UI 回调执行编码或等待队列。
//! 先复制到独立 CPU 内存再 Close 帧，避免归还帧池后继续访问其表面。
use super::{CaptureBackend, NativeSource, RecorderConfig, VideoFrame};
use crate::capture::Rect;
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
};
use windows::{
    core::{factory, Interface, HSTRING},
    Foundation::{Metadata::ApiInformation, TypedEventHandler},
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HMODULE, HWND, POINT, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::*,
            Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
            Dxgi::{Common::*, IDXGIDevice},
            Gdi::*,
        },
        System::{
            Performance::*,
            WinRT::{
                Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop, RoInitialize,
                RoUninitialize, RO_INIT_MULTITHREADED,
            },
        },
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsIconic, IsWindow},
    },
};

fn error(e: windows::core::Error) -> String {
    format!("Windows 屏幕采集失败：{e}。请检查显示器、图形驱动及屏幕捕获权限后重试。")
}

/// DWM 返回可见边框的物理像素，与 WGC 窗口表面、截图窗口候选一致。
fn window_bounds(id: u32) -> Result<RECT, String> {
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            HWND(id as usize as *mut _),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .map_err(error)?;
    }
    Ok(rect)
}

/// 查询和实际采集共用同一来源解析；WGC 表面大小不能用 DWM 窗口边界代替。
fn capture_item(source: NativeSource) -> Result<GraphicsCaptureItem, String> {
    unsafe {
        let interop: IGraphicsCaptureItemInterop =
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(error)?;
        match source {
            NativeSource::Monitor { x, y } => {
                let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
                if monitor.0.is_null() {
                    return Err("录制显示器已移除。".into());
                }
                interop.CreateForMonitor(monitor).map_err(error)
            }
            NativeSource::Window { id, pid } => {
                let hwnd = HWND(id as usize as *mut _);
                let mut current_pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut current_pid));
                if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() || current_pid != pid
                {
                    return Err("录制窗口已关闭、最小化或被替换。".into());
                }
                interop.CreateForWindow(hwnd).map_err(error)
            }
        }
    }
}

/// 在创建编码器之前读取真实物理像素尺寸，不启动采集或计时。
pub fn source_dimensions(source: NativeSource) -> Result<(u32, u32), String> {
    let _dpi = crate::desktop_capture::PhysicalDpi::enter()?;
    let _runtime = Runtime::new()?;
    let size = capture_item(source)?.Size().map_err(error)?;
    if size.Width < 2 || size.Height < 2 {
        return Err("录制来源尺寸无效。".into());
    }
    Ok((size.Width as u32, size.Height as u32))
}

struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
    fn new() -> Result<Self, String> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED).map_err(error)?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

/// QPC 与 WGC SystemRelativeTime 同源，整数换算避免长时间运行后的浮点精度损失。
pub fn now() -> i64 {
    unsafe {
        let (mut count, mut frequency) = (0, 0);
        // Windows 10 上这两个系统调用保证成功，频率在开机后固定。
        QueryPerformanceCounter(&mut count).expect("QPC unavailable");
        QueryPerformanceFrequency(&mut frequency).expect("QPC frequency unavailable");
        (i128::from(count) * 10_000_000 / i128::from(frequency)) as i64
    }
}

pub fn supported() -> Result<(), String> {
    let _runtime = Runtime::new()?;
    if !GraphicsCaptureSession::IsSupported().map_err(error)? {
        return Err("当前设备不支持 Windows Graphics Capture。".into());
    }
    // 光标控制随 Windows 10 2004 引入；低版本同时不能可靠排除辅助窗口。
    if !ApiInformation::IsPropertyPresent(
        &HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession"),
        &HSTRING::from("IsCursorCaptureEnabled"),
    )
    .map_err(error)?
    {
        return Err("录屏需要 Windows 10 2004 或更新版本（光标控制及辅助窗口捕获排除）。".into());
    }
    Ok(())
}

pub struct Capture {
    config: RecorderConfig,
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    item: GraphicsCaptureItem,
    closed_token: i64,
    arrived_token: i64,
    arrived: Receiver<()>,
    closed: Arc<AtomicBool>,
    staging: ID3D11Texture2D,
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    region: Rect,
    size: (u32, u32),
    // DWM 边界与 WGC 表面各自保存基准，避免把固有边框差异误判为窗口缩放。
    window_size: Option<(i32, i32)>,
    started: bool,
    source: NativeSource,
    _runtime: Runtime,
}

impl Capture {
    pub fn new(
        config: &RecorderConfig,
        size: (u32, u32),
        source: NativeSource,
    ) -> Result<Self, String> {
        // 编码准备线程与实际采集线程都显式使用物理像素，避免跨 DPI 上下文比较尺寸。
        let _dpi = crate::desktop_capture::PhysicalDpi::enter()?;
        supported()?;
        let runtime = Runtime::new()?;
        unsafe {
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(error)?;
            let device = device.ok_or("无法创建 D3D11 设备。")?;
            let context = context.ok_or("无法创建 D3D11 上下文。")?;
            let dxgi: IDXGIDevice = device.cast().map_err(error)?;
            let direct: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .map_err(error)?
                .cast()
                .map_err(error)?;
            let item = capture_item(source)?;
            let native_size = item.Size().map_err(error)?;
            let native_dimensions = (native_size.Width as u32, native_size.Height as u32);
            if native_dimensions != size {
                return Err("录制来源尺寸已变化，请重新选择录屏区域。".into());
            }
            let region = config.region;
            let window_size = if let NativeSource::Window { id, .. } = source {
                let bounds = window_bounds(id)?;
                Some((bounds.right - bounds.left, bounds.bottom - bounds.top))
            } else {
                None
            };
            let desc = D3D11_TEXTURE2D_DESC {
                Width: region.width,
                Height: region.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut staging = None;
            device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .map_err(error)?;
            let staging = staging.ok_or("无法创建录屏裁剪缓冲。")?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &direct,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                native_size,
            )
            .map_err(error)?;
            let session = match pool.CreateCaptureSession(&item) {
                Ok(value) => value,
                Err(e) => {
                    let _ = pool.Close();
                    return Err(error(e));
                }
            };
            let closed = Arc::new(AtomicBool::new(false));
            let signal = closed.clone();
            let setup = (|| -> Result<i64, String> {
                session
                    .SetIsCursorCaptureEnabled(config.capture_cursor)
                    .map_err(error)?;
                let token = item
                    .Closed(&TypedEventHandler::<
                        GraphicsCaptureItem,
                        windows::core::IInspectable,
                    >::new(move |_, _| {
                        signal.store(true, Ordering::Release);
                        Ok(())
                    }))
                    .map_err(error)?;
                Ok(token)
            })();
            let closed_token = match setup {
                Ok(token) => token,
                Err(e) => {
                    let _ = session.Close();
                    let _ = pool.Close();
                    return Err(e);
                }
            };
            // 容量为 1 的通知合并高频回调；像素复制仍在采集线程完成。
            let (notify, arrived) = mpsc::sync_channel(1);
            let arrived_token = match pool.FrameArrived(&TypedEventHandler::<
                Direct3D11CaptureFramePool,
                windows::core::IInspectable,
            >::new(move |_, _| {
                let _ = notify.try_send(());
                Ok(())
            })) {
                Ok(token) => token,
                Err(e) => {
                    let _ = item.RemoveClosed(closed_token);
                    let _ = session.Close();
                    let _ = pool.Close();
                    return Err(error(e));
                }
            };
            Ok(Self {
                config: *config,
                session,
                pool,
                item,
                closed_token,
                arrived_token,
                arrived,
                closed,
                staging,
                context,
                device,
                region,
                size,
                window_size,
                started: false,
                source,
                _runtime: runtime,
            })
        }
    }

    fn copy(&self, frame: &Direct3D11CaptureFrame) -> Result<VideoFrame, String> {
        let size = frame.ContentSize().map_err(error)?;
        if (size.Width as u32, size.Height as u32) != self.size {
            return Err("显示器像素尺寸改变，请重新录屏。".into());
        }
        let source_time = frame.SystemRelativeTime().map_err(error)?.Duration;
        let surface = frame.Surface().map_err(error)?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(error)?;
        unsafe {
            let texture: ID3D11Texture2D = access.GetInterface().map_err(error)?;
            let r = self.region;
            let region = D3D11_BOX {
                left: r.x,
                top: r.y,
                front: 0,
                right: r.x + r.width,
                bottom: r.y + r.height,
                back: 1,
            };
            self.context.CopySubresourceRegion(
                &self.staging,
                0,
                0,
                0,
                0,
                &texture,
                0,
                Some(&region),
            );
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(error)?;
            let mut image = super::super::frame::BgraImage::new(r.width, r.height);
            // RowPitch 可能包含行尾对齐；整行复制 BGRA，不逐像素交换通道。
            for y in 0..r.height as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    r.width as usize * 4,
                );
                let target =
                    &mut image.data[y * r.width as usize * 4..(y + 1) * r.width as usize * 4];
                target.copy_from_slice(row);
            }
            self.context.Unmap(&self.staging, 0);
            Ok(VideoFrame { source_time, image })
        }
    }
}

impl CaptureBackend for Capture {
    fn desktop_origin(&self) -> Result<(i32, i32), String> {
        match self.source {
            NativeSource::Monitor { x, y } => Ok((x, y)),
            NativeSource::Window { id, .. } => {
                let rect = window_bounds(id)?;
                Ok((rect.left, rect.top))
            }
        }
    }
    fn check_source(&self) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("录制来源已关闭。".into());
        }
        if let NativeSource::Window { id, pid } = self.source {
            unsafe {
                let hwnd = HWND(id as usize as *mut _);
                let mut current_pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut current_pid));
                if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() || current_pid != pid
                {
                    return Err("录制窗口已关闭或最小化，录制已结束。".into());
                }
            }
            let bounds = window_bounds(id)?;
            if Some((bounds.right - bounds.left, bounds.bottom - bounds.top)) != self.window_size {
                return Err("录制窗口尺寸已改变，录制已结束；请重新选择来源。".into());
            }
        }
        let size = self.item.Size().map_err(error)?;
        if (size.Width as u32, size.Height as u32) != self.size {
            return Err("录制来源尺寸已改变，录制已结束；请重新选择来源。".into());
        }
        Ok(())
    }
    fn refresh(&mut self) -> Result<(), String> {
        if self.started {
            // 新会话首次 StartCapture 会提供当前桌面，旧池随赋值释放并注销回调。
            // 创建失败交给 manager 统一结束，不能回退到过期底图。
            *self = Self::new(&self.config, self.size, self.source)?;
        }
        Ok(())
    }

    fn wait_for_frame(&self, timeout: std::time::Duration) {
        let _ = self.arrived.recv_timeout(timeout);
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, String> {
        self.check_source()?;
        // 待开始只准备设备和帧池，用户点击开始后第一次拉帧才启动系统捕获。
        if !self.started {
            self.session.StartCapture().map_err(error)?;
            self.started = true;
        }
        if self.closed.load(Ordering::Acquire) {
            return Err("录屏来源已关闭，请重新选择显示器。".into());
        }
        unsafe {
            self.device.GetDeviceRemovedReason().map_err(error)?;
        }
        let frame = match self.pool.TryGetNextFrame() {
            Ok(frame) => frame,
            // windows-core 0.62 把成功返回的空对象映射为 Error::empty（code 为 S_OK）。
            // 只忽略此空帧结果；设备丢失和权限错误仍立即上报。
            Err(e) if e.code().0 == 0 => return Ok(None),
            Err(e) => return Err(error(e)),
        };
        let result = self.copy(&frame);
        let closed = frame.Close().map_err(error);
        let value = result?;
        closed?;
        Ok(Some(value))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.arrived_token);
        let _ = self.session.Close();
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.pool.Close();
    }
}
