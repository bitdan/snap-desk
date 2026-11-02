//! 将固定的同源空白页开窗请求转换为系统窗口，供 Vue Teleport 保留原有状态和命令归属。
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    webview::NewWindowResponse, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry,
};

const URL: &str = "about:blank#snap-desk-dialog";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 不允许任意链接进入应用 WebView；空白子窗口也不授予原生命令或事件权限。
pub(crate) fn attach<'a, M: Manager<Wry>>(
    builder: WebviewWindowBuilder<'a, Wry, M>,
    app: &AppHandle,
    owner: &str,
) -> WebviewWindowBuilder<'a, Wry, M> {
    let app = app.clone();
    let owner = owner.to_owned();
    builder.on_new_window(move |url, features| {
        if url.as_str() != URL {
            return NewWindowResponse::Deny;
        }
        let Some(parent) = app.get_webview_window(&owner) else {
            return NewWindowResponse::Deny;
        };
        let label = format!("dialog-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
            .window_features(features)
            .title("Snap Desk")
            .decorations(true)
            .resizable(true)
            .min_inner_size(320.0, 160.0)
            .skip_taskbar(true)
            .on_navigation(|url| url.as_str() == URL || url.as_str() == "about:blank")
            .on_document_title_changed(|window, title| {
                let _ = window.set_title(&title);
            });
        let Ok(builder) = builder.parent(&parent) else {
            return NewWindowResponse::Deny;
        };
        match attach(builder, &app, &label).build() {
            Ok(window) => NewWindowResponse::Create { window },
            Err(_) => {
                log::warn!("event=app.dialog_window_failed module=app code=DIALOG_WINDOW_FAILED");
                NewWindowResponse::Deny
            }
        }
    })
}
