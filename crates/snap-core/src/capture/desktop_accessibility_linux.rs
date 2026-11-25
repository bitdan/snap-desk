//! Linux（X11）悬停探测：经 AT-SPI（无障碍 D-Bus 总线）查出鼠标处由外到内的控件链。
//!
//! 只读控件几何，不读名称、文本或值。系统未开启无障碍服务、应用不提供 AT-SPI 信息，
//! 或应用报告的坐标与窗口对不上（GTK4、XWayland 下常见）时返回空结果，
//! 由调用方改用按画面识别。Wayland 没有全局坐标，不使用这里。
use super::{Elements, NativeWindow, Rect, Target};
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use zbus::{
    blocking::{connection::Builder, Connection, Proxy},
    zvariant::OwnedObjectPath,
};

const PROBE_BUDGET: Duration = Duration::from_millis(150);
const PROBE_DEPTH: usize = 32;
const PROBE_IDLE: Duration = Duration::from_secs(30);
/// 单次 D-Bus 调用的超时；应用无响应时尽快放弃。
const CALL_TIMEOUT: Duration = Duration::from_millis(80);
/// AT-SPI 坐标类型：屏幕坐标。
const SCREEN: u32 = 0;
const NULL_PATH: &str = "/org/a11y/atspi/null";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";

/// 悬停探测所需的窗口身份：只含进程与几何，可跨线程。
#[derive(Clone, Debug)]
pub struct Probe {
    roots: Vec<(u32, usize, Rect)>,
    origin: (i32, i32),
    size: (u32, u32),
}

/// 由已识别的窗口候选生成悬停探测请求；Wayland 会话没有请求。
pub(super) fn probe_request(
    windows: &[NativeWindow],
    targets: &[Target],
    origin: (i32, i32),
    size: (u32, u32),
) -> Option<Probe> {
    if super::linux::wayland() {
        return None;
    }
    let roots: Vec<_> = targets
        .iter()
        .filter(|target| target.kind == "window")
        .filter_map(|target| Some((windows.get(target.window_index)?.pid().ok()?, target.window_index, target.rect)))
        .collect();
    (!roots.is_empty()).then_some(Probe { roots, origin, size })
}

struct ProbeJob {
    pid: u32,
    index: usize,
    parent: Rect,
    point: (i32, i32),
    origin: (i32, i32),
    size: (u32, u32),
    reply: mpsc::Sender<Vec<Target>>,
}

/// 常驻探测线程（空闲 30 秒退出，下次探测再启动），复用无障碍总线连接与进程到应用的映射。
static PROBER: Mutex<Option<mpsc::Sender<ProbeJob>>> = Mutex::new(None);

impl Probe {
    /// 截图像素 `point` 处、属于 `window_index` 窗口的控件链（阻塞，上限约 250 毫秒）。
    pub fn probe(&self, window_index: usize, point: (u32, u32)) -> Vec<Target> {
        let Some(&(pid, index, parent)) = self.roots.iter().find(|(_, index, _)| *index == window_index) else {
            return Vec::new();
        };
        let global = (
            self.origin.0.saturating_add(point.0 as i32),
            self.origin.1.saturating_add(point.1 as i32),
        );
        let (reply, result) = mpsc::channel();
        let job = ProbeJob { pid, index, parent, point: global, origin: self.origin, size: self.size, reply };
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

/// 连接无障碍总线；系统未开启无障碍服务时失败（不替用户开启）。
fn connect() -> Option<Connection> {
    let session = Connection::session().ok()?;
    let bus = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus").ok()?;
    let address: String = bus.call("GetAddress", &()).ok()?;
    Builder::address(address.as_str()).ok()?.method_timeout(CALL_TIMEOUT).build().ok()
}

type Node = (String, OwnedObjectPath);

struct Walker {
    connection: Connection,
    /// 进程号到其 AT-SPI 应用根节点；找不到时重新列举一次。
    apps: HashMap<u32, Node>,
}

impl Walker {
    fn call<B, R>(&self, node: &Node, interface: &str, method: &str, body: &B) -> Option<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        let proxy = Proxy::new(&self.connection, node.0.as_str(), node.1.as_str(), interface).ok()?;
        proxy.call(method, body).ok()
    }

    fn children(&self, node: &Node) -> Vec<Node> {
        self.call(node, ACCESSIBLE, "GetChildren", &()).unwrap_or_default()
    }

    fn extents(&self, node: &Node) -> Option<(i32, i32, i32, i32)> {
        self.call(node, COMPONENT, "GetExtents", &(SCREEN,))
    }

    fn app(&mut self, pid: u32) -> Option<Node> {
        if let Some(app) = self.apps.get(&pid) {
            return Some(app.clone());
        }
        let registry = ("org.a11y.atspi.Registry".to_owned(), OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root").ok()?);
        let dbus = Proxy::new(&self.connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").ok()?;
        self.apps.clear();
        for app in self.children(&registry).into_iter().take(256) {
            if let Ok(owner) = dbus.call::<_, _, u32>("GetConnectionUnixProcessID", &(app.0.as_str(),)) {
                self.apps.entry(owner).or_insert(app);
            }
        }
        self.apps.get(&pid).cloned()
    }

    fn walk(&mut self, job: &ProbeJob) -> Vec<Target> {
        let deadline = Instant::now() + PROBE_BUDGET;
        let Some(app) = self.app(job.pid) else {
            return Vec::new();
        };
        let (x, y) = job.point;
        // 窗口的全局物理边界；应用报告的框架坐标必须与之吻合，否则说明坐标不可信。
        let window = (
            job.origin.0 + job.parent.x as i32,
            job.origin.1 + job.parent.y as i32,
            job.parent.width as i32,
            job.parent.height as i32,
        );
        let matches_window = |(fx, fy, fw, fh): (i32, i32, i32, i32)| {
            const SLACK: i32 = 64;
            (fx - window.0).abs() <= SLACK
                && (fy - window.1).abs() <= SLACK
                && (fw - window.2).abs() <= SLACK
                && (fh - window.3).abs() <= SLACK
        };
        let Some(mut node) = self
            .children(&app)
            .into_iter()
            .take(64)
            .find(|frame| self.extents(frame).is_some_and(matches_window))
        else {
            return Vec::new();
        };
        let mut state = Elements { targets: Vec::new(), index: job.index, parent: job.parent, origin: job.origin, size: job.size };
        for _ in 0..PROBE_DEPTH {
            if Instant::now() >= deadline {
                break;
            }
            let Some(child) = self.call::<_, Node>(&node, COMPONENT, "GetAccessibleAtPoint", &(x, y, SCREEN)) else {
                break;
            };
            if child.1.as_str() == NULL_PATH || child == node {
                break;
            }
            if let Some((cx, cy, cw, ch)) = self.extents(&child) {
                state.push_edges(cx, cy, cx.saturating_add(cw), cy.saturating_add(ch));
            }
            node = child;
        }
        state.targets
    }
}

fn prober_loop(jobs: mpsc::Receiver<ProbeJob>) {
    let Some(connection) = connect() else {
        // 无障碍服务未开启：把已排队和之后的请求都答复为空，空闲后退出。
        while let Ok(job) = jobs.recv_timeout(PROBE_IDLE) {
            let _ = job.reply.send(Vec::new());
        }
        return;
    };
    let mut walker = Walker { connection, apps: HashMap::new() };
    while let Ok(mut job) = jobs.recv_timeout(PROBE_IDLE) {
        while let Ok(newer) = jobs.try_recv() {
            let _ = job.reply.send(Vec::new());
            job = newer;
        }
        let found = walker.walk(&job);
        let _ = job.reply.send(found);
    }
}
