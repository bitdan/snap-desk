//! 贴图窗口：透明、置顶、不进任务栏的 PopUp 窗口，显示一张图片。
//!
//! 来源：截图选区（原位贴图，图片首像素对准选区）或剪贴板（图片、文件路径、文字，居中显示）。
//! 操作：拖动移动、滚轮/±键缩放（10% 步进）、右键菜单（复制、保存、置顶、锁定、鼠标穿透、
//! 变换、不透明度、关闭）、Ctrl+C / Ctrl+S / Ctrl+W / Esc、Ctrl+Alt+P 切换穿透。
//! 鼠标穿透后窗口收不到点击，由全局贴图快捷键或托盘“贴图”恢复（与 Tauri 版一致）。
//!
//! 尺寸与 Tauri 版共用 `snap_core::pins::PinLayout`：缩放 1 表示一图像像素对应一物理像素，
//! 图片四周保留 (padding + border) 的透明边距，按物理像素取整后再换算成窗口逻辑像素。
//! 像素预算、变换、文字排版同样来自 core。未接入：标注、OCR/翻译面板、拖出内容、HTML 贴图。
use gpui_kit::*;
use image::RgbaImage;
use snap_core::{
    pins::{self as core_pins, PinAppearance, PinLayout, PinShadow, Transform},
    settings::{ExportSettings, Language},
};
use std::{sync::Arc, time::Duration};

/// 右键菜单尺寸（逻辑像素）；打开时窗口临时扩大以容纳菜单，关闭后恢复。
const MENU_WIDTH: f32 = 196.0;
const ITEM_HEIGHT: f32 = 28.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const OPACITIES: [f32; 4] = [1.0, 0.8, 0.6, 0.4];

/// 贴图位置：原位（截图选区）或居中（剪贴板）。
pub enum Placement {
    /// `origin` 为图片首像素的全局物理坐标，`scale` 为所在屏缩放。
    Source { origin: (i32, i32), scale: f64, display: Option<DisplayId> },
    Centered,
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

/// Rust 侧常驻像素：原图 + 与原图不同的当前图（与 Tauri 版的预算口径一致）。
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

/// 新建贴图。容量不足、窗口创建失败时返回错误，不留下半个窗口。
pub fn create(cx: &mut App, image: RgbaImage, placement: Placement) -> Result<(), String> {
    let pins = alive(cx);
    let used = pins.iter().map(|(_, pin)| retained(pin.read(cx))).sum();
    core_pins::validate_capacity(image.width(), image.height(), pins.len(), used)?;
    let settings = crate::shell::settings(cx);
    let layout = PinLayout::load();
    let (width, height) = image.dimensions();
    let (bounds, display, zoom, scale) = match placement {
        Placement::Source { origin, scale, display } => {
            let zoom = 1.0_f64.max(layout.minimum_zoom(width, height, scale));
            let inset = layout.image_inset(scale);
            let size = window_size(width, height, zoom, scale);
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
            let size = window_size(width, height, zoom, scale);
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
            let pin = cx.new(|cx| Pin {
                original: image.clone(),
                image,
                frame,
                zoom,
                opacity: 1.0,
                locked: false,
                topmost: true,
                click_through: false,
                appearance: settings.pin_appearance.clone(),
                export: settings.export.clone(),
                language: settings.appearance.language.clone(),
                menu: None,
                badge: None,
                badge_task: None,
                busy: false,
                focus: cx.focus_handle(),
                focused: false,
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
    if let Some(weak) = weak {
        cx.default_global::<Registry>().pins.push((handle, weak));
    }
    log_memory(cx, "created");
    Ok(())
}

/// 图片 + 两侧透明边距的窗口逻辑尺寸；边距按物理像素取整，首像素落在整数屏幕坐标上。
fn window_size(width: u32, height: u32, zoom: f64, scale: f64) -> Size<Pixels> {
    let layout = PinLayout::load();
    let inset = layout.image_inset(scale);
    let (w, h) = layout.displayed(width, height, zoom);
    size(
        px((w + inset * 2) as f32 / scale as f32),
        px((h + inset * 2) as f32 / scale as f32),
    )
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

/// 剪贴板贴图：图片优先，其次文件路径、文字（排成图片）。解码与排版在后台线程进行。
pub fn from_clipboard(cx: &mut App) {
    enum Source {
        Image(Vec<u8>),
        Text(String),
    }
    let source = cx.read_from_clipboard().and_then(|item| {
        let entries = item.entries();
        entries
            .iter()
            .find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(Source::Image(image.bytes.clone())),
                _ => None,
            })
            .or_else(|| {
                entries.iter().find_map(|entry| match entry {
                    ClipboardEntry::ExternalPaths(paths) => Some(Source::Text(
                        paths.paths().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"),
                    )),
                    _ => None,
                })
            })
            .or_else(|| {
                entries.iter().find_map(|entry| match entry {
                    ClipboardEntry::String(text) => Some(Source::Text(text.text().clone())),
                    _ => None,
                })
            })
    });
    let Some(source) = source else {
        log::warn!("event=gpui.pin_clipboard_empty message=剪贴板没有可读取的图片或文字。");
        return;
    };
    cx.spawn(async move |cx| {
        let image = cx
            .background_executor()
            .spawn(async move {
                match source {
                    Source::Image(bytes) => decode_clipboard_image(&bytes),
                    Source::Text(text) => {
                        let font = core_pins::text::font_bytes(user_font())?;
                        core_pins::text::render(&text, &font)
                    }
                }
            })
            .await;
        cx.update(|cx| match image.and_then(|image| create(cx, image, Placement::Centered)) {
            Ok(()) => log::info!("event=gpui.pin_created source=clipboard"),
            Err(error) => log::error!("event=gpui.pin_failed source=clipboard message={error}"),
        });
    })
    .detach();
}

/// Tauri 版“通用设置”中下载或选择的导出字体；读不到或无法解析时回退系统字体。
fn user_font() -> Option<Vec<u8>> {
    let path = snap_core::paths::data_dir()?.join("resources").join("export-font.ttf");
    let bytes = std::fs::read(path).ok()?;
    ab_glyph::FontRef::try_from_slice(&bytes).is_ok().then_some(bytes)
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
    Copy,
    Save,
    Topmost,
    Lock,
    ClickThrough,
    Transform(Transform),
    Opacity(usize),
    Close,
    Separator,
}

const MENU: [Item; 21] = [
    Item::Copy,
    Item::Save,
    Item::Separator,
    Item::Topmost,
    Item::Lock,
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
    Item::Close,
];

fn menu_height() -> f32 {
    MENU.iter()
        .map(|item| if *item == Item::Separator { SEPARATOR_HEIGHT } else { ITEM_HEIGHT })
        .sum::<f32>()
        + 8.0
}

pub struct Pin {
    original: Arc<RgbaImage>,
    image: Arc<RgbaImage>,
    frame: Arc<RenderImage>,
    zoom: f64,
    opacity: f32,
    locked: bool,
    topmost: bool,
    click_through: bool,
    appearance: PinAppearance,
    export: ExportSettings,
    language: Language,
    /// 右键菜单在窗口内的位置（逻辑像素）。
    menu: Option<Point<Pixels>>,
    /// 缩放比例、“已复制”等短暂提示。
    badge: Option<String>,
    badge_task: Option<Task<()>>,
    busy: bool,
    focus: FocusHandle,
    focused: bool,
}

impl Pin {
    fn text(&self, message: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, message).to_owned()
    }

    fn base_size(&self, scale: f32) -> Size<Pixels> {
        window_size(self.image.width(), self.image.height(), self.zoom, f64::from(scale))
    }

    /// 菜单打开时窗口扩到能放下菜单；图片左上角不动（窗口只向右下扩展）。
    fn fit_window(&self, window: &mut Window) {
        let mut size = self.base_size(window.scale_factor());
        if let Some(at) = self.menu {
            size.width = size.width.max(at.x + px(MENU_WIDTH + 4.0));
            size.height = size.height.max(at.y + px(menu_height() + 4.0));
        }
        window.resize(size);
    }

    fn flash(&mut self, message: String, cx: &mut Context<Self>) {
        self.badge = Some(message);
        self.badge_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1200)).await;
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

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            self.fit_window(window);
            cx.notify();
        }
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
            Item::Copy => self.copy(window, cx),
            Item::Save => self.save(window, cx),
            Item::Topmost => match crate::native::set_topmost(window, !self.topmost) {
                Ok(()) => self.topmost = !self.topmost,
                Err(error) => log::warn!("event=gpui.pin_topmost_failed message={error}"),
            },
            Item::Lock => self.locked = !self.locked,
            Item::ClickThrough => self.set_click_through(!self.click_through, window, cx),
            Item::Transform(action) => self.transform(action, cx),
            Item::Opacity(index) => self.opacity = OPACITIES[index],
            Item::Close => window.remove_window(),
            Item::Separator => {}
        }
        cx.notify();
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let owner = crate::native::hwnd(window);
        let image = self.image.clone();
        cx.spawn(async move |this, cx| {
            let png = cx.background_executor().spawn({
                let image = image.clone();
                async move { crate::capture::png(&image) }
            }).await;
            let _ = this.update(cx, |pin, cx| {
                pin.busy = false;
                match png.and_then(|png| crate::capture::write_clipboard(cx, owner, &image, png)) {
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
        let image = self.image.clone();
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
        let (label, extensions) = export.format.filter();
        let record = snap_core::export::directory_record();
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title(self.text("保存图片"))
            .add_filter(self.text(label), extensions)
            .set_file_name(name)
            .set_parent(window);
        if let Some(directory) = record
            .as_deref()
            .and_then(|path| snap_core::export::preferred_directory(path, export.remember_directory))
        {
            dialog = dialog.set_directory(directory);
        }
        cx.spawn(async move |this, cx| {
            let Some(file) = dialog.save_file().await else {
                let _ = this.update(cx, |pin, _| pin.busy = false);
                return;
            };
            let path = file.path().to_path_buf();
            let written = cx
                .background_executor()
                .spawn({
                    let (path, export) = (path.clone(), export.clone());
                    async move { snap_core::export::write_file(&path, &image, &export) }
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

    /// 像素变换在后台线程计算；预先按 Tauri 版口径检查预算，恢复原图直接共享原缓冲。
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
            let _ = this.update(cx, |pin, cx| {
                pin.busy = false;
                pin.image = image;
                pin.frame = frame;
                cx.notify();
            });
            // 旋转会交换宽高：按新尺寸重设窗口。
            let _ = cx.update(|cx| {
                let windows = alive(cx);
                if let Some((handle, pin)) = windows.into_iter().find(|(_, pin)| this.upgrade().as_ref() == Some(pin)) {
                    let _ = handle.update(cx, |_, window, cx| pin.read(cx).fit_window(window));
                }
                log_memory(cx, "transformed");
            });
        })
        .detach();
    }

    fn press(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.close_menu(window, cx);
            return;
        }
        if !self.locked {
            window.start_window_move();
        }
    }

    fn right_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = Some(event.position);
        self.fit_window(window);
        cx.notify();
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
        match keystroke.key.as_str() {
            "escape" if self.menu.is_some() => self.close_menu(window, cx),
            "escape" => window.remove_window(),
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
            Item::Copy => (self.text("复制图片"), false),
            Item::Save => (self.text("保存图片"), false),
            Item::Topmost => (self.text("窗口置顶"), self.topmost),
            Item::Lock => (self.text("锁定贴图"), self.locked),
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

    fn shortcut(item: Item) -> &'static str {
        match item {
            Item::Copy => "Ctrl+C",
            Item::Save => "Ctrl+S",
            Item::ClickThrough => "Ctrl+Alt+P",
            Item::Close => "Esc",
            _ => "",
        }
    }
}

fn hex(value: &str) -> u32 {
    u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(0xa1a1aa)
}

impl Render for Pin {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus, cx);
            self.focused = true;
        }
        let scale = window.scale_factor();
        let layout = PinLayout::load();
        let inset = layout.image_inset(f64::from(scale)) as f32 / scale;
        let (w, h) = layout.displayed(self.image.width(), self.image.height(), self.zoom);
        let (w, h) = (w as f32 / scale, h as f32 / scale);
        let border = layout.border as f32;
        let radius = (self.appearance.corner_radius as f32).min(w.min(h) / 2.0);
        let mut stage = div()
            .absolute()
            .left(px(inset - border))
            .top(px(inset - border))
            .w(px(w + border * 2.0))
            .h(px(h + border * 2.0))
            .border_1()
            .border_color(rgb(hex(&self.appearance.border_color)))
            .rounded(px(radius))
            .overflow_hidden()
            .opacity(self.opacity)
            .child(img(self.frame.clone()).size_full().object_fit(ObjectFit::Fill));
        stage = match self.appearance.shadow {
            PinShadow::None => stage,
            PinShadow::Soft => stage.shadow_sm(),
            PinShadow::Auto => stage.shadow_md(),
        };
        let mut root = div()
            .id("pin")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_click))
            .on_scroll(cx.listener(Self::wheel))
            .child(stage);
        let badge = |text: String, top: f32| {
            div()
                .absolute()
                .left(px(inset + 8.0))
                .top(px(top))
                .px_1()
                .rounded_sm()
                .bg(rgba(0x000000b0))
                .text_xs()
                .text_color(rgb(0xffffff))
                .child(text)
        };
        if let Some(text) = self.badge.clone() {
            root = root.child(badge(text, inset + 8.0));
        }
        if self.click_through {
            let hint = self.text("鼠标穿透中 · 用全局贴图快捷键或托盘恢复");
            root = root.child(badge(hint, (inset + h - 24.0).max(inset + 8.0)));
        }
        if let Some(at) = self.menu {
            let mut menu = div()
                .id("pin-menu")
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(px(MENU_WIDTH))
                .py_1()
                .flex()
                .flex_col()
                .rounded_md()
                .bg(rgb(0x18181b))
                .shadow_lg()
                .text_sm()
                .text_color(rgb(0xf4f4f5))
                // 菜单内的按下不能冒泡成拖动窗口或关闭菜单。
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
            for (index, item) in MENU.into_iter().enumerate() {
                if item == Item::Separator {
                    menu = menu.child(div().h(px(SEPARATOR_HEIGHT)).flex().items_center().child(div().h(px(1.0)).w_full().bg(rgb(0x3f3f46))));
                    continue;
                }
                let (label, checked) = self.label(item);
                menu = menu.child(
                    div()
                        .id(("pin-menu-item", index))
                        .h(px(ITEM_HEIGHT))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x3f3f46)))
                        .child(div().w(px(12.0)).child(if checked { "✓" } else { "" }))
                        .child(div().flex_1().child(label))
                        .child(div().text_xs().text_color(rgb(0xa1a1aa)).child(Self::shortcut(item)))
                        .on_click(cx.listener(move |pin, _: &ClickEvent, window, cx| pin.select(item, window, cx))),
                );
            }
            root = root.child(menu);
        }
        root
    }
}
