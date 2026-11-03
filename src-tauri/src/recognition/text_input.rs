//! 文字翻译入口：输入、划词及截图识别共用独立窗口，原文只由该窗口领取。
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

/// 挂起文本由翻译窗口一次性领取，兼容冷启动时事件早于监听器注册。
#[tauri::command]
pub fn read_translation_input(
    app: AppHandle,
    window: WebviewWindow,
    kind: TextInputKind,
) -> Result<Option<String>, String> {
    crate::capture::require_window(&window, "translation")?;
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

/// 先确保窗口存在，再写入原文；普通打开保留当前内容，不自动读取剪贴板。
pub fn publish(app: &AppHandle, text: Option<String>) -> Result<(), String> {
    let window = crate::shared::auxiliary_windows::ensure(app, "translation")?;
    if let Some(text) = text {
        *app.state::<TextInputState>()
        .0
        .lock()
        .map_err(|_| "翻译输入正在使用。")? = Some(text);
    }
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    app.emit_to("translation", "translation-open", ())
        .map_err(|_| "无法打开翻译窗口。".into())
}

/// 创建 WebView 放到后台，避免命令运行在 UI 线程时等待窗口创建而阻塞。
#[tauri::command]
pub async fn open_translation(app: AppHandle, window: WebviewWindow, text: Option<String>) -> Result<(), String> {
    if !matches!(window.label(), "main" | "capture" | "translation") {
        crate::pins::require_pin(&app, &window)?;
    }
    let text = text.map(validate).transpose()?;
    tauri::async_runtime::spawn_blocking(move || publish(&app, text))
        .await.map_err(|e| e.to_string())?
}

/// Esc 与系统关闭按钮都销毁翻译窗口，原生窗口事件取消它持有的任务。
#[tauri::command]
pub fn close_translation(window: WebviewWindow) -> Result<(), String> {
    crate::capture::require_window(&window, "translation")?;
    window.close().map_err(|e| e.to_string())
}

/// 无标题栏翻译浮窗仅开放拖动与置顶，不接受任意窗口标签或属性。
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslationWindowAction { Drag, Pin, Unpin }

#[tauri::command]
pub fn control_translation_window(window: WebviewWindow, action: TranslationWindowAction) -> Result<(), String> {
    crate::capture::require_window(&window, "translation")?;
    match action {
        TranslationWindowAction::Drag => window.start_dragging(),
        TranslationWindowAction::Pin => window.set_always_on_top(true),
        TranslationWindowAction::Unpin => window.set_always_on_top(false),
    }.map_err(|e| e.to_string())
}

/// 截图意图随会话帧传递，框选完成即 OCR；取消截图后恢复原翻译内容。
#[tauri::command]
pub async fn start_translation_capture(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    crate::capture::require_window(&window, "translation")?;
    let result = crate::capture::begin_translation(app.clone()).await;
    if result.is_err() { let _ = publish(&app, None); }
    result
}

/// 有选区时带入原文，否则直接打开翻译；一次只执行一个复制任务，避免长按重复触发。
pub fn open(app: &AppHandle) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let target = super::selected_text::foreground();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let text = target
            .and_then(|target| super::selected_text::copy(&app, target))
            .and_then(validate)
            .ok();
        // 未获得有效选区也允许手动输入；不以旧剪贴板内容作为自动翻译原文。
        if let Err(error) = publish(&app, text) {
            crate::capture::report_error(&app, error);
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
