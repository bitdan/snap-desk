//! 保存成功提示：主屏右下角的小提示窗，几秒后自动消失；单击在文件管理器中定位刚保存的文件。
//!
//! 用于快速保存（截图浮层随即关闭，没有别处可以提示）。提示窗不抢焦点，也不排除在采集之外
//! （排除的窗口紧接着截图时会在原位置留下黑块）；开始新的截图时先收起它，避免拍进截图。
use gpui_kit::*;
use std::{path::PathBuf, time::Duration};

/// 提示窗大小（逻辑像素）与自动消失时间。
const WIDTH: f32 = 340.0;
const HEIGHT: f32 = 60.0;
const MARGIN: f32 = 16.0;
const LIFETIME: Duration = Duration::from_secs(4);

#[derive(Default)]
struct Current(Option<AnyWindowHandle>);
impl Global for Current {}

struct Toast {
    title: String,
    name: String,
    path: PathBuf,
}

impl Render for Toast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("save-toast")
            .size_full()
            .px(px(14.0))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(2.0))
            // 与截图浮层提示同一套深色圆角框。
            .rounded(px(10.0))
            .bg(rgba(0x242330ed))
            .text_color(rgb(0xffffff))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0x2f2d3ef2)))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(self.title.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0xb7b4c8))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.name.clone()),
            )
            .on_click(cx.listener(|toast, _: &ClickEvent, window, _| {
                if let Err(error) = snap_core::platform::reveal_path(&toast.path) {
                    log::warn!("event=gpui.toast_reveal_failed message={error}");
                }
                window.remove_window();
            }))
    }
}

/// 提示“已保存”，单击定位文件；同时只保留一个提示窗。
pub fn saved(cx: &mut App, path: PathBuf) {
    dismiss(cx);
    let language = crate::shell::settings(cx).appearance.language;
    let title = snap_core::i18n::for_language(&language, "已保存，单击在文件夹中显示").to_owned();
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let display = cx.primary_display();
    let Some(area) = display.as_ref().map(|d| d.visible_bounds()) else {
        return;
    };
    let origin = point(
        area.origin.x + area.size.width - px(WIDTH + MARGIN),
        area.origin.y + area.size.height - px(HEIGHT + MARGIN),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size(px(WIDTH), px(HEIGHT))))),
        display_id: display.map(|d| d.id()),
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
    let opened = cx.open_window(options, |window, cx| {
        if let Err(error) = crate::native::set_topmost(window, true) {
            log::warn!("event=gpui.toast_topmost_failed message={error}");
        }
        cx.new(|_| Toast { title, name, path })
    });
    let handle: AnyWindowHandle = match opened {
        Ok(handle) => handle.into(),
        Err(error) => {
            log::warn!("event=gpui.toast_failed message={error}");
            return;
        }
    };
    cx.default_global::<Current>().0 = Some(handle);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(LIFETIME).await;
        cx.update(|cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            let current = cx.default_global::<Current>();
            if current.0 == Some(handle) {
                current.0 = None;
            }
        });
    })
    .detach();
}

/// 收起正在显示的提示窗；返回是否收起了一个（调用方据此稍等合成器移除它再拍屏）。
pub fn dismiss(cx: &mut App) -> bool {
    let Some(handle) = cx.default_global::<Current>().0.take() else {
        return false;
    };
    handle.update(cx, |_, window, _| window.remove_window()).is_ok()
}
