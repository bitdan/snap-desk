//! macOS 控件级吸附：在独立线程按“辅助功能”树读取控件几何，不读取名称、文本或值。
//!
//! 与 Windows 的 UIA 路径一致：总预算 240 毫秒、每个窗口 80 毫秒，节点数与深度受限，
//! 鼠标所在的窗口优先；超时后只保留已识别的候选，不让无响应的应用拖慢截图。
//! 坐标均为全局点（左上角为原点），乘以截图缩放即为截图像素。
use super::{Elements, NativeWindow, Target};
use crate::capture::Rect;
use std::{
    collections::VecDeque,
    ffi::{c_char, c_void, CString},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

const BUDGET: Duration = Duration::from_millis(240);
const WINDOW_BUDGET: Duration = Duration::from_millis(80);
const MAX_NODES: usize = 1024;
const MAX_TARGETS: usize = 512;
const MAX_DEPTH: usize = 16;
/// 每次跨进程请求的超时（秒）；应用无响应时尽快放弃。
const MESSAGING_TIMEOUT: f32 = 0.08;
static RUNNING: AtomicBool = AtomicBool::new(false);

type CFTypeRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

/// kAXValueCGPointType / kAXValueCGSizeType。
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;
/// kCFStringEncodingUTF8。
const UTF8: u32 = 0x0800_0100;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyElementAtPosition(application: CFTypeRef, x: f32, y: f32, element: *mut CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementCopyMultipleAttributeValues(
        element: CFTypeRef,
        attributes: CFTypeRef,
        options: u32,
        values: *mut CFTypeRef,
    ) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXUIElementGetTypeID() -> usize;
    fn AXValueGetType(value: CFTypeRef) -> u32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRetain(value: CFTypeRef) -> CFTypeRef;
    fn CFRelease(value: CFTypeRef);
    fn CFGetTypeID(value: CFTypeRef) -> usize;
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFArrayCreate(allocator: CFTypeRef, values: *const CFTypeRef, count: isize, callbacks: *const c_void) -> CFTypeRef;
    fn CFStringCreateWithCString(allocator: CFTypeRef, text: *const c_char, encoding: u32) -> CFTypeRef;
}

/// 持有 +1 引用的 CoreFoundation 对象，离开作用域时释放。
struct Cf(CFTypeRef);
impl Cf {
    fn owned(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
    /// 数组内的元素只是借用，需要先保留才能放进待遍历队列。
    fn retained(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then(|| Self(unsafe { CFRetain(value) }))
    }
    fn string(text: &str) -> Option<Self> {
        let text = CString::new(text).ok()?;
        Self::owned(unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) })
    }
}
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

/// 只读的属性名常量与批量查询数组，整个扫描线程内复用。
struct Attributes {
    _names: [Cf; 3],
    windows: Cf,
    /// [AXPosition, AXSize, AXChildren]
    geometry: Cf,
}
impl Attributes {
    fn new() -> Option<Self> {
        let position = Cf::string("AXPosition")?;
        let size = Cf::string("AXSize")?;
        let children = Cf::string("AXChildren")?;
        let windows = Cf::string("AXWindows")?;
        let values = [position.0, size.0, children.0];
        // 不传回调：数组不保留元素，属性名由 _names 持有到线程结束。
        let geometry = Cf::owned(unsafe {
            CFArrayCreate(std::ptr::null(), values.as_ptr(), 3, std::ptr::null())
        })?;
        Some(Self {
            _names: [position, size, children],
            windows,
            geometry,
        })
    }
}

/// 一次跨进程请求读出位置、尺寸与子节点；读取失败的项为错误值，按缺失处理。
fn geometry(element: CFTypeRef, attributes: &Attributes) -> (Option<(f64, f64, f64, f64)>, Vec<Cf>) {
    let mut values: CFTypeRef = std::ptr::null();
    let status = unsafe {
        AXUIElementCopyMultipleAttributeValues(element, attributes.geometry.0, 0, &mut values)
    };
    let Some(values) = (status == 0).then(|| Cf::owned(values)).flatten() else {
        return (None, Vec::new());
    };
    let item = |index| unsafe {
        (CFArrayGetCount(values.0) > index)
            .then(|| CFArrayGetValueAtIndex(values.0, index))
            .filter(|value| !value.is_null())
    };
    let mut point = CGPoint::default();
    let mut size = CGSize::default();
    let bounds = (ax_value(item(0), AX_POINT, &mut point) && ax_value(item(1), AX_SIZE, &mut size))
        .then_some((point.x, point.y, size.width, size.height));
    let children = item(2).map(array_items).unwrap_or_default();
    (bounds, children)
}

fn ax_value<T>(value: Option<CFTypeRef>, kind: u32, out: &mut T) -> bool {
    value.is_some_and(|value| unsafe {
        CFGetTypeID(value) == AXValueGetTypeID()
            && AXValueGetType(value) == kind
            && AXValueGetValue(value, kind, (out as *mut T).cast())
    })
}

fn array_items(array: CFTypeRef) -> Vec<Cf> {
    unsafe {
        if CFGetTypeID(array) != CFArrayGetTypeID() {
            return Vec::new();
        }
        let count = CFArrayGetCount(array).max(0).min(MAX_NODES as isize);
        (0..count)
            .filter_map(|index| Cf::retained(CFArrayGetValueAtIndex(array, index)))
            .collect()
    }
}

/// 扫描所需的窗口身份：进程、点坐标边界、候选序号与截图像素边界。
struct Root {
    pid: i32,
    points: (f64, f64, f64, f64),
    index: usize,
    rect: Rect,
}

/// 为已识别的窗口候选补充控件候选；未授予“辅助功能”权限时直接跳过，不弹授权框。
pub(super) fn collect(
    windows: &[NativeWindow],
    targets: &mut Vec<Target>,
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
) {
    if !crate::macos::accessibility_trusted() {
        return;
    }
    let mut roots: Vec<Root> = targets
        .iter()
        .filter(|target| target.kind == "window")
        .filter_map(|target| {
            let window = windows.get(target.window_index)?;
            Some(Root {
                pid: i32::try_from(window.pid().ok()?).ok()?,
                points: (
                    f64::from(window.x().ok()?),
                    f64::from(window.y().ok()?),
                    f64::from(window.width().ok()?),
                    f64::from(window.height().ok()?),
                ),
                index: target.window_index,
                rect: target.rect,
            })
        })
        .collect();
    if roots.is_empty()
        || RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    // 先查鼠标所在的最上层窗口，其余保持原先的前后顺序。
    if let Some((cx, cy)) = crate::macos::cursor_point() {
        let x = (cx * scale).round() as i64 - i64::from(origin.0);
        let y = (cy * scale).round() as i64 - i64::from(origin.1);
        if let Some(position) = roots.iter().position(|root| {
            let r = root.rect;
            x >= i64::from(r.x)
                && y >= i64::from(r.y)
                && x < i64::from(r.x) + i64::from(r.width)
                && y < i64::from(r.y) + i64::from(r.height)
        }) {
            let root = roots.remove(position);
            roots.insert(0, root);
        }
    }
    let deadline = Instant::now() + BUDGET;
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = cancelled.clone();
    let (sender, receiver) = mpsc::sync_channel(MAX_TARGETS);
    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            RUNNING.store(false, Ordering::Release);
        }
    }
    // 辅助功能对象只在扫描线程内创建和释放；跨线程的只有纯几何候选。
    if std::thread::Builder::new()
        .name("capture-accessibility".into())
        .spawn(move || {
            let _running = Running;
            scan(roots, origin, size, scale, deadline, &stop, &sender);
        })
        .is_err()
    {
        RUNNING.store(false, Ordering::Release);
        return;
    }
    let mut received = 0;
    while received < MAX_TARGETS && targets.len() < 4096 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(target) = receiver.recv_timeout(remaining) else {
            break;
        };
        received += 1;
        if !targets.iter().any(|existing| {
            existing.window_index == target.window_index
                && super::same_bounds(existing.rect, target.rect)
        }) {
            targets.push(target);
        }
    }
    cancelled.store(true, Ordering::Release);
}

/// 同一应用的窗口按边界匹配（误差 2 点内）；系统没有公开从窗口号取辅助功能对象的接口。
fn matching_window(app: &Cf, points: (f64, f64, f64, f64), attributes: &Attributes) -> Option<Cf> {
    let mut list: CFTypeRef = std::ptr::null();
    if unsafe { AXUIElementCopyAttributeValue(app.0, attributes.windows.0, &mut list) } != 0 {
        return None;
    }
    let list = Cf::owned(list)?;
    array_items(list.0).into_iter().find(|window| {
        geometry_only(window.0, attributes).is_some_and(|(x, y, w, h)| {
            (x - points.0).abs() <= 2.0
                && (y - points.1).abs() <= 2.0
                && (w - points.2).abs() <= 2.0
                && (h - points.3).abs() <= 2.0
        })
    })
}

fn geometry_only(element: CFTypeRef, attributes: &Attributes) -> Option<(f64, f64, f64, f64)> {
    geometry(element, attributes).0
}

fn scan(
    roots: Vec<Root>,
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
    deadline: Instant,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Target>,
) {
    let Some(attributes) = Attributes::new() else {
        return;
    };
    let active = || !cancelled.load(Ordering::Acquire) && Instant::now() < deadline;
    let px = |value: f64| (value * scale).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    let mut visited = 0;
    let mut emitted = 0;
    for root in roots.into_iter().take(16) {
        if !active() || visited >= MAX_NODES || emitted >= MAX_TARGETS {
            break;
        }
        let Some(app) = Cf::owned(unsafe { AXUIElementCreateApplication(root.pid) }) else {
            continue;
        };
        unsafe {
            AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT);
        }
        let window_deadline = (Instant::now() + WINDOW_BUDGET).min(deadline);
        let Some(window) = matching_window(&app, root.points, &attributes) else {
            continue;
        };
        let mut state = Elements {
            targets: Vec::new(),
            index: root.index,
            parent: root.rect,
            origin,
            size,
        };
        let mut queue = VecDeque::from([(window, 0usize)]);
        while let Some((node, depth)) = queue.pop_front() {
            if !active()
                || Instant::now() >= window_deadline
                || visited >= MAX_NODES
                || emitted >= MAX_TARGETS
            {
                break;
            }
            visited += 1;
            let (bounds, children) = geometry(node.0, &attributes);
            if let Some((x, y, w, h)) = bounds.filter(|_| depth > 0) {
                let previous = state.targets.len();
                state.push_edges(px(x), px(y), px(x + w), px(y + h));
                if state.targets.len() > previous {
                    if sender
                        .try_send(state.targets.last().unwrap().clone())
                        .is_err()
                    {
                        return;
                    }
                    emitted += 1;
                }
            }
            // 与整窗重合的容器也继续下钻：网页与分栏视图常把内容放在这类容器里。
            if depth + 1 < MAX_DEPTH && queue.len() < MAX_NODES {
                queue.extend(children.into_iter().map(|child| (child, depth + 1)));
            }
        }
    }
}

/// 悬停探测所需的窗口身份：只含进程与几何，可跨线程。
#[derive(Clone, Debug)]
pub struct Probe {
    roots: Vec<(i32, usize, Rect)>,
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
}

/// 由已识别的窗口候选生成悬停探测请求；未授予“辅助功能”权限时没有请求。
pub(super) fn probe_request(
    windows: &[NativeWindow],
    targets: &[Target],
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
) -> Option<Probe> {
    if !crate::macos::accessibility_trusted() {
        return None;
    }
    let roots: Vec<_> = targets
        .iter()
        .filter(|target| target.kind == "window")
        .filter_map(|target| {
            let pid = i32::try_from(windows.get(target.window_index)?.pid().ok()?).ok()?;
            Some((pid, target.window_index, target.rect))
        })
        .collect();
    (!roots.is_empty()).then_some(Probe { roots, origin, size, scale })
}

const PROBE_BUDGET: Duration = Duration::from_millis(150);
const PROBE_DEPTH: usize = 32;
const PROBE_IDLE: Duration = Duration::from_secs(30);

struct ProbeJob {
    pid: i32,
    index: usize,
    parent: Rect,
    /// 鼠标的全局点坐标。
    point: (f64, f64),
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
    reply: mpsc::Sender<Vec<Target>>,
}

/// 常驻探测线程（空闲 30 秒退出，下次探测再启动）。
static PROBER: Mutex<Option<mpsc::Sender<ProbeJob>>> = Mutex::new(None);

impl Probe {
    /// 截图像素 `point` 处、属于 `window_index` 窗口的控件链：系统按位置直接给出最内层控件，
    /// 再沿 AXParent 向外走到窗口为止。只命中该应用自己的界面，不受截图浮层遮挡。
    pub fn probe(&self, window_index: usize, point: (u32, u32)) -> Vec<Target> {
        let Some(&(pid, index, parent)) = self.roots.iter().find(|(_, index, _)| *index == window_index) else {
            return Vec::new();
        };
        let global = (
            (f64::from(self.origin.0) + f64::from(point.0) + 0.5) / self.scale,
            (f64::from(self.origin.1) + f64::from(point.1) + 0.5) / self.scale,
        );
        let (reply, result) = mpsc::channel();
        let job = ProbeJob {
            pid,
            index,
            parent,
            point: global,
            origin: self.origin,
            size: self.size,
            scale: self.scale,
            reply,
        };
        {
            let mut prober = PROBER.lock().unwrap_or_else(|e| e.into_inner());
            let job = match prober.as_ref() {
                Some(sender) => sender.send(job).err().map(|mpsc::SendError(job)| job),
                None => Some(job),
            };
            if let Some(job) = job {
                let (sender, jobs) = mpsc::channel();
                if std::thread::Builder::new()
                    .name("capture-accessibility-probe".into())
                    .spawn(move || prober_loop(jobs))
                    .is_err()
                {
                    return Vec::new();
                }
                let _ = sender.send(job);
                *prober = Some(sender);
            }
        }
        result
            .recv_timeout(PROBE_BUDGET + Duration::from_millis(100))
            .unwrap_or_default()
    }
}

/// [AXPosition, AXSize, AXParent, AXRole] 与用于比较的 "AXWindow"。
struct ChainAttributes {
    _names: [Cf; 4],
    query: Cf,
    window_role: Cf,
}
impl ChainAttributes {
    fn new() -> Option<Self> {
        let names = [
            Cf::string("AXPosition")?,
            Cf::string("AXSize")?,
            Cf::string("AXParent")?,
            Cf::string("AXRole")?,
        ];
        let values = [names[0].0, names[1].0, names[2].0, names[3].0];
        let query = Cf::owned(unsafe { CFArrayCreate(std::ptr::null(), values.as_ptr(), 4, std::ptr::null()) })?;
        Some(Self { _names: names, query, window_role: Cf::string("AXWindow")? })
    }
}

fn prober_loop(jobs: mpsc::Receiver<ProbeJob>) {
    let Some(attributes) = ChainAttributes::new() else {
        return;
    };
    while let Ok(mut job) = jobs.recv_timeout(PROBE_IDLE) {
        // 鼠标移动快于探测时只处理最新一次。
        while let Ok(newer) = jobs.try_recv() {
            let _ = job.reply.send(Vec::new());
            job = newer;
        }
        let found = chain(&job, &attributes);
        let _ = job.reply.send(found);
    }
}

fn chain(job: &ProbeJob, attributes: &ChainAttributes) -> Vec<Target> {
    let deadline = Instant::now() + PROBE_BUDGET;
    let Some(app) = Cf::owned(unsafe { AXUIElementCreateApplication(job.pid) }) else {
        return Vec::new();
    };
    unsafe {
        AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT);
    }
    let mut hit: CFTypeRef = std::ptr::null();
    let status =
        unsafe { AXUIElementCopyElementAtPosition(app.0, job.point.0 as f32, job.point.1 as f32, &mut hit) };
    let Some(mut node) = (status == 0).then(|| Cf::owned(hit)).flatten() else {
        return Vec::new();
    };
    let px = |value: f64| (value * job.scale).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    let mut state = Elements { targets: Vec::new(), index: job.index, parent: job.parent, origin: job.origin, size: job.size };
    for _ in 0..PROBE_DEPTH {
        if Instant::now() >= deadline {
            break;
        }
        let mut values: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyMultipleAttributeValues(node.0, attributes.query.0, 0, &mut values) };
        let Some(values) = (status == 0).then(|| Cf::owned(values)).flatten() else {
            break;
        };
        let item = |index| unsafe {
            (CFArrayGetCount(values.0) > index)
                .then(|| CFArrayGetValueAtIndex(values.0, index))
                .filter(|value| !value.is_null())
        };
        // 走到窗口即停：窗口本身已是一级吸附候选。
        if item(3).is_some_and(|role| unsafe { CFEqual(role, attributes.window_role.0) } != 0) {
            break;
        }
        let mut point = CGPoint::default();
        let mut size = CGSize::default();
        if ax_value(item(0), AX_POINT, &mut point) && ax_value(item(1), AX_SIZE, &mut size) {
            state.push_edges(px(point.x), px(point.y), px(point.x + size.width), px(point.y + size.height));
        }
        // AXParent 读取失败时是错误值而不是元素，类型不对就停止。
        let Some(parent) = item(2)
            .filter(|value| unsafe { CFGetTypeID(*value) == AXUIElementGetTypeID() })
            .and_then(Cf::retained)
        else {
            break;
        };
        node = parent;
    }
    state.targets
}
