//! 原生菜单与系统对话框的显示文案。设置和 IPC 仍使用原始稳定值。
//! 与前端共用词典，避免同一操作在不同窗口中出现两套译法。

use crate::settings::{Language, SettingsState};
use tauri::{menu::MenuItem, tray::TrayIcon, AppHandle, Manager, Wry};

/// 词典与按语言取文案在 core，GPUI 版托盘共用同一份译法。
pub use snap_core::i18n::for_language;

/// 在创建原生对话框前读取语言快照；绝不持锁等待用户关闭对话框。
pub fn text(app: &AppHandle, message: &str) -> String {
    let language = app
        .try_state::<SettingsState>()
        .and_then(|state| {
            // 先取得拥有所有权的快照，让锁守卫在 State 句柄之前释放。
            let language = state
                .0
                .lock()
                .ok()
                .map(|s| s.value.appearance.language.clone());
            language
        })
        .unwrap_or_default();
    for_language(&language, message).to_owned()
}

/// 保留菜单句柄，在设置成功提交时同步现有托盘，而不是重新创建托盘或注册热键。
pub struct TrayLabels {
    pub items: Vec<(&'static str, MenuItem<Wry>)>,
    pub icon: TrayIcon<Wry>,
}

impl TrayLabels {
    pub fn update(&self, language: &Language) {
        for (message, item) in &self.items {
            if item.set_text(for_language(language, message)).is_err() {
                log::warn!("event=app.tray_language_failed module=app");
            }
        }
        if self
            .icon
            .set_tooltip(Some(for_language(language, "Snap Desk · 截图工具")))
            .is_err()
        {
            log::warn!("event=app.tray_language_failed module=app");
        }
    }
}
