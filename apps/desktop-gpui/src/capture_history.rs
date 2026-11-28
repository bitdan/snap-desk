//! 截图会话回放：只在内存保留成功截图的压缩底图、选区和标注，
//! 独立于剪贴板历史。`,` 回到上一张、`.` 前往下一张，越过最新一张时回到本次现场，不重新抓屏。
//! F5 刷新（[`refresh`]）则重新抓屏替换现场画面，与回放共用换图逻辑。
//!
//! 回放的画面没有吸附候选与窗口来源（历史 HWND 已失效），不执行“截图后动作”，也不再计入历史。
use super::{current, display_frame, open_overlays, Overlay, Session};
use gpui_kit::*;
use image::RgbaImage;
use snap_core::{
    annotations::{edit::Bounds as ImageBounds, Annotation},
    capture::{Display, Rect},
};
use std::{collections::VecDeque, sync::Arc};

const MAX_ENTRIES: usize = 20;
const MAX_BYTES: usize = 128 * 1024 * 1024;
/// 超过该像素数的截图不加入历史。
const MAX_PIXELS: u64 = 32_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Previous,
    Next,
}

/// 冻结画面与其坐标系；回放与返回现场时整体替换。
#[derive(Clone)]
struct Frame {
    origin: (i32, i32),
    displays: Vec<Display>,
    layout_key: String,
}

struct Snapshot {
    frame: Frame,
    png: Arc<Vec<u8>>,
    rect: Rect,
    shapes: Vec<Annotation>,
}

/// 开始回放前的现场；向前越过最新历史时恢复。
struct Live {
    frame: Frame,
    image: Arc<RgbaImage>,
    targets: Vec<crate::selection::Snap>,
    window_targets: Vec<crate::recording::WindowTarget>,
    selection: Option<Rect>,
    shapes: Vec<Annotation>,
}

#[derive(Default)]
struct History {
    entries: VecDeque<Snapshot>,
    cursor: Option<usize>,
    live: Option<Live>,
}
impl Global for History {}

impl History {
    /// 始终保留较新的条目；容量按压缩字节计，绝不靠条数掩盖大图内存。
    fn trim(&mut self) {
        trim(&mut self.entries, |s| s.png.len(), MAX_ENTRIES, MAX_BYTES);
    }
}

fn trim<T>(
    entries: &mut VecDeque<T>,
    size: impl Fn(&T) -> usize,
    max_entries: usize,
    max_bytes: usize,
) {
    while entries.len() > max_entries || entries.iter().map(&size).sum::<usize>() > max_bytes {
        entries.pop_front();
    }
}

/// 下一个位置：`None` 为现场。
fn destination(len: usize, cursor: Option<usize>, direction: Direction) -> Option<usize> {
    match (direction, cursor) {
        (Direction::Previous, None) => len.checked_sub(1),
        (Direction::Previous, Some(index)) => Some(index.saturating_sub(1)),
        (Direction::Next, Some(index)) if index + 1 < len => Some(index + 1),
        (Direction::Next, _) => None,
    }
}

/// 回放中显示的位置（从 1 开始）与总数。
pub fn position(cx: &App) -> Option<(usize, usize)> {
    let history = cx.try_global::<History>()?;
    history
        .cursor
        .map(|index| (index + 1, history.entries.len()))
}

pub fn has_entries(cx: &App) -> bool {
    cx.try_global::<History>()
        .is_some_and(|h| !h.entries.is_empty())
}

/// 新会话开始或会话结束：回到现场状态。
pub fn reset(cx: &mut App) {
    if cx.has_global::<History>() {
        let history = cx.global_mut::<History>();
        history.cursor = None;
        history.live = None;
    }
}

/// 复制、保存或贴图成功后记住本次截图；压缩在后台进行，失败只记日志，不影响导出结果。
pub fn remember(cx: &mut App, session: &Entity<Session>) {
    let s = session.read(cx);
    let Some(rect) = s.selection else {
        return;
    };
    if s.recording || position(cx).is_some() {
        return;
    }
    let image = s.image.clone();
    if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
        return;
    }
    let frame = Frame {
        origin: s.origin,
        displays: s.displays.clone(),
        layout_key: s.layout_key.clone(),
    };
    let shapes = s.editor.shapes().to_vec();
    cx.spawn(async move |cx| {
        let encoded = cx
            .background_executor()
            .spawn(async move { super::png(&image) })
            .await;
        match encoded {
            Ok(png) if png.len() <= MAX_BYTES => cx.update(|cx| {
                let history = cx.default_global::<History>();
                history.entries.push_back(Snapshot {
                    frame,
                    png: Arc::new(png),
                    rect,
                    shapes,
                });
                history.trim();
            }),
            Ok(_) => log::info!("event=gpui.capture_history_skipped reason=too_large"),
            Err(error) => log::warn!("event=gpui.capture_history_failed message={error}"),
        }
    })
    .detach();
}

/// 切换到上一张或下一张。只在空闲的截图会话中生效（录屏框选、导出中、正在绘制或输入时忽略）。
pub fn replay(cx: &mut App, direction: Direction) {
    let Some(session) = current(cx) else {
        return;
    };
    {
        let s = session.read(cx);
        if s.recording || s.busy || s.drag.is_some() || s.editor.pending || s.editor.interacting() {
            return;
        }
    }
    let Some(history) = cx.try_global::<History>() else {
        return;
    };
    let next = destination(history.entries.len(), history.cursor, direction);
    if history.entries.is_empty() || next == history.cursor {
        return;
    }
    match next {
        Some(index) => {
            let png = history.entries[index].png.clone();
            session.update(cx, |s, _| s.busy = true);
            cx.spawn(async move |cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        image::load_from_memory(&png)
                            .map(|image| image.to_rgba8())
                            .map_err(|e| format!("读取截图历史失败：{e}"))
                    })
                    .await;
                cx.update(|cx| {
                    session.update(cx, |s, _| s.busy = false);
                    match decoded {
                        Ok(image) => show_entry(cx, &session, index, image),
                        Err(error) => {
                            log::warn!("event=gpui.capture_replay_failed message={error}")
                        }
                    }
                });
            })
            .detach();
        }
        None => show_live(cx, &session),
    }
}

fn show_entry(cx: &mut App, session: &Entity<Session>, index: usize, image: RgbaImage) {
    // 解码期间会话可能已结束或换了新会话。
    if current(cx).is_none_or(|active| active != *session) {
        return;
    }
    let history = cx.default_global::<History>();
    let Some(saved) = history.entries.get(index) else {
        return;
    };
    let (frame, rect, shapes) = (saved.frame.clone(), saved.rect, saved.shapes.clone());
    let total = history.entries.len();
    let leaving_live = history.cursor.is_none();
    history.cursor = Some(index);
    if leaving_live {
        let s = session.read(cx);
        let live = Live {
            frame: Frame {
                origin: s.origin,
                displays: s.displays.clone(),
                layout_key: s.layout_key.clone(),
            },
            image: s.image.clone(),
            targets: s.targets.clone(),
            window_targets: s.window_targets.clone(),
            selection: s.selection,
            shapes: s.editor.shapes().to_vec(),
        };
        cx.global_mut::<History>().live = Some(live);
    }
    log::info!(
        "event=gpui.capture_replay index={} total={total}",
        index + 1
    );
    apply(
        cx,
        session,
        frame,
        Arc::new(image),
        Vec::new(),
        Vec::new(),
        Some(rect),
        shapes,
    );
}

fn show_live(cx: &mut App, session: &Entity<Session>) {
    let history = cx.default_global::<History>();
    history.cursor = None;
    let Some(live) = history.live.take() else {
        return;
    };
    log::info!("event=gpui.capture_replay_live");
    apply(
        cx,
        session,
        live.frame,
        live.image,
        live.targets,
        live.window_targets,
        live.selection,
        live.shapes,
    );
}

/// 刷新截图（F5）：隐藏浮层重新拍屏，换上最新画面，保留选区与标注；显示器布局变了则清空重选。
/// 回放历史、录屏框选、导出中、正在绘制或输入时忽略。
pub fn refresh(cx: &mut App) {
    let Some(session) = current(cx) else {
        return;
    };
    {
        let s = session.read(cx);
        if s.recording || s.busy || s.drag.is_some() || s.editor.pending || s.editor.interacting() || s.editor.text.is_some() {
            return;
        }
    }
    if position(cx).is_some() {
        return;
    }
    let include_cursor = session.read(cx).include_cursor;
    let snap = crate::shell::settings(cx).screenshot_snap;
    let windows = session.update(cx, |s, _| {
        s.busy = true;
        s.windows.clone()
    });
    for handle in &windows {
        let _ = handle.update(cx, |_, window, _| crate::native::set_visible(window, false));
    }
    cx.spawn(async move |cx| {
        // 等桌面合成去掉浮层再拍屏，与开始截图时收起暂存架的等待相同。
        cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
        let started = std::time::Instant::now();
        let result = cx.background_executor().spawn(async move { super::grab(include_cursor, snap) }).await;
        let deferred = cx.update(|cx| {
            session.update(cx, |s, _| s.busy = false);
            // 拍屏期间会话可能已被取消。
            if current(cx).is_none_or(|active| active != session) {
                return None;
            }
            let deferred = match result {
                Ok(mut grabbed) => {
                    let deferred = grabbed.deferred.take();
                    let layout_key = super::recent::layout_key(&grabbed.displays);
                    let (selection, shapes) = {
                        let s = session.read(cx);
                        let same = s.layout_key == layout_key && s.origin == grabbed.origin && s.displays.len() == grabbed.displays.len();
                        if same { (s.selection, s.editor.shapes().to_vec()) } else { (None, Vec::new()) }
                    };
                    log::info!(
                        "event=gpui.capture_refreshed width={} height={} kept_selection={} capture_ms={}",
                        grabbed.image.width(),
                        grabbed.image.height(),
                        selection.is_some(),
                        started.elapsed().as_millis()
                    );
                    let frame = Frame { origin: grabbed.origin, displays: grabbed.displays, layout_key };
                    session.update(cx, |s, _| {
                        s.natives = grabbed.natives;
                        s.sources = grabbed.sources;
                        s.include_cursor = grabbed.include_cursor;
                        s.probe = grabbed.probe.take().map(Arc::new);
                        s.probe_seq += 1;
                        s.regions = None;
                    });
                    apply(cx, &session, frame, Arc::new(grabbed.image), grabbed.targets, grabbed.windows, selection, shapes);
                    deferred
                }
                Err(error) => {
                    log::warn!("event=gpui.capture_refresh_failed message={error}");
                    None
                }
            };
            // 布局不变时浮层就地换图，需重新显示；重建的浮层本就可见。
            let windows = session.read(cx).windows.clone();
            for handle in &windows {
                let _ = handle.update(cx, |_, window, _| crate::native::set_visible(window, true));
            }
            if let Some(handle) = windows.first() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
            deferred.map(|request| (session, request))
        });
        if let Some((session, request)) = deferred {
            super::complete_targets(cx, session, request).await;
        }
    })
    .detach();
}

/// 替换会话画面、选区与标注；显示器布局不变时就地换图，否则重建浮层。
#[allow(clippy::too_many_arguments)]
fn apply(
    cx: &mut App,
    session: &Entity<Session>,
    frame: Frame,
    image: Arc<RgbaImage>,
    targets: Vec<crate::selection::Snap>,
    window_targets: Vec<crate::recording::WindowTarget>,
    selection: Option<Rect>,
    shapes: Vec<Annotation>,
) {
    let same_layout = {
        let s = session.read(cx);
        s.layout_key == frame.layout_key
            && s.origin == frame.origin
            && s.displays.len() == frame.displays.len()
    };
    session.update(cx, |s, cx| {
        s.image = image;
        s.origin = frame.origin;
        s.displays = frame.displays;
        s.layout_key = frame.layout_key;
        s.targets = targets;
        s.window_targets = window_targets;
        s.record_window = false;
        s.selection = selection;
        s.hover = None;
        s.drag = None;
        s.text_input = None;
        s.drop_size_editor();
        s.editor = annotate_editor(shapes);
        s.layers.clear();
        s.sync();
        cx.notify();
    });
    if same_layout {
        let (handles, frames) = {
            let s = session.read(cx);
            let frames: Vec<_> = (0..s.displays.len())
                .map(|i| display_frame(&s.image, s.region(i)))
                .collect();
            (s.windows.clone(), frames)
        };
        for (handle, frame) in handles.into_iter().zip(frames) {
            if let Some(overlay) = handle.downcast::<Overlay>() {
                let _ = overlay.update(cx, |overlay, _, cx| {
                    overlay.frame = frame;
                    cx.notify();
                });
            }
        }
        super::flush(session, None, cx);
    } else {
        // 旧窗口的纹理随窗口释放；新布局按回放画面的显示器重建浮层。
        super::close_overlays(cx, session);
        session.update(cx, |s, _| {
            let _ = s.layers.take_retired();
        });
        open_overlays(cx, session);
    }
}

fn annotate_editor(shapes: Vec<Annotation>) -> crate::annotate::Editor {
    // 编辑范围由 `sync` 按选区重设。
    crate::annotate::Editor::new(
        ImageBounds {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        },
        shapes,
    )
}

#[cfg(test)]
mod tests {
    // 不用 `super::*`：gpui 的 glob 导入带有同名 `test` 宏。
    use super::{destination, trim, Direction};
    use std::collections::VecDeque;

    #[test]
    fn previous_and_next_walk_entries_then_return_to_live() {
        assert_eq!(destination(0, None, Direction::Previous), None);
        assert_eq!(destination(3, None, Direction::Previous), Some(2));
        assert_eq!(destination(3, Some(2), Direction::Previous), Some(1));
        assert_eq!(destination(3, Some(0), Direction::Previous), Some(0));
        assert_eq!(destination(3, Some(1), Direction::Next), Some(2));
        assert_eq!(destination(3, Some(2), Direction::Next), None);
        assert_eq!(destination(3, None, Direction::Next), None);
    }

    #[test]
    fn trim_keeps_newest_within_count_and_bytes() {
        let mut entries: VecDeque<usize> = (1..=5).collect();
        trim(&mut entries, |v| *v, 3, usize::MAX);
        assert_eq!(entries, VecDeque::from([3, 4, 5]));
        trim(&mut entries, |v| *v, 10, 9);
        assert_eq!(entries, VecDeque::from([4, 5]));
    }
}
