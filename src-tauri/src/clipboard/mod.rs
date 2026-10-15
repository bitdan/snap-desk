//! 剪贴板历史的受限 Tauri 入口；每个命令只允许 main 窗口访问。
mod blob_store;
pub mod export;
pub mod model;
mod platform;
mod reader;
mod repository;
mod service;
use base64::{engine::general_purpose::STANDARD, Engine};
use model::*;
use service::Content;
pub use service::Service;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// 重任务统一移出 UI 线程；此处只验证窗口，不对任意文件路径提供入口。
async fn work<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    f: impl FnOnce(&AppHandle, &Service) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || f(&app, &app.state::<Service>()))
        .await
        .map_err(|_| "历史任务执行失败。".to_string())?
}
pub fn initialize(app: &AppHandle) {
    app.manage(Service::new(app));
    let settings = app
        .state::<crate::settings::SettingsState>()
        .0
        .lock()
        .map(|s| s.value.clipboard.clone())
        .unwrap_or_default();
    if let Err(e) = app.state::<Service>().configure(app, settings) {
        app.state::<Service>().report(app, e);
    }
    if let Err(e) = app.state::<Service>().maintain(app) {
        app.state::<Service>().report(app, e);
    }
}
#[tauri::command]
pub async fn clipboard_status(app: AppHandle, window: WebviewWindow) -> Result<Status, String> {
    work(app, window, |_, s| Ok(s.status())).await
}
#[tauri::command]
pub async fn clipboard_list(
    app: AppHandle,
    window: WebviewWindow,
    query: Query,
) -> Result<Page, String> {
    work(app, window, move |_, s| {
        s.data()?.repo.list(
            &query,
            s.revision.load(std::sync::atomic::Ordering::Acquire),
        )
    })
    .await
}
#[tauri::command]
pub async fn clipboard_get_text(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<String, String> {
    work(app, window, move |_, s| {
        s.data()?
            .repo
            .get(id)?
            .text
            .ok_or("此条目不是文本。".into())
    })
    .await
}
#[tauri::command]
pub async fn clipboard_get_thumbnail(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<String, String> {
    work(app, window, move |_, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let item = d.repo.get(id)?;
        let path = d.blobs.path(&item.thumb.ok_or("此条目不是图片。")?)?;
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "缩略图不可用。")?
            .take(256 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "缩略图读取失败。")?;
        if bytes.len() > 256 * 1024 {
            return Err("缩略图大小异常。".into());
        }
        Ok(format!("data:image/webp;base64,{}", STANDARD.encode(bytes)))
    })
    .await
}
#[tauri::command]
pub async fn clipboard_restore(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    work(app, window, move |app, s| {
        let content = {
            let d = s.data()?;
            let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
            d.content(id)?
        };
        let _gate = s.write_gate.lock().map_err(|_| "剪贴板写入状态不可用。")?;
        match &content {
            Content::Text(t) => app
                .clipboard()
                .write_text(t.clone())
                .map_err(|_| "复制文本失败，请重试。".to_string())?,
            Content::Image(i) => crate::image_io::copy(app, i)?,
        }
        *s.restored.lock().map_err(|_| "恢复状态不可用。")? =
            Some((platform::sequence(), content.hash()));
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn clipboard_pin_item(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    work(app, window, move |app, s| {
        let content = {
            let d = s.data()?;
            let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
            d.content(id)?
        };
        match content {
            Content::Image(i) => crate::pins::create_visible(app, i, vec![]),
            Content::Text(_) => Err("请选择图片贴图。".into()),
        }
    })
    .await
}
#[tauri::command]
pub async fn clipboard_set_favorite(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
    favorite: bool,
) -> Result<(), String> {
    work(app, window, move |app, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        d.repo.favorite(id, favorite)?;
        s.changed(app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn clipboard_delete(
    app: AppHandle,
    window: WebviewWindow,
    ids: Vec<i64>,
) -> Result<(), String> {
    validate_ids(&ids)?;
    work(app, window, move |app, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        d.repo.delete(&ids)?;
        let cleanup = d.blobs.repair(&d.repo);
        s.changed(app);
        cleanup
    })
    .await
}
#[tauri::command]
pub async fn clipboard_clear(
    app: AppHandle,
    window: WebviewWindow,
    include_favorites: bool,
) -> Result<(), String> {
    work(app, window, move |app, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        d.repo.clear(include_favorites)?;
        let cleanup = d.blobs.repair(&d.repo);
        s.changed(app);
        cleanup
    })
    .await
}
/// 历史设置独立合并，防止快捷键页面的旧草稿重新打开已暂停的监听。
#[tauri::command]
pub async fn clipboard_configure(
    app: AppHandle,
    window: WebviewWindow,
    settings: ClipboardSettings,
) -> Result<Status, String> {
    settings.validate()?;
    work(app, window, move |app, s| {
        let state = app.state::<crate::settings::SettingsState>();
        let mut current = state.0.lock().map_err(|_| "设置不可用。")?;
        let old = current.value.clipboard.clone();
        if let Err(error) = s.configure(app, settings.clone()) {
            let _ = s.configure(app, old.clone());
            s.report(app, error.clone());
            let _ = app.emit_to("main", "clipboard-state", s.status());
            return Err(error);
        }
        let mut next = current.value.clone();
        next.clipboard = settings;
        let path = app
            .path()
            .app_config_dir()
            .map_err(|_| "配置目录不可用。")?
            .join("settings.json");
        if let Err(e) = crate::settings::persist(&path, &next) {
            if let Err(rollback) = s.configure(app, old) {
                s.report(app, rollback);
            }
            return Err(e);
        }
        current.value = next;
        drop(current);
        // 先保存配置成功再清理；配置写盘失败不能按未提交的配额删除历史。
        if let Err(error) = s.maintain(app) {
            s.report(app, error);
        }
        let _ = app.emit_to("main", "settings-changed", ());
        let status = s.status();
        let _ = app.emit_to("main", "clipboard-state", &status);
        Ok(status)
    })
    .await
}
pub fn open_history(app: &AppHandle) {
    crate::capture::show_main(app);
    let _ = app.emit_to("main", "clipboard-open", ());
}
