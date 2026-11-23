//! 设置“识别与翻译”页的本地翻译管理。
//!
//! 引擎（CTranslate2 worker，Windows x64）与两个语言包（英→中、中→英）的状态、下载、导入、删除。
//! 校验与安装在 core；替换前先停止并锁住缓存的 worker（`local_translate::with_worker_stopped`），
//! 进度与资源下载共用同一显示。语言包与引擎目录与旧版相同。
use super::{SettingsView, col, row};
use gpui_kit::*;
use snap_core::recognition::{
    local_models::{self, IDS, LocalStatus},
    local_runtime,
};
use std::path::PathBuf;

/// 本地翻译的管理操作。
#[derive(Clone, Copy, Debug)]
enum Action {
    DownloadEngine,
    ImportEngine,
    Download(usize),
    Import(usize),
    Delete(usize),
}

fn model_label(index: usize) -> &'static str {
    if index == 0 {
        "英译中语言包"
    } else {
        "中译英语言包"
    }
}

impl SettingsView {
    /// 后台读取本地翻译状态（完整校验）。
    pub(super) fn load_local(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let status = cx
                .background_executor()
                .spawn(async { crate::local_translate::status() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.local = Some(status);
                cx.notify();
            });
        })
        .detach();
    }

    fn local_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.resource_busy {
            return;
        }
        let language = crate::shell::settings(cx).appearance.language;
        let text = |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let picker = match action {
            Action::ImportEngine => Some(
                rfd::AsyncFileDialog::new()
                    .set_title(text("选择翻译引擎 ZIP"))
                    .add_filter("ZIP", &["zip"]),
            ),
            Action::Import(_) => Some(
                rfd::AsyncFileDialog::new()
                    .set_title(text("选择可信的离线语言包目录（含 pack.json）")),
            ),
            _ => None,
        };
        let progress = self.resource_progress.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // 导入先选择文件或目录；取消选择不做任何改动。
            let source: Option<PathBuf> = match (action, picker) {
                (Action::ImportEngine, Some(dialog)) => {
                    let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
                    let picked = dialog.pick_file().await;
                    cx.update(|cx| crate::pins::show_pins(cx, hidden));
                    match picked {
                        Some(file) => Some(file.path().to_path_buf()),
                        None => return,
                    }
                }
                (Action::Import(_), Some(dialog)) => {
                    let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
                    let picked = dialog.pick_folder().await;
                    cx.update(|cx| crate::pins::show_pins(cx, hidden));
                    match picked {
                        Some(folder) => Some(folder.path().to_path_buf()),
                        None => return,
                    }
                }
                _ => None,
            };
            let started = this.update(cx, |view, cx| {
                if view.resource_busy {
                    return false;
                }
                view.resource_busy = true;
                view.error = None;
                if let Ok(mut slot) = view.resource_progress.lock() {
                    *slot = None;
                }
                view.watch_progress(cx);
                cx.notify();
                true
            });
            if !started.unwrap_or(false) {
                return;
            }
            let sink = progress.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::local_translate::with_worker_stopped(|dir| match action {
                        Action::DownloadEngine => local_runtime::install(dir, None),
                        Action::ImportEngine => local_runtime::install(dir, source.as_deref()),
                        Action::Download(index) => {
                            let pack = local_models::downloadable(IDS[index])?;
                            local_models::install(dir, &pack, None, &mut |name, received, total| {
                                if let Ok(mut slot) = sink.lock() {
                                    *slot = Some((name.to_owned(), received, total));
                                }
                            })
                        }
                        Action::Import(index) => {
                            let source = source.ok_or("未选择目录。")?;
                            let pack = local_models::read_pack(&source)?;
                            if pack.model_id != IDS[index] {
                                return Err("所选语言包方向不匹配。".into());
                            }
                            local_models::install(dir, &pack, Some(&source), &mut |_, _, _| {})
                        }
                        Action::Delete(index) => local_models::delete(dir, IDS[index]),
                    })
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.resource_busy = false;
                match result {
                    Ok(()) => {
                        log::info!("event=gpui.local_translation_action action={action:?}");
                        view.saved = true;
                    }
                    Err(error) => {
                        log::warn!("event=gpui.local_translation_action_failed action={action:?}");
                        view.error = Some(error);
                    }
                }
                view.load_local(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn local_button(
        &self,
        id: (&'static str, usize),
        label: String,
        enabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
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
            button
                .cursor_pointer()
                .hover(|s| s.bg(rgb(col::page_bg())))
                .on_click(
                    cx.listener(move |view, _: &ClickEvent, _, cx| view.local_action(action, cx)),
                )
        } else {
            button.opacity(0.5)
        }
    }

    /// 本地翻译的引擎与语言包行（选用本地翻译时显示）。
    pub(super) fn local_rows(
        &self,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        if !local_runtime::supported() {
            return vec![row(
                t("本地翻译"),
                Some(t("翻译引擎下载和导入目前仅支持 Windows x64。")),
                div(),
            )];
        }
        let enabled = !self.resource_busy;
        let status: Option<&LocalStatus> = self.local.as_ref().and_then(|s| s.as_ref().ok());
        let engine_hint = match (&self.local, status) {
            (None, _) => t("正在检查…"),
            (Some(Err(error)), _) => error.clone(),
            (_, Some(s)) if s.runtime_ready => t("本地引擎已安装"),
            _ => t("未安装翻译引擎"),
        };
        let progress = self.resource_busy.then(|| {
            self.resource_progress
                .lock()
                .ok()
                .and_then(|p| p.clone())
                .map(|(name, received, total)| {
                    format!(
                        "{} {name} · {:.1} / {:.1} MiB",
                        t("正在下载"),
                        received as f64 / 1048576.0,
                        total as f64 / 1048576.0
                    )
                })
                .unwrap_or_else(|| t("正在处理…"))
        });
        let mut rows = vec![row(
            t("翻译引擎"),
            Some(progress.clone().unwrap_or(engine_hint)),
            div()
                .flex()
                .gap(px(6.0))
                .child(self.local_button(
                    ("local-engine", 0),
                    t("下载引擎"),
                    enabled,
                    Action::DownloadEngine,
                    cx,
                ))
                .child(self.local_button(
                    ("local-engine", 1),
                    t("导入 ZIP…"),
                    enabled,
                    Action::ImportEngine,
                    cx,
                )),
        )];
        for index in 0..IDS.len() {
            let model = status.and_then(|s| s.models.get(index));
            let hint = match model {
                None => t("正在检查…"),
                Some(m) if m.ready => t("已安装"),
                Some(m) if m.downloadable => format!(
                    "{}（{:.0} MiB）",
                    t("未安装"),
                    m.download_bytes as f64 / 1048576.0
                ),
                Some(_) => t("未安装，可导入离线包"),
            };
            let installed = model.is_some_and(|m| m.ready);
            let downloadable = model.is_some_and(|m| m.downloadable);
            let mut actions = div()
                .flex()
                .gap(px(6.0))
                .child(self.local_button(
                    ("local-download", index),
                    t("下载"),
                    enabled && downloadable,
                    Action::Download(index),
                    cx,
                ))
                .child(self.local_button(
                    ("local-import", index),
                    t("导入…"),
                    enabled,
                    Action::Import(index),
                    cx,
                ));
            if installed {
                actions = actions.child(self.local_button(
                    ("local-delete", index),
                    t("删除"),
                    enabled,
                    Action::Delete(index),
                    cx,
                ));
            }
            rows.push(row(t(model_label(index)), Some(hint), actions));
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::model_label;
    use snap_core::recognition::local_models::IDS;

    #[test]
    fn labels_follow_model_order() {
        assert!(IDS[0].contains("en-zh"));
        assert_eq!(model_label(0), "英译中语言包");
        assert_eq!(model_label(1), "中译英语言包");
    }
}
