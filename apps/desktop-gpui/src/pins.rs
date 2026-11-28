//! 贴图窗口：透明、置顶、不进任务栏的 PopUp 窗口，显示一张图片。
//!
//! 来源：截图选区（原位贴图，图片首像素对准选区）、剪贴板（图片、文件路径、文字，居中显示），
//! 以及剪贴板历史、暂存架和主界面“贴图”页的“打开图片贴图”（[`open_file`]）。
//! 操作：拖动移动、滚轮/±键缩放（10% 步进）、右键菜单（复制、保存、置顶、锁定、鼠标穿透、
//! 变换、不透明度、关闭）、双击图片关闭、Ctrl+C / Ctrl+S / Ctrl+W / Esc、Ctrl+Alt+P 切换穿透。
//! 鼠标穿透后窗口收不到点击，由全局贴图快捷键或托盘“贴图”恢复。
//!
//! 尺寸见 `snap_core::pins::PinLayout`：缩放 1 表示一图像像素对应一物理像素，
//! 窗口只包住图片（不留透明边距）；右键菜单是独立的弹出窗口，不撑大贴图窗口。
//! 像素预算、变换、文字排版同样来自 core。识别文字/二维码/翻译图片打开独立结果窗口（`recognition.rs`）。
//! 按住 Ctrl（macOS 为 ⌥）拖动把内容拖到其他应用：文字贴图拖出文字，其余拖出合成后的 PNG。
//! Windows 用 core 的 OLE 拖拽（`snap_core::drag`）；macOS 与 Linux Wayland 用元素拖拽（`drag_out.rs`），
//! X11 后端不支持拖到窗口外。
//! 剪贴板富文本（CF_HTML）由 core 的白名单排版器转成图片（旧版用 WebView 排版，效果略有差异）。
//!
//! 标注：空格（或右键菜单“显示工具栏”）在图片下方展开工具栏，窗口随之加高、加宽；选择标注工具后
//! 图片上的按下交给编辑器（`annotate.rs`），“移动贴图”按钮回到拖动窗口。标注坐标为贴图图像像素，
//! 复制/保存时合成到底图副本，不改变可编辑对象；旋转/翻转用 core `transform_annotations` 随图变换，
//! “恢复原图”恢复贴图创建时的标注。
//!
//! 选择文字：`T`、工具栏或右键菜单进入。用设置中所选的 OCR 引擎识别底图（不含标注），
//! 把逐行识别框铺成一层不可见的文字层（core `recognition::text_layer`），像网页一样拖选、
//! 双击取词、三击取行、Ctrl/⌘+A 全选、Ctrl/⌘+C 复制文字、`H` 把选区转成荧光笔标注；Esc 退出。
//! 文字层按底图版本缓存，旋转、翻转等变换后作废并按新图重新识别，不推算旧坐标。
//! 坐标与标注相同用图像像素，绘制时随缩放与 DPI 换算，因此缩放后选区仍贴合文字。
use crate::annotate::{self, Action, Editor, Job, Layers, Tool, ToolbarProps};
use gpui_kit::{
    component::input::{Textarea, TextareaState},
    *,
};
use image::RgbaImage;
use snap_core::{
    annotations::{Annotation, Point as ImagePoint, edit::Bounds as ImageBounds},
    pins::{self as core_pins, PinAppearance, PinLayout, PinShadow, Transform},
    recognition::{
        ocr::OcrResult,
        text_layer::{self, Selection, TextLayer, Unavailable},
    },
    settings::{ExportSettings, Language},
};
use std::{sync::Arc, time::Duration};

/// 右键菜单尺寸（逻辑像素）；打开时窗口临时扩大以容纳菜单，关闭后恢复。
const MENU_WIDTH: f32 = 224.0;
const ITEM_HEIGHT: f32 = 28.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const OPACITIES: [f32; 4] = [1.0, 0.8, 0.6, 0.4];
/// 工具栏完整显示所需宽度（逻辑像素）：移动 + 7 个工具分组 + 撤销/重做 + 缩放、选择文字、识别、处理、
/// 置顶、保存、关闭、复制。选择文字模式下多一个“高亮”按钮，另加 [`TEXT_BUTTON_WIDTH`]。
const TOOLBAR_WIDTH: f32 = 712.0;
/// 工具栏一个图标按钮占的宽度（30 + 间距）。
const TEXT_BUTTON_WIDTH: f32 = 32.0;
/// 选中文字的底色：主题强调色 #3b5bdb，约 35% 不透明，下面的文字仍清晰可读。
const TEXT_SELECTION: u32 = 0x3b5bdb59;
/// “高亮所选文字”生成的荧光笔：色板里的黄色，透明度与荧光笔工具默认值一致。
const TEXT_HIGHLIGHT_COLOR: &str = "#facc15";
const TEXT_HIGHLIGHT_OPACITY: f64 = 0.35;
/// “选择文字”按钮图标（Lucide text-cursor，ISC 许可，见 assets/icons/LICENSE-LUCIDE）。
const TEXT_CURSOR_ICON: &[u8] = include_bytes!("../assets/icons/text-cursor.svg");

/// 贴图位置：原位（截图选区）或居中（剪贴板）。
pub enum Placement {
    /// `origin` 为图片首像素的全局物理坐标，`scale` 为所在屏缩放。
    Source { origin: (i32, i32), scale: f64, display: Option<DisplayId> },
    Centered,
    /// 从贴图历史恢复：回到关闭时的位置与缩放，不透明度与锁定一并恢复。
    Restored(core_pins::history::State),
}

/// 所有贴图窗口；窗口关闭后弱引用失效，访问时顺手清理。
#[derive(Default)]
struct Registry {
    pins: Vec<(AnyWindowHandle, WeakEntity<Pin>)>,
}
impl Global for Registry {}

fn alive(cx: &mut App) -> Vec<(AnyWindowHandle, Entity<Pin>)> {
    let registry = cx.default_global::<Registry>();
    registry.pins.retain(|(_, pin)| pin.upgrade().is_some());
    registry
        .pins
        .iter()
        .filter_map(|(handle, pin)| Some((*handle, pin.upgrade()?)))
        .collect()
}

/// Rust 侧常驻像素：原图 + 与原图不同的当前图。
fn retained(pin: &Pin) -> u64 {
    let pixels = |image: &RgbaImage| u64::from(image.width()) * u64::from(image.height());
    pixels(&pin.original) + if Arc::ptr_eq(&pin.original, &pin.image) { 0 } else { pixels(&pin.image) }
}

fn log_memory(cx: &mut App, reason: &str) {
    let pins = alive(cx);
    let bytes: u64 = pins.iter().map(|(_, pin)| retained(pin.read(cx)) * 4).sum();
    log::info!("event=gpui.pins_memory reason={reason} count={} rgba_bytes={bytes}", pins.len());
}

/// BGRA 帧交给 GPU；原图 RGBA 仍保留用于导出和变换。
fn render_image(image: &RgbaImage) -> Arc<RenderImage> {
    let mut frame = image.clone();
    for pixel in frame.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(frame)]))
}

/// 正在退出：贴图随进程关闭，不记入“已关闭”历史（仍打开的贴图另存为“上次退出”）。
#[derive(Default)]
struct Quitting(bool);
impl Global for Quitting {}

/// 新建贴图。容量不足、窗口创建失败时返回错误，不留下半个窗口。
pub fn create(
    cx: &mut App,
    image: RgbaImage,
    annotations: Vec<Annotation>,
    placement: Placement,
    drag_text: Option<String>,
) -> Result<(), String> {
    snap_core::annotations::validate(&annotations, image.width(), image.height())?;
    let pins = alive(cx);
    let used = pins.iter().map(|(_, pin)| retained(pin.read(cx))).sum();
    core_pins::validate_capacity(image.width(), image.height(), pins.len(), used)?;
    let settings = crate::shell::settings(cx);
    let layout = PinLayout::load();
    let (width, height) = image.dimensions();
    let mut restored = None;
    let (bounds, display, zoom, scale) = match placement {
        Placement::Restored(state) => {
            let scale = if state.scale.is_finite() && state.scale > 0.0 { state.scale } else { 1.0 };
            let minimum = layout.minimum_zoom(width, height, scale);
            let zoom = if state.zoom.is_finite() && state.zoom > 0.0 { state.zoom.max(minimum) } else { 1.0_f64.max(minimum) };
            let inset = image_inset(&settings.pin_appearance.shadow, scale);
            let size = window_size(width, height, zoom, scale, &settings.pin_appearance.shadow);
            let origin = point(
                px((state.origin.0 - inset) as f32 / scale as f32),
                px((state.origin.1 - inset) as f32 / scale as f32),
            );
            let mut bounds = Bounds::new(origin, size);
            // 显示器布局变了、位置已不在任何屏幕上：改为在主屏居中。
            let visible = cx.displays().iter().any(|d| d.bounds().intersects(&bounds));
            if !visible {
                bounds = Bounds::centered(cx.primary_display().map(|d| d.id()), size, cx);
            }
            restored = Some((state.opacity.clamp(0.1, 1.0), state.locked));
            (bounds, None, zoom, scale)
        }
        Placement::Source { origin, scale, display } => {
            let zoom = 1.0_f64.max(layout.minimum_zoom(width, height, scale));
            let inset = image_inset(&settings.pin_appearance.shadow, scale);
            let size = window_size(width, height, zoom, scale, &settings.pin_appearance.shadow);
            let origin = point(
                px((origin.0 - inset) as f32 / scale as f32),
                px((origin.1 - inset) as f32 / scale as f32),
            );
            (Bounds::new(origin, size), display, zoom, scale)
        }
        Placement::Centered => {
            let primary = cx.primary_display();
            let scale = snap_core::capture::primary_scale();
            let visible = primary.as_ref().map(|d| d.visible_bounds());
            let work = visible
                .map(|b| core_pins::WorkArea {
                    x: (f32::from(b.origin.x) * scale as f32) as i32,
                    y: (f32::from(b.origin.y) * scale as f32) as i32,
                    width: (f32::from(b.size.width) * scale as f32) as i32,
                    height: (f32::from(b.size.height) * scale as f32) as i32,
                })
                .unwrap_or(core_pins::WorkArea { x: 0, y: 0, width: 1920, height: 1040 });
            let zoom = layout.fit_zoom(width, height, scale, work);
            let size = window_size(width, height, zoom, scale, &settings.pin_appearance.shadow);
            let id = primary.map(|d| d.id());
            (Bounds::centered(id, size, cx), id, zoom, scale)
        }
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let image = Arc::new(image);
    let frame = render_image(&image);
    let mut weak = None;
    let handle = cx
        .open_window(options, |window, cx| {
            let crop = image_bounds(&image);
            let editor = Editor::new(crop, annotations.clone());
            // 截图时画的标注随贴图带过来：创建时就画出图层，否则要等下一次编辑才显示。
            let mut layers = Layers::default();
            layers.update(&editor);
            let pin = cx.new(|cx| {
                // 关闭（非退出）时存入“已关闭”历史，供“恢复上次关闭的贴图”与贴图历史使用。
                cx.on_release(|pin: &mut Pin, cx| {
                    if !cx.default_global::<Quitting>().0 {
                        remember_closed(pin, cx);
                    }
                })
                .detach();
                Pin {
                original: image.clone(),
                editor,
                original_annotations: annotations,
                layers,
                annotating: false,
                text_select: None,
                text_cache: None,
                image_serial: 0,
                text_pending: None,
                toolbar: false,
                drag_text: drag_text.clone(),
                hidden: false,
                text_input: None,
                image,
                frame,
                zoom,
                opacity: restored.map_or(1.0, |(opacity, _)| opacity),
                locked: restored.is_some_and(|(_, locked)| locked),
                placed: None,
                topmost: true,
                click_through: false,
                move_from: None,
                appearance: settings.pin_appearance.clone(),
                export: settings.export.clone(),
                language: settings.appearance.language.clone(),
                menu: None,
                badge: None,
                badge_task: None,
                busy: false,
                focus: cx.focus_handle(),
                focused: false,
                own: None,
                toolbar_window: None,
                _bounds: None,
            }});
            // 贴图移动或缩放时，工具栏窗口跟着重新定位。
            crate::native::frameless(window);
            pin.update(cx, |pin, cx| {
                pin.remember_place(window);
                pin._bounds = Some(cx.observe_window_bounds(window, |pin: &mut Pin, window, cx| {
                    pin.remember_place(window);
                    pin.place_toolbar(window, cx);
                }));
            });
            // 实际所在屏的缩放与预估不同（多屏混合 DPI）时，按实际缩放修正窗口尺寸。
            if (f64::from(window.scale_factor()) - scale).abs() > 0.01 {
                let size = pin.read(cx).base_size(window.scale_factor());
                window.resize(size);
            }
            weak = Some(pin.downgrade());
            pin
        })
        .map_err(|e| e.to_string())?;
    let handle: AnyWindowHandle = handle.into();
    let _ = handle.update(cx, |_, window, _| window.activate_window());
    if let Some(weak) = &weak {
        let _ = weak.update(cx, |pin, _| pin.own = Some(handle));
    }
    if let Some(weak) = weak {
        cx.default_global::<Registry>().pins.push((handle, weak));
    }
    log_memory(cx, "created");
    Ok(())
}

/// 阴影的模糊半径与透明边距（逻辑像素）：阴影用主题色、范围小；关闭阴影时窗口正好是图片大小。
fn shadow_size(shadow: &PinShadow) -> (f32, f32) {
    match shadow {
        PinShadow::None => (0.0, 0.0),
        PinShadow::Soft => (4.0, 5.0),
        PinShadow::Auto => (6.0, 8.0),
    }
}

/// 图片首像素到窗口原点的物理距离：只留出阴影需要的边距。
fn image_inset(shadow: &PinShadow, scale: f64) -> i32 {
    (f64::from(shadow_size(shadow).1) * scale).round() as i32
}

/// 阴影颜色取设置（默认主题色 #3b5bdb），按档位调整浓淡。
fn pin_shadow(appearance: &PinAppearance) -> Vec<BoxShadow> {
    let shadow = &appearance.shadow;
    let (blur, _) = shadow_size(shadow);
    if blur == 0.0 {
        return Vec::new();
    }
    let alpha = if matches!(shadow, PinShadow::Auto) { 0x66 } else { 0x40 };
    let color = u32::from_str_radix(appearance.shadow_color.trim_start_matches('#'), 16)
        .unwrap_or_else(|_| u32::from_str_radix(&core_pins::DEFAULT_SHADOW_COLOR[1..], 16).unwrap_or(0x3b5bdb));
    vec![BoxShadow {
        color: rgba((color << 8) | alpha).into(),
        offset: point(px(0.0), px(1.0)),
        blur_radius: px(blur),
        spread_radius: px(0.0),
        inset: false,
    }]
}

fn image_bounds(image: &RgbaImage) -> ImageBounds {
    ImageBounds { x: 0.0, y: 0.0, width: f64::from(image.width()), height: f64::from(image.height()) }
}

/// 图片 + 两侧透明边距的窗口逻辑尺寸；边距按物理像素取整，首像素落在整数屏幕坐标上。
fn window_size(width: u32, height: u32, zoom: f64, scale: f64, shadow: &PinShadow) -> Size<Pixels> {
    let layout = PinLayout::load();
    let inset = image_inset(shadow, scale);
    let (w, h) = layout.displayed(width, height, zoom);
    size(
        px((w + inset * 2) as f32 / scale as f32),
        px((h + inset * 2) as f32 / scale as f32),
    )
}

/// 打开/保存对话框期间临时隐藏可见贴图，防止置顶贴图盖住系统对话框。
/// `except` 为对话框的父贴图（对话框归它所有，总在它上面）。返回本次隐藏的窗口，只恢复这些。
pub fn hide_pins(cx: &mut App, except: Option<AnyWindowHandle>) -> Vec<AnyWindowHandle> {
    let mut hidden = Vec::new();
    for (handle, pin) in alive(cx) {
        // 先比较句柄：调用方可能正处于自身贴图的事件处理中，不能读取它的实体。
        if Some(handle) == except || pin.read(cx).hidden {
            continue;
        }
        let result = handle.update(cx, |_, window, _| crate::native::set_visible(window, false));
        if let Some(toolbar) = pin.read(cx).toolbar_window {
            let _ = toolbar.update(cx, |_, window, _| crate::native::set_visible(window, false));
        }
        match result {
            Ok(Ok(())) => {
                pin.update(cx, |pin, _| pin.hidden = true);
                hidden.push(handle);
            }
            Ok(Err(error)) => log::warn!("event=gpui.pin_hide_failed message={error}"),
            Err(_) => {}
        }
    }
    if !hidden.is_empty() {
        log::info!("event=gpui.pins_hidden count={}", hidden.len());
    }
    hidden
}

/// 恢复 [`hide_pins`] 隐藏的贴图（不抢焦点）；已关闭的窗口跳过。
pub fn show_pins(cx: &mut App, hidden: Vec<AnyWindowHandle>) {
    for (handle, pin) in alive(cx) {
        if !hidden.contains(&handle) {
            continue;
        }
        let result = handle.update(cx, |_, window, _| crate::native::set_visible(window, true));
        if let Some(toolbar) = pin.read(cx).toolbar_window {
            let _ = toolbar.update(cx, |_, window, _| crate::native::set_visible(window, true));
        }
        if let Ok(Err(error)) = result {
            log::warn!("event=gpui.pin_show_failed message={error}");
        }
        pin.update(cx, |pin, _| pin.hidden = false);
    }
}

/// 全局贴图键：先恢复穿透中的贴图（它们收不到鼠标，只能这样找回来）；返回是否恢复了任何贴图。
pub fn restore_click_through(cx: &mut App) -> bool {
    let mut restored = 0;
    for (handle, pin) in alive(cx) {
        if !pin.read(cx).click_through {
            continue;
        }
        let _ = handle.update(cx, |_, window, cx| {
            pin.update(cx, |pin, cx| pin.set_click_through(false, window, cx));
        });
        restored += 1;
    }
    if restored > 0 {
        log::info!("event=gpui.pins_click_through_restored count={restored}");
    }
    restored > 0
}

/// “已关闭”贴图历史的列表缓存；磁盘写入完成后刷新，设置窗口观察它重绘。
#[derive(Default)]
pub struct HistoryList(pub Vec<core_pins::history::Entry>);
impl Global for HistoryList {}

/// 启动时调用：读取贴图历史列表；开启“启动时恢复贴图”时重新贴出上次退出时打开的贴图。
pub fn init(cx: &mut App, restore_session: bool) {
    refresh_history(cx);
    let Some(session) = core_pins::history::Store::session() else {
        return;
    };
    if !restore_session {
        // 关闭了该选项：不保留旧的退出现场，免得以后再开启时恢复很久以前的贴图。
        let _ = session.clear();
        return;
    }
    cx.spawn(async move |cx| {
        let records = cx
            .background_executor()
            .spawn(async move {
                let records: Vec<_> = session.list().iter().rev().filter_map(|entry| session.load(&entry.id).ok()).collect();
                let _ = session.clear();
                records
            })
            .await;
        cx.update(|cx| {
            let count = records.len();
            for (image, state) in records {
                if let Err(error) = create(cx, image, state.annotations.clone(), Placement::Restored(state.clone()), state.drag_text) {
                    log::warn!("event=gpui.pin_session_restore_failed message={error}");
                }
            }
            if count > 0 {
                log::info!("event=gpui.pins_session_restored count={count}");
            }
        });
    })
    .detach();
}

fn refresh_history(cx: &mut App) {
    let entries = core_pins::history::Store::closed().map(|store| store.list()).unwrap_or_default();
    cx.set_global(HistoryList(entries));
}

/// 贴图关闭：后台把当前底图与标注写入“已关闭”历史。
fn remember_closed(pin: &Pin, cx: &mut App) {
    let Some(store) = core_pins::history::Store::closed() else {
        return;
    };
    let image = pin.image.clone();
    let state = pin.history_state(core_pins::history::now_millis());
    cx.spawn(async move |cx| {
        let result = cx.background_executor().spawn(async move { store.push(&image, &state) }).await;
        cx.update(|cx| match result {
            Ok(_) => refresh_history(cx),
            Err(error) => log::warn!("event=gpui.pin_history_save_failed message={error}"),
        });
    })
    .detach();
}

/// 退出前调用：开启“启动时恢复贴图”时保存仍打开的贴图；之后关闭的窗口不再记入“已关闭”历史。
pub fn save_session(cx: &mut App) {
    cx.set_global(Quitting(true));
    if !crate::shell::settings(cx).pin_restore_on_startup {
        return;
    }
    let Some(session) = core_pins::history::Store::session() else {
        return;
    };
    let now = core_pins::history::now_millis();
    // 按创建顺序编号，恢复时保持原来的叠放次序。
    let records: Vec<_> = alive(cx)
        .iter()
        .enumerate()
        .map(|(index, (_, pin))| {
            let pin = pin.read(cx);
            (pin.image.clone(), pin.history_state(now + index as i64))
        })
        .collect();
    let refs: Vec<_> = records.iter().map(|(image, state)| (image.as_ref(), state)).collect();
    match session.replace(&refs) {
        Ok(()) => log::info!("event=gpui.pins_session_saved count={}", refs.len()),
        Err(error) => log::warn!("event=gpui.pins_session_save_failed message={error}"),
    }
}

/// 重新贴出一条“已关闭”历史；贴出后从历史移除（再次关闭时重新记入）。
pub fn restore(cx: &mut App, id: String) {
    let Some(store) = core_pins::history::Store::closed() else {
        return;
    };
    cx.spawn(async move |cx| {
        let loaded = cx
            .background_executor()
            .spawn(async move {
                let loaded = store.load(&id);
                if loaded.is_ok() {
                    let _ = store.remove(&id);
                }
                loaded
            })
            .await;
        cx.update(|cx| {
            let result = loaded.and_then(|(image, state)| {
                create(cx, image, state.annotations.clone(), Placement::Restored(state.clone()), state.drag_text)
            });
            match result {
                Ok(()) => log::info!("event=gpui.pin_created source=history"),
                Err(error) => log::error!("event=gpui.pin_failed source=history message={error}"),
            }
            refresh_history(cx);
        });
    })
    .detach();
}

/// 恢复最近关闭的一张贴图；没有历史时返回 false。
pub fn restore_last(cx: &mut App) -> bool {
    let Some(id) = cx.default_global::<HistoryList>().0.first().map(|entry| entry.id.clone()) else {
        log::info!("event=gpui.pin_history_empty");
        return false;
    };
    restore(cx, id);
    true
}

pub fn remove_history(cx: &mut App, id: &str) {
    if let Some(store) = core_pins::history::Store::closed() {
        if let Err(error) = store.remove(id) {
            log::warn!("event=gpui.pin_history_remove_failed message={error}");
        }
    }
    refresh_history(cx);
}

pub fn clear_history(cx: &mut App) {
    if let Some(store) = core_pins::history::Store::closed() {
        if let Err(error) = store.clear() {
            log::warn!("event=gpui.pin_history_clear_failed message={error}");
        }
    }
    refresh_history(cx);
}

/// 剪贴板贴图：图片优先，其次文件路径、文字（排成图片）。解码与排版在后台线程进行。
pub fn from_clipboard(cx: &mut App) {
    enum Source {
        Image(Vec<u8>),
        Text(String),
        /// 富文本与其纯文字（拖出时提供文字）。
        Html(String, Option<String>),
    }
    // 顺序：文件路径 → 图片 → 富文本 → 纯文字。
    let item = cx.read_from_clipboard();
    let entries = item.as_ref().map(|item| item.entries().to_vec()).unwrap_or_default();
    let plain = entries.iter().find_map(|entry| match entry {
        ClipboardEntry::String(text) => Some(text.text().to_string()),
        _ => None,
    });
    let source = entries
        .iter()
        .find_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => Some(Source::Text(
                paths.paths().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"),
            )),
            _ => None,
        })
        .or_else(|| {
            entries.iter().find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(Source::Image(image.bytes.clone())),
                _ => None,
            })
        })
        .or_else(|| match snap_core::pins::html::read() {
            Ok(Some(html)) => Some(Source::Html(html, plain.clone())),
            Ok(None) => None,
            Err(error) => {
                log::warn!("event=gpui.pin_clipboard_html_unreadable message={error}");
                None
            }
        })
        .or_else(|| plain.clone().map(Source::Text));
    let Some(source) = source else {
        log::warn!("event=gpui.pin_clipboard_empty message=剪贴板没有可读取的图片或文字。");
        return;
    };
    cx.spawn(async move |cx| {
        let image = cx
            .background_executor()
            .spawn(async move {
                match source {
                    Source::Image(bytes) => decode_clipboard_image(&bytes).map(|image| (image, None)),
                    Source::Text(text) => {
                        let font = core_pins::text::font_bytes(annotate::user_font())?;
                        let drag = core_pins::text::drag_text(&text);
                        core_pins::text::render(&text, &font).map(|image| (image, Some(drag)))
                    }
                    Source::Html(html, text) => {
                        // 没有 WebView：用 core 的白名单排版器（常规 / 粗体 / 等宽字体，缺字回退中文字体）。
                        let fonts = snap_core::annotations::Fonts::system(annotate::user_font(), Some("mono"), true)?;
                        let drag = text.map(|text| core_pins::text::drag_text(&text));
                        core_pins::html::render(&html, &fonts).map(|image| (image, drag))
                    }
                }
            })
            .await;
        cx.update(|cx| match image.and_then(|(image, drag)| create(cx, image, Vec::new(), Placement::Centered, drag)) {
            Ok(()) => log::info!("event=gpui.pin_created source=clipboard"),
            Err(error) => log::error!("event=gpui.pin_failed source=clipboard message={error}"),
        });
    })
    .detach();
}

/// 主界面“贴图”页的“打开图片贴图”：选择 PNG / JPEG / WebP，解码与像素上限检查见 core `read_image`，居中显示。
/// 取消不创建贴图。
pub fn open_file(cx: &mut App) {
    let language = crate::shell::settings(cx).appearance.language;
    let text = |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(text("打开图片贴图"))
        .add_filter(text("图片（PNG / JPEG / WebP）"), &["png", "jpg", "jpeg", "webp"]);
    let hidden = hide_pins(cx, None);
    cx.spawn(async move |cx| {
        let picked = dialog.pick_file().await;
        cx.update(|cx| show_pins(cx, hidden));
        let Some(file) = picked else {
            log::info!("event=gpui.pin_file_cancelled");
            return;
        };
        let path = file.path().to_path_buf();
        let image = cx
            .background_executor()
            .spawn(async move { core_pins::content::read_image(&path) })
            .await;
        cx.update(|cx| match image.and_then(|image| create(cx, image, Vec::new(), Placement::Centered, None)) {
            Ok(()) => log::info!("event=gpui.pin_created source=file"),
            Err(error) => log::error!("event=gpui.pin_failed source=file message={error}"),
        });
    })
    .detach();
}

/// 剪贴板图片可能是 PNG、JPEG 或由 CF_DIB 转来的 BMP。32 位 DIB 常把 alpha 全写成 0，
/// 此时按不透明处理，避免贴出一张全透明的图。
fn decode_clipboard_image(bytes: &[u8]) -> Result<RgbaImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut image = reader.decode().map_err(|e| format!("剪贴板图片解码失败：{e}"))?.to_rgba8();
    if u64::from(image.width()) * u64::from(image.height()) > core_pins::MAX_IMAGE_PIXELS {
        return Err("图片为空或超过 1600 万像素限制。".into());
    }
    if image.pixels().all(|p| p.0[3] == 0) {
        for pixel in image.pixels_mut() {
            pixel.0[3] = 255;
        }
    }
    Ok(image)
}

/// 右键菜单项；文案是词典键（中文原文）。
#[derive(Clone, Copy, PartialEq)]
enum Item {
    Recognize(crate::recognition::Mode),
    /// 进入 / 退出选择文字模式。
    SelectText,
    /// 有选中文字时才出现在菜单顶部。
    CopyText,
    HighlightText,
    Copy,
    Save,
    Topmost,
    Lock,
    Toolbar,
    ClickThrough,
    Transform(Transform),
    Opacity(usize),
    ClearAnnotations,
    Close,
    Separator,
}

const MENU: [Item; 28] = [
    Item::Recognize(crate::recognition::Mode::Ocr),
    Item::Recognize(crate::recognition::Mode::Qr),
    Item::Recognize(crate::recognition::Mode::Translate),
    Item::SelectText,
    Item::Separator,
    Item::Copy,
    Item::Save,
    Item::Separator,
    Item::Topmost,
    Item::Lock,
    Item::Toolbar,
    Item::ClickThrough,
    Item::Separator,
    Item::Transform(Transform::RotateLeft),
    Item::Transform(Transform::RotateRight),
    Item::Transform(Transform::FlipHorizontal),
    Item::Transform(Transform::FlipVertical),
    Item::Transform(Transform::Grayscale),
    Item::Transform(Transform::Invert),
    Item::Transform(Transform::Reset),
    Item::Separator,
    Item::Opacity(0),
    Item::Opacity(1),
    Item::Opacity(2),
    Item::Opacity(3),
    Item::Separator,
    Item::ClearAnnotations,
    Item::Close,
];

/// macOS / Linux 贴图拖出的元素拖拽值。
struct PinDrag;

/// 右键菜单配色。
struct MenuColors {
    bg: u32,
    text: u32,
    border: u32,
    hover_bg: u32,
    hover_text: u32,
    separator: u32,
}

impl MenuColors {
    /// 右键菜单：浅色白底、悬停浅蓝；深色主题深灰底。
    fn current() -> Self {
        if crate::theme::dark() {
            MenuColors { bg: 0x20252c, text: 0xe6edf3, border: 0x39424e, hover_bg: 0x303f56, hover_text: 0xe6edf3, separator: 0x39424e }
        } else {
            MenuColors { bg: 0xffffff, text: 0x1f2328, border: 0xe4e4e7, hover_bg: 0xeff6ff, hover_text: 0x2563eb, separator: 0xe4e4e7 }
        }
    }
}

/// 打开菜单时的一项（文案与勾选状态在打开时确定）。
struct Entry {
    item: Item,
    label: String,
    checked: bool,
    shortcut: &'static str,
}

/// 贴图右键菜单窗口：失去焦点、Esc 或选中一项后关闭，选中项交回所属贴图执行。
struct PinMenu {
    pin: WindowHandle<Pin>,
    entries: Vec<Entry>,
    focus: FocusHandle,
}

impl PinMenu {
    fn new(pin: WindowHandle<Pin>, entries: Vec<Entry>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::native::frameless(window);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.observe_window_activation(window, |_, window, _| {
            if !window.is_window_active() {
                window.remove_window();
            }
        })
        .detach();
        Self { pin, entries, focus }
    }
}

impl Render for PinMenu {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let menu = MenuColors::current();
        let mut list = div()
            .id("pin-menu")
            .track_focus(&self.focus)
            .size_full()
            .p(px(5.0))
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(menu.border))
            .bg(rgb(menu.bg))
            .text_size(px(12.0))
            .text_color(rgb(menu.text))
            .on_key_down(|event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            });
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.item == Item::Separator {
                list = list.child(div().h(px(SEPARATOR_HEIGHT)).px(px(4.0)).flex().items_center().child(div().h(px(1.0)).w_full().bg(rgb(menu.separator))));
                continue;
            }
            let (hover_bg, hover_text) = (menu.hover_bg, menu.hover_text);
            let (item, pin) = (entry.item, self.pin);
            list = list.child(
                div()
                    .id(("pin-menu-item", index))
                    .h(px(ITEM_HEIGHT))
                    .flex_none()
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(hover_bg)).text_color(rgb(hover_text)))
                    .child(div().w(px(12.0)).text_color(rgb(0x2563eb)).child(if entry.checked { "✓" } else { "" }))
                    .child(div().flex_1().child(entry.label.clone()))
                    .child(div().text_size(px(10.0)).text_color(rgb(0xa1a1aa)).child(entry.shortcut))
                    .on_click(move |_, window, cx| {
                        window.remove_window();
                        let _ = pin.update(cx, |pin, window, cx| {
                            pin.menu = None;
                            pin.select(item, window, cx);
                        });
                    }),
            );
        }
        list
    }
}

/// 分组下拉展开时工具栏窗口加高的量（最多一组 6 个工具）。
const TOOLBAR_POPOVER: f32 = 210.0;

/// 贴图的独立工具栏窗口：内容由贴图生成，贴图变化时重绘并调整位置；贴图关闭时随之关闭。
struct PinToolbar {
    /// 弱引用：工具栏不能让贴图在关闭后继续存活（否则收不到释放通知，工具栏关不掉）。
    pin: WeakEntity<Pin>,
}

/// 工具栏窗口四周的透明边距，容纳工具条的阴影（太窄时阴影被窗口边缘截断，像多了一层边框）。
const TOOLBAR_SHADOW: f32 = 16.0;

impl PinToolbar {
    fn new(pin: Entity<Pin>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::native::frameless(window);
        cx.observe_in(&pin, window, |_, pin, _, cx| {
            // 分组下拉的开合改变窗口高度。
            pin.update(cx, |pin, cx| {
                if let Some(own) = pin.own {
                    let weak = cx.entity().downgrade();
                    cx.defer(move |cx| {
                        let _ = own.update(cx, |_, window, cx| {
                            let _ = weak.update(cx, |pin, cx| pin.place_toolbar(window, cx));
                        });
                    });
                }
            });
            cx.notify();
        })
        .detach();
        cx.observe_release_in(&pin, window, |_, _, window, _| window.remove_window()).detach();
        Self { pin: pin.downgrade() }
    }
}

impl Render for PinToolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Ok(toolbar) = self.pin.update(cx, |pin, cx| pin.toolbar(window, cx)) else {
            window.remove_window();
            return div();
        };
        let pin = self.pin.clone();
        div()
            .size_full()
            .p(px(TOOLBAR_SHADOW))
            // 焦点落在工具栏时，快捷键（空格、Ctrl+C、Esc 等）仍交给贴图处理。
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let event = event.clone();
                let _ = pin.update(cx, |pin, cx| pin.in_own_window(cx, move |pin, window, cx| pin.key(&event, window, cx)));
            })
            .child(toolbar)
    }
}

fn menu_height(items: &[Item]) -> f32 {
    items
        .iter()
        .map(|item| if *item == Item::Separator { SEPARATOR_HEIGHT } else { ITEM_HEIGHT })
        .sum::<f32>()
        + 12.0
}

/// “选择文字”模式的状态；文字层在识别完成前为空（显示“正在识别文字…”）。
struct TextSelect {
    layer: Option<Arc<TextLayer>>,
    selection: Option<Selection>,
    /// 按下落在文字上、正在拖选；双击取词、三击取行后拖动不再改成逐字选择。
    dragging: bool,
}

pub struct Pin {
    original: Arc<RgbaImage>,
    editor: Editor,
    /// 贴图创建时的标注，“恢复原图”时还原。
    original_annotations: Vec<Annotation>,
    layers: Layers,
    /// 选择了标注工具：图片上的按下交给编辑器，否则拖动窗口。
    annotating: bool,
    /// 选择文字模式（与标注工具互斥）；None 为普通模式。
    text_select: Option<TextSelect>,
    /// 按底图版本缓存的文字层：同一张图再次进入模式时不重复识别。
    text_cache: Option<(u64, Arc<TextLayer>)>,
    /// 底图版本，每次变换（旋转、翻转、灰度、反色、恢复原图）加一，旧文字层随之作废。
    image_serial: u64,
    /// 正在识别的底图版本；同一时间只识别一次（本地引擎不允许并发识别）。
    text_pending: Option<u64>,
    /// 外置工具栏是否显示（空格切换）。
    toolbar: bool,
    /// 被 [`hide_pins`] 临时隐藏（对话框期间）。
    hidden: bool,
    /// 文字/文件路径贴图拖出时提供的文字（颜色值规范化为 HEX，见 core `drag_text`）。
    drag_text: Option<String>,
    /// 文字输入框与所属文字草稿编号（TextDraft::id）。
    text_input: Option<(u64, Entity<TextareaState>)>,
    image: Arc<RgbaImage>,
    frame: Arc<RenderImage>,
    zoom: f64,
    opacity: f32,
    locked: bool,
    topmost: bool,
    click_through: bool,
    /// 按下图片时的位置（窗口内逻辑像素）：拖出几个像素后才交给系统移动窗口。按下就进入系统移动循环会吞掉
    /// 抬起，第二次按下认不出双击，双击关闭就失效了。
    move_from: Option<Point<Pixels>>,
    appearance: PinAppearance,
    export: ExportSettings,
    language: Language,
    /// 右键菜单在窗口内的位置（逻辑像素）。
    /// 右键菜单（独立弹出窗口，不撑大贴图窗口）。
    menu: Option<AnyWindowHandle>,
    /// 缩放比例、“已复制”等短暂提示。
    badge: Option<String>,
    badge_task: Option<Task<()>>,
    busy: bool,
    focus: FocusHandle,
    focused: bool,
    /// 自身窗口（工具栏窗口里的操作转回这里执行）。
    own: Option<AnyWindowHandle>,
    /// 图片首像素的全局物理坐标与所在屏缩放；随窗口移动更新，存入贴图历史。
    placed: Option<((i32, i32), f64)>,
    /// 空格展开的工具栏：独立的弹出窗口，跟随贴图移动。
    toolbar_window: Option<AnyWindowHandle>,
    _bounds: Option<Subscription>,
}

impl Pin {
    fn text(&self, message: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, message).to_owned()
    }

    /// 图片左上角距窗口顶边（逻辑像素）：透明边距，工具栏在上方时再加工具栏区域。
    fn image_top(&self, scale: f32) -> f32 {
        let inset = image_inset(&self.appearance.shadow, f64::from(scale)) as f32 / scale;
        inset
    }

    fn remember_place(&mut self, window: &Window) {
        let scale = f64::from(window.scale_factor());
        let origin = window.bounds().origin;
        let inset = image_inset(&self.appearance.shadow, scale);
        let x = (f64::from(f32::from(origin.x)) * scale).round() as i32 + inset;
        let y = (f64::from(f32::from(origin.y)) * scale).round() as i32 + inset;
        self.placed = Some(((x, y), scale));
    }

    /// 重新贴出所需的状态：当前底图之外的部分。
    fn history_state(&self, closed_at: i64) -> core_pins::history::State {
        let (origin, scale) = self.placed.unwrap_or(((0, 0), 1.0));
        core_pins::history::State {
            annotations: self.editor.shapes().to_vec(),
            drag_text: self.drag_text.clone(),
            origin,
            scale,
            zoom: self.zoom,
            opacity: self.opacity,
            locked: self.locked,
            closed_at,
        }
    }

    fn base_size(&self, scale: f32) -> Size<Pixels> {
        window_size(self.image.width(), self.image.height(), self.zoom, f64::from(scale), &self.appearance.shadow)
    }

    /// 工具栏或菜单打开时窗口扩到能放下它们；图片左上角不动（窗口只向右下扩展）。
    fn fit_window(&self, window: &mut Window) {
        let scale = window.scale_factor();
        window.resize(self.base_size(scale));
    }

    fn flash(&mut self, message: String, cx: &mut Context<Self>) {
        self.flash_for(message, 1200, cx);
    }

    /// 错误提示停留更久，来得及读完；仍是贴图内的短提示，不弹模态框。
    fn flash_for(&mut self, message: String, millis: u64, cx: &mut Context<Self>) {
        self.badge = Some(message);
        self.badge_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(millis)).await;
            let _ = this.update(cx, |pin, cx| {
                pin.badge = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn zoom_by(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        let scale = f64::from(window.scale_factor());
        let minimum = PinLayout::load().minimum_zoom(self.image.width(), self.image.height(), scale);
        let next = core_pins::step_zoom(self.zoom, direction, minimum);
        if next != self.zoom {
            self.zoom = next;
            self.fit_window(window);
        }
        self.flash(format!("{}%", (self.zoom * 100.0).round()), cx);
    }

    fn close_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            let _ = menu.update(cx, |_, window, _| window.remove_window());
            cx.notify();
        }
    }

    /// 在贴图内 `position`（窗口逻辑坐标）处打开右键菜单：独立的弹出窗口，放不下时向上/向左挪进屏幕。
    fn open_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        let Some(pin) = window.window_handle().downcast::<Pin>() else {
            return;
        };
        let mut items = Vec::with_capacity(MENU.len() + 3);
        if self.selected_text().is_some() {
            items.extend([Item::CopyText, Item::HighlightText, Item::Separator]);
        }
        items.extend(MENU);
        let height = menu_height(&items);
        let entries: Vec<Entry> = items
            .into_iter()
            .map(|item| {
                let (label, checked) = self.label(item);
                Entry { item, label, checked, shortcut: Self::shortcut(item) }
            })
            .collect();
        let size = size(px(MENU_WIDTH + 2.0), px(height + 2.0));
        let origin = window.bounds().origin + position;
        let screen = window.display(cx).map(|d| d.bounds());
        let mut at = origin;
        if let Some(screen) = screen {
            if at.y + size.height > screen.origin.y + screen.size.height {
                at.y = (at.y - size.height).max(screen.origin.y);
            }
            if at.x + size.width > screen.origin.x + screen.size.width {
                at.x = (at.x - size.width).max(screen.origin.x);
            }
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(at, size))),
            display_id: window.display(cx).map(|d| d.id()),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        match cx.open_window(options, |window, cx| cx.new(|cx| PinMenu::new(pin, entries, window, cx))) {
            Ok(handle) => {
                let handle: AnyWindowHandle = handle.into();
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                self.menu = Some(handle);
            }
            Err(error) => log::warn!("event=gpui.pin_menu_failed message={error}"),
        }
        cx.notify();
    }

    fn set_click_through(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        match crate::native::set_click_through(window, enabled) {
            Ok(()) => self.click_through = enabled,
            Err(error) => log::warn!("event=gpui.pin_click_through_failed message={error}"),
        }
        cx.notify();
    }

    fn select(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        match item {
            Item::Recognize(mode) => self.recognize(mode, cx),
            Item::SelectText => self.toggle_text_select(window, cx),
            Item::CopyText => self.copy_text(cx),
            Item::HighlightText => self.highlight_text(window, cx),
            Item::Copy => self.copy(window, cx),
            Item::Save => self.save(window, cx),
            Item::Topmost => match crate::native::set_topmost(window, !self.topmost) {
                Ok(()) => self.topmost = !self.topmost,
                Err(error) => log::warn!("event=gpui.pin_topmost_failed message={error}"),
            },
            Item::Lock => {
                self.locked = !self.locked;
                if self.locked {
                    self.annotating = false;
                    self.editor.abort();
                }
            }
            Item::Toolbar => self.toggle_toolbar(window, cx),
            Item::ClickThrough => self.set_click_through(!self.click_through, window, cx),
            Item::Transform(action) => self.transform(action, cx),
            Item::Opacity(index) => self.opacity = OPACITIES[index],
            Item::ClearAnnotations => {
                self.editor.clear();
                self.after_edit(window, cx);
            }
            Item::Close => window.remove_window(),
            Item::Separator => {}
        }
        cx.notify();
    }

    /// 系统拖出：后台准备内容（合成 PNG 写入临时文件），回到主线程在框架状态借用之外发起拖拽；
    /// 准备期间已松开左键则不拖出（core 内检查）。
    fn drag_out(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.editor.pending || self.editor.interacting() {
            return;
        }
        if !cfg!(windows) {
            return;
        }
        let (image, shapes, text) = (self.image.clone(), self.editor.shapes().to_vec(), self.drag_text.clone());
        cx.spawn(async move |this, cx| {
            let payload = match text {
                Some(text) => Ok(snap_core::drag::Payload::Text(text)),
                None => cx
                    .background_executor()
                    .spawn(async move {
                        let image = composed(&image, &shapes)?;
                        snap_core::drag::image_file(&image).map(|path| snap_core::drag::Payload::Files(vec![path]))
                    })
                    .await,
            };
            #[cfg(windows)]
            let result = payload.and_then(snap_core::drag::start);
            #[cfg(not(windows))]
            let result = payload.map(|_| ());
            match result {
                Ok(()) => log::info!("event=gpui.pin_drag_finished"),
                Err(error) => {
                    log::warn!("event=gpui.pin_drag_failed message={error}");
                    let _ = this.update(cx, |pin, cx| pin.flash(error, cx));
                }
            }
        })
        .detach();
    }

    /// 识别当前图像（含标注，与复制/保存看到的一致）；结果在独立窗口中显示，贴图保持不变。
    fn recognize(&mut self, mode: crate::recognition::Mode, cx: &mut Context<Self>) {
        if self.busy || self.editor.pending || self.editor.interacting() {
            return;
        }
        let (image, shapes) = (self.image.clone(), self.editor.shapes().to_vec());
        cx.spawn(async move |_, cx| {
            let composed = cx.background_executor().spawn(async move { composed(&image, &shapes) }).await;
            cx.update(|cx| match composed {
                Ok(image) => crate::recognition::open(cx, image, mode),
                Err(error) => log::error!("event=gpui.pin_recognize_failed message={error}"),
            });
        })
        .detach();
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let owner = crate::native::hwnd(window);
        let (image, shapes) = (self.image.clone(), self.editor.shapes().to_vec());
        cx.spawn(async move |this, cx| {
            let encoded = cx.background_executor().spawn(async move {
                let image = composed(&image, &shapes)?;
                let png = crate::capture::png(&image)?;
                Ok::<_, String>((image, png))
            }).await;
            let _ = this.update(cx, |pin, cx| {
                pin.busy = false;
                match encoded.and_then(|(image, png)| crate::capture::write_clipboard(cx, owner, &image, png)) {
                    Ok(()) => {
                        let message = pin.text("已复制");
                        pin.flash(message, cx);
                    }
                    Err(error) => log::error!("event=gpui.pin_copy_failed message={error}"),
                }
            });
        })
        .detach();
    }

    /// 保存对话框以贴图窗口为父窗口，显示在置顶贴图之上；文件名、格式与目录记忆来自 core。
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.editor.pending || self.editor.text.is_some() {
            return;
        }
        let (image, shapes) = (self.image.clone(), self.editor.shapes().to_vec());
        let export = self.export.clone();
        let name = match snap_core::export::filename(
            &export,
            &snap_core::export::timestamp(),
            image.width(),
            image.height(),
        ) {
            Ok(name) => name,
            Err(error) => {
                log::error!("event=gpui.pin_save_failed message={error}");
                return;
            }
        };
        self.busy = true;
        let record = snap_core::export::directory_record();
        let directory = record
            .as_deref()
            .and_then(|path| snap_core::export::preferred_directory(path, export.remember_directory));
        let dialog = crate::capture::SaveDialog::new(
            self.text("保存图片"),
            name,
            directory,
            export.format,
            &self.language,
            Some(window),
        );
        let hidden = hide_pins(cx, Some(window.window_handle()));
        cx.spawn(async move |this, cx| {
            let picked = dialog.pick().await;
            cx.update(|cx| show_pins(cx, hidden));
            let Some(path) = picked else {
                let _ = this.update(cx, |pin, _| pin.busy = false);
                return;
            };
            let written = cx
                .background_executor()
                .spawn({
                    let (path, export) = (path.clone(), export.clone());
                    async move {
                        let image = composed(&image, &shapes)?;
                        snap_core::export::write_file(&path, &image, &export)
                    }
                })
                .await;
            let _ = this.update(cx, |pin, cx| {
                pin.busy = false;
                match written {
                    Ok(()) => {
                        if export.remember_directory {
                            if let (Some(record), Some(parent)) = (record, path.parent()) {
                                let _ = snap_core::export::remember_directory(&record, parent);
                            }
                        }
                        let message = pin.text("图片已保存");
                        pin.flash(message, cx);
                    }
                    Err(error) => log::error!("event=gpui.pin_save_failed message={error}"),
                }
            });
        })
        .detach();
    }

    /// 像素变换在后台线程计算；预先检查像素预算，恢复原图直接共享原缓冲。
    fn transform(&mut self, action: Transform, cx: &mut Context<Self>) {
        if self.locked || self.busy {
            return;
        }
        let pixels = |image: &RgbaImage| u64::from(image.width()) * u64::from(image.height());
        let me = cx.entity();
        let others: u64 = alive(cx)
            .iter()
            .filter(|(_, pin)| *pin != me)
            .map(|(_, pin)| retained(pin.read(cx)))
            .sum();
        let next = pixels(&self.original) + if action == Transform::Reset { 0 } else { pixels(&self.image) };
        if others.saturating_add(next) > core_pins::MAX_TOTAL_PIXELS {
            log::warn!("event=gpui.pin_transform_rejected reason=budget");
            return;
        }
        if self.editor.pending || self.editor.interacting() {
            return;
        }
        // 标注随图变换；恢复原图时还原创建时的标注。几何变换前先校验，失败不改图。
        let shapes = if action == Transform::Reset {
            self.original_annotations.clone()
        } else {
            match core_pins::transform_annotations(self.editor.shapes(), self.image.width(), self.image.height(), action) {
                Ok(shapes) => shapes,
                Err(error) => {
                    log::warn!("event=gpui.pin_transform_rejected message={error}");
                    return;
                }
            }
        };
        self.busy = true;
        let (image, original) = (self.image.clone(), self.original.clone());
        cx.spawn(async move |this, cx| {
            let (image, frame) = cx
                .background_executor()
                .spawn(async move {
                    let image = if action == Transform::Reset {
                        original
                    } else {
                        Arc::new(core_pins::transform(&image, &original, action))
                    };
                    let frame = render_image(&image);
                    (image, frame)
                })
                .await;
            let old = this
                .update(cx, |pin, cx| {
                    pin.busy = false;
                    pin.image = image;
                    let old = std::mem::replace(&mut pin.frame, frame);
                    pin.editor.crop = image_bounds(&pin.image);
                    pin.editor.reset(shapes);
                    pin.layers.update(&pin.editor);
                    pin.image_changed(cx);
                    cx.notify();
                    old
                })
                .ok();
            // 旋转会交换宽高：按新尺寸重设窗口；释放旧画面与旧标注图层的纹理。
            let _ = cx.update(|cx| {
                let windows = alive(cx);
                if let Some((handle, pin)) = windows.into_iter().find(|(_, pin)| this.upgrade().as_ref() == Some(pin)) {
                    let _ = handle.update(cx, |_, window, cx| {
                        pin.read(cx).fit_window(window);
                        let retired = pin.update(cx, |pin, _| pin.layers.take_retired());
                        for image in retired.into_iter().chain(old) {
                            cx.drop_image(image, Some(window));
                        }
                    });
                }
                log_memory(cx, "transformed");
            });
        })
        .detach();
    }

    /// 窗口内逻辑坐标 → 贴图图像像素。
    fn to_point(&self, position: Point<Pixels>, scale: f32) -> ImagePoint {
        let layout = PinLayout::load();
        let inset = image_inset(&self.appearance.shadow, f64::from(scale)) as f32 / scale;
        let (w, h) = layout.displayed(self.image.width(), self.image.height(), self.zoom);
        let (w, h) = (w as f32 / scale, h as f32 / scale);
        let top = self.image_top(scale);
        ImagePoint {
            x: f64::from((f32::from(position.x) - inset) * self.image.width() as f32 / w),
            y: f64::from((f32::from(position.y) - top) * self.image.height() as f32 / h),
        }
    }

    /// 编辑器状态变化后：刷新图层、释放旧纹理、按需创建或移除文字输入框。
    fn after_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.update(&self.editor);
        for image in self.layers.take_retired() {
            cx.drop_image(image, Some(window));
        }
        if let Some(error) = self.editor.error.take() {
            self.flash(error, cx);
        }
        // 按草稿编号判断：提交后栅格化期间输入框仍在，下一段文字要换新的空输入框（见 TextDraft::id）。
        match (self.editor.text.as_ref(), self.text_input.as_ref().map(|(id, _)| *id)) {
            (Some(text), has) if has != Some(text.id) => {
                let value = text.shape.text.clone().unwrap_or_default();
                let state = cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx);
                    state.set_auto_grow(1, 20, cx);
                    state.set_value(value, window, cx);
                    state
                });
                state.update(cx, |state, cx| state.focus(window, cx));
                // 输入框在按下鼠标时创建；阻止 GPUI 随后把焦点交还给贴图，否则还要再点一次输入框才能打字。
                window.prevent_default();
                self.text_input = Some((text.id, state));
            }
            (None, Some(_)) => {
                self.text_input = None;
                window.focus(&self.focus, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 在后台线程栅格化，完成后写回编辑器。
    fn run(&mut self, job: Option<Job>, cx: &mut Context<Self>) {
        let Some(job) = job else {
            return;
        };
        let image = self.image.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let job = job.clone();
                    async move { job.run(&image, annotate::user_font()) }
                })
                .await;
            let _ = this.update(cx, |pin, cx| {
                pin.editor.complete(job, result);
                pin.layers.update(&pin.editor);
                // 不在窗口事件中：所有窗口都在应用的窗口表里，可直接释放。
                for image in pin.layers.take_retired() {
                    cx.drop_image(image, None);
                }
                if let Some(error) = pin.editor.error.take() {
                    pin.flash(error, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.text_input.as_ref().map(|(_, state)| state.read(cx).value().to_string()).unwrap_or_default();
        let job = self.editor.finish_text(&value);
        self.run(job, cx);
        self.after_edit(window, cx);
    }

    fn toggle_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.pending || self.editor.interacting() {
            return;
        }
        self.toolbar = !self.toolbar;
        if self.toolbar {
            self.open_toolbar(window, cx);
        } else {
            self.annotating = false;
            self.editor.select(None);
            if let Some(toolbar) = self.toolbar_window.take() {
                let _ = toolbar.update(cx, |_, window, _| window.remove_window());
            }
        }
        self.after_edit(window, cx);
    }

    /// 工具栏窗口的位置与大小：贴图下方，放不下时放上方；左缘对齐贴图并收进屏幕。
    fn toolbar_bounds(&self, window: &Window, cx: &App) -> Bounds<Pixels> {
        let margin = px(shadow_size(&self.appearance.shadow).1);
        let outer = window.bounds();
        let pin = Bounds::new(outer.origin + point(margin, margin), size(outer.size.width - margin * 2.0, outer.size.height - margin * 2.0));
        // 分组下拉展开时窗口向下加高，容纳下拉列表。
        let extra = if self.editor.open_group.is_some() { TOOLBAR_POPOVER } else { 0.0 };
        // 窗口贴合工具栏：工具行 40 + 上下边距；选了标注工具时多一行样式条（行距 6）。
        let options = self.annotating && !(self.editor.tool == Tool::Edit && self.editor.selected.is_none());
        // 文字、矩形、箭头的样式条比工具行宽，按样式条估算宽度加宽；工具栏随贴图状态变化重新定位时一并更新。
        // 选择文字模式时工具行多一个按钮。
        let text_button = if self.text_select.is_some() { TEXT_BUTTON_WIDTH } else { 0.0 };
        let width = text_button
            + if options {
                (TOOLBAR_WIDTH + 72.0).max(annotate::options_width(&self.editor) + 32.0)
            } else {
                TOOLBAR_WIDTH + 32.0
            };
        let height = 40.0 + if options { 46.0 } else { 0.0 } + extra;
        // 窗口比工具条大一圈透明边距放阴影；定位按工具条本身计算。
        let margin = px(TOOLBAR_SHADOW);
        let size = size(px(width) + margin * 2.0, px(height) + margin * 2.0);
        let gap = px(PinLayout::load().gap as f32);
        let screen = window.display(cx).map(|d| d.visible_bounds());
        let mut at = point(pin.origin.x - margin, pin.bottom() + gap - margin);
        if let Some(screen) = screen {
            if at.y + size.height - margin > screen.bottom() {
                at.y = (pin.origin.y - gap - size.height + margin).max(screen.origin.y - margin);
            }
            at.x = at.x.min(screen.right() - size.width + margin).max(screen.origin.x - margin);
        }
        Bounds::new(at, size)
    }

    fn open_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.toolbar_bounds(window, cx);
        let pin = cx.entity();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: window.display(cx).map(|d| d.id()),
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
        // 新窗口创建时立即绘制，绘制要读贴图；贴图此刻正在处理按键（被借用），所以等这次更新结束再打开。
        cx.defer(move |cx| {
            if !pin.read(cx).toolbar || pin.read(cx).toolbar_window.is_some() {
                return;
            }
            let view = pin.clone();
            match cx.open_window(options, |window, cx| cx.new(|cx| PinToolbar::new(view, window, cx))) {
                Ok(handle) => pin.update(cx, |pin, _| pin.toolbar_window = Some(handle.into())),
                Err(error) => {
                    log::warn!("event=gpui.pin_toolbar_failed message={error}");
                    pin.update(cx, |pin, cx| {
                        pin.toolbar = false;
                        cx.notify();
                    });
                }
            }
        });
    }

    /// 贴图移动、缩放或下拉展开后重新放置工具栏窗口。
    fn place_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(toolbar) = self.toolbar_window else {
            return;
        };
        let bounds = self.toolbar_bounds(window, cx);
        // 调整大小会让工具栏窗口重绘（读贴图），同样等贴图的这次更新结束再做。
        let pin = cx.entity().downgrade();
        cx.defer(move |cx| {
            let alive = toolbar.update(cx, |_, window, _| {
                let current = window.bounds();
                if current.size != bounds.size {
                    window.resize(bounds.size);
                }
                let moved = (current.origin.x - bounds.origin.x).abs() > px(0.5) || (current.origin.y - bounds.origin.y).abs() > px(0.5);
                if moved {
                    crate::native::move_to(window, bounds.origin);
                }
            });
            if alive.is_err() {
                let _ = pin.update(cx, |pin, _| {
                    if pin.toolbar_window == Some(toolbar) {
                        pin.toolbar_window = None;
                        pin.toolbar = false;
                    }
                });
            }
        });
    }

    /// 工具栏窗口里的操作在贴图自己的窗口里执行（缩放、菜单、关闭等都以贴图窗口为准）。
    fn in_own_window(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Pin, &mut Window, &mut Context<Pin>) + 'static) {
        let (Some(handle), weak) = (self.own, cx.entity().downgrade()) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |pin, cx| f(pin, window, cx));
            });
        });
    }

    fn remote_action(this: &mut Self, action: Action, _: &mut Window, cx: &mut Context<Self>) {
        this.in_own_window(cx, move |pin, window, cx| Self::action(pin, action, window, cx));
    }

    /// 工具栏与样式条的操作；选择工具时进入标注模式。
    fn action(this: &mut Self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if this.locked || this.busy {
            return;
        }
        let e = &mut this.editor;
        let job = match action {
            Action::Tool(tool) => {
                this.annotating = true;
                this.text_select = None;
                e.set_tool(tool);
                None
            }
            Action::Ui(ui) => {
                e.ui(ui);
                None
            }
            Action::Undo => {
                e.undo();
                None
            }
            Action::Redo => {
                e.redo();
                None
            }
            Action::Delete => {
                e.remove();
                None
            }
            Action::Clear => {
                e.clear();
                None
            }
            Action::Style(patch) => e.style(patch),
        };
        this.run(job, cx);
        this.after_edit(window, cx);
    }

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 菜单失焦时已自行关闭；只有它还开着时，这次按下才只用来关菜单。
        if let Some(menu) = self.menu.take() {
            if menu.update(cx, |_, window, _| window.remove_window()).is_ok() {
                cx.notify();
                return;
            }
        }
        if self.editor.text.is_some() {
            self.commit_text(window, cx);
            return;
        }
        // Windows/Linux 按住 Ctrl，macOS 按住 ⌥ 拖动即拖出内容。
        let drag_modifier = if cfg!(target_os = "macos") { event.modifiers.alt } else { event.modifiers.control };
        if drag_modifier && !self.locked && !self.click_through {
            // macOS / Linux 由根元素的元素拖拽接管（render 里按修饰键挂上），这里不拖动窗口。
            if cfg!(windows) {
                self.drag_out(cx);
            }
            return;
        }
        let scale = window.scale_factor();
        let p = self.to_point(event.position, scale);
        let crop = image_bounds(&self.image);
        let inside = crop.contains(p);
        // 容差按屏幕 6 逻辑像素换算到图像像素。
        let (w, _) = PinLayout::load().displayed(self.image.width(), self.image.height(), self.zoom);
        let tolerance = 6.0 * f64::from(scale) * f64::from(self.image.width()) / f64::from(w.max(1));
        if self.press_text(p, tolerance, event.click_count, event.modifiers.shift, cx) {
            return;
        }
        if self.annotating && !self.locked {
            let handle = self.editor.handle_at(p, tolerance);
            if inside || handle.is_some() {
                if event.click_count >= 2 {
                    if let Some(i) = self.editor.hit(p, tolerance).filter(|&i| {
                        self.editor.shapes()[i].kind == snap_core::annotations::Kind::Text
                    }) {
                        self.editor.edit_text(i);
                        self.after_edit(window, cx);
                        return;
                    }
                }
                self.editor.begin(p, tolerance, handle, event.modifiers.alt);
                self.after_edit(window, cx);
                return;
            }
        }
        // 双击图片关闭贴图。锁定、标注和选择文字时双击另有用途或需要防误触，不关闭；
        // 只认图片区域，连点工具栏按钮不会关掉贴图。
        if event.click_count >= 2 && inside && !self.locked && !self.annotating && self.text_select.is_none() {
            window.remove_window();
            return;
        }
        if !self.locked {
            self.move_from = Some(event.position);
        }
    }

    fn motion(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(from) = self.move_from {
            if event.pressed_button != Some(MouseButton::Left) {
                self.move_from = None;
            } else if (event.position.x - from.x).abs() > px(3.0) || (event.position.y - from.y).abs() > px(3.0) {
                self.move_from = None;
                crate::native::start_move(window);
                return;
            }
        }
        if self.text_select.as_ref().is_some_and(|mode| mode.dragging) {
            let p = self.to_point(event.position, window.scale_factor());
            self.drag_text_selection(p, event.pressed_button == Some(MouseButton::Left), cx);
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) || self.editor.draft.is_none() || self.editor.text.is_some() {
            return;
        }
        let p = self.to_point(event.position, window.scale_factor());
        self.editor.drag(p);
        self.layers.update(&self.editor);
        for image in self.layers.take_retired() {
            cx.drop_image(image, Some(window));
        }
        cx.notify();
    }

    fn release(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.move_from = None;
        if let Some(mode) = self.text_select.as_mut() {
            mode.dragging = false;
        }
        if self.editor.draft.is_none() {
            return;
        }
        let job = self.editor.finish(false);
        self.run(job, cx);
        self.after_edit(window, cx);
    }

    fn right_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 右键先放弃未完成的笔画或输入；完成后再次右键才显示菜单。
        if self.editor.text.is_some() {
            self.editor.cancel_text();
            self.after_edit(window, cx);
            return;
        }
        if self.editor.interacting() {
            self.editor.abort();
            self.after_edit(window, cx);
            return;
        }
        self.open_menu(event.position, window, cx);
    }

    fn wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let dy = f32::from(event.delta.pixel_delta(px(20.0)).y);
        if dy != 0.0 {
            self.zoom_by(if dy > 0.0 { 1 } else { -1 }, window, cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;
        let key = keystroke.key.as_str();
        if self.editor.text.is_some() {
            // 输入中只处理取消与完成，其余按键交给输入框。
            match key {
                "escape" => {
                    self.editor.cancel_text();
                    self.after_edit(window, cx);
                    cx.stop_propagation();
                }
                "enter" if command => {
                    self.commit_text(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        let editable = !self.locked && !self.busy;
        match key {
            "escape" if self.menu.is_some() => self.close_menu(window, cx),
            "escape" if self.editor.interacting() => {
                self.editor.abort();
                self.after_edit(window, cx);
            }
            // 选择文字模式中先退出模式，再按一次才关闭贴图。
            "escape" if self.text_select.is_some() => self.leave_text_select(cx),
            "escape" => window.remove_window(),
            "t" if !command && !keystroke.modifiers.alt => self.toggle_text_select(window, cx),
            "a" if command && self.text_select.is_some() => self.select_all_text(cx),
            "h" if !command && !keystroke.modifiers.alt && self.selected_text().is_some() => self.highlight_text(window, cx),
            "space" if !command => self.toggle_toolbar(window, cx),
            "z" if command && editable => {
                if keystroke.modifiers.shift {
                    self.editor.redo();
                } else {
                    self.editor.undo();
                }
                self.after_edit(window, cx);
            }
            "y" if command && editable => {
                self.editor.redo();
                self.after_edit(window, cx);
            }
            "delete" | "backspace" if editable => {
                self.editor.remove();
                self.after_edit(window, cx);
            }
            "enter" if self.editor.tool == Tool::Polyline && self.editor.draft.is_some() => {
                let job = self.editor.finish(true);
                self.run(job, cx);
                self.after_edit(window, cx);
            }
            "left" | "right" | "up" | "down" if editable && self.editor.selected.is_some() => {
                let step = if keystroke.modifiers.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    "left" => (-step, 0.0),
                    "right" => (step, 0.0),
                    "up" => (0.0, -step),
                    _ => (0.0, step),
                };
                let job = self.editor.nudge(dx, dy).1;
                self.run(job, cx);
                self.after_edit(window, cx);
            }
            // 有选中文字时复制文字，否则复制图片。
            "c" if command && self.selected_text().is_some() => self.copy_text(cx),
            "c" if command => self.copy(window, cx),
            "s" if command => self.save(window, cx),
            "w" if command => window.remove_window(),
            "p" if command && keystroke.modifiers.alt => {
                self.set_click_through(!self.click_through, window, cx)
            }
            "+" | "=" => self.zoom_by(1, window, cx),
            "-" => self.zoom_by(-1, window, cx),
            _ => {}
        }
    }

    fn label(&self, item: Item) -> (String, bool) {
        match item {
            Item::Recognize(mode) => (
                self.text(match mode {
                    crate::recognition::Mode::Ocr => "识别文字",
                    crate::recognition::Mode::Qr => "识别二维码 / 条形码",
                    crate::recognition::Mode::Translate => "翻译图片",
                }),
                false,
            ),
            Item::SelectText => (self.text("选择文字"), self.text_select.is_some()),
            Item::CopyText => (self.text("复制所选文字"), false),
            Item::HighlightText => (self.text("高亮所选文字"), false),
            Item::Copy => (self.text("复制图片"), false),
            Item::Save => (self.text("保存图片"), false),
            Item::Topmost => (self.text("窗口置顶"), self.topmost),
            Item::Lock => (self.text("锁定贴图"), self.locked),
            Item::Toolbar => (self.text("显示工具栏"), self.toolbar),
            Item::ClearAnnotations => (self.text("清空标注"), false),
            Item::ClickThrough => (self.text("鼠标穿透"), self.click_through),
            Item::Transform(action) => (
                self.text(match action {
                    Transform::RotateLeft => "左转",
                    Transform::RotateRight => "右转",
                    Transform::FlipHorizontal => "水平翻转",
                    Transform::FlipVertical => "垂直翻转",
                    Transform::Grayscale => "灰度",
                    Transform::Invert => "反色",
                    Transform::Reset => "恢复原图",
                }),
                false,
            ),
            Item::Opacity(index) => (
                format!("{} {}%", self.text("不透明度"), (OPACITIES[index] * 100.0) as u32),
                (self.opacity - OPACITIES[index]).abs() < 0.001,
            ),
            Item::Close => (self.text("关闭贴图"), false),
            Item::Separator => (String::new(), false),
        }
    }

    /// 进入或退出选择文字模式。进入时退出标注工具；同一张图已识别过就直接用缓存，
    /// 否则先确认所选引擎会返回文字位置（在线 OCR 不返回，提前提示，不上传图片），再在后台识别。
    fn toggle_text_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_select.is_some() {
            self.leave_text_select(cx);
            return;
        }
        if self.editor.pending || self.editor.interacting() || self.click_through {
            return;
        }
        let cached = self.text_cache.as_ref().filter(|(serial, _)| *serial == self.image_serial).map(|(_, layer)| layer.clone());
        if cached.is_none() && self.text_pending.is_none() {
            let settings = crate::shell::settings(cx);
            let error = match snap_core::recognition::locates_text(&settings) {
                Ok(true) => None,
                Ok(false) => Some(Unavailable::NoPositions.message().to_owned()),
                Err(error) => Some(error),
            };
            if let Some(error) = error {
                log::info!("event=gpui.pin_text_select_unavailable message={error}");
                let message = snap_core::i18n::for_language(&self.language, &error).to_owned();
                self.flash_for(message, 3000, cx);
                return;
            }
        }
        self.annotating = false;
        self.editor.select(None);
        let ready = cached.is_some();
        self.text_select = Some(TextSelect { layer: cached, selection: None, dragging: false });
        if !ready {
            self.recognize_text(cx);
        }
        log::info!("event=gpui.pin_text_select_entered cached={ready}");
        self.after_edit(window, cx);
    }

    fn leave_text_select(&mut self, cx: &mut Context<Self>) {
        if self.text_select.take().is_some() {
            cx.notify();
        }
    }

    /// 后台识别当前底图（不含标注：标注不是图片里的文字，荧光笔也会干扰识别）。
    /// 已有识别在进行时不再发起，完成后若底图已变，由 [`Pin::text_recognized`] 按新图补一次。
    fn recognize_text(&mut self, cx: &mut Context<Self>) {
        if self.text_pending.is_some() {
            return;
        }
        let serial = self.image_serial;
        self.text_pending = Some(serial);
        let (width, height) = self.image.dimensions();
        let task = crate::recognition::recognize_text(cx, self.image.clone());
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |pin, cx| pin.text_recognized(serial, width, height, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn text_recognized(&mut self, serial: u64, width: u32, height: u32, result: Result<OcrResult, String>, cx: &mut Context<Self>) {
        self.text_pending = None;
        let waiting = self.text_select.as_ref().is_some_and(|mode| mode.layer.is_none());
        if serial != self.image_serial {
            // 识别期间图片已变换：旧坐标不适用新图，丢弃结果；仍在等待文字层时按新图重新识别。
            if waiting {
                self.recognize_text(cx);
            }
            return;
        }
        let layer = match result {
            Ok(result) => {
                log::info!(
                    "event=gpui.pin_text_recognized provider={} lines={} elapsed_ms={}",
                    result.provider,
                    result.lines.len(),
                    result.elapsed_ms
                );
                TextLayer::new(&result, width, height).map_err(|reason| self.text(reason.message()))
            }
            Err(error) => {
                log::warn!("event=gpui.pin_text_recognize_failed message={error}");
                Err(snap_core::i18n::for_language(&self.language, &error).to_owned())
            }
        };
        match layer {
            Ok(layer) => {
                let layer = Arc::new(layer);
                let count = layer.lines().len();
                self.text_cache = Some((serial, layer.clone()));
                if let Some(mode) = self.text_select.as_mut().filter(|mode| mode.layer.is_none()) {
                    mode.layer = Some(layer);
                    let message = self.text("已识别 {0} 行文字").replace("{0}", &count.to_string());
                    self.flash(message, cx);
                }
            }
            // 失败只在仍等待结果时提示并退出模式；用户已退出模式就不再打扰。
            Err(message) if waiting => {
                self.text_select = None;
                self.flash_for(message, 3000, cx);
            }
            Err(_) => {}
        }
        cx.notify();
    }

    /// 底图换了（变换或恢复原图）：文字层作废；仍在选择文字模式时清空选区并按新图重新识别。
    fn image_changed(&mut self, cx: &mut Context<Self>) {
        self.image_serial = self.image_serial.wrapping_add(1);
        self.text_cache = None;
        if let Some(mode) = self.text_select.as_mut() {
            mode.layer = None;
            mode.selection = None;
            mode.dragging = false;
            self.recognize_text(cx);
        }
    }

    /// 文字层与非空选区（选择文字模式、识别完成且选中了内容时才有）。
    fn text_selection(&self) -> Option<(&TextLayer, Selection)> {
        let mode = self.text_select.as_ref()?;
        let selection = mode.selection.filter(|selection| !selection.is_empty())?;
        Some((mode.layer.as_deref()?, selection))
    }

    /// 选中的文字；只有空白时视为没有选中。
    fn selected_text(&self) -> Option<String> {
        let (layer, selection) = self.text_selection()?;
        Some(layer.text(selection)).filter(|text| !text.trim().is_empty())
    }

    /// 选择文字模式下的按下：落在文字上（或按住 Shift 扩展选区）时开始拖选，双击取词、三击取行，返回 true；
    /// 识别中或落在文字以外时清除选区并返回 false，按下照常拖动贴图。`p` 与 `tolerance` 为图像像素。
    fn press_text(&mut self, p: ImagePoint, tolerance: f64, clicks: usize, extend: bool, cx: &mut Context<Self>) -> bool {
        let Some(mode) = self.text_select.as_mut() else {
            return false;
        };
        let Some(layer) = mode.layer.clone() else {
            return false;
        };
        let extend = extend && mode.selection.is_some();
        if !extend && !layer.hit(p, tolerance) {
            if mode.selection.take().is_some() {
                cx.notify();
            }
            return false;
        }
        mode.selection = match clicks {
            2 => layer.word_at(p),
            n if n >= 3 => layer.line_at(p),
            _ => layer.caret_at(p).map(|caret| match mode.selection {
                Some(current) if extend => Selection { anchor: current.anchor, focus: caret },
                _ => Selection::at(caret),
            }),
        };
        mode.dragging = clicks < 2;
        cx.notify();
        true
    }

    /// 拖选：焦点跟随指针，拖出文字范围时停在最近一行的行首或行尾。松开左键后结束。
    fn drag_text_selection(&mut self, p: ImagePoint, pressed: bool, cx: &mut Context<Self>) {
        let Some(mode) = self.text_select.as_mut() else {
            return;
        };
        if !pressed {
            mode.dragging = false;
            return;
        }
        let caret = mode.layer.as_ref().and_then(|layer| layer.caret_at(p));
        if let (Some(caret), Some(selection)) = (caret, mode.selection.as_mut()) {
            if selection.focus != caret {
                selection.focus = caret;
                cx.notify();
            }
        }
    }

    fn select_all_text(&mut self, cx: &mut Context<Self>) {
        if let Some(mode) = self.text_select.as_mut() {
            if let Some(layer) = mode.layer.as_ref() {
                mode.selection = layer.select_all();
                mode.dragging = false;
                cx.notify();
            }
        }
    }

    /// 复制选中的文字：按阅读顺序，换行保留（拼接规则见 core `TextLayer::text`）。
    fn copy_text(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.selected_text() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let message = self.text("已复制文字");
        self.flash(message, cx);
    }

    /// 把选区转成荧光笔标注（每段文字一笔，一步撤销），之后可用标注工具改色、移动或删除；
    /// 完成后清除选区，露出高亮效果。数量超限等校验失败时只提示，不改标注。
    fn highlight_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked || self.busy || self.editor.pending || self.editor.interacting() {
            return;
        }
        let Some((layer, selection)) = self.text_selection() else {
            return;
        };
        let (width, height) = self.image.dimensions();
        let marks = text_layer::highlighter_marks(&layer.highlights(selection), TEXT_HIGHLIGHT_COLOR, TEXT_HIGHLIGHT_OPACITY, width, height);
        let mut combined = self.editor.shapes().to_vec();
        combined.extend(marks.iter().cloned());
        if let Err(error) = snap_core::annotations::validate(&combined, width, height) {
            let message = snap_core::i18n::for_language(&self.language, &error).to_owned();
            self.flash_for(message, 3000, cx);
            return;
        }
        if self.editor.append(marks) {
            if let Some(mode) = self.text_select.as_mut() {
                mode.selection = None;
            }
        }
        self.after_edit(window, cx);
    }

    fn shortcut(item: Item) -> &'static str {
        match item {
            Item::SelectText => "T",
            Item::CopyText => "Ctrl+C",
            Item::HighlightText => "H",
            Item::Copy => "Ctrl+C",
            Item::Save => "Ctrl+S",
            Item::ClickThrough => "Ctrl+Alt+P",
            Item::Toolbar => "Space",
            Item::Close => "Esc",
            _ => "",
        }
    }
}

/// 导出用的合成图：底图副本 + 标注，不改变可编辑对象。
fn composed(image: &RgbaImage, shapes: &[Annotation]) -> Result<RgbaImage, String> {
    let mut output = image.clone();
    if !shapes.is_empty() {
        snap_core::annotations::render(&mut output, shapes, ImagePoint { x: 0.0, y: 0.0 })?;
    }
    Ok(output)
}

impl Render for Pin {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus, cx);
            self.focused = true;
        }
        let scale = window.scale_factor();
        let layout = PinLayout::load();
        let inset = image_inset(&self.appearance.shadow, f64::from(scale)) as f32 / scale;
        let top = self.image_top(scale);
        let (w, h) = layout.displayed(self.image.width(), self.image.height(), self.zoom);
        let (w, h) = (w as f32 / scale, h as f32 / scale);
        let border = layout.border as f32;
        let radius = (self.appearance.corner_radius as f32).min(w.min(h) / 2.0);
        // 图片像素 → 图片内容区内的逻辑坐标（标注图层、选中框、输入框共用）。
        let (kx, ky) = (w / self.image.width() as f32, h / self.image.height() as f32);
        // GPUI 的 overflow_hidden 只按矩形裁剪子元素，圆角必须设在图片本身上才会生效。
        let mut content = div()
            .relative()
            .size_full()
            .child(img(self.frame.clone()).absolute().size_full().rounded(px(radius)).object_fit(ObjectFit::Fill));
        for layer in [self.layers.base(), self.layers.draft()].into_iter().flatten() {
            content = content.child(
                img(layer.image.clone())
                    .absolute()
                    .left(px(layer.x as f32 * kx))
                    .top(px(layer.y as f32 * ky))
                    .w(px(layer.width as f32 * kx))
                    .h(px(layer.height as f32 * ky))
                    .object_fit(ObjectFit::Fill),
            );
        }
        let map = |p: ImagePoint| point(px(p.x as f32 * kx), px(p.y as f32 * ky));
        content = content.children(annotate::selection_overlay(&self.editor, &map));
        // 选择文字：每个识别框一块透明区域（鼠标移上去显示文字光标），选区按字符边界画半透明底色。
        // 文字层是图像像素，与标注一样乘 kx / ky，缩放和 DPI 变化后仍贴合文字。
        if let Some(mode) = self.text_select.as_ref() {
            if let Some(layer) = mode.layer.as_ref() {
                let area = |r: snap_core::recognition::overlay::Rect| {
                    div().absolute().left(px(r.x * kx)).top(px(r.y * ky)).w(px(r.width * kx)).h(px(r.height * ky))
                };
                content = content.children(layer.lines().iter().map(|line| area(line.rect).cursor_text()));
                if let Some(selection) = mode.selection.filter(|selection| !selection.is_empty()) {
                    content = content.children(layer.highlights(selection).into_iter().map(|r| area(r).bg(rgba(TEXT_SELECTION))));
                }
            }
        }
        // 不画边框（用户反馈不需要），只保留圆角与阴影。
        let _ = border;
        let mut stage = div()
            .absolute()
            .left(px(inset))
            .top(px(top))
            .w(px(w))
            .h(px(h))
            .rounded(px(radius))
            .overflow_hidden()
            .opacity(self.opacity)
            .child(content);
        stage = stage.shadow(pin_shadow(&self.appearance));
        let mut root = div()
            .id("pin")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_move(cx.listener(Self::motion))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::release))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_click))
            .on_scroll(cx.listener(Self::wheel))
            // 按下/松开拖出修饰键时重绘，决定是否挂上元素拖拽。
            .on_modifiers_changed(cx.listener(|_, _: &ModifiersChangedEvent, _, cx| cx.notify()))
            .child(stage);
        // macOS / Linux：按住 ⌥（Linux 为 Ctrl）拖出窗口时，把贴图（含标注）或原文字交给系统文件拖拽。
        let held = if cfg!(target_os = "macos") { window.modifiers().alt } else { window.modifiers().control };
        if !cfg!(windows) && held && !self.locked && !self.click_through && !self.busy {
            let (image, shapes, text) = (self.image.clone(), self.editor.shapes().to_vec(), self.drag_text.clone());
            root = root
                .on_drag(PinDrag, |_, _, _, cx| cx.new(|_| crate::drag_out::Ghost))
                .external_drag_payload(move |_: &PinDrag, _, _| {
                    crate::drag_out::files(match &text {
                        Some(text) => Ok(snap_core::drag::Payload::Text(text.clone())),
                        None => composed(&image, &shapes)
                            .and_then(|image| snap_core::drag::image_file(&image))
                            .map(|path| snap_core::drag::Payload::Files(vec![path])),
                    })
                });
        }
        if self.annotating && !self.locked {
            root = root.cursor_crosshair();
        }
        // 文字输入框：放在文字起点，宽度到图片右边缘，与最终排版的换行宽度一致。
        let current = self.editor.text.as_ref().map(|t| t.id);
        let input = self.text_input.as_ref().filter(|(id, _)| Some(*id) == current);
        if let (Some(text), Some((_, state))) = (self.editor.text.as_ref(), input) {
            let at = map(text.shape.start);
            let width = ((self.image.width() as f64 - text.shape.start.x).min(2048.0) as f32 * kx).max(40.0);
            let font = (text.shape.font_size.unwrap_or(24.0) as f32 * ky).max(8.0);
            let (color, background) = annotate::text_input_colors(&text.shape);
            root = root.child(
                div()
                    .absolute()
                    .left(px(inset) + at.x)
                    .top(px(top) + at.y)
                    .w(px(width))
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(0x2f80ff))
                    .bg(rgba(background))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Textarea::new(state).appearance(false).text_size(px(font)).text_color(rgb(color))),
            );
        }
        let badge = |text: String, top: f32| {
            div()
                .absolute()
                .left(px(inset + 8.0))
                .top(px(top))
                // 较长的错误提示在图片宽度内换行，不伸出贴图。
                .max_w(px((w - 16.0).max(48.0)))
                .px_1()
                .rounded_sm()
                .bg(rgba(0x000000b0))
                .text_xs()
                .text_color(rgb(0xffffff))
                .child(text)
        };
        if let Some(text) = self.badge.clone() {
            root = root.child(badge(text, top + 8.0));
        } else if self.text_select.as_ref().is_some_and(|mode| mode.layer.is_none()) {
            root = root.child(badge(self.text("正在识别文字…"), top + 8.0));
        }
        if self.click_through {
            let hint = self.text("鼠标穿透中 · 用全局贴图快捷键或托盘恢复");
            root = root.child(badge(hint, (top + h - 24.0).max(top + 8.0)));
        }
        root
    }
}

impl Pin {
    /// 图片下方的外置工具栏：移动、标注工具、撤销重做 + 缩放比例、图片处理、置顶、保存、关闭、复制。
    fn toolbar(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let view = cx.entity();
        let text = |key: &'static str| self.text(key);
        let disabled = self.locked || self.busy;
        let props = ToolbarProps { editor: &self.editor, active: self.annotating, disabled, compact: true, grouped: true, collapsible: false, popover_up: false, text: &text };
        let host = annotate::Host { view: &view, window, dispatch: Self::remote_action };
        let selecting = self.text_select.is_some();
        let leading = vec![
            annotate::icon_button("pin-move", annotate::icons::MOVE, self.text("移动贴图"), !self.annotating && !selecting, disabled)
                .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| {
                    pin.in_own_window(cx, |pin, window, cx| {
                        if pin.editor.interacting() || pin.editor.pending {
                            return;
                        }
                        pin.leave_text_select(cx);
                        pin.annotating = false;
                        pin.editor.select(None);
                        pin.after_edit(window, cx);
                    })
                }))
                .into_any_element(),
            annotate::divider().into_any_element(),
        ];
        let zoom = div()
            .id("pin-zoom")
            .h(px(30.0))
            .px(px(6.0))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(annotate::hover())))
            .text_xs()
            .text_color(rgb(annotate::text()))
            .child(format!("{}%", (self.zoom * 100.0).round()))
            .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| {
                pin.in_own_window(cx, |pin, window, cx| {
                    if pin.locked {
                        return;
                    }
                    pin.zoom = 1.0_f64.max(PinLayout::load().minimum_zoom(pin.image.width(), pin.image.height(), f64::from(window.scale_factor())));
                    pin.fit_window(window);
                    cx.notify();
                })
            }));
        let mut trailing = vec![
            zoom.into_any_element(),
            annotate::icon_button("pin-select-text", TEXT_CURSOR_ICON, self.text("选择文字 T"), selecting, self.click_through)
                .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| {
                    pin.in_own_window(cx, |pin, window, cx| pin.toggle_text_select(window, cx))
                }))
                .into_any_element(),
        ];
        if selecting {
            // 只在选择文字模式里出现；没有选中文字时置灰。
            let empty = self.selected_text().is_none() || disabled;
            trailing.push(
                annotate::icon_button("pin-highlight-text", annotate::icons::HIGHLIGHTER, self.text("高亮所选文字 H"), false, empty)
                    .on_click(cx.listener(move |pin, _: &ClickEvent, _, cx| {
                        if !empty {
                            pin.in_own_window(cx, |pin, window, cx| pin.highlight_text(window, cx));
                        }
                    }))
                    .into_any_element(),
            );
        }
        trailing.extend([
            annotate::icon_button("pin-recognize", annotate::icons::SCAN_TEXT, self.text("识别"), false, self.busy)
                .on_click(cx.listener(|pin, event: &ClickEvent, window, cx| {
                    // 识别菜单与右键菜单共用（识别项在最上方）；按屏幕位置换算回贴图窗口坐标。
                    let global = window.bounds().origin + event.position();
                    pin.in_own_window(cx, move |pin, window, cx| {
                        let local = global - window.bounds().origin;
                        pin.open_menu(local, window, cx)
                    })
                }))
                .into_any_element(),
            annotate::icon_button("pin-adjust", annotate::icons::SLIDERS, self.text("图片处理"), false, disabled)
                .on_click(cx.listener(|pin, event: &ClickEvent, window, cx| {
                    let global = window.bounds().origin + event.position();
                    pin.in_own_window(cx, move |pin, window, cx| {
                        let local = global - window.bounds().origin;
                        pin.open_menu(local, window, cx)
                    })
                }))
                .into_any_element(),
            annotate::divider().into_any_element(),
            annotate::icon_button(
                "pin-topmost",
                if self.topmost { annotate::icons::PIN } else { annotate::icons::PIN_OFF },
                self.text(if self.topmost { "取消置顶" } else { "窗口置顶" }),
                self.topmost,
                self.busy,
            )
            .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| pin.in_own_window(cx, |pin, window, cx| pin.select(Item::Topmost, window, cx))))
            .into_any_element(),
            annotate::icon_button("pin-save", annotate::icons::SAVE, self.text("保存图片 Ctrl/⌘+S"), false, self.busy)
                .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| pin.in_own_window(cx, |pin, window, cx| pin.save(window, cx))))
                .into_any_element(),
            annotate::icon_button("pin-close", annotate::icons::CLOSE, self.text("关闭贴图 Esc"), false, false)
                .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| pin.in_own_window(cx, |_, window, _| window.remove_window())))
                .into_any_element(),
            annotate::icon_button("pin-copy", annotate::icons::COPY, self.text("复制图片 Ctrl/⌘+C"), false, self.busy)
                .bg(rgb(0x2563eb))
                .on_click(cx.listener(|pin, _: &ClickEvent, _, cx| pin.in_own_window(cx, |pin, window, cx| pin.copy(window, cx))))
                .into_any_element(),
        ]);
        div()
            .id("pin-toolbar")
            .flex()
            .flex_col()
            .items_start()
            .gap(px(6.0))
            .cursor_default()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(annotate::tool_row(&props, leading, trailing, &host))
            .children(annotate::options_row(&props, &host))
    }
}
