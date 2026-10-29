//! 在独立 MTA 线程读取控件几何；不读取名称、文本或值，不让外部无响应控件阻塞截图。
use super::Elements;
use crate::capture::{desktop::Target, Rect};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, POINT},
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
    UI::{Accessibility::*, WindowsAndMessaging::GetCursorPos},
};

const BUDGET: Duration = Duration::from_millis(240);
const MAX_NODES: usize = 1024;
const MAX_TARGETS: usize = 512;
static RUNNING: AtomicBool = AtomicBool::new(false);

/// 整个进程最多一个探测线程；提供者超时后仍卡住时，后续截图直接使用 HWND 回退。
struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

/// 接收已完成的候选而不是等待整个树；达到总预算立即停止等待，保留局部识别结果。
pub(super) fn collect(
    mut roots: Vec<(u32, usize, Rect)>,
    origin: (i32, i32),
    size: (u32, u32),
) -> Vec<Target> {
    if roots.is_empty()
        || RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return Vec::new();
    }
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_ok() {
        let x = i64::from(cursor.x) - i64::from(origin.0);
        let y = i64::from(cursor.y) - i64::from(origin.1);
        // 先查鼠标所在的最上层窗口，保持原 window_index，避免改变遮挡关系。
        if let Some(index) = roots.iter().position(|(_, _, r)| {
            x >= i64::from(r.x)
                && y >= i64::from(r.y)
                && x < i64::from(r.x) + i64::from(r.width)
                && y < i64::from(r.y) + i64::from(r.height)
        }) {
            let root = roots.remove(index);
            roots.insert(0, root);
        }
    }
    let deadline = Instant::now() + BUDGET;
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = cancelled.clone();
    let (sender, receiver) = mpsc::sync_channel(MAX_TARGETS);
    // 所有 COM 对象在辅助线程创建、使用和销毁；只有纯几何候选跨线程。
    if std::thread::Builder::new()
        .name("capture-accessibility".into())
        .spawn(move || {
            let _running = Running;
            let _ = scan(roots, origin, size, deadline, &stop, &sender);
        })
        .is_err()
    {
        RUNNING.store(false, Ordering::Release);
        return Vec::new();
    }
    let mut targets = Vec::new();
    while targets.len() < MAX_TARGETS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(target) => targets.push(target),
            Err(_) => break,
        }
    }
    cancelled.store(true, Ordering::Release);
    targets
}

/// 每层只请求几何缓存，不读取控件内容；预算、深度和节点数共同约束复杂网页。
fn scan(
    roots: Vec<(u32, usize, Rect)>,
    origin: (i32, i32),
    size: (u32, u32),
    deadline: Instant,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Target>,
) -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _apartment = Apartment;
    let _dpi = match super::PhysicalDpi::enter() {
        Ok(dpi) => dpi,
        Err(_) => return Ok(()),
    };
    let automation: IUIAutomation2 =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)? };
    unsafe {
        automation.SetConnectionTimeout(80)?;
        automation.SetTransactionTimeout(80)?;
    }
    let cache = unsafe { automation.CreateCacheRequest()? };
    // 遍历原始树，不能只筛 ControlView 的直接子节点：WebView 常在非控件容器下放正文/面板。
    let condition = unsafe { automation.CreateTrueCondition()? };
    unsafe {
        cache.SetTreeScope(TreeScope_Element)?;
        cache.SetTreeFilter(&condition)?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        cache.AddProperty(UIA_IsOffscreenPropertyId)?;
    }
    let active = || !cancelled.load(Ordering::Acquire) && Instant::now() < deadline;
    let mut visited = 0;
    let mut emitted = 0;
    for (id, index, parent) in roots.into_iter().take(16) {
        if !active() || visited >= MAX_NODES || emitted >= MAX_TARGETS {
            break;
        }
        let Ok(root) = (unsafe { automation.ElementFromHandle(HWND(id as usize as *mut _)) })
        else {
            continue;
        };
        let mut queue = VecDeque::from([(root, 0usize)]);
        let mut state = Elements {
            targets: Vec::new(),
            index,
            parent,
            origin,
            size,
        };
        // 一棵树最多占用总预算的一部分，给其他可见应用留出识别机会。
        let window_deadline = (Instant::now() + Duration::from_millis(80)).min(deadline);
        while let Some((node, depth)) = queue.pop_front() {
            if !active()
                || Instant::now() >= window_deadline
                || visited >= MAX_NODES
                || emitted >= MAX_TARGETS
            {
                break;
            }
            if depth >= 16 {
                continue;
            }
            let Ok(children) =
                (unsafe { node.FindAllBuildCache(TreeScope_Children, &condition, &cache) })
            else {
                continue;
            };
            let Ok(length) = (unsafe { children.Length() }) else {
                continue;
            };
            let length = length.max(0) as usize;
            for child_index in 0..length.min(MAX_NODES - visited) {
                if !active() || Instant::now() >= window_deadline || emitted >= MAX_TARGETS {
                    break;
                }
                visited += 1;
                let Ok(child) = (unsafe { children.GetElement(child_index as i32) }) else {
                    continue;
                };
                // 离屏的整棵子树不参与当前可见截图；同尺寸容器仍继续下钻。
                if unsafe { child.CachedIsOffscreen() }.is_ok_and(|value| value.as_bool()) {
                    continue;
                }
                if let Ok(bounds) = unsafe { child.CachedBoundingRectangle() } {
                    let previous = state.targets.len();
                    state.push(bounds);
                    if state.targets.len() > previous {
                        if sender
                            .try_send(state.targets.last().unwrap().clone())
                            .is_err()
                        {
                            return Ok(());
                        }
                        emitted += 1;
                    }
                }
                queue.push_back((child, depth + 1));
            }
        }
    }
    Ok(())
}
