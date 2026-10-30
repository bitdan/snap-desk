//! 主窗口专属的本地诊断入口：读取、导出与清理走阻塞线程，不授予前端文件权限。
mod catalog;
#[cfg(windows)]
mod export;
mod logs;
pub mod runtime;

use crate::capture::require_window;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

// 只串行化诊断操作，不与日志写入、截图或录屏共用锁。
static OPERATION: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    level: logs::Level,
    directory: Option<String>,
    error: Option<String>,
    total_bytes: u64,
    archives: usize,
    recent_errors: usize,
    export_supported: bool,
}

fn location(app: &AppHandle) -> Result<(std::path::PathBuf, String), String> {
    let root = app
        .path()
        .app_log_dir()
        .map_err(|_| "无法定位应用日志目录。")?;
    // 与日志插件默认文件名使用同一个 package_info.name，不硬编码产品显示名称。
    Ok((root, app.package_info().name.clone()))
}

fn status(app: &AppHandle) -> Status {
    let mut status = Status {
        level: runtime::level(),
        directory: None,
        error: None,
        total_bytes: 0,
        archives: 0,
        recent_errors: 0,
        export_supported: cfg!(windows),
    };
    let result = (|| {
        let (root, base) = location(app)?;
        status.directory = Some(root.to_string_lossy().into_owned());
        let files = logs::files(&root, &base)?;
        status.total_bytes = files
            .iter()
            .fold(0_u64, |total, f| total.saturating_add(f.size));
        status.archives = files.iter().filter(|f| !f.active).count();
        let recent = logs::recent(
            &files,
            &logs::Query {
                limit: 500,
                level: None,
                module: None,
            },
        )?;
        status.recent_errors = recent
            .entries
            .iter()
            .filter(|e| e.level == logs::Level::Error)
            .count();
        Ok::<_, String>(())
    })();
    status.error = result.err();
    status
}

#[tauri::command]
pub async fn diagnostics_status(app: AppHandle, window: WebviewWindow) -> Result<Status, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION
            .try_lock()
            .map_err(|_| "诊断操作正在进行，请稍后刷新。")?;
        Ok(status(&app))
    })
    .await
    .map_err(|_| "诊断状态读取失败。")?
}

#[tauri::command]
pub async fn diagnostics_recent_logs(
    app: AppHandle,
    window: WebviewWindow,
    query: logs::Query,
) -> Result<logs::Recent, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION
            .try_lock()
            .map_err(|_| "诊断操作正在进行，请稍后刷新。")?;
        log::logger().flush();
        let (root, base) = location(&app)?;
        logs::recent(&logs::files(&root, &base)?, &query)
    })
    .await
    .map_err(|_| "读取日志任务失败。")?
}

/// 级别使用枚举白名单；持久化成功后立即生效，返回后端实际状态。
#[tauri::command]
pub async fn diagnostics_set_level(
    app: AppHandle,
    window: WebviewWindow,
    level: logs::Level,
) -> Result<Status, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION.try_lock().map_err(|_| "诊断操作正在进行。")?;
        runtime::set_level(&app, level)?;
        log::info!("event=diagnostics.level_changed module=diagnostics level={:?}", level);
        Ok(status(&app))
    })
    .await
    .map_err(|_| "修改日志级别失败。")?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    cancelled: bool,
    deleted: usize,
    failed: usize,
}

/// 确认在原生端完成，即使绕过页面调用也不能静默删除归档；活动日志始终保留。
#[tauri::command]
pub async fn diagnostics_clear_archives(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ClearResult, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION.try_lock().map_err(|_| "诊断操作正在进行。")?;
        let confirmed = app
            .dialog()
            .message(crate::shared::native_i18n::text(
                &app,
                "仅删除历史日志归档，保留当前正在写入的日志。此操作无法撤销。",
            ))
            .title(crate::shared::native_i18n::text(&app, "清理历史日志"))
            .parent(&window)
            .buttons(MessageDialogButtons::OkCancelCustom(
                crate::shared::native_i18n::text(&app, "确认清理"),
                crate::shared::native_i18n::text(&app, "取消"),
            ))
            .blocking_show();
        let mut result = ClearResult {
            cancelled: !confirmed,
            deleted: 0,
            failed: 0,
        };
        if !confirmed {
            return Ok(result);
        }
        let (root, base) = location(&app)?;
        (result.deleted, result.failed) = logs::clear_archives(&root, &base)?;
        log::info!("event=diagnostics.archives_cleared module=diagnostics");
        Ok(result)
    })
    .await
    .map_err(|_| "清理历史日志失败。")?
}

/// 导出内容固定为近期诊断，无任意 scope/path 参数。取消保存返回 false。
#[tauri::command]
pub async fn diagnostics_export(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    require_window(&window, "main")?;
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = OPERATION.try_lock().map_err(|_| "诊断操作正在进行。")?;
            export::save(&app, &window)
        })
        .await
        .map_err(|_| "诊断包导出任务失败。")?
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("当前平台暂不支持 ZIP 诊断包导出。".into())
    }
}
