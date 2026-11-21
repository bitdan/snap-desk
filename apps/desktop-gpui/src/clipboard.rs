//! 剪贴板历史：后台记录服务（`snap_core::clipboard::service`）与历史窗口。
//!
//! 记录：启动时按设置 `clipboard` 启用；监听、读取、去重、写入 SQLite 与图片仓库都在 core 的工作线程，
//! 变化与错误经回调转发到主线程刷新窗口。数据目录与 Tauri 版相同（`app_local_data_dir/clipboard`），
//! 两个版本共用同一份历史；开发调试可用 `SNAP_DESK_CLIPBOARD_DIR` 指向另一目录，避免写入真实历史。
//!
//! 窗口（对照 Vue 版 `ClipboardHistory.vue`）：搜索、分类（全部/文本/链接/颜色/文件路径/图片/文件/收藏）、
//! 分页加载、键盘导航；回车/双击按设置“唤出后直接粘贴”粘贴到唤出前的窗口（否则只复制），
//! Ctrl+1～9 取第 N 条；每条可复制、贴图、收藏、删除；清空（默认保留收藏，二次确认）；
//! “固定”时粘贴后不收起。单击选中、Ctrl/⌘ 多选、Shift 连选（按选择顺序，最多 100 条）；
//! 多选后回车或 Ctrl+C 按 core `service::combine` 合并（文本逐行拼接，图片/文件合成文件列表）；
//! 所选文本与图片可组合导出为 PDF、长图或分别保存（`clipboard_export.rs`）；文本与图片可在独立窗口预览
//! 全文或原图（`clipboard_preview.rs`）。按住条目拖动超过几像素即系统拖出（Windows OLE，core `snap_core::drag`）：
//! 文本拖出文字，图片拖出临时 PNG，文件拖出原路径；拖动多选中的条目时按合并规则一起拖出。
//! 分类标签可拖动排序，或 Alt+←/→ 移动当前分类；顺序保存在设置文件旁的 `clipboard-category-order.json`
//! （Vue 版存于 WebView localStorage，两版不共用）。
//! 唤出时窗口左上角贴近光标并夹紧在所在显示器工作区内；失去焦点约 200 毫秒后仍未回到本窗口即收起
//! （固定、确认中、拖出中、预览或导出窗口打开时不收起）；固定时失焦记下切换到的窗口，作为之后的粘贴目标。
//! 底部开关可暂停/继续记录、切换“回车或双击后直接粘贴到原窗口”，写回 settings.json（`shell::update_clipboard`）。
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
    service::{self, Content, Event, Service, MAX_SELECTION},
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

/// 记录服务句柄（暂存架等其他窗口共用同一份历史）。
pub fn service(cx: &App) -> Option<Service> {
    cx.try_global::<History>().map(|h| h.service.clone())
}

/// 写回系统剪贴板，并标记自写入（监听线程跳过，不记成新条目）。`window` 是 Windows 图片剪贴板的所有者。
pub fn write_back(service: &Service, content: &Content, window: &Window, cx: &mut App) -> Result<(), String> {
    let _gate = service.write_gate.lock().map_err(|_| "剪贴板写入状态不可用。")?;
    match content {
        Content::Text(text) => cx.write_to_clipboard(ClipboardItem::new_string(text.clone())),
        Content::Image(image) => {
            let png = crate::capture::png(image)?;
            crate::capture::write_clipboard(cx, crate::native::hwnd(window), image, png)?;
        }
        Content::Files(paths) => platform::write_files(paths)?,
    }
    service.mark_restored(content)
}

/// 后台读取一条历史并创建居中贴图（文字与文件路径排版成图片）。
pub fn pin_item(cx: &mut App, service: Service, id: i64) -> Task<Result<(), String>> {
    cx.spawn(async move |cx| {
        let prepared = cx.background_executor().spawn(async move { prepare_pin(&service, id) }).await;
        cx.update(|cx| {
            prepared.and_then(|(image, drag)| crate::pins::create(cx, image, Vec::new(), crate::pins::Placement::Centered, drag))
        })
    })
}

/// 系统拖出：后台读取并按合并规则准备内容，回到主线程在框架状态借用之外发起拖拽（DoDragDrop 自带消息循环，
/// 会重入窗口过程，不能在 entity update 中调用）。准备期间已松开左键则不拖出（core 内检查）。
pub fn drag_ids(cx: &mut App, service: Service, ids: Vec<i64>) -> Task<Result<(), String>> {
    cx.spawn(async move |cx| {
        let payload = cx
            .background_executor()
            .spawn(async move {
                let contents = service.contents(&ids)?;
                let content = service::combine(contents, snap_core::drag::image_file)?;
                Ok::<_, String>(match content {
                    Content::Text(text) => snap_core::drag::Payload::Text(text),
                    Content::Files(paths) => snap_core::drag::Payload::Files(paths),
                    Content::Image(image) => snap_core::drag::Payload::Files(vec![snap_core::drag::image_file(&image)?]),
                })
            })
            .await;
        #[cfg(windows)]
        let result = payload.and_then(snap_core::drag::start);
        #[cfg(not(windows))]
        let result = payload.and_then(|_| Err("当前平台尚未支持内容拖出。".to_string()));
        result
    })
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
    let (bounds, display_id) = placement(cx, size(px(460.0), px(640.0)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
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

/// 光标右下方 12 像素，夹紧到工作区（单轴）；窗口比工作区大时贴齐起点。
fn near_cursor(cursor: f32, size: f32, start: f32, extent: f32) -> f32 {
    (cursor + 12.0).min(start + extent - size).max(start)
}

/// 窗口靠近光标显示（与 Tauri 版 `window::place` 一致）；取不到光标或显示器时居中。
fn placement(cx: &App, size: Size<Pixels>) -> (Bounds<Pixels>, Option<DisplayId>) {
    let centered = || (Bounds::centered(None, size, cx), None);
    let Some(cursor) = crate::native::cursor_point() else {
        return centered();
    };
    let displays = cx.displays();
    let Some(display) = displays.iter().find(|d| d.bounds().contains(&cursor)) else {
        return centered();
    };
    let area = display.visible_bounds();
    let x = near_cursor(f32::from(cursor.x), f32::from(size.width), f32::from(area.origin.x), f32::from(area.size.width));
    let y = near_cursor(f32::from(cursor.y), f32::from(size.height), f32::from(area.origin.y), f32::from(area.size.height));
    (Bounds::new(point(px(x), px(y)), size), Some(display.id()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// 默认顺序与 Vue 版 `use-category-order.ts` 一致。
const CATEGORIES: [Category; 8] = [
    Category::All,
    Category::Text,
    Category::Image,
    Category::File,
    Category::Link,
    Category::Path,
    Category::Color,
    Category::Favorites,
];

impl Category {
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Text => "文本",
            Self::Link => "链接",
            Self::Color => "颜色",
            Self::Path => "文件路径",
            Self::Image => "图片",
            Self::File => "文件",
            Self::Favorites => "收藏",
        }
    }

    /// 持久化用的键，与 Vue 版的分类值一致（全部为空字符串）。
    fn key(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Text => "text",
            Self::Link => "link",
            Self::Color => "color",
            Self::Path => "path",
            Self::Image => "image",
            Self::File => "file",
            Self::Favorites => "favorites",
        }
    }

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

/// 读取保存的顺序：去掉未知与重复的键，缺少的分类按默认顺序补在末尾；无效内容回到默认顺序。
fn parse_order(json: &str) -> Vec<Category> {
    let saved: Vec<String> = serde_json::from_str(json).unwrap_or_default();
    let mut order: Vec<Category> = Vec::new();
    for key in saved {
        if let Some(category) = CATEGORIES.into_iter().find(|c| c.key() == key) {
            if !order.contains(&category) {
                order.push(category);
            }
        }
    }
    let missing: Vec<Category> = CATEGORIES.into_iter().filter(|c| !order.contains(c)).collect();
    order.extend(missing);
    order
}

/// 把 `source` 移到 `destination` 当前所在的位置（与前端 save 的 splice 规则一致）。
fn move_category(order: &[Category], source: Category, destination: Category) -> Vec<Category> {
    let mut next = order.to_vec();
    let (Some(from), Some(to)) = (next.iter().position(|c| *c == source), next.iter().position(|c| *c == destination)) else {
        return next;
    };
    let item = next.remove(from);
    next.insert(to, item);
    next
}

fn order_path() -> Option<PathBuf> {
    crate::shell::settings_dir().map(|dir| dir.join("clipboard-category-order.json"))
}

fn load_order() -> Vec<Category> {
    order_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|json| parse_order(&json))
        .unwrap_or_else(|| CATEGORIES.to_vec())
}

fn save_order(order: &[Category]) -> Result<(), String> {
    let path = order_path().ok_or("应用数据目录不可用。")?;
    let keys: Vec<&str> = order.iter().map(|c| c.key()).collect();
    let json = serde_json::to_string(&keys).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|_| "分类顺序仅在当前窗口生效，无法保存。".to_string())
}

/// 拖动中的分类标签。
struct CategoryDrag(Category);
struct DragChip(String);
impl Render for DragChip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(6.0))
            .text_xs()
            .bg(rgb(annotate::ACTIVE_BG))
            .text_color(rgb(annotate::ACTIVE_FG))
            .opacity(0.85)
            .child(self.0.clone())
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

/// 选择规则（与前端 `selectClipboardItems` 一致）：普通单击替换为单选；`toggle`（Ctrl/⌘）保留顺序增删；
/// `range`（Shift）按当前列表从锚点连选，同时按 Ctrl 时并入原选择。最多 [`MAX_SELECTION`] 条。
fn select_items(ids: &[i64], id: i64, ordered: &[i64], anchor: Option<i64>, toggle: bool, range: bool) -> Vec<i64> {
    let position = |value: i64| ordered.iter().position(|v| *v == value);
    if range {
        if let (Some(start), Some(end)) = (anchor.and_then(position), position(id)) {
            let mut result: Vec<i64> = if toggle { ids.to_vec() } else { Vec::new() };
            for value in &ordered[start.min(end)..=start.max(end)] {
                if !result.contains(value) {
                    result.push(*value);
                }
            }
            result.truncate(MAX_SELECTION);
            return result;
        }
    }
    if !toggle {
        return vec![id];
    }
    if ids.contains(&id) {
        ids.iter().copied().filter(|v| *v != id).collect()
    } else if ids.len() < MAX_SELECTION {
        ids.iter().copied().chain([id]).collect()
    } else {
        ids.to_vec()
    }
}

/// 所选条目能否一起复制：文本不能与图片/文件混选（core 合并时同样拒绝），按钮提前说明。
fn can_combine(kinds: &[Kind]) -> bool {
    !kinds.is_empty() && (kinds.iter().all(|k| *k == Kind::Text) || kinds.iter().all(|k| *k != Kind::Text))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Clear,
    Delete,
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
    order: Vec<Category>,
    items: Vec<Item>,
    next_cursor: Option<String>,
    /// 键盘焦点所在行（浏览不改变多选）。
    selected: usize,
    /// 多选的条目 ID，按选择顺序（即合并与导出顺序）。
    picked: Vec<i64>,
    /// 所选条目的快照：后台刷新把条目挤出当前页时仍能判断类型。
    picked_items: HashMap<i64, Item>,
    anchor: Option<i64>,
    /// 左键按下的行与位置；移动超过阈值后发起拖出。
    press: Option<(usize, Point<Pixels>)>,
    /// 系统拖出进行中（模态消息循环期间焦点可能离开窗口，不能当作失焦收起）。
    dragging: bool,
    loading: bool,
    /// 每次查询递增，丢弃过期结果。
    generation: u64,
    thumbs: HashMap<i64, Arc<RenderImage>>,
    thumbs_pending: std::collections::HashSet<i64>,
    error: Option<String>,
    message: Option<String>,
    keep_open: bool,
    confirm: Option<Confirm>,
    focus: FocusHandle,
    _search: Subscription,
    _activation: Subscription,
}

impl HistoryView {
    fn new(service: Service, settings: snap_core::settings::Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = settings.appearance.language.clone();
        let placeholder = snap_core::i18n::for_language(&language, "搜索剪贴板历史（Ctrl+F）").to_owned();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe_in(&search, window, |this: &mut Self, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.clear_selection();
                this.reload(cx);
            }
        });
        search.update(cx, |state, cx| state.focus(window, cx));
        let activation = cx.observe_window_activation(window, |view: &mut Self, window, cx| {
            if !window.is_window_active() {
                view.blurred(window, cx);
            }
        });
        let mut view = Self {
            service,
            language,
            paste_on_activate: settings.clipboard.paste_on_activate,
            search,
            category: Category::All,
            order: load_order(),
            items: Vec::new(),
            next_cursor: None,
            selected: 0,
            picked: Vec::new(),
            picked_items: HashMap::new(),
            anchor: None,
            press: None,
            dragging: false,
            loading: false,
            generation: 0,
            thumbs: HashMap::new(),
            thumbs_pending: Default::default(),
            error: None,
            message: None,
            keep_open: false,
            confirm: None,
            focus: cx.focus_handle(),
            _search: subscription,
            _activation: activation,
        };
        view.reload(cx);
        view
    }

    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    /// 预览、导出、确认或拖出期间失焦不收起。
    fn occupied(&self, cx: &App) -> bool {
        self.confirm.is_some()
            || self.dragging
            || crate::clipboard_preview::is_open(cx)
            || crate::clipboard_export::is_open(cx)
    }

    /// 失焦后延迟复核：点击标题栏、打开系统菜单等会短暂失焦再取回；仍未回到本窗口才收起。
    fn blurred(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
            let active = handle.update(cx, |_, window, _| window.is_window_active()).unwrap_or(true);
            if active {
                return;
            }
            let keep = this
                .update(cx, |view, cx| {
                    // 切到了其他应用：记下它作为之后的粘贴目标（前台仍是本程序时 core 返回 0，保留原目标）。
                    let switched = platform::foreground_window();
                    if switched != 0 {
                        if cx.has_global::<History>() {
                            cx.global_mut::<History>().target = switched;
                        }
                    }
                    view.keep_open || view.occupied(cx)
                })
                .unwrap_or(true);
            if !keep {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                log::info!("event=gpui.clipboard_window_hidden reason=blur");
            }
        })
        .detach();
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
                        // 同一条目以新数据为准（收藏状态等）；不在当前页的所选条目保留旧快照。
                        for item in &view.items {
                            if let Some(picked) = view.picked_items.get_mut(&item.id) {
                                *picked = item.clone();
                            }
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

    fn reorder(&mut self, source: Category, destination: Category, cx: &mut Context<Self>) {
        if source == destination {
            return;
        }
        self.order = move_category(&self.order, source, destination);
        if let Err(error) = save_order(&self.order) {
            self.error = Some(error);
        }
        cx.notify();
    }

    /// 暂停/继续记录：先落盘，再在后台按新设置重启监听与维护（与 Tauri 版 `clipboard_set_enabled` 一致）。
    fn set_recording(&mut self, enabled: bool, cx: &mut Context<Self>) {
        match crate::shell::update_clipboard(cx, |c| c.enabled = enabled) {
            Ok(settings) => {
                self.error = None;
                self.message = enabled.then(|| self.t("已开始记录，仅记录此后复制的内容。"));
                let service = self.service.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx.background_executor().spawn(async move { service.configure(settings) }).await;
                    let _ = this.update(cx, |view, cx| {
                        if let Err(error) = result {
                            view.service.report(error.clone());
                            view.error = Some(error);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn set_paste_on_activate(&mut self, value: bool, cx: &mut Context<Self>) {
        match crate::shell::update_clipboard(cx, |c| c.paste_on_activate = value) {
            Ok(settings) => {
                self.paste_on_activate = settings.paste_on_activate;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn set_category(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = category;
        self.selected = 0;
        self.clear_selection();
        self.reload(cx);
    }

    fn write_back(&self, content: &Content, window: &Window, cx: &mut App) -> Result<(), String> {
        write_back(&self.service, content, window, cx)
    }

    /// 按选择顺序读取并合并：一条原样写回；多条交给 core `combine`（图片写成临时 PNG 作为文件引用）。
    fn combined(&self, ids: &[i64]) -> Result<Content, String> {
        let contents = self.service.contents(ids)?;
        service::combine(contents, snap_core::drag::image_file)
    }

    fn clear_selection(&mut self) {
        self.picked.clear();
        self.picked_items.clear();
        self.anchor = None;
    }

    fn pick(&mut self, index: usize, toggle: bool, range: bool) {
        let Some(item) = self.items.get(index).cloned() else {
            return;
        };
        let ordered: Vec<i64> = self.items.iter().map(|i| i.id).collect();
        self.picked = select_items(&self.picked, item.id, &ordered, self.anchor, toggle, range);
        if !range {
            self.anchor = Some(item.id);
        }
        let previous = std::mem::take(&mut self.picked_items);
        let items = &self.items;
        self.picked_items = self
            .picked
            .iter()
            .filter_map(|id| {
                let item = items.iter().find(|i| i.id == *id).cloned().or_else(|| previous.get(id).cloned())?;
                Some((*id, item))
            })
            .collect();
    }

    fn picked_kinds(&self) -> Vec<Kind> {
        self.picked.iter().filter_map(|id| self.picked_items.get(id).map(|i| i.kind)).collect()
    }

    /// 回车、双击：焦点条目在多选之中时作用于整组，否则只取这一条。
    fn group_for(&self, index: usize) -> Vec<i64> {
        let Some(id) = self.items.get(index).map(|i| i.id) else {
            return Vec::new();
        };
        if self.picked.len() > 1 && self.picked.contains(&id) { self.picked.clone() } else { vec![id] }
    }

    /// 复制所选条目；`paste` 时交还唤出前的窗口并发送粘贴键。
    fn activate(&mut self, index: usize, paste: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.group_for(index);
        self.activate_ids(ids, paste, window, cx);
    }

    fn activate_ids(&mut self, ids: Vec<i64>, paste: bool, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let result = self.combined(&ids).and_then(|content| self.write_back(&content, window, cx));
        if let Err(error) = result {
            self.error = Some(error);
            cx.notify();
            return;
        }
        if ids.len() > 1 && paste && !self.keep_open {
            // 收起时清除选择，下次唤出回到最新记录。
            self.clear_selection();
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
            log::info!("event=gpui.clipboard_pasted count={}", ids.len());
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

    fn motion(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((index, origin)) = self.press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.press = None;
            return;
        }
        let delta = event.position - origin;
        if delta.x.abs() + delta.y.abs() > px(6.0) {
            self.press = None;
            self.drag_out(index, cx);
        }
    }

    /// 后台读取并准备内容，回到主线程在框架状态借用之外发起系统拖拽（DoDragDrop 自带消息循环，
    /// 会重入窗口过程，不能在 entity update 中调用）。准备期间已松开左键则不拖出（core 内检查）。
    fn drag_out(&mut self, index: usize, cx: &mut Context<Self>) {
        if !cfg!(windows) {
            self.error = Some(self.t("当前平台尚未支持内容拖出。"));
            cx.notify();
            return;
        }
        let ids = self.group_for(index);
        if ids.is_empty() {
            return;
        }
        self.dragging = true;
        let task = drag_ids(cx, self.service.clone(), ids);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, _| view.dragging = false);
            match result {
                Ok(()) => log::info!("event=gpui.clipboard_drag_finished"),
                Err(error) => {
                    log::warn!("event=gpui.clipboard_drag_failed message={error}");
                    let _ = this.update(cx, |view, cx| {
                        view.error = Some(error);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 底栏“贴图”：逐条创建贴图，失败时说明已创建几张。
    fn pin_picked(&mut self, cx: &mut Context<Self>) {
        let ids = self.picked.clone();
        let service = self.service.clone();
        let created = self.t("已创建贴图");
        cx.spawn(async move |this, cx| {
            let mut done = 0;
            let mut failure = None;
            for id in ids {
                let service = service.clone();
                let prepared = cx.background_executor().spawn(async move { prepare_pin(&service, id) }).await;
                let result = cx.update(|cx| {
                    prepared.and_then(|(image, drag)| {
                        crate::pins::create(cx, image, Vec::new(), crate::pins::Placement::Centered, drag)
                    })
                });
                match result {
                    Ok(()) => done += 1,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            let _ = this.update(cx, |view, cx| {
                match failure {
                    Some(error) => view.error = Some(format!("{created} {done}；{error}")),
                    None => view.message = Some(format!("{created} {done}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pin(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(id) = self.items.get(index).map(|i| i.id) else {
            return;
        };
        let task = pin_item(cx, self.service.clone(), id);
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
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
                if self.confirm.is_some() {
                    self.confirm = None;
                } else if !self.picked.is_empty() {
                    self.clear_selection();
                } else if !self.search.read(cx).value().is_empty() {
                    self.search.update(cx, |state, cx| state.set_value("", window, cx));
                } else {
                    window.remove_window();
                }
            }
            "left" | "right" if keystroke.modifiers.alt => {
                // Alt+←/→：把当前分类与相邻分类交换位置。
                let index = self.order.iter().position(|c| *c == self.category).unwrap_or(0);
                let target = if key == "left" { index.checked_sub(1) } else { Some(index + 1) };
                if let Some(destination) = target.and_then(|i| self.order.get(i).copied()) {
                    self.reorder(self.category, destination, cx);
                }
            }
            "down" => self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1)),
            "up" => self.selected = self.selected.saturating_sub(1),
            "enter" => self.activate(self.selected, self.paste_on_activate, window, cx),
            "f" if command => self.search.update(cx, |state, cx| state.focus(window, cx)),
            "c" if command && !self.search.read(cx).focus_handle(cx).is_focused(window) => {
                self.activate(self.selected, false, window, cx)
            }
            "space" if !self.search.read(cx).focus_handle(cx).is_focused(window) => self.pick(self.selected, true, false),
            "delete" if !self.search.read(cx).focus_handle(cx).is_focused(window) => {
                if self.group_for(self.selected).len() > 1 {
                    self.confirm = Some(Confirm::Delete);
                } else if let Some(id) = self.items.get(self.selected).map(|i| i.id) {
                    let result = self.service.delete(&[id]);
                    self.run(result, cx);
                }
            }
            digit if command && digit.len() == 1 && ('1'..='9').contains(&digit.chars().next().unwrap_or('0')) => {
                // Ctrl/⌘+1～9 始终只取第 N 条，不作用于多选。
                let index = digit.parse::<usize>().unwrap_or(1) - 1;
                if let Some(id) = self.items.get(index).map(|i| i.id) {
                    self.activate_ids(vec![id], self.paste_on_activate, window, cx);
                }
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

/// 读取一条历史并准备贴图画面：图片原样，文字与文件路径排版成图片。
fn prepare_pin(service: &Service, id: i64) -> Result<(RgbaImage, Option<String>), String> {
    match service.contents(&[id])?.remove(0) {
        Content::Image(image) => Ok((image, None)),
        Content::Text(text) => text_pin(&text),
        Content::Files(paths) => text_pin(&paths.join("\n")),
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
        for (index, category) in self.order.clone().into_iter().enumerate() {
            let active = self.category == category;
            let label = category.label();
            let chip = text(label);
            tabs = tabs.child(
                div()
                    .id(("clipboard-tab", index))
                    .border_l_2()
                    .border_color(rgb(0x181b20))
                    .on_drag(CategoryDrag(category), move |_, _, _, cx| cx.new(|_| DragChip(chip.clone())))
                    .drag_over::<CategoryDrag>(|style, _, _, _| style.border_color(rgb(annotate::ACTIVE_FG)))
                    .on_drop(cx.listener(move |view, drag: &CategoryDrag, _, cx| view.reorder(drag.0, category, cx)))
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
                annotate::icon_button("clipboard-keep-open", annotate::icons::PIN, text("固定窗口，失去焦点时不收起"), keep_open, false)
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
            let picked = self.picked.contains(&item.id);
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
                    .bg(rgb(if picked { annotate::ACTIVE_BG } else if selected { 0x232830 } else { 0x181b20 }))
                    .border_1()
                    .border_color(rgb(if selected { annotate::BORDER } else { 0x181b20 }))
                    .hover(|s| s.bg(rgb(annotate::HOVER)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |view, event: &MouseDownEvent, _, _| {
                        view.press = Some((index, event.position));
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|view, _: &MouseUpEvent, _, _| view.press = None))
                    .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| view.motion(event, cx)))
                    .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                        view.selected = index;
                        if event.click_count() >= 2 {
                            view.activate(index, view.paste_on_activate, window, cx);
                        } else {
                            let modifiers = event.modifiers();
                            view.pick(index, modifiers.control || modifiers.platform, modifiers.shift);
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
                            .children((item.kind != Kind::File).then(|| {
                                let item = item.clone();
                                self.small_button(("clipboard-preview", index), text("预览")).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    crate::clipboard_preview::open(cx, view.service.clone(), item.clone());
                                }))
                            }))
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
                                view.picked.retain(|v| *v != id);
                                view.picked_items.remove(&id);
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
        let picked = self.picked.len();
        let summary = if picked > 0 {
            format!("{} {picked}", text("已选"))
        } else {
            format!(
                "{} {} · {:.1} MiB · {}",
                text("共"),
                status.count,
                status.byte_size as f64 / 1_048_576.0,
                text(if status.enabled { "记录中" } else { "已暂停记录" })
            )
        };
        let actions = match self.confirm {
            Some(confirm) => {
                let label = match confirm {
                    Confirm::Clear => text("确认清空（保留收藏）"),
                    Confirm::Delete => format!("{} {picked}", text("确认删除所选")),
                };
                div()
                    .flex()
                    .gap(px(4.0))
                    .child(self.small_button("clipboard-confirm", label).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.confirm = None;
                        let result = match confirm {
                            Confirm::Clear => view.service.clear(false),
                            Confirm::Delete => {
                                let ids = view.picked.clone();
                                view.clear_selection();
                                view.service.delete(&ids)
                            }
                        };
                        view.run(result, cx);
                    })))
                    .child(self.small_button("clipboard-confirm-cancel", text("取消")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.confirm = None;
                        cx.notify();
                    })))
            }
            None if picked > 0 => {
                let kinds = self.picked_kinds();
                let combinable = can_combine(&kinds);
                let exportable = kinds.len() == picked && kinds.iter().all(|k| *k != Kind::File);
                let copy_label = text(if combinable { "复制所选" } else { "文本不能与图片或文件一起复制" });
                div()
                    .flex()
                    .gap(px(2.0))
                    .child(self.small_button("clipboard-copy-picked", copy_label).on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        if combinable {
                            let ids = view.picked.clone();
                            view.activate_ids(ids, false, window, cx);
                        }
                    })))
                    .child(self.small_button("clipboard-export-picked", text(if exportable { "组合导出" } else { "组合导出仅支持文本和图片，文件引用请复制后粘贴" })).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        if exportable {
                            let items: Vec<Item> = view.picked.iter().filter_map(|id| view.picked_items.get(id).cloned()).collect();
                            crate::clipboard_export::open(cx, view.service.clone(), items);
                        }
                    })))
                    .child(self.small_button("clipboard-pin-picked", text("贴图")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.pin_picked(cx))))
                    .child(self.small_button("clipboard-delete-picked", text("删除所选")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.confirm = Some(Confirm::Delete);
                        cx.notify();
                    })))
                    .child(self.small_button("clipboard-unpick", text("取消选择")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.clear_selection();
                        cx.notify();
                    })))
            }
            None => div().child(self.small_button("clipboard-clear", text("清空")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.confirm = Some(Confirm::Clear);
                cx.notify();
            }))),
        };
        let toggle = |id: &'static str, label: String, on: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .text_xs()
                .text_color(rgb(if on { annotate::ACTIVE_FG } else { annotate::MUTED }))
                .hover(|s| s.bg(rgb(annotate::HOVER)))
                .child(if on { "☑" } else { "☐" })
                .child(label)
        };
        let enabled = status.enabled;
        let paste = self.paste_on_activate;
        let switches = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .child(toggle("clipboard-recording", text("记录新复制的内容"), enabled).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_recording(!enabled, cx))))
            .child(toggle("clipboard-paste-toggle", text("回车或双击后直接粘贴到原窗口"), paste).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_paste_on_activate(!paste, cx))));
        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_xs()
            .text_color(rgb(if picked > 0 { annotate::ACTIVE_FG } else { annotate::MUTED }))
            .child(div().flex_1().min_w(px(0.0)).truncate().child(summary))
            .child(actions);
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
            .children((status.supported && !status.enabled).then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .bg(rgb(0x22364d))
                    .text_xs()
                    .text_color(rgb(0xc4dfff))
                    .child(div().flex_1().child(text("记录已暂停，新复制的内容不会保存。")))
                    .child(self.small_button("clipboard-resume", text("继续记录")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.set_recording(true, cx))))
            }))
            .child(list)
            .children(notice)
            .child(switches)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::{can_combine, near_cursor, move_category, parse_order, select_items, swatch, Category, CATEGORIES};
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
    fn selection_follows_click_modifiers() {
        let ordered = [5, 4, 3, 2, 1];
        assert_eq!(select_items(&[5, 3], 2, &ordered, Some(3), false, false), vec![2]);
        assert_eq!(select_items(&[5, 3], 2, &ordered, Some(3), true, false), vec![5, 3, 2]);
        assert_eq!(select_items(&[5, 3, 2], 3, &ordered, Some(3), true, false), vec![5, 2]);
        assert_eq!(select_items(&[5], 2, &ordered, Some(4), false, true), vec![4, 3, 2]);
        assert_eq!(select_items(&[1], 2, &ordered, Some(4), true, true), vec![1, 4, 3, 2]);
        // 锚点不在当前列表时 Shift 退化为普通单击。
        assert_eq!(select_items(&[], 2, &ordered, Some(9), false, true), vec![2]);
        let full: Vec<i64> = (0..100).collect();
        assert_eq!(select_items(&full, 200, &ordered, None, true, false).len(), 100);
    }

    #[test]
    fn category_order_is_restored_and_moved() {
        assert_eq!(parse_order("not json"), CATEGORIES.to_vec());
        let order = parse_order(r#"["favorites","bogus","image","image"]"#);
        assert_eq!(&order[..3], &[Category::Favorites, Category::Image, Category::All]);
        assert_eq!(order.len(), CATEGORIES.len());
        let moved = move_category(&CATEGORIES, Category::Favorites, Category::All);
        assert_eq!(moved[0], Category::Favorites);
        assert_eq!(moved[1], Category::All);
        let moved = move_category(&CATEGORIES, Category::All, Category::Text);
        assert_eq!(&moved[..2], &[Category::Text, Category::All]);
    }

    #[test]
    fn window_is_placed_near_cursor_inside_work_area() {
        assert_eq!(near_cursor(100.0, 460.0, 0.0, 1920.0), 112.0);
        // 靠近右/下边缘时贴住工作区边界。
        assert_eq!(near_cursor(1800.0, 460.0, 0.0, 1920.0), 1460.0);
        // 副屏在左侧（负坐标）。
        assert_eq!(near_cursor(-50.0, 460.0, -1280.0, 1280.0), -460.0);
        // 窗口比工作区大时贴齐起点。
        assert_eq!(near_cursor(10.0, 900.0, 0.0, 800.0), 0.0);
    }

    #[test]
    fn mixed_text_and_images_cannot_combine() {
        assert!(can_combine(&[Kind::Text, Kind::Text]));
        assert!(can_combine(&[Kind::Image, Kind::File]));
        assert!(!can_combine(&[Kind::Text, Kind::Image]));
        assert!(!can_combine(&[]));
    }

    #[test]
    fn color_swatches_normalize_hex() {
        assert_eq!(swatch("#ff0000"), Some(0xff0000));
        assert_eq!(swatch(" #0f0 "), Some(0x00ff00));
        assert_eq!(swatch("hello"), None);
    }
}
