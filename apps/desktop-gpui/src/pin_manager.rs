//! 贴图管理窗口：列出当前贴图（缩略图、尺寸、缩放、置顶/锁定/穿透/隐藏状态、标注数），
//! 可定位（显示并解除穿透）或关闭单张贴图；顶部提供剪贴板贴图、打开图片贴图、全部隐藏/显示、关闭全部。
//!
//! Tauri 版没有独立的管理列表（主窗口贴图页只有创建入口与外观设置，单张贴图在自身菜单里操作）；
//! GPUI 版没有主窗口，托盘“贴图管理”打开本窗口。列表每秒刷新一次；缩略图最长边 96 像素，
//! 按贴图图像缓存，图像变化（变换）或贴图关闭后释放对应纹理。
use crate::annotate;
use gpui_kit::*;
use image::RgbaImage;
use std::{collections::HashMap, sync::Arc, time::Duration};

const THUMB: u32 = 96;

#[derive(Default)]
struct ManagerState {
    window: Option<AnyWindowHandle>,
}
impl Global for ManagerState {}

/// 打开（或激活已打开的）贴图管理窗口。
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<ManagerState>().and_then(|s| s.window) {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let settings = crate::shell::settings(cx);
    let text = |key: &'static str| snap_core::i18n::for_language(&settings.appearance.language, key).to_owned();
    let size = size(px(560.0), px(520.0));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions { title: Some(text("贴图管理").into()), ..Default::default() }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(gpui_kit::size(px(420.0), px(300.0))),
        ..Default::default()
    };
    let language = settings.appearance.language.clone();
    match cx.open_window(options, |_, cx| {
        cx.new(|cx| {
            // 每秒刷新：贴图在其他窗口中创建、关闭或改变状态时同步列表。
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            })
            .detach();
            Manager { language, thumbs: HashMap::new(), focus: cx.focus_handle() }
        })
    }) {
        Ok(handle) => {
            cx.default_global::<ManagerState>().window = Some(handle.into());
            log::info!("event=gpui.pin_manager_opened");
        }
        Err(error) => log::error!("event=gpui.pin_manager_open_failed message={error}"),
    }
}

struct Manager {
    language: snap_core::settings::Language,
    /// 以贴图图像地址为键的缩略图；与当前贴图对不上的在渲染时释放。
    thumbs: HashMap<usize, Arc<RenderImage>>,
    focus: FocusHandle,
}

fn thumbnail(image: &RgbaImage) -> Arc<RenderImage> {
    let scale = (f64::from(THUMB) / f64::from(image.width().max(image.height()))).min(1.0);
    let w = ((f64::from(image.width()) * scale).round() as u32).max(1);
    let h = ((f64::from(image.height()) * scale).round() as u32).max(1);
    let mut small = image::imageops::thumbnail(image, w, h);
    for pixel in small.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(small)]))
}

impl Manager {
    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn button(&self, id: impl Into<ElementId>, label: String, danger: bool) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(annotate::BORDER))
            .bg(rgb(annotate::SURFACE))
            .text_sm()
            .text_color(rgb(if danger { 0xfb7185 } else { annotate::TEXT }))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(annotate::HOVER)))
            .child(label)
    }

    fn tag(&self, label: String) -> Div {
        div()
            .px(px(6.0))
            .py(px(1.0))
            .rounded(px(4.0))
            .bg(rgb(annotate::ACTIVE_BG))
            .text_xs()
            .text_color(rgb(annotate::ACTIVE_FG))
            .child(label)
    }
}

impl Render for Manager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pins = crate::pins::summaries(cx);
        // 释放已关闭或已变换贴图的缩略图纹理，并为新图生成缩略图。
        let live: Vec<usize> = pins.iter().map(|p| Arc::as_ptr(&p.image) as usize).collect();
        let stale: Vec<usize> = self.thumbs.keys().filter(|k| !live.contains(k)).copied().collect();
        for key in stale {
            if let Some(image) = self.thumbs.remove(&key) {
                cx.drop_image(image, Some(window));
            }
        }
        for pin in &pins {
            self.thumbs
                .entry(Arc::as_ptr(&pin.image) as usize)
                .or_insert_with(|| thumbnail(&pin.image));
        }
        let any_hidden = pins.iter().any(|p| p.hidden);
        let header = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .child(self.button("pins-clipboard", self.t("剪贴板贴图"), false).on_click(|_, _, cx| crate::pins::from_clipboard(cx)))
            .child(self.button("pins-file", self.t("打开图片贴图"), false).on_click(|_, _, cx| crate::pins::open_file(cx)))
            .child(if any_hidden {
                self.button("pins-show", self.t("显示全部贴图"), false).on_click(|_, _, cx| crate::pins::show_hidden(cx))
            } else {
                self.button("pins-hide", self.t("隐藏全部贴图"), false).on_click(|_, _, cx| {
                    // 手动隐藏与对话框共用隐藏标记，“显示全部贴图”或定位时恢复。
                    let _ = crate::pins::hide_pins(cx, None);
                })
            })
            .child(self.button("pins-close-all", self.t("关闭全部贴图"), true).on_click(|_, _, cx| {
                crate::pins::close_all(cx);
            }));
        let mut list = div().id("pins-list").flex_1().flex().flex_col().gap(px(6.0)).overflow_y_scroll();
        if pins.is_empty() {
            list = list.child(div().p(px(16.0)).text_sm().text_color(rgb(annotate::MUTED)).child(self.t("当前没有贴图。")));
        }
        for (index, pin) in pins.iter().enumerate() {
            let handle = pin.handle;
            let thumb = self.thumbs.get(&(Arc::as_ptr(&pin.image) as usize)).cloned();
            let mut tags = div().flex().flex_wrap().gap(px(4.0));
            if pin.topmost {
                tags = tags.child(self.tag(self.t("窗口置顶")));
            }
            if pin.locked {
                tags = tags.child(self.tag(self.t("锁定贴图")));
            }
            if pin.click_through {
                tags = tags.child(self.tag(self.t("鼠标穿透")));
            }
            if pin.hidden {
                tags = tags.child(self.tag(self.t("已隐藏")));
            }
            if pin.annotations > 0 {
                tags = tags.child(self.tag(format!("{} {}", self.t("标注"), pin.annotations)));
            }
            list = list.child(
                div()
                    .p(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(annotate::BORDER))
                    .child(
                        div()
                            .size(px(THUMB as f32))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(rgb(0x0f1115))
                            .children(thumb.map(|t| img(t).max_w_full().max_h_full().object_fit(ObjectFit::Contain))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(div().text_sm().child(format!(
                                "{} × {} · {}%",
                                pin.image.width(),
                                pin.image.height(),
                                (pin.zoom * 100.0).round()
                            )))
                            .child(tags),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(self.button(("pin-reveal", index), self.t("定位"), false).on_click(move |_, _, cx| crate::pins::reveal(cx, handle)))
                            .child(self.button(("pin-close", index), self.t("关闭"), true).on_click(move |_, _, cx| crate::pins::close(cx, handle))),
                    ),
            );
        }
        div()
            .id("pin-manager")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(12.0))
            .bg(rgb(0x181b20))
            .text_color(rgb(annotate::TEXT))
            .child(header)
            .child(div().text_xs().text_color(rgb(annotate::MUTED)).child(format!("{} {}", self.t("当前贴图："), pins.len())))
            .child(list)
    }
}
