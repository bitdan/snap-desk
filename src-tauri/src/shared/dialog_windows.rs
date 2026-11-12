//! 将固定的同源空白页开窗请求转换为系统窗口，供 Vue Teleport 保留原有状态和命令归属。
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{
    webview::NewWindowResponse, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
    Wry,
};

const URL: &str = "about:blank#snap-desk-dialog";
const CLOSE_URL: &str = "about:blank?snap-desk-dialog-close";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// 子窗口标签 → 打开它的窗口；只读协议据此把识别/预览子窗口的请求归属到其所有者。
static OWNERS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// 沿子窗口链找到最初的应用窗口；非子窗口返回自身。层数受限，防止异常数据成环。
pub(crate) fn root_owner(label: &str) -> String {
    let mut current = label.to_owned();
    let Ok(owners) = OWNERS.lock() else {
        return current;
    };
    for _ in 0..8 {
        match owners.get(&current) {
            Some(owner) => current = owner.clone(),
            None => break,
        }
    }
    current
}

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
        let close_app = app.clone();
        let close_label = label.clone();
        let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
            .window_features(features)
            .title("Snap Desk")
            .decorations(true)
            .resizable(true)
            .min_inner_size(320.0, 160.0)
            .skip_taskbar(true)
            .on_navigation(move |url| {
                if url.as_str() == CLOSE_URL {
                    // JS 关闭可能仅释放 WebView 子窗口，必须销毁拥有标题栏的 Tauri 窗口。
                    // 固定信号只作用于捕获的当前窗口，不授予空白页 IPC 或跨窗口控制权限。
                    if let Some(window) = close_app.get_webview_window(&close_label) {
                        if let Err(error) = window.destroy() {
                            log::warn!(
                                "event=app.dialog_window_close_failed module=app error={error}"
                            );
                        }
                    }
                    return false;
                }
                url.as_str() == URL || url.as_str() == "about:blank"
            })
            .on_document_title_changed(|window, title| {
                let _ = window.set_title(&title);
            });
        let Ok(builder) = builder.parent(&parent) else {
            return NewWindowResponse::Deny;
        };
        match attach(builder, &app, &label).build() {
            Ok(window) => {
                if let Ok(mut owners) = OWNERS.lock() {
                    owners.insert(label.clone(), owner.clone());
                }
                window.on_window_event(move |event| {
                    if matches!(event, WindowEvent::Destroyed) {
                        if let Ok(mut owners) = OWNERS.lock() {
                            owners.remove(&label);
                        }
                    }
                });
                NewWindowResponse::Create { window }
            }
            Err(_) => {
                log::warn!("event=app.dialog_window_failed module=app code=DIALOG_WINDOW_FAILED");
                NewWindowResponse::Deny
            }
        }
    })
}
