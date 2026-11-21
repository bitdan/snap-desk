//! 设置窗口“关于与更新”页（对照 Vue `SupportPages.vue`/`UpdateSettings.vue` 与 Tauri `app/updater.rs`）。
//!
//! 版本号取产品版本（package.json，与 Tauri 版一致），用于和发布列表比较。检查更新复用 core
//! `updates::latest`（只认带固定更新清单的应用发布，测试版按设置或当前版本决定）。GPUI 预览版尚无安装包，
//! 发现新版本时只提供“打开发布页”，不在应用内下载安装；也不做后台定时复查。
use super::{card, heading, row, SettingsView, BORDER, MUTED, PAGE_BG, SURFACE};
use gpui_kit::*;
use semver::Version;

/// 产品版本（与 Tauri 版共用 package.json）。
pub(super) fn product_version() -> String {
    serde_json::from_str::<serde_json::Value>(include_str!("../../../package.json"))
        .ok()
        .and_then(|value| value.get("version")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "0.0.0".into())
}

/// 检查更新的结果。
pub(super) enum UpdateState {
    Checking,
    Latest,
    Available {
        version: String,
        notes: Option<String>,
    },
    Failed(String),
}

impl SettingsView {
    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update, Some(UpdateState::Checking)) {
            return;
        }
        let settings = crate::shell::settings(cx);
        self.update = Some(UpdateState::Checking);
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let current = Version::parse(&product_version())
                        .map_err(|_| "当前版本号无效。".to_string())?;
                    // 当前运行的本身是测试版时始终纳入测试版，否则开关关闭时收不到后续测试版。
                    let preview = settings.receive_prerelease_updates || !current.pre.is_empty();
                    snap_core::updates::latest(
                        &current,
                        preview,
                        Some(&settings.update_network.proxy),
                    )
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.update = Some(match result {
                    Ok(Some(found)) => {
                        log::info!("event=gpui.update_available");
                        UpdateState::Available {
                            version: found.version.to_string(),
                            notes: found.notes,
                        }
                    }
                    Ok(None) => UpdateState::Latest,
                    Err(error) => {
                        log::warn!("event=gpui.update_check_failed message={error}");
                        UpdateState::Failed(error)
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn open_release(&mut self, version: Option<String>, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { snap_core::updates::open_release_page(version.as_deref()) })
                .await;
            if let Err(error) = result {
                let _ = this.update(cx, |view, cx| {
                    view.error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn about_page(
        &self,
        s: &snap_core::settings::Settings,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
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
        let (status, available) = match &self.update {
            None => (t("尚未检查。"), None),
            Some(UpdateState::Checking) => (t("正在检查更新…"), None),
            Some(UpdateState::Latest) => (t("已是最新版本。"), None),
            Some(UpdateState::Available { version, .. }) => (
                format!("{} {version}", t("发现新版本")),
                Some(version.clone()),
            ),
            Some(UpdateState::Failed(error)) => (
                snap_core::i18n::for_language(&s.appearance.language, error).to_owned(),
                None,
            ),
        };
        let notes = match &self.update {
            Some(UpdateState::Available {
                notes: Some(notes), ..
            }) if !notes.trim().is_empty() => Some(
                div()
                    .mt(px(8.0))
                    .p(px(10.0))
                    .rounded(px(8.0))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .max_h(px(160.0))
                    .overflow_hidden()
                    .child(notes.chars().take(1200).collect::<String>()),
            ),
            _ => None,
        };
        let check = button("update-check", t("检查更新"))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.check_updates(cx)));
        let release =
            button(
                "update-release",
                t(if available.is_some() {
                    "打开发布页下载"
                } else {
                    "打开发布页"
                }),
            )
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.open_release(available.clone(), cx)
            }));
        vec![
            heading(t("关于")).into_any_element(),
            card(vec![
                row(
                    "Snap Desk".into(),
                    Some(format!(
                        "{} {} · {}",
                        t("版本"),
                        product_version(),
                        t("GPUI 预览版")
                    )),
                    button("about-home", t("项目主页")).on_click(cx.listener(
                        |view, _: &ClickEvent, _, cx| {
                            if let Err(error) = snap_core::platform::open_project_homepage() {
                                view.error = Some(error);
                                cx.notify();
                            }
                        },
                    )),
                ),
                row(t("许可证"), Some("GPL-3.0-only".into()), div()),
            ])
            .into_any_element(),
            heading(t("更新")).into_any_element(),
            card(vec![
                row(
                    t("接收测试版更新"),
                    Some(t("包括 alpha、beta 与 rc 版本；当前为测试版时始终接收。")),
                    self.toggle(
                        "update-preview",
                        s.receive_prerelease_updates,
                        cx,
                        |s, v| s.receive_prerelease_updates = v,
                    ),
                ),
                row(
                    t("更新代理"),
                    Some(t("留空时使用系统代理；支持 http、https 与 socks5。")),
                    self.text_input(&self.update_proxy, 240.0),
                ),
                row(
                    t("检查更新"),
                    Some(status),
                    div().flex().gap(px(6.0)).child(check).child(release),
                ),
            ])
            .into_any_element(),
            div()
                .mt(px(10.0))
                .text_xs()
                .text_color(rgb(MUTED))
                .child(t("GPUI 预览版暂不支持应用内安装更新，请从发布页下载。"))
                .into_any_element(),
            div().children(notes).into_any_element(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::product_version;

    #[test]
    fn product_version_is_semver() {
        assert!(semver::Version::parse(&product_version()).is_ok());
    }
}
