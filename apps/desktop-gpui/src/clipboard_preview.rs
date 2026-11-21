//! 剪贴板条目预览窗口（对照 Vue 版 `ClipboardPreview.vue`）：文本显示全文，图片按窗口等比缩放显示原图。
//!
//! 链接可交给系统浏览器打开（core `classification::link_url` 校验协议）；文件路径只在文件管理器中定位，
//! 不执行文件本身。内容在后台读取，窗口关闭即释放。同一时间只有一个预览窗口，再次预览替换内容。
use crate::annotate;
use gpui_kit::*;
use snap_core::clipboard::{
    classification,
    model::{Item, Kind},
    service::{Content, Service},
};
use std::sync::Arc;

/// 过长的文本只显示开头，避免一次排版数 MB 文字；复制仍使用完整原文。
const MAX_CHARS: usize = 200_000;
const MAX_LINES: usize = 5_000;

struct PreviewState {
    window: Option<AnyWindowHandle>,
}
impl Global for PreviewState {}

/// 预览窗口是否打开（历史窗口失焦时据此不收起）。
pub fn is_open(cx: &App) -> bool {
    cx.try_global::<PreviewState>()
        .and_then(|s| s.window)
        .is_some_and(|handle| {
            cx.windows()
                .iter()
                .any(|w| w.window_id() == handle.window_id())
        })
}

pub fn open(cx: &mut App, service: Service, item: Item) {
    if item.kind == Kind::File {
        return;
    }
    if let Some(handle) = cx.try_global::<PreviewState>().and_then(|s| s.window) {
        // 替换为新条目：先关掉旧窗口，尺寸按新内容重新计算。
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    let language = crate::shell::settings(cx).appearance.language;
    let title = snap_core::i18n::for_language(&language, "预览").to_owned();
    let (width, height) = match item.kind {
        Kind::Image => {
            let (w, h) = (
                item.width.unwrap_or(640) as f32,
                item.height.unwrap_or(480) as f32,
            );
            // 原图过大时按 960×720 等比缩小，窗口不超过常见屏幕。
            let scale = (960.0 / w).min(720.0 / h).min(1.0);
            ((w * scale).max(320.0), (h * scale + 52.0).max(240.0))
        }
        _ => (640.0, 480.0),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(size(px(280.0), px(200.0))),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            let mut view = Preview {
                service,
                language,
                item,
                body: Body::Loading,
                error: None,
                message: None,
                focus,
            };
            view.load(cx);
            view
        })
    }) {
        Ok(handle) => cx.set_global(PreviewState {
            window: Some(handle.into()),
        }),
        Err(error) => log::error!("event=gpui.clipboard_preview_failed message={error}"),
    }
}

enum Body {
    Loading,
    Text {
        lines: Vec<String>,
        truncated: bool,
        link: Option<String>,
        path: Option<String>,
    },
    Image(Arc<RenderImage>),
}

struct Preview {
    service: Service,
    language: snap_core::settings::Language,
    item: Item,
    body: Body,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
}

/// 文本按行切分并截断到显示上限。
fn text_lines(text: &str) -> (Vec<String>, bool) {
    let mut truncated = false;
    let shown: String = if text.chars().count() > MAX_CHARS {
        truncated = true;
        text.chars().take(MAX_CHARS).collect()
    } else {
        text.to_string()
    };
    let mut lines: Vec<String> = shown
        .replace("\r\n", "\n")
        .split('\n')
        .map(str::to_string)
        .collect();
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES);
        truncated = true;
    }
    (lines, truncated)
}

impl Preview {
    fn t(&self, key: &'static str) -> String {
        snap_core::i18n::for_language(&self.language, key).to_owned()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (service, id) = (self.service.clone(), self.item.id);
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    match service.contents(&[id])?.remove(0) {
                        Content::Text(text) => {
                            let (lines, truncated) = text_lines(&text);
                            let link = classification::link_url(&text);
                            let path = classification::file_path(&text).map(str::to_string);
                            Ok(Body::Text {
                                lines,
                                truncated,
                                link,
                                path,
                            })
                        }
                        Content::Image(mut image) => {
                            for pixel in image.pixels_mut() {
                                pixel.0.swap(0, 2);
                            }
                            Ok(Body::Image(Arc::new(RenderImage::new(vec![
                                image::Frame::new(image),
                            ]))))
                        }
                        Content::Files(_) => Err("文件条目没有预览。".to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match loaded {
                    Ok(body) => view.body = body,
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn button(&self, id: &'static str, label: String) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(9.0))
            .py(px(3.0))
            .rounded(px(6.0))
            .text_xs()
            .cursor_pointer()
            .text_color(rgb(annotate::muted()))
            .bg(rgb(annotate::surface()))
            .hover(|s| s.text_color(rgb(annotate::text())).bg(rgb(annotate::hover())))
            .child(label)
    }

    fn result(
        &mut self,
        result: Result<(), String>,
        success: &'static str,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.error = None;
                self.message = Some(self.t(success));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
}

impl Render for Preview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.language.clone();
        let text =
            move |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let mut actions = div().flex().gap(px(6.0)).items_center();
        let mut note = None;
        let content: AnyElement = match &self.body {
            Body::Loading => div()
                .p(px(16.0))
                .text_sm()
                .text_color(rgb(annotate::muted()))
                .child(text("正在加载…"))
                .into_any_element(),
            Body::Image(image) => div()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(crate::theme::c().window_bg))
                .child(
                    img(image.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Body::Text {
                lines,
                truncated,
                link,
                path,
            } => {
                if *truncated {
                    note = Some(text("内容较长，仅显示开头部分；复制仍使用完整原文。"));
                }
                if let Some(url) = link.clone() {
                    actions = actions.child(
                        self.button("preview-link", text("打开链接"))
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                cx.open_url(&url);
                                view.result(Ok(()), "已交给浏览器打开。", cx);
                            })),
                    );
                }
                if let Some(path) = path.clone() {
                    actions = actions.child(
                        self.button("preview-path", text("在文件夹中显示"))
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                let result =
                                    snap_core::platform::reveal_path(std::path::Path::new(&path));
                                view.result(result, "已在文件夹中显示。", cx);
                            })),
                    );
                }
                let mut body = div()
                    .flex()
                    .flex_col()
                    .p(px(12.0))
                    .text_sm()
                    .font_family("Consolas")
                    .text_color(rgb(annotate::text()));
                for line in lines {
                    // 空行保留高度。
                    body = body.child(div().min_h(px(18.0)).child(line.clone()));
                }
                div()
                    .id("preview-text")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .bg(rgb(crate::theme::c().inset))
                    .child(body)
                    .into_any_element()
            }
        };
        actions = actions.child(
            self.button("preview-close", text("关闭"))
                .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window())),
        );
        let notice = match (&self.error, &self.message) {
            (Some(error), _) => Some(
                div()
                    .text_xs()
                    .text_color(rgb(crate::theme::c().error_text))
                    .child(error.clone()),
            ),
            (None, Some(message)) => Some(
                div()
                    .text_xs()
                    .text_color(rgb(crate::theme::c().ok_text))
                    .child(message.clone()),
            ),
            _ => None,
        };
        let meta = match self.item.kind {
            Kind::Image => format!(
                "{} {}×{}",
                text("图片"),
                self.item.width.unwrap_or(0),
                self.item.height.unwrap_or(0)
            ),
            _ => format!("{} · {} B", text("文本"), self.item.byte_size),
        };
        div()
            .id("clipboard-preview")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(10.0))
            .bg(rgb(crate::theme::c().panel))
            .text_color(rgb(annotate::text()))
            .child(content)
            .children(note.map(|n| div().text_xs().text_color(rgb(annotate::muted())).child(n)))
            .children(notice)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(annotate::muted()))
                            .child(meta),
                    )
                    .child(actions),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_LINES, text_lines};

    #[test]
    fn long_text_is_truncated_for_display() {
        let (lines, truncated) = text_lines("a\r\nb\n\nc");
        assert_eq!(lines, vec!["a", "b", "", "c"]);
        assert!(!truncated);
        let (lines, truncated) = text_lines(&"x\n".repeat(MAX_LINES + 10));
        assert_eq!(lines.len(), MAX_LINES);
        assert!(truncated);
    }
}
