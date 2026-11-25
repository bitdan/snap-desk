//! 设置窗口“关于与帮助”页。
//!
//! 应用更新：测试版开关、检查更新间隔、下载镜像、更新代理、当前版本与“检查更新 / 有更新”；发现新版本后
//! 在下方展开更新面板（下载进度、退出并安装、下载失败时在浏览器中下载）。检查、下载、校验与安装在
//! `crate::updater`，状态是全局的，离开页面后下载继续。
use super::{SettingsView, card, col, heading, row};
use crate::updater::{self, Phase};
use gpui_kit::*;
use snap_core::settings::UPDATE_CHECK_MINUTES;

/// 当前平台录屏的最低系统要求，与各平台采集后端的检查一致。
#[cfg(windows)]
const RECORDING_REQUIREMENT: &str = "Windows 10 2004 或更新版本";
#[cfg(target_os = "macos")]
const RECORDING_REQUIREMENT: &str = "macOS 13.3 或更新版本";
#[cfg(target_os = "linux")]
const RECORDING_REQUIREMENT: &str = "X11，或带屏幕共享门户与 PipeWire 的 Wayland";
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
const RECORDING_REQUIREMENT: &str = "当前平台暂不支持录屏";

/// 带红点的描边按钮（“有更新”）。
pub(super) fn dot_button(id: &'static str, label: String, enabled: bool) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(28.0))
        .px(px(12.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .text_size(px(13.0))
        .border_1()
        .border_color(rgb(col::line()))
        .bg(rgb(col::surface()))
        .text_color(rgb(col::text()))
        .child(div().size(px(6.0)).flex_none().rounded_full().bg(rgb(0xdc2626)))
        .child(label);
    if enabled {
        let hover = col::hover();
        button.cursor_pointer().hover(move |s| s.bg(rgb(hover)))
    } else {
        button.opacity(0.45)
    }
}

impl SettingsView {
    fn open_release(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = updater::open_release_page(cx) {
            self.error = Some(error);
            cx.notify();
        }
    }

    /// 发现新版本后的更新面板。
    fn update_panel(&self, view: &updater::View, t: &dyn Fn(&str) -> String, language: &snap_core::settings::Language, cx: &mut Context<Self>) -> Div {
        let status = t(match view.phase {
            Phase::Latest => "当前渠道暂无更新",
            Phase::Ready => "更新已下载并通过校验",
            Phase::Downloading => "正在下载更新…",
            Phase::Installing => "正在退出并安装…",
            _ => "发现新版本",
        });
        let progress = view.total.filter(|t| *t > 0).map(|total| (view.downloaded.saturating_mul(100) / total).min(100));
        let operating = view.operating();
        let mut line = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .children(matches!(view.phase, Phase::Ready | Phase::Latest).then(|| {
                        component::Icon::default().data(super::icons::CHECK).size(px(14.0)).text_color(rgb(col::success()))
                    }))
                    .child(match &view.version {
                        Some(version) => format!("{status} · v{version}"),
                        None => status,
                    }),
            );
        if view.phase == Phase::Downloading {
            line = line.child(div().text_color(rgb(col::muted())).child(match progress {
                Some(p) => format!("{p}%"),
                None => format!("{:.1} MiB", view.downloaded as f64 / 1_048_576.0),
            }));
        }
        if matches!(view.phase, Phase::Available | Phase::Error) {
            line = line.child(
                super::button("update-download", t("下载更新"), super::ButtonKind::Primary, !operating)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| updater::download(cx))),
            );
        }
        if view.phase == Phase::Ready {
            line = line.child(
                super::button("update-install", t("退出并安装"), super::ButtonKind::Primary, !operating)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| updater::install(cx))),
            );
        }
        let mut panel = div()
            .mt(px(8.0))
            .p(px(16.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(col::border()))
            .bg(rgb(col::surface()))
            .text_size(px(13.0))
            .child(line);
        if view.phase == Phase::Downloading {
            let width = progress.unwrap_or(0) as f32;
            panel = panel.child(
                div()
                    .mt(px(8.0))
                    .h(px(4.0))
                    .w_full()
                    .rounded_full()
                    .bg(rgb(col::line()))
                    .child(div().h_full().w(relative(width / 100.0)).rounded_full().bg(rgb(col::accent()))),
            );
        }
        if view.phase == Phase::Ready {
            panel = panel.child(div().mt(px(8.0)).text_xs().text_color(rgb(col::muted())).child(t("安装会退出应用，请先保存工作并结束录屏。")));
        }
        if let Some(error) = &view.error {
            panel = panel.child(
                div().mt(px(8.0)).text_xs().text_color(rgb(col::danger())).child(snap_core::i18n::for_language(language, error).to_owned()),
            );
        }
        // 自动下载失败（多为无法直连 GitHub）时提供手动下载入口；发布页地址由原生端决定。
        if view.phase == Phase::Error && view.version.is_some() {
            panel = panel.child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(8.0))
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .child(
                        super::button("update-browser", t("在浏览器中下载"), super::ButtonKind::Outline, true)
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.open_release(cx))),
                    )

            );
        }
        if let Some(notes) = view.notes.as_ref().filter(|n| !n.trim().is_empty()) {
            panel = panel.child(
                div()
                    .mt(px(10.0))
                    .pt(px(10.0))
                    .border_t_1()
                    .border_color(rgb(col::sep()))
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .max_h(px(160.0))
                    .overflow_hidden()
                    .child(notes.chars().take(1200).collect::<String>()),
            );
        }
        panel
    }

    pub(super) fn about_page(
        &self,
        s: &snap_core::settings::Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let language = s.appearance.language.clone();
        let outline = |id: &'static str, label: String| super::button(id, label, super::ButtonKind::Outline, true);
        let view = updater::view(cx);
        let enabled = view.supported && view.configured;
        let has_update = view.has_update();
        let prerelease_build = view.current.contains('-');
        // 版本行状态：只在检查中、已是最新、失败时显示。
        let status = (!has_update && matches!(view.phase, Phase::Checking | Phase::Latest | Phase::Error)).then(|| {
            let latest = view.phase == Phase::Latest;
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_xs()
                .text_color(rgb(if latest { col::success() } else { col::muted() }))
                .children(latest.then(|| component::Icon::default().data(super::icons::CHECK).size(px(14.0)).text_color(rgb(col::success()))))
                .child(t(match view.phase {
                    Phase::Checking => "正在检查更新…",
                    Phase::Latest => "当前渠道暂无更新",
                    _ => "更新失败，可重试",
                }))
        });
        // 不能在线更新时（开发构建、未配置公钥、不支持的安装方式），版本行的按钮换成“打开发布页”，原因写在版本号下方。
        let unavailable = if view.kind == "unsupported" {
            Some(t("当前安装方式不支持在线更新，请从发布页下载新版本。"))
        } else if !view.supported {
            Some(t("在线更新仅支持已发布平台的正式构建。"))
        } else if !view.configured {
            Some(t("此版本未配置更新签名公钥。"))
        } else {
            None
        };
        let action = if !enabled {
            // 不能在线更新：检查更新保留（禁用），旁边给“打开发布页”。
            div()
                .flex()
                .gap(px(6.0))
                .child(outline("update-release", t("打开发布页")).on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.open_release(cx))))
                .child(super::button("update-check", t("检查更新"), super::ButtonKind::Outline, false))
                .into_any_element()
        } else if has_update {
            dot_button("update-check", t("有更新"), true).into_any_element()
        } else {
            let mut check = super::button("update-check", t("检查更新"), super::ButtonKind::Outline, enabled && !view.operating());
            if enabled && !view.operating() {
                check = check.on_click(cx.listener(|_, _: &ClickEvent, _, cx| updater::check(cx, false, |_| {})));
            }
            check.into_any_element()
        };
        let interval = self.segmented(
            "update-interval",
            s.update_check_minutes,
            UPDATE_CHECK_MINUTES
                .iter()
                .map(|m| {
                    let label = match m {
                        30 => "30 分钟",
                        60 => "1 小时",
                        360 => "6 小时",
                        _ => "24 小时",
                    };
                    (*m, t(label))
                })
                .collect(),
            cx,
            |s, v| s.update_check_minutes = v,
        );
        let preview_toggle = if prerelease_build {
            super::switch("update-preview", true, false).opacity(0.45).into_any_element()
        } else {
            self.toggle("update-preview", s.receive_prerelease_updates, cx, |s, v| s.receive_prerelease_updates = v)
                .into_any_element()
        };
        let goto = |id: &'static str, label: String, page: super::Page, cx: &mut Context<Self>| {
            outline(id, label).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.page = page;
                if page == super::Page::Logs {
                    view.load_logs(cx);
                }
                if page == super::Page::Recognition {
                    view.load_recognition(cx);
                    view.load_local(cx);
                    view.load_resources(cx);
                }
                cx.notify();
            }))
        };
        let mut elements = vec![
            super::hero(super::icons::SCAN, "Snap Desk".into(), Some(format!("v{}", view.current)), None, Vec::new()).into_any_element(),
            heading(t("应用更新")).into_any_element(),
            card(vec![
                row(
                    t("测试版更新"),
                    None,
                    preview_toggle,
                ),
                row(t("检查更新间隔"), None, interval),
                row(
                    t("下载镜像"),
                    None,
                    self.text_input(&self.update_mirror, 240.0),
                ),
                row(
                    t("更新代理"),
                    None,
                    self.text_input(&self.update_proxy, 240.0),
                ),
                row(
                    format!("v{}", view.current),
                    unavailable,
                    div().flex().items_center().gap(px(12.0)).children(status).child(action),
                ),
            ])
            .into_any_element(),
        ];
        if has_update || matches!(view.phase, Phase::Downloading | Phase::Ready | Phase::Installing) {
            elements.push(self.update_panel(&view, t, &language, cx).into_any_element());
        } else if let Some(error) = view.error.as_ref().filter(|_| view.phase == Phase::Error) {
            elements.push(super::note(snap_core::i18n::for_language(&language, error).to_owned()).text_color(rgb(col::danger())).into_any_element());
        }
        elements.extend([
            heading(t("帮助")).into_any_element(),
            card(vec![
                row(t("快捷键没有响应"), None, goto("help-shortcuts", t("检查快捷键"), super::Page::Shortcuts, cx)),
                row(t("识别或翻译不可用"), None, goto("help-recognition", t("文字识别"), super::Page::Recognition, cx)),
                row(t("遇到其他问题"), None, goto("help-logs", t("查看日志"), super::Page::Logs, cx)),
            ])
            .into_any_element(),
            heading(t("应用信息")).into_any_element(),
            card(vec![
                row(t("开源许可"), None, div().text_color(rgb(col::muted())).child("GPL-3.0-only")),
                row(t("录屏系统要求"), None, div().text_color(rgb(col::muted())).child(t(RECORDING_REQUIREMENT))),
                row(
                    t("项目主页"),
                    Some("github.com/bitdan/snap-desk".into()),
                    outline("about-home", "GitHub ↗".into()).on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        if let Err(error) = snap_core::platform::open_project_homepage() {
                            view.error = Some(error);
                            cx.notify();
                        }
                    })),
                ),
            ])
            .into_any_element(),
        ]);
        elements
    }
}
