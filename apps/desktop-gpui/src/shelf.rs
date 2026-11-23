//! 顶部暂存架：剪贴板历史按所选类别的最近条目，
//! 像照片一样挂在鼠标所在显示器顶部中央的一根绳上。
//!
//! 设置、固定/隐藏的条目与快照拼装在 core `snap_core::shelf`（`shelf.json` 与旧版相同，开发调试时随
//! `SNAP_DESK_SETTINGS` 放在临时设置目录）。唤出：“展开 / 收起暂存架”快捷键，或开启“顶部悬停”后鼠标停在
//! 屏幕顶边中央约 0.35 秒。鼠标离开约 0.7 秒后收起；截图期间临时隐藏，结束后恢复并多停留一会儿。
//! 卡片：单击复制，按住拖动超过 6 像素拖出（Windows），可固定在架上、取下为贴图、从架上移除（保留历史，可撤销）。
//! 暂存架窗口不抢焦点，因此不支持方向键翻页、Esc 与 Ctrl+Z；改用翻页按钮、滚轮与撤销按钮。
use crate::annotate;
use gpui_kit::*;
use snap_core::{
    clipboard::{
        model::{Item, Kind},
        service::Service,
    },
    shelf::{self, Preferences, ShelfAction, ShelfSnapshot as Snapshot},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

const CARD: f32 = 174.0;
const HEIGHT: f32 = 248.0;
const PER_PAGE: usize = 5;

struct ShelfState {
    prefs: Result<Preferences, String>,
    path: Option<PathBuf>,
    window: Option<WindowHandle<ShelfView>>,
    /// Some 表示截图期间临时隐藏，值为截图前是否展开。
    suspended: Option<bool>,
}
impl Global for ShelfState {}

fn path() -> Option<PathBuf> {
    crate::shell::settings_dir().map(|dir| dir.join("shelf.json"))
}

/// 读取 `shelf.json` 并启动顶部悬停采样（只在开启时检查光标，主线程每 100 毫秒一次）。
pub fn init(cx: &mut App) {
    let path = path();
    let prefs = path
        .as_deref()
        .ok_or_else(|| "应用数据目录不可用。".to_string())
        .and_then(Preferences::load);
    if let Err(error) = &prefs {
        log::warn!("event=gpui.shelf_preferences_failed message={error}");
    }
    cx.set_global(ShelfState {
        prefs,
        path,
        window: None,
        suspended: None,
    });
    cx.spawn(async move |cx| {
        let mut since: Option<std::time::Instant> = None;
        let mut armed = true;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let hovering = cx.update(|cx| {
                let enabled = cx
                    .try_global::<ShelfState>()
                    .and_then(|s| s.prefs.as_ref().ok())
                    .is_some_and(|p| p.settings.enabled && p.settings.top_hover);
                (enabled && !crate::capture::active(cx)).then(|| at_top_center(cx))
            });
            match hovering {
                Some(true) => {
                    let started = since.get_or_insert_with(std::time::Instant::now);
                    if armed && started.elapsed() >= Duration::from_millis(350) {
                        armed = false;
                        cx.update(|cx| show(cx, 0));
                    }
                }
                Some(false) => {
                    since = None;
                    armed = true;
                }
                None => since = None,
            }
        }
    })
    .detach();
}

/// 光标是否停在所在显示器顶边（2 像素内）的中央 160 像素范围。
fn at_top_center(cx: &App) -> bool {
    let Some(cursor) = crate::native::cursor_point() else {
        return false;
    };
    cx.displays().iter().any(|display| {
        let bounds = display.bounds();
        bounds.contains(&cursor)
            && (cursor.y - bounds.origin.y).abs() <= px(2.0)
            && (cursor.x - bounds.center().x).abs() <= px(80.0)
    })
}

fn is_visible(cx: &App) -> bool {
    cx.try_global::<ShelfState>()
        .and_then(|s| s.window)
        .is_some_and(|handle| {
            cx.windows()
                .iter()
                .any(|w| w.window_id() == handle.window_id())
        })
}

/// 快捷键：展开时收起，否则展开；未开启暂存架时只记录。
pub fn toggle(cx: &mut App) {
    if is_visible(cx) {
        hide(cx);
    } else {
        show(cx, 0);
    }
}

fn hide(cx: &mut App) {
    let handle = cx
        .has_global::<ShelfState>()
        .then(|| cx.global_mut::<ShelfState>().window.take())
        .flatten();
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// 截图前调用：展开中的暂存架临时关闭，返回是否真的关闭了（调用方据此等待桌面合成）。
pub fn suspend(cx: &mut App) -> bool {
    if cx
        .try_global::<ShelfState>()
        .is_none_or(|s| s.suspended.is_some())
    {
        return false;
    }
    let visible = is_visible(cx);
    cx.global_mut::<ShelfState>().suspended = Some(visible);
    if visible {
        hide(cx);
    }
    visible
}

/// 截图结束（完成、取消或失败）后恢复原先展开的暂存架，并给 1.8 秒查看时间。
pub fn resume(cx: &mut App) {
    let previous = cx
        .has_global::<ShelfState>()
        .then(|| cx.global_mut::<ShelfState>().suspended.take())
        .flatten();
    if previous == Some(true) {
        show(cx, 1800);
    }
}

fn snapshot(cx: &App) -> Result<(Snapshot, Service), String> {
    let prefs = cx
        .try_global::<ShelfState>()
        .ok_or("暂存架不可用。")?
        .prefs
        .clone()?;
    if !prefs.settings.enabled {
        return Err("请先开启顶部暂存架。".into());
    }
    let service = crate::clipboard::service(cx).ok_or("剪贴板历史不可用。")?;
    Ok((shelf::snapshot(&prefs, &service)?, service))
}

/// 设置窗口用：当前暂存架设置（shelf.json）；读取失败时为 None。
pub fn settings(cx: &App) -> Option<shelf::ShelfSettings> {
    cx.try_global::<ShelfState>()?.prefs.as_ref().ok().map(|p| p.settings.clone())
}

/// 设置窗口修改暂存架设置；关闭时收起展开中的暂存架。
pub fn set_settings(cx: &mut App, settings: shelf::ShelfSettings) -> Result<(), String> {
    settings.validate()?;
    let enabled = settings.enabled;
    update(cx, move |prefs| {
        prefs.settings = settings;
        Ok(())
    })?;
    if !enabled {
        hide(cx);
    }
    Ok(())
}

/// 设置窗口“展开暂存架”。
pub fn open(cx: &mut App) {
    show(cx, 1500);
}

/// 剪贴板历史右键菜单用：条目是否正显示在暂存架上；暂存架未开启或不可用时为 None（不显示联动入口）。
pub fn shows(cx: &App, id: i64) -> Option<bool> {
    let (snapshot, _) = snapshot(cx).ok()?;
    Some(snapshot.items.iter().any(|item| item.id == id))
}

/// 放到暂存架（固定，并撤销“已从架上移除”）或从架上取下（只影响架子，不删除历史）；展开中的暂存架随之刷新。
pub fn place(cx: &mut App, id: i64, on_shelf: bool) -> Result<(), String> {
    let service = crate::clipboard::service(cx).ok_or("剪贴板历史不可用。")?;
    let data = service.data()?.clone();
    data.repo.get(id)?;
    let action = if on_shelf { ShelfAction::Remove } else { ShelfAction::Keep };
    update(cx, |prefs| prefs.apply(id, action, |value| data.repo.get(value).is_ok()))?;
    if let Some(handle) = cx.try_global::<ShelfState>().and_then(|s| s.window) {
        let _ = handle.update(cx, |view, _, cx| view.refresh(cx));
    }
    Ok(())
}

/// 窗口宽度：每张卡 174 像素，最多一排 5 张，两侧各留 48 像素。
fn width_for(count: usize) -> f32 {
    (count.clamp(1, PER_PAGE) as f32) * CARD + 96.0
}

/// 展开（已展开时只刷新）。`grace_ms` 是自动收起前的最短停留时间。
fn show(cx: &mut App, grace_ms: u64) {
    if crate::capture::active(cx) {
        return;
    }
    let (snapshot, service) = match snapshot(cx) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("event=gpui.shelf_show_failed message={error}");
            return;
        }
    };
    if let Some(handle) = cx.try_global::<ShelfState>().and_then(|s| s.window) {
        if handle
            .update(cx, |view, _, cx| {
                view.replace(snapshot.clone(), grace_ms, cx)
            })
            .is_ok()
        {
            return;
        }
    }
    let Some(cursor) = crate::native::cursor_point() else {
        return;
    };
    let displays = cx.displays();
    let Some(display) = displays
        .iter()
        .find(|d| d.bounds().contains(&cursor))
        .or(displays.first())
    else {
        return;
    };
    let area = display.bounds();
    let width = width_for(snapshot.items.len()).min(f32::from(area.size.width));
    let bounds = Bounds::new(
        point(
            area.origin.x + (area.size.width - px(width)) / 2.0,
            area.origin.y,
        ),
        size(px(width), px(HEIGHT)),
    );
    let language = crate::shell::settings(cx).appearance.language;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: Some(display.id()),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    match cx.open_window(options, |_, cx| {
        cx.new(|cx| {
            let mut view = ShelfView {
                service,
                language,
                snapshot,
                page: 0,
                thumbs: HashMap::new(),
                pending: Default::default(),
                hovered: false,
                hide_generation: 0,
                busy: false,
                removed: None,
                press: None,
                error: None,
                message: None,
            };
            view.schedule_hide(grace_ms.max(700), cx);
            view
        })
    }) {
        Ok(handle) => {
            cx.global_mut::<ShelfState>().window = Some(handle);
            log::info!("event=gpui.shelf_shown");
        }
        Err(error) => log::error!("event=gpui.shelf_open_failed message={error}"),
    }
}

/// 修改并保存偏好；失败时内存与文件都保持原样。
fn update(
    cx: &mut App,
    change: impl FnOnce(&mut Preferences) -> Result<(), String>,
) -> Result<(), String> {
    let state = cx.try_global::<ShelfState>().ok_or("暂存架不可用。")?;
    let mut next = state.prefs.clone()?;
    let path = state.path.clone().ok_or("应用数据目录不可用。")?;
    change(&mut next)?;
    next.save(&path)?;
    cx.global_mut::<ShelfState>().prefs = Ok(next);
    Ok(())
}

/// 暂存架卡片上的小图标按钮（浅色背景上用深色图标）。
/// 暂存架图标（Lucide）。
mod icons {
    macro_rules! icon {
        ($name:ident, $file:literal) => {
            pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));
        };
    }
    icon!(PANEL, "panels-top-left");
    icon!(LEFT, "chevron-left");
    icon!(RIGHT, "chevron-right");
    icon!(TYPE, "type");
    icon!(FILE, "file");
}

/// 卡片操作条里的圆形按钮（27×25）。
fn icon_pill(id: impl Into<ElementId>, data: &'static [u8], tooltip: String, pressed: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(27.0))
        .h(px(25.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .bg(rgba(if pressed { 0xe8efffff } else { 0x00000000 }))
        .hover(|s| s.bg(rgb(0xe2e8f0)))
        .child(annotate::icon(data, if pressed { 0x2563eb } else { 0x475569 }).size(px(14.0)))
        .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(tooltip.clone().into())).into())
}

/// 右上角与翻页的圆形按钮（26×26）。
fn round_icon(id: impl Into<ElementId>, data: &'static [u8], tooltip: String, enabled: bool) -> Stateful<Div> {
    let button = div()
        .id(id)
        .size(px(26.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .child(annotate::icon(data, 0x334155).size(px(13.0)))
        .tooltip(move |_, cx| cx.new(|_| annotate::Tooltip(tooltip.clone().into())).into());
    if enabled {
        button.cursor_pointer().hover(|s| s.bg(rgb(0xe2e8f0)))
    } else {
        button.opacity(0.3)
    }
}

/// 照片尺寸：最长边适配 144×108（与 `shelf-layout.ts` 一致）。
fn photo_size(width: Option<u32>, height: Option<u32>) -> (f32, f32) {
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => {
            let scale = (144.0 / w as f32).min(108.0 / h as f32);
            (
                (w as f32 * scale).round().max(1.0),
                (h as f32 * scale).round().max(1.0),
            )
        }
        _ => (144.0, 96.0),
    }
}

/// 卡片沿下垂绳索的纵向位置：抛物线，最大下垂 min(20, 宽度×2.4%)。
fn rope_y(x: f32, width: f32) -> f32 {
    let sag = (width * 0.024).min(20.0);
    12.0 + 4.0 * sag * (x / width) * (1.0 - x / width)
}

struct ShelfView {
    service: Service,
    language: snap_core::settings::Language,
    snapshot: Snapshot,
    page: usize,
    thumbs: HashMap<i64, Arc<RenderImage>>,
    pending: std::collections::HashSet<i64>,
    hovered: bool,
    /// 每次重新计时递增，过期的收起任务不生效。
    hide_generation: u64,
    busy: bool,
    /// 最近一次移除（条目 ID，移除前是否固定），供撤销。
    removed: Option<(i64, bool)>,
    press: Option<(i64, Point<Pixels>)>,
    error: Option<String>,
    message: Option<String>,
}

impl ShelfView {
    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn replace(&mut self, snapshot: Snapshot, grace_ms: u64, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        self.page = self.page.min(self.pages().saturating_sub(1));
        self.schedule_hide(grace_ms.max(700), cx);
        cx.notify();
    }

    fn pages(&self) -> usize {
        self.snapshot.items.len().div_ceil(PER_PAGE).max(1)
    }

    /// 鼠标不在架上且没有进行中的操作时，延迟收起。
    fn schedule_hide(&mut self, delay_ms: u64, cx: &mut Context<Self>) {
        self.hide_generation += 1;
        let generation = self.hide_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay_ms))
                .await;
            let close = this
                .update(cx, |view, _| {
                    generation == view.hide_generation && !view.hovered && !view.busy
                })
                .unwrap_or(false);
            if close {
                cx.update(hide);
            }
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        match snapshot(cx) {
            Ok((snapshot, _)) => {
                self.snapshot = snapshot;
                self.page = self.page.min(self.pages().saturating_sub(1));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn act(&mut self, id: i64, action: ShelfAction, cx: &mut Context<Self>) -> Result<(), String> {
        let data = self.service.data()?.clone();
        data.repo.get(id)?;
        update(cx, |prefs| {
            prefs.apply(id, action, |value| data.repo.get(value).is_ok())
        })
    }

    fn finish(
        &mut self,
        result: Result<(), String>,
        success: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.error = None;
                self.message = success.map(|key| self.t(key));
            }
            Err(error) => {
                self.message = None;
                self.error = Some(error);
            }
        }
        self.refresh(cx);
        if !self.hovered {
            self.schedule_hide(700, cx);
        }
    }

    fn copy(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.service.contents(&[id]).and_then(|mut c| {
            crate::clipboard::write_back(&self.service, &c.remove(0), window, cx)
        });
        self.finish(result, Some("已复制"), cx);
    }

    fn remove(&mut self, id: i64, cx: &mut Context<Self>) {
        let kept = self.snapshot.kept_ids.contains(&id);
        let result = self.act(id, ShelfAction::Remove, cx);
        if result.is_ok() {
            self.removed = Some((id, kept));
        }
        self.finish(result, None, cx);
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some((id, kept)) = self.removed.take() else {
            return;
        };
        let result = self.act(
            id,
            if kept {
                ShelfAction::Keep
            } else {
                ShelfAction::Restore
            },
            cx,
        );
        self.finish(result, None, cx);
    }

    fn keep(&mut self, id: i64, cx: &mut Context<Self>) {
        let action = if self.snapshot.kept_ids.contains(&id) {
            ShelfAction::Unkeep
        } else {
            ShelfAction::Keep
        };
        let result = self.act(id, action, cx);
        self.finish(result, None, cx);
    }

    /// 取下为贴图：创建贴图后从架上移除。
    fn pin(&mut self, id: i64, cx: &mut Context<Self>) {
        self.busy = true;
        let task = crate::clipboard::pin_item(cx, self.service.clone(), id);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(()) => view.remove(id, cx),
                    Err(error) => view.finish(Err(error), None, cx),
                }
            });
        })
        .detach();
    }

    fn drag(&mut self, id: i64, cx: &mut Context<Self>) {
        self.busy = true;
        let task = crate::clipboard::drag_ids(cx, self.service.clone(), vec![id]);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                if let Err(error) = result {
                    view.error = Some(error);
                }
                if !view.hovered {
                    view.schedule_hide(700, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn turn(&mut self, delta: isize, cx: &mut Context<Self>) {
        let last = self.pages() as isize - 1;
        self.page = (self.page as isize + delta).clamp(0, last) as usize;
        cx.notify();
    }

    fn thumbnail(&mut self, id: i64, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        if let Some(thumb) = self.thumbs.get(&id) {
            return Some(thumb.clone());
        }
        if self.pending.insert(id) {
            let service = self.service.clone();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        let bytes = service.thumbnail(id)?;
                        let mut image = image::load_from_memory(&bytes)
                            .map_err(|e| e.to_string())?
                            .to_rgba8();
                        for pixel in image.pixels_mut() {
                            pixel.0.swap(0, 2);
                        }
                        Ok::<_, String>(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
                    })
                    .await;
                let _ = this.update(cx, |view, cx| {
                    view.pending.remove(&id);
                    if let Ok(thumb) = decoded {
                        view.thumbs.insert(id, thumb);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        None
    }

    fn card(&mut self, item: &Item, left: f32, top: f32, cx: &mut Context<Self>) -> AnyElement {
        let id = item.id;
        let kept = self.snapshot.kept_ids.contains(&id);
        let language = self.language.clone();
        let text = move |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        // 内容尺寸：图片按比例适配 144×108，文字便签 144×100。
        let (w, h) = match item.kind {
            Kind::Image => photo_size(item.width, item.height),
            _ => (144.0, 100.0),
        };
        let content: AnyElement = match item.kind {
            Kind::Image => match self.thumbnail(id, cx) {
                Some(thumb) => img(thumb).size_full().rounded(px(9.0)).object_fit(ObjectFit::Contain).into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(rgb(0x475569))
                    .child(text("正在加载…"))
                    .into_any_element(),
            },
            kind => div()
                .size_full()
                .pt(px(12.0))
                .px(px(10.0))
                .pb(px(8.0))
                .rounded(px(9.0))
                .bg(rgb(if kind == Kind::File { 0xedf2fa } else { 0xfff8da }))
                .overflow_hidden()
                .text_xs()
                .line_height(relative(1.5))
                .text_color(rgb(0x475569))
                .child(annotate::icon(if kind == Kind::File { icons::FILE } else { icons::TYPE }, 0x8b7e58).size(px(16.0)))
                .child(div().mt(px(5.0)).line_clamp(3).child(item.preview.chars().take(300).collect::<String>()))
                .into_any_element(),
        };
        let group = SharedString::from(format!("shelf-card-{id}"));
        let frame_height = h + 10.0;
        // 金属夹子：固定项为深色。
        let (clip_from, clip_to, clip_border) = if kept {
            (0x414a55, 0x89939f, 0xafb8c1)
        } else {
            (0x9d9fa2, 0xf3f4f5, 0xffffffa6)
        };
        let clip = div()
            .absolute()
            .top_0()
            .left(px(78.0 - 4.0))
            .w(px(8.0))
            .h(px(25.0))
            .rounded(px(3.0))
            .border_1()
            .border_color(if kept { rgb(clip_border).into() } else { rgba(clip_border) })
            .bg(linear_gradient(90.0, linear_color_stop(rgb(clip_from), 0.0), linear_color_stop(rgb(clip_to), 0.5)))
            .shadow_sm()
            .child(div().absolute().left(px(1.0)).right(px(1.0)).top(px(8.0)).h(px(1.0)).bg(rgba(0x5c606675)));
        let remove = icon_pill(("shelf-remove", id as u64), annotate::icons::CLOSE, text("从架子移除，保留历史"), false)
            .absolute()
            .top(px(-7.0))
            .left(px(-7.0))
            .size(px(24.0))
            .border_1()
            .border_color(rgb(0xffffff))
            .bg(rgba(0xf8fafcf2))
            .shadow_sm()
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.remove(id, cx)));
        let actions = div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(frame_height + 7.0))
            .flex()
            .justify_center()
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .child(
                div()
                    .p(px(3.0))
                    .flex()
                    .gap(px(2.0))
                    .rounded(px(18.0))
                    .border_1()
                    .border_color(rgba(0xffffffcf))
                    .bg(rgba(0xfafafaf2))
                    .shadow_md()
                    .child(
                        icon_pill(("shelf-copy", id as u64), annotate::icons::COPY, text("复制"), false)
                            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| view.copy(id, window, cx))),
                    )
                    .child(
                        icon_pill(
                            ("shelf-keep", id as u64),
                            if kept { annotate::icons::PIN_OFF } else { annotate::icons::PIN },
                            text(if kept { "取消固定" } else { "固定在架上" }),
                            kept,
                        )
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.keep(id, cx))),
                    )
                    .child(
                        icon_pill(("shelf-pin", id as u64), icons::PANEL, text("取下为贴图"), false)
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.pin(id, cx))),
                    ),
            );
        let frame = div()
            .id(("shelf-card", id as u64))
            .relative()
            .p(px(4.0))
            .rounded(px(13.0))
            .border_1()
            .border_color(rgba(0xffffffce))
            .bg(linear_gradient(145.0, linear_color_stop(rgba(0xffffffe0), 0.0), linear_color_stop(rgba(0xf4f4f5c0), 1.0)))
            .shadow_md()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |view, event: &MouseDownEvent, _, _| {
                view.press = Some((id, event.position))
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|view, _: &MouseUpEvent, _, _| view.press = None))
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                let Some((id, origin)) = view.press else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    view.press = None;
                } else if (event.position - origin).x.abs() + (event.position - origin).y.abs() > px(6.0) {
                    view.press = None;
                    view.drag(id, cx);
                }
            }))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                if !view.busy {
                    view.copy(id, window, cx);
                }
            }))
            .tooltip({
                let tip = text("单击复制，拖动分享");
                move |_, cx| cx.new(|_| annotate::Tooltip(tip.clone().into())).into()
            })
            .child(div().w(px(w)).h(px(h)).child(content))
            .child(remove);
        div()
            .id(("shelf-item", id as u64))
            .group(group)
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(156.0))
            .h(px(18.0 + frame_height + 48.0))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(18.0))
            .child(frame)
            .child(clip)
            .child(actions)
            .into_any_element()
    }
}

impl Render for ShelfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 释放已不在架上的缩略图纹理。
        let ids: std::collections::HashSet<i64> =
            self.snapshot.items.iter().map(|i| i.id).collect();
        let stale: Vec<i64> = self
            .thumbs
            .keys()
            .filter(|id| !ids.contains(id))
            .copied()
            .collect();
        for id in stale {
            if let Some(thumb) = self.thumbs.remove(&id) {
                cx.drop_image(thumb, Some(window));
            }
        }
        let width = f32::from(window.viewport_size().width);
        let start = self.page * PER_PAGE;
        let visible: Vec<Item> = self
            .snapshot
            .items
            .iter()
            .skip(start)
            .take(PER_PAGE)
            .cloned()
            .collect();
        let spacing = CARD.min((width - 48.0).max(1.0));
        let mut cards = div().absolute().top_0().left_0().size_full();
        for (index, item) in visible.iter().enumerate() {
            let x = width / 2.0 + (index as f32 - (visible.len() as f32 - 1.0) / 2.0) * spacing;
            let card = self.card(item, x - 78.0, rope_y(x, width) - 9.0, cx);
            cards = cards.child(card);
        }
        let language = self.language.clone();
        let text =
            move |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        // 右上角：撤销移除、收起（悬停时显示）。
        let control = |button: Stateful<Div>| {
            button.bg(rgba(0xf8fafce8)).border_1().border_color(rgba(0xffffffc7)).shadow_sm()
        };
        let mut controls = div().absolute().top(px(5.0)).right(px(8.0)).flex().gap(px(5.0));
        if !self.hovered {
            controls = controls.invisible();
        }
        if self.removed.is_some() {
            controls = controls.child(
                control(round_icon("shelf-undo", annotate::icons::UNDO, text("撤销移除"), true))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.undo(cx))),
            );
        }
        controls = controls.child(
            control(round_icon("shelf-close", annotate::icons::CLOSE, text("收起"), true))
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| hide(cx))),
        );
        let pages = self.pages();
        let page = self.page;
        let pager = (pages > 1).then(|| {
            let mut prev = round_icon("shelf-prev", icons::LEFT, text("上一组"), page > 0);
            if page > 0 {
                prev = prev.on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.turn(-1, cx)));
            }
            let last = page + 1 >= pages;
            let mut next = round_icon("shelf-next", icons::RIGHT, text("下一组"), !last);
            if !last {
                next = next.on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.turn(1, cx)));
            }
            div().absolute().bottom(px(9.0)).left_0().right_0().flex().justify_center().child(
                div()
                    .px(px(4.0))
                    .py(px(2.0))
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .rounded(px(18.0))
                    .border_1()
                    .border_color(rgba(0xffffffb8))
                    .bg(rgba(0xfafafae8))
                    .shadow_sm()
                    .opacity(if self.hovered { 1.0 } else { 0.65 })
                    .child(prev)
                    .child(
                        div()
                            .min_w(px(34.0))
                            .flex()
                            .justify_center()
                            .text_size(px(11.0))
                            .text_color(rgb(0x334155))
                            .child(format!("{} / {pages}", page + 1)),
                    )
                    .child(next),
            )
        });
        let feedback = match (&self.error, &self.message) {
            (Some(error), _) => Some((error.clone(), 0xb42318)),
            (None, Some(message)) => Some((message.clone(), 0x24834b)),
            _ => None,
        };
        let empty = visible.is_empty().then(|| {
            div().absolute().top(px(64.0)).left_0().right_0().flex().justify_center().child(
                div()
                    .px(px(14.0))
                    .py(px(9.0))
                    .rounded(px(18.0))
                    .border_1()
                    .border_color(rgba(0xffffffb8))
                    .bg(rgba(0xfafafaf2))
                    .shadow_sm()
                    .text_xs()
                    .text_color(rgb(0x334155))
                    .child(text("暂无匹配内容")),
            )
        });
        // 下垂的绳子：沿 `rope_y` 的抛物线用短线段拼出。
        let mut rope = div().absolute().top_0().left_0().size_full();
        let step = 4.0;
        let mut x = 0.0;
        while x < width {
            let y = rope_y(x + step / 2.0, width);
            rope = rope
                .child(div().absolute().left(px(x)).top(px(y + 1.0)).w(px(step)).h(px(2.0)).bg(rgba(0x00000018)))
                .child(div().absolute().left(px(x)).top(px(y)).w(px(step)).h(px(1.0)).bg(rgb(0x888b8e)));
            x += step;
        }
        div()
            .id("shelf")
            .size_full()
            .relative()
            // Esc 收起（暂存架被点击激活后才收得到按键）。
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    hide(cx);
                }
            }))
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                view.hovered = *hovered;
                if !*hovered {
                    view.schedule_hide(700, cx);
                }
            }))
            .on_scroll(cx.listener(|view, event: &ScrollEvent, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                let value = if delta.x.abs() > delta.y.abs() {
                    delta.x
                } else {
                    delta.y
                };
                if value.abs() >= px(12.0) {
                    view.turn(if value > px(0.0) { -1 } else { 1 }, cx);
                }
            }))
            .child(rope)
            .child(cards)
            .children(empty)
            .child(controls)
            .children(pager)
            .children(feedback.map(|(message, color)| {
                // 成功提示在顶部，错误在底部。
                let bubble = div()
                    .px(px(11.0))
                    .py(px(5.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(rgba(0xffffffc7))
                    .bg(rgba(0xfafafaf5))
                    .shadow_sm()
                    .text_xs()
                    .text_color(rgb(color))
                    .child(message);
                let row = div().absolute().left(px(16.0)).right(px(16.0)).flex().justify_center().child(bubble);
                if color == 0x24834b { row.top(px(3.0)) } else { row.bottom(px(48.0)) }
            }))
            .text_color(rgb(annotate::text()))
    }
}

#[cfg(test)]
mod tests {
    use super::{photo_size, rope_y, width_for};

    #[test]
    fn shelf_layout() {
        assert_eq!(width_for(0), 270.0);
        assert_eq!(width_for(9), 966.0);
        assert_eq!(photo_size(Some(320), Some(180)), (144.0, 81.0));
        assert_eq!(photo_size(Some(100), Some(400)), (27.0, 108.0));
        assert_eq!(photo_size(None, Some(1)), (144.0, 96.0));
        assert_eq!(rope_y(0.0, 1000.0), 12.0);
        assert_eq!(rope_y(500.0, 1000.0), 32.0);
    }
}
