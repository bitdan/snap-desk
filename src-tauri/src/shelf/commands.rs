//! 设置仅限主窗口；卡片命令只接受历史 ID，不接收磁盘路径或原始内容。
use super::{window, ShelfSettings, ShelfSnapshot, ShelfState};
use tauri::{AppHandle, Manager, WebviewWindow};

fn require(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "shelf") {
        Ok(())
    } else {
        Err("窗口无权访问暂存架。".into())
    }
}

#[tauri::command]
pub async fn shelf_snapshot(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ShelfSnapshot, String> {
    require(&window)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<ShelfState>().snapshot(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn shelf_configure(
    app: AppHandle,
    window: WebviewWindow,
    settings: ShelfSettings,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let enabled = settings.enabled;
        app.state::<ShelfState>().update(&app, |prefs| {
            prefs.settings = settings;
            Ok(())
        })?;
        if !enabled {
            window::hide(&app)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn shelf_visibility(
    app: AppHandle,
    window: WebviewWindow,
    visible: bool,
) -> Result<(), String> {
    require(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        if visible {
            window::show(&app)
        } else {
            window::hide(&app)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShelfAction {
    Keep,
    Unkeep,
    Remove,
    Restore,
}

#[tauri::command]
pub async fn shelf_item_action(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
    action: ShelfAction,
) -> Result<(), String> {
    require(&window)?;
    if id <= 0 {
        return Err("条目 ID 无效。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let clipboard = app.state::<crate::clipboard::Service>();
        let data = clipboard.data()?;
        data.repo.get(id)?;
        app.state::<ShelfState>().update(&app, |prefs| {
            // 清理已被历史删除的引用，避免失效固定项占满 30 个位置。
            prefs.kept_ids.retain(|value| data.repo.get(*value).is_ok());
            match action {
                ShelfAction::Keep => {
                    if !prefs.kept_ids.contains(&id) {
                        if prefs.kept_ids.len() >= 30 {
                            return Err("暂存架最多固定 30 项。".into());
                        }
                        prefs.kept_ids.push(id);
                    }
                    prefs.hidden_ids.retain(|value| *value != id);
                }
                ShelfAction::Unkeep => prefs.kept_ids.retain(|value| *value != id),
                ShelfAction::Remove => {
                    prefs.kept_ids.retain(|value| *value != id);
                    prefs.hidden_ids.retain(|value| *value != id);
                    prefs.hidden_ids.push(id);
                    if prefs.hidden_ids.len() > 1000 {
                        prefs.hidden_ids.remove(0);
                    }
                }
                ShelfAction::Restore => prefs.hidden_ids.retain(|value| *value != id),
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
