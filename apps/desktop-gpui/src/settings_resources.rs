//! 设置窗口中的资源下载（对照 Vue `ResourceSettings.vue` 与 Tauri `resources/mod.rs`）。
//!
//! 本地 PP-OCR 模型（Windows 上连同 ONNX 运行库）与导出字体的状态、下载、选择本机字体。下载、校验与
//! 安装在 core `snap_core::resources`（只用内置清单的地址与摘要，校验通过才原子替换）；资源目录与
//! Tauri 版相同，两个版本共用已下载的资源。下载在后台线程执行，界面每 150 ms 读取一次进度。
use super::{SettingsView, col, row};
use gpui_kit::*;
use snap_core::resources::{self, ResourceKind, Status};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// 下载进度：文件名、已接收、总字节。
pub(super) type Progress = Arc<Mutex<Option<(String, u64, u64)>>>;

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1024.0 / 1024.0)
}

impl SettingsView {
    /// 后台校验资源状态（流式摘要，模型较大时需要数百毫秒）。
    pub(super) fn load_resources(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let dir = resources::directory().ok_or("应用数据目录不可用。")?;
                    resources::status(&dir)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(status) => view.resources = Some(status),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn download_resources(&mut self, kind: ResourceKind, cx: &mut Context<Self>) {
        if self.resource_busy {
            return;
        }
        self.resource_busy = true;
        self.error = None;
        let progress = self.resource_progress.clone();
        if let Ok(mut slot) = progress.lock() {
            *slot = None;
        }
        self.watch_progress(cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let sink = progress.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let dir = resources::directory().ok_or("应用数据目录不可用。")?;
                    resources::download(&dir, kind, &mut |name, received, total| {
                        if let Ok(mut slot) = sink.lock() {
                            *slot = Some((name.to_owned(), received, total));
                        }
                    })
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.resource_busy = false;
                match result {
                    Ok(()) => {
                        log::info!("event=gpui.resources_downloaded kind={kind:?}");
                        view.saved = true;
                    }
                    Err(error) => {
                        log::warn!("event=gpui.resources_download_failed kind={kind:?}");
                        view.error = Some(error);
                    }
                }
                view.load_resources(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 下载期间定时重绘进度，直到 `resource_busy` 复位。
    pub(super) fn watch_progress(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let busy = this.update(cx, |view, cx| {
                    cx.notify();
                    view.resource_busy
                });
                if !busy.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// 选择本机静态 TTF 字体，校验后复制到资源目录。
    fn choose_font(&mut self, cx: &mut Context<Self>) {
        if self.resource_busy {
            return;
        }
        let language = crate::shell::settings(cx).appearance.language;
        let text = |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let dialog = rfd::AsyncFileDialog::new()
            .set_title(text("选择静态 TTF 导出字体"))
            .add_filter(text("TrueType 字体"), &["ttf"]);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
            let picked = dialog.pick_file().await;
            cx.update(|cx| crate::pins::show_pins(cx, hidden));
            let Some(file) = picked else {
                return;
            };
            let path = file.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = resources::read_font(&path)?;
                    resources::install_font(
                        &resources::directory().ok_or("应用数据目录不可用。")?,
                        &bytes,
                    )
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(()) => view.saved = true,
                    Err(error) => view.error = Some(error),
                }
                view.load_resources(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn resource_button(&self, id: &'static str, label: String, enabled: bool) -> Stateful<Div> {
        let button = div()
            .id(id)
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::surface()))
            .text_xs()
            .child(label);
        if enabled {
            button.cursor_pointer().hover(|s| s.bg(rgb(col::page_bg())))
        } else {
            button.opacity(0.5)
        }
    }

    /// 进度说明（下载中时）。
    fn progress_text(&self, t: &dyn Fn(&str) -> String) -> Option<String> {
        if !self.resource_busy {
            return None;
        }
        let progress = self.resource_progress.lock().ok().and_then(|p| p.clone());
        Some(match progress {
            Some((name, received, total)) => format!(
                "{} {name} · {} / {} MiB",
                t("正在下载"),
                megabytes(received),
                megabytes(total)
            ),
            None => t("正在准备下载…"),
        })
    }

    /// 识别页：本地 PP-OCR 资源状态与下载。
    pub(super) fn ocr_resource_row(
        &self,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let status: Option<&Status> = self.resources.as_ref();
        let hint = self.progress_text(t).unwrap_or_else(|| match status {
            None => t("正在检查识别资源…"),
            Some(s) if s.ocr_ready() => t("已就绪"),
            Some(s) if !s.runtime_ready => format!(
                "{}（{} {} MiB）",
                t("未下载识别资源"),
                t("约"),
                s.runtime_download_mib
            ),
            Some(_) => t("未下载识别资源"),
        });
        let ready = status.is_some_and(Status::ocr_ready);
        let label = t(if ready {
            "重新校验"
        } else {
            "下载识别资源"
        });
        row(
            t("本地识别资源"),
            Some(hint),
            self.resource_button("resources-ocr", label, !self.resource_busy)
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.download_resources(ResourceKind::Ocr, cx)
                })),
        )
    }

    /// 通用页：导出字体（剪贴板组合导出长图/PDF、文字贴图使用）。
    pub(super) fn font_resource_row(
        &self,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let hint = self
            .progress_text(t)
            .unwrap_or_else(|| match &self.resources {
                None => t("正在检查字体…"),
                Some(s) if s.font_ready => t("已配置导出字体"),
                Some(_) => t("未配置导出字体，组合导出与文字贴图使用系统字体。"),
            });
        row(
            t("导出字体"),
            Some(hint),
            div()
                .flex()
                .gap(px(6.0))
                .child(
                    self.resource_button("resources-font", t("下载推荐字体"), !self.resource_busy)
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.download_resources(ResourceKind::Font, cx)
                        })),
                )
                .child(
                    self.resource_button(
                        "resources-font-choose",
                        t("选择字体…"),
                        !self.resource_busy,
                    )
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.choose_font(cx))),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::megabytes;

    #[test]
    fn sizes_show_one_decimal() {
        assert_eq!(megabytes(0), "0.0");
        assert_eq!(megabytes(1536 * 1024), "1.5");
    }
}
