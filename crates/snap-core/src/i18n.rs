//! 原生菜单与系统对话框的显示文案。设置和 IPC 仍使用原始稳定值。
//! 与前端共用词典（`src/shared/i18n/messages.native.en.json`），避免同一操作在不同窗口中出现两套译法。
use crate::settings::Language;
use std::{collections::HashMap, sync::OnceLock};

fn catalog() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../../src/shared/i18n/messages.native.en.json"))
            .expect("native language catalog must be valid JSON")
    })
}

/// 中文原文即词典键；英文缺词时回退原文，未知的系统错误详情原样保留。
pub fn for_language<'a>(language: &Language, message: &'a str) -> &'a str {
    if *language == Language::English {
        catalog()
            .get(message)
            .map(String::as_str)
            .unwrap_or(message)
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_and_dialogs_use_the_saved_language_and_preserve_unknown_details() {
        assert_eq!(for_language(&Language::Chinese, "保存图片"), "保存图片");
        assert_eq!(for_language(&Language::English, "保存图片"), "Save image");
        assert_eq!(for_language(&Language::English, "退出"), "Quit");
        assert_eq!(
            for_language(&Language::English, "device error 0x1234"),
            "device error 0x1234"
        );
    }
}
