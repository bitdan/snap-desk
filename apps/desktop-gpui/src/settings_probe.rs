//! 设置“快捷键”页的兼容测试（对照 Vue `ShortcutCompatibility.vue` 与 Tauri `settings/shortcut_probe.rs`）。
//!
//! 测试与平台探测在 core（`snap_core::settings::shortcut_probe`）。测试期间外壳的热键回调先交给
//! [`receive`]，返回 true 时不执行动作；托盘或菜单发起的动作调用 [`interrupt`] 结束测试。
//! macOS 输入法标识需在主线程读取，GPUI 版暂报告为未知。
use super::{col, row, SettingsView};
use gpui_kit::*;
use snap_core::settings::shortcut_probe::{self, ProbeReport, ProbeState};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    LazyLock,
};

static STATE: LazyLock<ProbeState> = LazyLock::new(ProbeState::default);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// 热键按下时调用（截图、录屏、滚动截图进行中不调用）；测试中返回 true，不执行功能动作。
pub fn receive(shortcut: String) -> bool {
    STATE.receive(shortcut)
}

/// 托盘或菜单开始截图、录屏等动作时结束测试。
pub fn interrupt() {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

/// 测试状态：进行中、结果或错误。
#[derive(Default)]
pub(super) struct Probe {
    running: bool,
    result: Option<Result<ProbeReport, String>>,
}

impl SettingsView {
    fn start_probe(&mut self, cx: &mut Context<Self>) {
        if self.probe.running {
            return;
        }
        if crate::capture::active(cx) || crate::recording::active(cx) || crate::scroll::active(cx) {
            self.probe.result = Some(Err("请先结束截图、滚动截图或录屏，再测试快捷键。".into()));
            cx.notify();
            return;
        }
        let registration = crate::shell::probe_registration(cx);
        INTERRUPTED.store(false, Ordering::Relaxed);
        self.probe = Probe {
            running: true,
            result: None,
        };
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    STATE.run(
                        registration,
                        &|| INTERRUPTED.load(Ordering::Relaxed),
                        &|| None,
                    )
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                // 测试期间用户可能修改了设置，报告用最新注册状态。
                let result = result.map(|mut report| {
                    report.set_registration(crate::shell::probe_registration(cx));
                    report
                });
                match &result {
                    Ok(report) => log::info!(
                        "event=gpui.shortcut_probe_finished received={} target={}",
                        report.received.len(),
                        report.target.is_some()
                    ),
                    Err(_) => log::info!("event=gpui.shortcut_probe_failed"),
                }
                view.probe = Probe {
                    running: false,
                    result: Some(result),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// “排查”卡片：开始检测按钮、进行中提示与排查建议。
    pub(super) fn probe_rows(
        &self,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let running = self.probe.running;
        let mut button = self.action_button(
            "shortcut-probe",
            t(if running {
                "检测中…"
            } else {
                "开始检测"
            }),
            !running,
        );
        if !running {
            button =
                button.on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.start_probe(cx)));
        }
        let hint = if running {
            t("切换到目标软件并按快捷键；测试不会执行截图或录屏。")
        } else {
            t("开始后五秒内切换到目标软件并按快捷键，收到按键即结束。")
        };
        let mut rows = vec![row(t("快捷键没有反应"), Some(hint), button)];
        match &self.probe.result {
            Some(Err(error)) => rows.push(
                div()
                    .px(px(14.0))
                    .py(px(10.0))
                    .text_sm()
                    .text_color(rgb(col::danger()))
                    .child(t(error)),
            ),
            Some(Ok(report)) => {
                let mut list = div()
                    .px(px(14.0))
                    .py(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0));
                for (template, args) in shortcut_probe::messages(report) {
                    let mut text = t(template);
                    for (index, value) in args.iter().enumerate() {
                        text = text.replace(&format!("{{{index}}}"), &t(value));
                    }
                    list = list.child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .text_xs()
                            .text_color(rgb(col::text()))
                            .child("•")
                            .child(div().flex_1().child(text)),
                    );
                }
                rows.push(list);
            }
            None => {}
        }
        rows
    }
}
