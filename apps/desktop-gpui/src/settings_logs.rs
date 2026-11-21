//! 设置窗口“日志”页（对照 Vue `DiagnosticsSettings.vue` 与 Tauri `diagnostics/mod.rs`）。
//!
//! 读取 GPUI 版日志文件（`snap-desk-gpui.log` 与轮转的 `.log.old`）的有限尾部，经 core 白名单清洗后显示：
//! GPUI 外壳事件只显示事件名，core 事件按审计字段显示，其余行计入“已省略”。可修改日志级别（立即生效，
//! 与 Tauri 版共用本机偏好）、按级别筛选、刷新、打开目录、清理历史日志（确认后只删归档）、导出本地诊断包。
use super::{
    card, heading, row, SettingsView, ACCENT, BORDER, DANGER, MUTED, PAGE_BG, SURFACE, TEXT,
};
use gpui_kit::*;
use snap_core::diagnostics::{
    logs::{self, Entry, Level, Query},
    package,
};
use std::path::PathBuf;

/// 页面上最多显示的条数（扫描仍按 core 上限 500 条、2 MiB）。
const SHOWN: usize = 200;

/// 一次读取的结果。
pub(super) struct LogsView {
    total_bytes: u64,
    archives: usize,
    errors: usize,
    entries: Vec<Entry>,
    omitted: usize,
    truncated: bool,
}

fn directory() -> Result<PathBuf, String> {
    snap_core::paths::log_dir().ok_or_else(|| "无法定位应用日志目录。".to_owned())
}

fn snapshot() -> Result<(Vec<logs::LogFile>, logs::Recent), String> {
    log::logger().flush();
    let root = directory()?;
    let files = logs::files(&root, crate::logging::BASE)?;
    let recent = logs::recent(
        &files,
        &Query {
            limit: 500,
            level: None,
            module: None,
        },
        &[],
    )?;
    Ok((files, recent))
}

fn read() -> Result<LogsView, String> {
    let (files, recent) = snapshot()?;
    Ok(LogsView {
        total_bytes: files
            .iter()
            .fold(0u64, |total, f| total.saturating_add(f.size)),
        archives: files.iter().filter(|f| !f.active).count(),
        errors: recent
            .entries
            .iter()
            .filter(|e| e.level == Level::Error)
            .count(),
        omitted: recent.omitted_lines,
        truncated: recent.truncated,
        entries: recent.entries,
    })
}

fn size_label(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MiB", b as f64 / 1024.0 / 1024.0),
        b if b >= 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

/// RFC 3339 → “MM-DD HH:MM:SS”。
fn time_label(timestamp: &str) -> String {
    timestamp
        .get(5..19)
        .map(|t| t.replace('T', " "))
        .unwrap_or_else(|| timestamp.to_owned())
}

impl SettingsView {
    /// 后台读取日志与统计。
    pub(super) fn load_logs(&mut self, cx: &mut Context<Self>) {
        if self.logs_loading {
            return;
        }
        self.logs_loading = true;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async { read() }).await;
            let _ = this.update(cx, |view, cx| {
                view.logs_loading = false;
                match result {
                    Ok(logs) => view.logs = Some(logs),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_log_level(&mut self, value: Level, cx: &mut Context<Self>) {
        match crate::logging::set_level(value) {
            Ok(()) => {
                log::info!("event=diagnostics.level_changed module=diagnostics level={value:?}");
                self.error = None;
                self.saved = true;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    /// 确认后只删除历史归档，保留正在写入的日志。
    fn clear_archives(&mut self, cx: &mut Context<Self>) {
        let language = crate::shell::settings(cx).appearance.language;
        let text = |key: &'static str| snap_core::i18n::for_language(&language, key).to_owned();
        let dialog = rfd::AsyncMessageDialog::new()
            .set_title(text("清理历史日志"))
            .set_description(text(
                "仅删除历史日志归档，保留当前正在写入的日志。此操作无法撤销。",
            ))
            .set_buttons(rfd::MessageButtons::OkCancelCustom(
                text("确认清理"),
                text("取消"),
            ))
            .set_level(rfd::MessageLevel::Warning);
        let confirm = text("确认清理");
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let answer = dialog.show().await;
            let confirmed = match answer {
                rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes => true,
                rfd::MessageDialogResult::Custom(label) => label == confirm,
                _ => false,
            };
            if !confirmed {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async { logs::clear_archives(&directory()?, crate::logging::BASE) })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok((deleted, failed)) => {
                        log::info!("event=diagnostics.archives_cleared module=diagnostics");
                        view.error = (failed > 0)
                            .then(|| format!("{failed} 个历史日志未能删除，可能正被占用。"));
                        view.notice =
                            (failed == 0).then(|| format!("已删除 {deleted} 个历史日志。"));
                    }
                    Err(error) => view.error = Some(error),
                }
                view.load_logs(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 导出本地诊断包：固定条目、白名单日志与最少的运行时信息，不含配置、凭据或媒体。
    fn export_diagnostics(&mut self, cx: &mut Context<Self>) {
        let language = crate::shell::settings(cx).appearance.language;
        let title = snap_core::i18n::for_language(&language, "导出本地诊断包").to_owned();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let hidden = cx.update(|cx| crate::pins::hide_pins(cx, None));
            let picked = rfd::AsyncFileDialog::new()
                .set_title(title)
                .set_file_name(package::filename())
                .add_filter("ZIP", &["zip"])
                .save_file()
                .await;
            cx.update(|cx| crate::pins::show_pins(cx, hidden));
            let Some(file) = picked else {
                return;
            };
            let destination = file.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move {
                    package::validate_destination(&destination)?;
                    let (_, recent) = snapshot()?;
                    let metadata = serde_json::json!({
                        "schemaVersion": 1,
                        "app": "Snap Desk", "shell": "gpui", "appVersion": env!("CARGO_PKG_VERSION"),
                        "buildChannel": "gpui-preview",
                        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                        "collectedAt": chrono::Local::now().to_rfc3339(),
                        "logLevel": crate::logging::level(),
                        // 不为导出试录视频；仅为后端能力检查。
                        "recording": snap_core::recording::diagnostic_summary(),
                        "logs": package::log_summary(&recent)
                    });
                    package::write_package(&destination, &metadata, &recent)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(()) => {
                        log::info!("event=diagnostics.exported module=diagnostics");
                        view.error = None;
                        view.notice = Some("已导出诊断包。".into());
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn logs_page(
        &self,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let current = crate::logging::level();
        let mut levels = div()
            .flex()
            .p(px(2.0))
            .gap(px(2.0))
            .rounded(px(6.0))
            .bg(rgb(PAGE_BG));
        for (index, (value, label)) in [
            (Level::Error, "错误"),
            (Level::Warn, "警告"),
            (Level::Info, "信息"),
            (Level::Debug, "调试"),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = value == current;
            levels = levels.child(
                div()
                    .id(("log-level", index))
                    .h(px(26.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .text_xs()
                    .cursor_pointer()
                    .text_color(rgb(if selected { TEXT } else { MUTED }))
                    .bg(rgb(if selected { SURFACE } else { PAGE_BG }))
                    .child(t(label))
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        if !selected {
                            view.set_log_level(value, cx);
                        }
                    })),
            );
        }
        let level_hint = if crate::logging::fixed_by_environment() {
            Some(t(
                "当前由环境变量 SNAP_DESK_LOG 决定，修改将在下次启动时生效。",
            ))
        } else {
            Some(t("调试级别会记录更多细节，排错后请调回信息。"))
        };
        let summary = match &self.logs {
            Some(logs) => format!(
                "{} · {} {} · {} {}",
                size_label(logs.total_bytes),
                logs.archives,
                t("个历史日志"),
                logs.errors,
                t("条近期错误")
            ),
            None if self.logs_loading => t("正在读取…"),
            None => String::new(),
        };
        let button = |id: &'static str, label: String| {
            div()
                .id(id)
                .h(px(28.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(SURFACE))
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(PAGE_BG)))
                .child(label)
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .child(
                button("logs-refresh", t("刷新"))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.load_logs(cx))),
            )
            .child(self.open_button(
                "logs-open",
                t("打开日志目录"),
                snap_core::paths::log_dir(),
                cx,
            ))
            .child(
                button("logs-clear", t("清理历史日志"))
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.clear_archives(cx))),
            )
            .child(
                button("logs-export", t("导出本地诊断包…")).on_click(
                    cx.listener(|view, _: &ClickEvent, _, cx| view.export_diagnostics(cx)),
                ),
            );
        // 筛选：全部 / 错误 / 警告。
        let mut filters = div().flex().gap(px(4.0));
        for (index, (value, label)) in [
            (None, "全部"),
            (Some(Level::Error), "错误"),
            (Some(Level::Warn), "警告"),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = self.log_filter == value;
            filters = filters.child(
                div()
                    .id(("log-filter", index))
                    .h(px(24.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .text_xs()
                    .cursor_pointer()
                    .text_color(rgb(if selected { ACCENT } else { MUTED }))
                    .bg(rgb(if selected { 0xeff6ff } else { SURFACE }))
                    .child(t(label))
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.log_filter = value;
                        cx.notify();
                    })),
            );
        }
        let mut list = div().flex().flex_col();
        let mut shown = 0;
        if let Some(logs) = &self.logs {
            for entry in logs
                .entries
                .iter()
                .filter(|e| self.log_filter.is_none_or(|l| l == e.level))
                .take(SHOWN)
            {
                shown += 1;
                let (color, label) = match entry.level {
                    Level::Error => (DANGER, "ERROR"),
                    Level::Warn => (0xb45309, "WARN"),
                    Level::Info => (MUTED, "INFO"),
                    Level::Debug => (0xa1a1aa, "DEBUG"),
                };
                list = list.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .px(px(12.0))
                        .py(px(5.0))
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .text_xs()
                        .child(
                            div()
                                .flex_none()
                                .w(px(104.0))
                                .text_color(rgb(MUTED))
                                .child(time_label(&entry.timestamp)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(44.0))
                                .text_color(rgb(color))
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(76.0))
                                .text_color(rgb(MUTED))
                                .child(format!("{:?}", entry.module).to_lowercase()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(entry.message.clone()),
                        ),
                );
            }
            if shown == 0 {
                list = list.child(
                    div()
                        .px(px(12.0))
                        .py(px(16.0))
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(t("没有可显示的日志。")),
                );
            }
        }
        let footnote = self.logs.as_ref().map(|logs| {
            let mut text = format!(
                "{} {} {}",
                t("已省略"),
                logs.omitted,
                t("条未在白名单中的记录")
            );
            if logs.truncated {
                text.push_str(&format!(" · {}", t("只读取了最近的部分")));
            }
            text
        });
        vec![
            heading(t("日志级别")).into_any_element(),
            card(vec![row(t("记录级别"), level_hint, levels)]).into_any_element(),
            heading(t("近期日志")).into_any_element(),
            card(vec![row(t("日志文件"), Some(summary), actions)]).into_any_element(),
            div()
                .mt(px(10.0))
                .flex()
                .items_center()
                .justify_between()
                .child(filters)
                .into_any_element(),
            div()
                .mt(px(6.0))
                .rounded(px(8.0))
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgb(BORDER))
                .overflow_hidden()
                .child(list)
                .into_any_element(),
            div()
                .mt(px(8.0))
                .text_xs()
                .text_color(rgb(MUTED))
                .children(footnote)
                .into_any_element(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{size_label, time_label};

    #[test]
    fn labels_are_compact() {
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(2048), "2.0 KiB");
        assert_eq!(size_label(3 * 1024 * 1024), "3.0 MiB");
        assert_eq!(
            time_label("2026-11-21T10:05:07.123+08:00"),
            "11-21 10:05:07"
        );
        assert_eq!(time_label("bad"), "bad");
    }
}
