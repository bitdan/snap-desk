//! 剪贴板历史：后台记录服务（`snap_core::clipboard::service`）与历史窗口。
//!
//! 记录：启动时按设置 `clipboard` 启用；监听、读取、去重、写入 SQLite 与图片仓库都在 core 的工作线程，
//! 变化与错误经回调转发到主线程刷新窗口。数据目录与 Tauri 版相同（`app_local_data_dir/clipboard`），
//! 两个版本共用同一份历史；开发调试可用 `SNAP_DESK_CLIPBOARD_DIR` 指向另一目录，避免写入真实历史。
//!
//! 窗口（对照 Vue 版 `ClipboardHistory.vue`）：搜索、分类（全部/文本/链接/颜色/文件路径/图片/文件/收藏）、
//! 分页加载、键盘导航；回车/双击按设置“唤出后直接粘贴”粘贴到唤出前的窗口（否则只复制），
//! Ctrl+1～9 取第 N 条；每条可复制、贴图、收藏、删除；清空（默认保留收藏，二次确认）；
//! “固定”时粘贴后不收起。未接入：多选合并复制、导出（PDF/长图）、拖出、预览窗口、分类排序。
use crate::annotate;
use futures::{channel::mpsc, StreamExt};
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    *,
};
use image::RgbaImage;
use snap_core::clipboard::{
    classification,
    model::{ClipboardSettings, Item, Kind, Query, TextCategory},
    platform,
    service::{self, Content, Event, Service},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

const PAGE: u32 = 60;

struct History {
    service: Service,
    view: Option<WeakEntity<HistoryView>>,
    window: Option<AnyWindowHandle>,
    /// 唤出窗口前的前台窗口（粘贴目标）；0 表示没有。
    target: usize,
}
impl Global for History {}

fn root() -> Result<PathBuf, String> {
    std::env::var_os("SNAP_DESK_CLIPBOARD_DIR")
        .map(PathBuf::from)
        .or_else(snap_core::paths::clipboard_dir)
        .ok_or_else(|| "应用数据目录不可用。".to_string())
}

/// 启动记录服务；数据库损坏等错误只记录，窗口仍可打开并显示错误。
pub fn init(cx: &mut App, settings: &ClipboardSettings) {
    let (sender, mut receiver) = mpsc::unbounded::<Event>();
    let notify: service::Notify = Arc::new(move |event| {
        let _ = sender.unbounded_send(event);
    });
    let service = Service::new(root(), notify);
    if let Err(error) = service.data() {
        log::error!("event=gpui.clipboard_unavailable message={error}");
    }
    cx.set_global(History { service: service.clone(), view: None, window: None, target: 0 });
    // 变化与错误回到主线程，刷新已打开的历史窗口。
    cx.spawn(async move |cx| {
        while let Some(event) = receiver.next().await {
            cx.update(|cx| {
                let view = cx.try_global::<History>().and_then(|h| h.view.clone()).and_then(|v| v.upgrade());
                if let Some(view) = view {
                    view.update(cx, |view, cx| view.on_event(event, cx));
                }
            });
        }
    })
    .detach();
    let worker = service.clone();
    let settings = settings.clone();
    std::thread::spawn(move || {
        if let Err(error) = worker.configure(settings) {
            log::warn!("event=gpui.clipboard_configure_failed message={error}");
            worker.report(error);
        }
        // 过期淘汰与素材修复逐个核对文件，放在后台。
        if let Err(error) = worker.maintain() {
            worker.report(error);
        }
        log::info!("event=gpui.clipboard_started");
    });
}

/// 退出前停止监听并等待在途写入。
pub fn shutdown(cx: &mut App) {
    if let Some(history) = cx.try_global::<History>() {
        history.service.shutdown();
    }
}

/// 打开历史窗口；先记下当前前台窗口作为粘贴目标。
pub fn open(cx: &mut App) {
    let target = platform::foreground_window();
    if cx.try_global::<History>().is_none() {
        return;
    }
    let existing = cx.global::<History>().window;
    cx.global_mut::<History>().target = target;
    if let Some(handle) = existing {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let service = cx.global::<History>().service.clone();
    let settings = crate::shell::settings(cx);
    let size = size(px(460.0), px(640.0));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions {
            title: Some(snap_core::i18n::for_language(&settings.appearance.language, "剪贴板历史").to_owned().into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(gpui_kit::size(px(360.0), px(360.0))),
        ..Default::default()
    };
    let mut created = None;
    match cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| HistoryView::new(service, settings, window, cx));
        created = Some(view.downgrade());
        view
    }) {
        Ok(handle) => {
            let history = cx.global_mut::<History>();
            history.window = Some(handle.into());
            history.view = created;
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            log::info!("event=gpui.clipboard_window_opened");
        }
        Err(error) => log::error!("event=gpui.clipboard_window_failed message={error}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    All,
    Text,
    Link,
    Color,
    Path,
    Image,
    File,
    Favorites,
}

const CATEGORIES: [(Category, &str); 8] = [
    (Category::All, "全部"),
    (Category::Text, "文本"),
    (Category::Link, "链接"),
    (Category::Color, "颜色"),
    (Category::Path, "文件路径"),
    (Category::Image, "图片"),
    (Category::File, "文件"),
    (Category::Favorites, "收藏"),
];

impl Category {
    fn query(self, keyword: &str, cursor: Option<String>) -> Query {
        let (kind, text_category) = match self {
            Self::Text => (Some(Kind::Text), None),
            Self::Link => (Some(Kind::Text), Some(TextCategory::Link)),
            Self::Color => (Some(Kind::Text), Some(TextCategory::Color)),
            Self::Path => (Some(Kind::Text), Some(TextCategory::Path)),
            Self::Image => (Some(Kind::Image), None),
            Self::File => (Some(Kind::File), None),
            Self::All | Self::Favorites => (None, None),
        };
        Query {
            keyword: keyword.chars().take(200).collect(),
            kind,
            text_category,
            favorites_only: self == Self::Favorites,
            limit: Some(PAGE),
            cursor,
        }
    }
}

/// 相对时间：刚刚 / N 分钟前 / N 小时前 / 月-日 时:分。
fn ago(millis: i64, text: &dyn Fn(&'static str) -> String) -> String {
    let now = chrono::Utc::now().timestamp_millis();
    let minutes = (now - millis).max(0) / 60_000;
    if minutes < 1 {
        text("刚刚")
    } else if minutes < 60 {
        format!("{minutes} {}", text("分钟前"))
    } else if minutes < 24 * 60 {
        format!("{} {}", minutes / 60, text("小时前"))
    } else {
        chrono::DateTime::from_timestamp_millis(millis)
            .map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string())
            .unwrap_or_default()
    }
}

/// 颜色文本的色块（#RGB/#RRGGBB 等，经 core 的贴图拖出规范化为 HEX）。
fn swatch(text: &str) -> Option<u32> {
    let hex = snap_core::pins::text::drag_text(text.trim());
    let hex = hex.strip_prefix('#')?;
    (hex.len() >= 6).then(|| u32::from_str_radix(&hex[..6], 16).ok()).flatten()
}

struct HistoryView {
    service: Service,
    language: snap_core::settings::Language,
    paste_on_activate: bool,
    search: Entity<InputState>,
    category: Category,
    items: Vec<Item>,
    next_cursor: Option<String>,
    selected: usize,
    loading: bool,
    /// 每次查询递增，丢弃过期结果。
    generation: u64,
    thumbs: HashMap<i64, Arc<RenderImage>>,
    thumbs_pending: std::collections::HashSet<i64>,
    error: Option<String>,
    message: Option<String>,
    keep_open: bool,
    confirm_clear: bool,
    focus: FocusHandle,
    _search: Subscription,
}

impl HistoryView {
    fn new(service: Service, settings: snap_core::settings::Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = settings.appearance.language.clone();
        let placeholder = snap_core::i18n::for_language(&language, "搜索剪贴板历史（Ctrl+F）").to_owned();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe_in(&search, window, |this: &mut Self, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.reload(cx);
            }
        });
        search.update(cx, |state, cx| state.focus(window, cx));
        let mut view = Self {
            service,
            language,
            paste_on_activate: settings.clipboard.paste_on_activate,
            search,
            category: Category::All,
            items: Vec::new(),
            next_cursor: None,
            selected: 0,
            loading: false,
            generation: 0,
            thumbs: HashMap::new(),
            thumbs_pending: Default::default(),
            error: None,
            message: None,
            keep_open: false,
            confirm_clear: false,
            focus: cx.focus_handle(),
            _search: subscription,
        };
        view.reload(cx);
        view
    }

    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Changed(_) => self.reload(cx),
            Event::Error(error) => {
                self.error = (!error.is_empty()).then_some(error);
                cx.notify();
            }
        }
    }

    /// 重新查询第一页（搜索、分类或历史变化）。
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(None, cx);
    }

    fn load(&mut self, cursor: Option<String>, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let append = cursor.is_some();
        let keyword = self.search.read(cx).value().to_string();
        let query = self.category.query(&keyword, cursor);
        let service = self.service.clone();
        self.loading = true;
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_executor()
                .spawn(async move {
                    let revision = service.revision.load(std::sync::atomic::Ordering::Acquire);
                    service.data()?.repo.list(&query, revision)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if generation != view.generation {
                    return;
                }
                view.loading = false;
                match page {
                    Ok(page) => {
                        if append {
                            view.items.extend(page.items);
                        } else {
                            view.items = page.items;
                            view.selected = view.selected.min(view.items.len().saturating_sub(1));
                        }
                        view.next_cursor = page.next_cursor;
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_category(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = category;
        self.selected = 0;
        self.reload(cx);
    }

    /// 写回系统剪贴板，并标记自写入（监听线程跳过，不记成新条目）。
    fn write_back(&self, content: &Content, window: &Window, cx: &mut App) -> Result<(), String> {
        let _gate = self.service.write_gate.lock().map_err(|_| "剪贴板写入状态不可用。")?;
        match content {
            Content::Text(text) => cx.write_to_clipboard(ClipboardItem::new_string(text.clone())),
            Content::Image(image) => {
                let png = crate::capture::png(image)?;
                crate::capture::write_clipboard(cx, crate::native::hwnd(window), image, png)?;
            }
            Content::Files(paths) => platform::write_files(paths)?,
        }
        self.service.mark_restored(content)
    }

    fn content(&self, id: i64) -> Result<Content, String> {
        self.service.contents(&[id]).map(|mut c| c.remove(0))
    }

    /// 复制所选条目；`paste` 时交还唤出前的窗口并发送粘贴键。
    fn activate(&mut self, index: usize, paste: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index).cloned() else {
            return;
        };
        let result = self.content(item.id).and_then(|content| self.write_back(&content, window, cx));
        if let Err(error) = result {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let target = cx.try_global::<History>().map_or(0, |h| h.target);
        if paste && target != 0 {
            // 先交还焦点再收起：收起后本程序不再是前台进程，系统可能拒绝切换前台窗口。
            if let Err(error) = platform::activate(target) {
                self.error = Some(error);
                cx.notify();
                return;
            }
            if !self.keep_open {
                window.remove_window();
            }
            if let Err(error) = platform::send_paste(target) {
                log::warn!("event=gpui.clipboard_paste_failed message={error}");
            }
            log::info!("event=gpui.clipboard_pasted");
            return;
        }
        self.message = Some(self.t("已复制"));
        self.error = None;
        if paste && !self.keep_open {
            // 没有可粘贴的外部窗口（例如从托盘唤出）：只复制并收起。
            window.remove_window();
        }
        cx.notify();
    }

    fn pin(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index).cloned() else {
            return;
        };
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    let content = service.contents(&[item.id])?.remove(0);
                    match content {
                        Content::Image(image) => Ok((image, None)),
                        Content::Text(text) => text_pin(&text),
                        Content::Files(paths) => text_pin(&paths.join("\n")),
                    }
                })
                .await;
            let result = cx.update(|cx| {
                prepared.and_then(|(image, drag)| {
                    crate::pins::create(cx, image, Vec::new(), crate::pins::Placement::Centered, drag)
                })
            });
            if let Err(error) = result {
                let _ = this.update(cx, |view, cx| {
                    view.error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn run(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        if let Err(error) = result {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;
        let key = keystroke.key.as_str();
        match key {
            "escape" => {
                if self.confirm_clear {
                    self.confirm_clear = false;
                } else if !self.search.read(cx).value().is_empty() {
                    self.search.update(cx, |state, cx| state.set_value("", window, cx));
                } else {
                    window.remove_window();
                }
            }
            "down" => self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1)),
            "up" => self.selected = self.selected.saturating_sub(1),
            "enter" => self.activate(self.selected, self.paste_on_activate, window, cx),
            "f" if command => self.search.update(cx, |state, cx| state.focus(window, cx)),
            "c" if command && !self.search.read(cx).focus_handle(cx).is_focused(window) => {
                self.activate(self.selected, false, window, cx)
            }
            "delete" if !self.search.read(cx).focus_handle(cx).is_focused(window) => {
                if let Some(id) = self.items.get(self.selected).map(|i| i.id) {
                    let result = self.service.delete(&[id]);
                    self.run(result, cx);
                }
            }
            digit if command && digit.len() == 1 && ('1'..='9').contains(&digit.chars().next().unwrap_or('0')) => {
                let index = digit.parse::<usize>().unwrap_or(1) - 1;
                self.activate(index, self.paste_on_activate, window, cx);
            }
            _ => return,
        }
        cx.notify();
    }

    fn thumbnail(&mut self, id: i64, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        if let Some(thumb) = self.thumbs.get(&id) {
            return Some(thumb.clone());
        }
        if self.thumbs_pending.insert(id) {
            let service = self.service.clone();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        let bytes = service.thumbnail(id)?;
                        let mut image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
                        for pixel in image.pixels_mut() {
                            pixel.0.swap(0, 2);
                        }
                        Ok::<_, String>(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
                    })
                    .await;
                let _ = this.update(cx, |view, cx| {
                    view.thumbs_pending.remove(&id);
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

    fn small_button(&self, id: impl Into<ElementId>, label: String) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(6.0))
            .py(px(2.0))
            .rounded(px(4.0))
            .text_xs()
            .text_color(rgb(annotate::MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(annotate::HOVER)).text_color(rgb(annotate::TEXT)))
            .child(label)
    }
}

/// 文字（或文件路径）排成图片贴图，拖出时提供文字。
fn text_pin(text: &str) -> Result<(RgbaImage, Option<String>), String> {
    let font = snap_core::pins::text::font_bytes(annotate::user_font())?;
    let drag = snap_core::pins::text::drag_text(text);
    snap_core::pins::text::render(text, &font).map(|image| (image, Some(drag)))
}

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 释放已不在列表中的缩略图纹理。
        let ids: std::collections::HashSet<i64> = self.items.iter().map(|i| i.id).collect();
        let stale: Vec<i64> = self.thumbs.keys().filter(|id| !ids.contains(id)).copied().collect();
        for id in stale {
            if let Some(thumb) = self.thumbs.remove(&id) {
                cx.drop_image(thumb, Some(window));
            }
        }
        let language = self.language.clone();
        let text = move |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let mut tabs = div().flex().flex_wrap().gap(px(4.0));
        for (index, (category, label)) in CATEGORIES.into_iter().enumerate() {
            let active = self.category == category;
            tabs = tabs.child(
                div()
                    .id(("clipboard-tab", index))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(rgb(if active { annotate::ACTIVE_FG } else { annotate::MUTED }))
                    .bg(rgb(if active { annotate::ACTIVE_BG } else { annotate::SURFACE }))
                    .child(text(label))
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_category(category, cx))),
            );
        }
        let keep_open = self.keep_open;
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(div().flex_1().child(Input::new(&self.search)))
            .child(
                annotate::icon_button("clipboard-keep-open", annotate::icons::PIN, text("固定：粘贴后不收起窗口"), keep_open, false)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.keep_open = !view.keep_open;
                        cx.notify();
                    })),
            );
        let mut list = div().id("clipboard-list").flex_1().flex().flex_col().gap(px(2.0)).overflow_y_scroll();
        if self.items.is_empty() && !self.loading {
            let empty = if self.service.data().is_err() { "剪贴板历史不可用。" } else { "没有匹配的记录。" };
            list = list.child(div().p(px(16.0)).text_sm().text_color(rgb(annotate::MUTED)).child(text(empty)));
        }
        let items = self.items.clone();
        for (index, item) in items.iter().enumerate() {
            let selected = index == self.selected;
            let leading: AnyElement = match item.kind {
                Kind::Image => match self.thumbnail(item.id, cx) {
                    Some(thumb) => img(thumb).size(px(44.0)).object_fit(ObjectFit::Contain).into_any_element(),
                    None => div().size(px(44.0)).rounded(px(4.0)).bg(rgb(0x0f1115)).into_any_element(),
                },
                Kind::File => div().size(px(44.0)).flex().items_center().justify_center().text_xs().text_color(rgb(annotate::MUTED)).child(text("文件")).into_any_element(),
                Kind::Text => match classification::classify(&item.preview) {
                    Some(TextCategory::Color) => div()
                        .size(px(44.0))
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(rgb(annotate::BORDER))
                        .bg(rgb(swatch(&item.preview).unwrap_or(0)))
                        .into_any_element(),
                    Some(TextCategory::Link) => div().size(px(44.0)).flex().items_center().justify_center().text_xs().text_color(rgb(annotate::ACTIVE_FG)).child(text("链接")).into_any_element(),
                    Some(TextCategory::Path) => div().size(px(44.0)).flex().items_center().justify_center().text_xs().text_color(rgb(annotate::MUTED)).child(text("路径")).into_any_element(),
                    None => div().size(px(44.0)).flex().items_center().justify_center().text_xs().text_color(rgb(annotate::MUTED)).child("T").into_any_element(),
                },
            };
            let title = match item.kind {
                Kind::Image => format!("{} {}×{}", text("图片"), item.width.unwrap_or(0), item.height.unwrap_or(0)),
                _ => item.preview.lines().take(2).collect::<Vec<_>>().join(" ⏎ "),
            };
            let meta = format!(
                "{}{}{}",
                ago(item.last_seen_at, &text),
                if item.seen_count > 1 { format!(" · ×{}", item.seen_count) } else { String::new() },
                if index < 9 { format!(" · Ctrl+{}", index + 1) } else { String::new() },
            );
            let favorite = item.favorite;
            let id = item.id;
            list = list.child(
                div()
                    .id(("clipboard-item", index))
                    .p(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .bg(rgb(if selected { annotate::ACTIVE_BG } else { 0x181b20 }))
                    .hover(|s| s.bg(rgb(annotate::HOVER)))
                    .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                        view.selected = index;
                        if event.click_count() >= 2 {
                            view.activate(index, view.paste_on_activate, window, cx);
                        }
                        cx.notify();
                    }))
                    .child(div().flex_none().child(leading))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(div().text_sm().text_color(rgb(annotate::TEXT)).truncate().child(title))
                            .child(div().text_xs().text_color(rgb(annotate::MUTED)).child(meta)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(2.0))
                            .child(self.small_button(("clipboard-copy", index), text("复制")).on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                view.activate(index, false, window, cx);
                            })))
                            .child(self.small_button(("clipboard-pin", index), text("贴图")).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                view.pin(index, cx);
                            })))
                            .child(self.small_button(("clipboard-favorite", index), if favorite { "★".into() } else { "☆".into() }).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                let result = view.service.set_favorite(id, !favorite);
                                view.run(result, cx);
                            })))
                            .child(self.small_button(("clipboard-delete", index), text("删除")).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                let result = view.service.delete(&[id]);
                                view.run(result, cx);
                            }))),
                    ),
            );
        }
        if self.next_cursor.is_some() {
            list = list.child(self.small_button("clipboard-more", text("加载更多")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                let cursor = view.next_cursor.clone();
                view.load(cursor, cx);
            })));
        }
        let status = self.service.status();
        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_xs()
            .text_color(rgb(annotate::MUTED))
            .child(div().flex_1().child(format!(
                "{} {} · {}",
                text("共"),
                status.count,
                text(if status.enabled { "记录中" } else { "未记录（可在设置中开启）" })
            )))
            .child(if self.confirm_clear {
                div()
                    .flex()
                    .gap(px(4.0))
                    .child(self.small_button("clipboard-clear-confirm", text("确认清空（保留收藏）")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.confirm_clear = false;
                        let result = view.service.clear(false);
                        view.run(result, cx);
                    })))
                    .child(self.small_button("clipboard-clear-cancel", text("取消")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.confirm_clear = false;
                        cx.notify();
                    })))
            } else {
                div().child(self.small_button("clipboard-clear", text("清空")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.confirm_clear = true;
                    cx.notify();
                })))
            });
        let notice = if let Some(error) = self.error.clone().or(status.error.clone()) {
            Some(div().text_xs().text_color(rgb(0xfb7185)).child(error))
        } else {
            self.message.clone().map(|m| div().text_xs().text_color(rgb(0x86efac)).child(m))
        };
        div()
            .id("clipboard-history")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(10.0))
            .bg(rgb(0x181b20))
            .text_color(rgb(annotate::TEXT))
            .child(header)
            .child(tabs)
            .child(list)
            .children(notice)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::{swatch, Category};
    use snap_core::clipboard::model::{Kind, TextCategory};

    #[test]
    fn categories_map_to_repository_queries() {
        let q = Category::Link.query("abc", None);
        assert_eq!(q.kind, Some(Kind::Text));
        assert_eq!(q.text_category, Some(TextCategory::Link));
        assert!(!q.favorites_only);
        assert!(Category::Favorites.query("", Some("1:2".into())).favorites_only);
        assert_eq!(Category::Image.query(&"x".repeat(300), None).keyword.chars().count(), 200);
        q.validate().unwrap();
    }

    #[test]
    fn color_swatches_normalize_hex() {
        assert_eq!(swatch("#ff0000"), Some(0xff0000));
        assert_eq!(swatch(" #0f0 "), Some(0x00ff00));
        assert_eq!(swatch("hello"), None);
    }
}
