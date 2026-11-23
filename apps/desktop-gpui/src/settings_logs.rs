//! 设置窗口“日志”页。
//!
//! 读取 GPUI 版日志文件（`snap-desk-gpui.log` 与轮转的 `.log.old`）的有限尾部，经 core 白名单清洗后显示：
//! GPUI 外壳事件只显示事件名，core 事件按审计字段显示，其余行计入“已省略”。可修改日志级别（立即生效，
//! 与旧版相同的本机偏好）、按级别筛选、刷新、打开目录、清理历史日志（确认后只删归档）、导出本地诊断包。
use super::{ButtonKind, SettingsView, button, card, col, heading, row, switch};
use gpui_kit::{
    component::{input::Input, Icon, Sizable},
    *,
};
use snap_core::diagnostics::{
    logs::{self, Entry, Level, Module, Query},
    package,
};
use std::path::PathBuf;


/// 一次读取的结果。
pub(super) struct LogsView {
    archives: usize,
    entries: Vec<Entry>,
    omitted: usize,
    truncated: bool,
    scanned_bytes: u64,
}

fn directory() -> Result<PathBuf, String> {
    snap_core::paths::log_dir().ok_or_else(|| "无法定位应用日志目录。".to_owned())
}

fn snapshot(level: Option<Level>, module: Option<Module>) -> Result<(Vec<logs::LogFile>, logs::Recent), String> {
    log::logger().flush();
    let root = directory()?;
    let files = logs::files(&root, crate::logging::BASE)?;
    let recent = logs::recent(&files, &Query { limit: 500, level, module }, &[])?;
    Ok((files, recent))
}

fn read(level: Option<Level>, module: Option<Module>) -> Result<LogsView, String> {
    let (files, recent) = snapshot(level, module)?;
    Ok(LogsView {
        archives: files.iter().filter(|f| !f.active).count(),
        omitted: recent.omitted_lines,
        truncated: recent.truncated,
        scanned_bytes: recent.scanned_bytes,
        entries: recent.entries,
    })
}

/// 模块筛选。
const MODULES: [(Module, &str); 9] = [
    (Module::App, "应用"),
    (Module::Capture, "截图"),
    (Module::Scroll, "滚动截图"),
    (Module::Recording, "录屏"),
    (Module::Ocr, "OCR"),
    (Module::Translation, "翻译"),
    (Module::Clipboard, "剪贴板"),
    (Module::Frontend, "前端"),
    (Module::Diagnostics, "诊断"),
];

impl SettingsView {
    /// 后台读取日志与统计。
    pub(super) fn load_logs(&mut self, cx: &mut Context<Self>) {
        if self.logs_loading {
            return;
        }
        self.logs_loading = true;
        let (level, module) = (self.log_filter, self.log_module);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async move { read(level, module) }).await;
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

    /// 自动刷新：开启后每 2 秒重读一次，离开日志页或关闭开关即停止。
    fn set_log_live(&mut self, live: bool, cx: &mut Context<Self>) {
        self.log_live = live;
        cx.notify();
        if !live {
            return;
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let keep = this
                .update(cx, |view, cx| {
                    if !view.log_live || view.page != super::Page::Logs {
                        view.log_live = false;
                        return false;
                    }
                    view.load_logs(cx);
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        })
        .detach();
    }

    /// 复制日志目录路径。
    fn copy_log_path(&mut self, cx: &mut Context<Self>) {
        match directory() {
            Ok(path) => {
                cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
                self.error = None;
                self.notice = Some("目录路径已复制。".into());
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
                    let (_, recent) = snapshot(None, None)?;
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

    pub(super) fn logs_page(&self, t: &dyn Fn(&str) -> String, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // 布局：记录（状态与级别下拉）、最近日志（筛选、搜索、列表、统计）、文件。
        let current = crate::logging::level();
        let ready = directory().is_ok();
        let status = if ready {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_xs()
                .text_color(rgb(col::success()))
                .child(Icon::default().data(super::icons::CHECK).size(px(14.0)).text_color(rgb(col::success())))
                .child(t("已就绪"))
        } else {
            div().text_xs().text_color(rgb(col::muted())).child(t("日志不可用"))
        };
        let level_select = self.select_with(
            "log-level",
            current,
            [Level::Debug, Level::Info, Level::Warn, Level::Error]
                .into_iter()
                .map(|l| (l, format!("{l:?}").to_uppercase()))
                .collect(),
            cx,
            |view, value, cx| view.set_log_level(value, cx),
        );
        let level_hint = if crate::logging::fixed_by_environment() {
            Some(t("当前由环境变量 SNAP_DESK_LOG 决定，修改将在下次启动时生效。"))
        } else {
            (current == Level::Debug).then(|| t("会一直记录调试信息，问题解决后建议改回 INFO"))
        };

        // 最近日志：标题右侧刷新与自动刷新。
        let live = self.log_live;
        let busy = self.logs_loading;
        let refresh = div()
            .id("logs-refresh")
            .text_xs()
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(col::accent_fg()))
            .cursor_pointer()
            .child(t(if busy { "处理中…" } else { "刷新" }))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.load_logs(cx)));
        let actions = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(refresh)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_xs()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(rgb(col::muted()))
                    .child(t("自动刷新"))
                    .child(switch("logs-live", live, true).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_log_live(!live, cx)))),
            );
        let level_filter = self.select_with(
            "log-filter-level",
            self.log_filter,
            std::iter::once((None, t("全部级别")))
                .chain([Level::Debug, Level::Info, Level::Warn, Level::Error].into_iter().map(|l| (Some(l), format!("{l:?}").to_uppercase())))
                .collect(),
            cx,
            |view, value, cx| {
                view.log_filter = value;
                view.load_logs(cx);
                cx.notify();
            },
        );
        let module_filter = self.select_with(
            "log-filter-module",
            self.log_module,
            std::iter::once((None, t("全部模块")))
                .chain(MODULES.iter().map(|(m, label)| (Some(*m), t(label))))
                .collect(),
            cx,
            |view, value, cx| {
                view.log_module = value;
                view.load_logs(cx);
                cx.notify();
            },
        );
        let filters = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(rgb(col::sep()))
            .child(level_filter)
            .child(module_filter)
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.0))
                    .min_h(px(28.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(col::line()))
                    .bg(rgb(col::surface()))
                    .text_xs()
                    .child(Input::new(&self.log_search).appearance(false).small()),
            );
        let query = self.log_search.read(cx).value().to_lowercase();
        let mut list = div()
            .id("logs-list")
            .flex()
            .flex_col()
            .min_h(px(160.0))
            .max_h(px(360.0))
            .overflow_y_scroll()
            .text_xs()
            .font_family("Consolas");
        let mut shown = 0usize;
        if let Some(logs) = &self.logs {
            for entry in logs.entries.iter().filter(|e| query.is_empty() || e.message.to_lowercase().contains(&query)) {
                let level_color = match entry.level {
                    Level::Error => col::danger(),
                    Level::Warn => if crate::theme::dark() { 0xd29922 } else { 0x9a6700 },
                    Level::Debug => col::muted(),
                    _ => col::text(),
                };
                let first = shown == 0;
                shown += 1;
                list = list.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .px(px(12.0))
                        .py(px(4.0))
                        .when_first(first)
                        .child(div().w(px(58.0)).flex_none().text_color(rgb(col::muted())).child(entry.timestamp.get(11..19).unwrap_or(&entry.timestamp).to_owned()))
                        .child(div().w(px(46.0)).flex_none().font_weight(FontWeight::SEMIBOLD).text_color(rgb(level_color)).child(format!("{:?}", entry.level).to_uppercase()))
                        .child(div().w(px(84.0)).flex_none().text_color(rgb(col::muted())).child(format!("{:?}", entry.module).to_lowercase()))
                        .child(div().flex_1().min_w_0().truncate().child(entry.message.clone())),
                );
            }
        }
        if shown == 0 {
            let empty = if busy {
                t("正在读取…")
            } else if self.logs.is_some() {
                t("当前条件下没有日志。")
            } else {
                t("尚未读取日志。")
            };
            list = list.child(div().p(px(12.0)).font_family(".SystemUIFont").text_color(rgb(col::muted())).child(empty));
        }
        let footer = self.logs.as_ref().map(|logs| {
            let mut text = format!(
                "{} {shown} / {} {} {} {} {} {}",
                t("显示"),
                logs.entries.len(),
                t("条 · 扫描"),
                logs.scanned_bytes / 1024,
                t("KB · 省略"),
                logs.omitted,
                t("条非白名单记录"),
            );
            if logs.truncated {
                text.push_str(&t(" · 已达读取或条数上限，非完整历史"));
            }
            div().px(px(12.0)).py(px(6.0)).border_t_1().border_color(rgb(col::sep())).text_size(px(11.0)).text_color(rgb(col::muted())).child(text)
        });
        let viewer = div()
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::surface()))
            .overflow_hidden()
            .child(filters)
            .child(list)
            .children(footer);
        let archives = self.logs.as_ref().is_some_and(|l| l.archives > 0);
        vec![
            heading(t("记录")).into_any_element(),
            card(vec![row(t("记录级别"), level_hint, div().flex().items_center().gap(px(12.0)).child(status).child(level_select))]).into_any_element(),
            heading(t("最近日志")).child(actions).into_any_element(),
            viewer.into_any_element(),
            heading(t("文件")).into_any_element(),
            card(vec![
                row(
                    t("日志目录"),
                    None,
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(self.open_button("logs-open", t("打开"), snap_core::paths::log_dir(), cx))
                        .child(button("logs-copy", t("复制路径"), ButtonKind::Outline, ready).on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.copy_log_path(cx)))),
                ),
                row(
                    t("诊断包"),
                    None,
                    button("logs-export", t("导出…"), ButtonKind::Outline, ready)
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.export_diagnostics(cx))),
                ),
                row(t("历史归档"), None, {
                    let clear = button("logs-clear", t("清理"), ButtonKind::Outline, archives);
                    if archives { clear.on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.clear_archives(cx))) } else { clear }
                }),
            ])
            .into_any_element(),
        ]
    }
}

/// 列表首行不画分隔线。
trait FirstRow: Styled + Sized {
    fn when_first(self, first: bool) -> Self {
        if first { self } else { self.border_t_1().border_color(rgb(col::sep())) }
    }
}
impl<T: Styled> FirstRow for T {}
