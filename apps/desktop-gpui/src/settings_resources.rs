//! 设置窗口中的资源下载。
//!
//! 本地 PP-OCR 模型（Windows 上连同 ONNX 运行库）的状态与下载（GPUI 版不使用导出字体，不提供字体设置）。下载、校验与
//! 安装在 core `snap_core::resources`（只用内置清单的地址与摘要，校验通过才原子替换）；资源目录与
//! 旧版相同，升级后沿用已下载的资源。下载在后台线程执行，界面每 150 ms 读取一次进度。
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
