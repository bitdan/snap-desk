//! 文字翻译入口：快捷键先复制当前选区，再唤起主窗口自动翻译。
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Default)]
pub struct TextInputState(Mutex<Option<String>>);

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextInputKind {
    Clipboard,
    Pending,
}

/// 挂起文本由主窗口一次性领取；通知只负责导航，避免异步页面挂载丢失原文。
#[tauri::command]
pub fn read_translation_input(
    app: AppHandle,
    window: WebviewWindow,
    kind: TextInputKind,
) -> Result<Option<String>, String> {
    crate::capture::require_window(&window, "main")?;
    match kind {
        TextInputKind::Clipboard => clipboard(&app).map(Some),
        TextInputKind::Pending => app
            .state::<TextInputState>()
            .0
            .lock()
            .map_err(|_| "翻译输入正在使用。".into())
            .map(|mut value| value.take()),
    }
}

fn validate(text: String) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("没有可翻译的文字，请先选中或复制文字。".into());
    }
    if text.len() > 32_000 {
        return Err("翻译文字超过 32 KB，请减少文字后重试。".into());
    }
    Ok(text)
}

fn clipboard(app: &AppHandle) -> Result<String, String> {
    validate(
        app.clipboard()
            .read_text()
            .map_err(|_| "剪贴板中没有可读取的文字。")?,
    )
}

/// 按翻译快捷键后发布原文，由主窗口读取配置并自动翻译。
fn publish(app: &AppHandle, text: String) -> Result<(), String> {
    *app.state::<TextInputState>()
        .0
        .lock()
        .map_err(|_| "翻译输入正在使用。")? = Some(text);
    crate::capture::show_main(app);
    app.emit_to("main", "translation-open", ())
        .map_err(|_| "无法打开翻译页面。".into())
}

/// 一次只执行一个选区复制任务，防止长按快捷键重复复制和打开翻译。
pub fn open(app: &AppHandle) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let target = super::selected_text::foreground();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = target
            .and_then(|target| super::selected_text::copy(&app, target))
            .and_then(validate);
        match result {
            Ok(text) => {
                if let Err(error) = publish(&app, text) {
                    crate::capture::report_error(&app, error);
                }
            }
            Err(error) => crate::capture::report_error(&app, error),
        }
        RUNNING.store(false, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    use super::validate;
    #[test]
    fn translation_input_is_bounded_in_utf8_bytes_and_preserves_whitespace() {
        assert!(validate(" \n\t".into()).is_err());
        assert!(validate("中".repeat(11_000)).is_err());
        assert!(validate("a".repeat(32_000)).is_ok());
        assert_eq!(
            validate("  hello\nworld  ".into()).unwrap(),
            "  hello\nworld  "
        );
    }
}
