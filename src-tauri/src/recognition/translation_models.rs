//! 翻译语言包命令：清单、校验、下载与安装在 core `snap_core::recognition::local_models`；
//! 这里负责导入目录选择、按方向的状态缓存与 worker 互斥。
use crate::resources;
use serde::Deserialize;
use snap_core::recognition::local_models::{self as core, LocalStatus, ModelStatus, IDS};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

// 两个语言包各自缓存；安装/删除一个方向不触发另一个模型的整文件摘要。
static MODEL_STATUS: [resources::status_cache::StatusCache<ModelStatus>; IDS.len()] =
    [const { resources::status_cache::StatusCache::new() }; IDS.len()];

fn status(app: &AppHandle) -> Result<LocalStatus, String> {
    let catalog = core::catalog()?;
    let dir = resources::directory(app)?;
    let root = core::root(&dir);
    let models = IDS
        .iter()
        .enumerate()
        .map(|(index, id)| {
            MODEL_STATUS[index].read(&[root.join(id)], || Ok(core::model_status(&dir, &catalog, id)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(LocalStatus {
        runtime_ready: crate::recognition::translation_runtime::status_ready(app)?,
        runtime_supported: crate::recognition::translation_runtime::supported(),
        models,
    })
}

#[tauri::command]
pub async fn translation_model_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<LocalStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || status(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelAction {
    Download,
    Import,
    Delete,
}
#[tauri::command]
pub async fn translation_model_action(
    app: AppHandle,
    window: WebviewWindow,
    model_id: String,
    action: ModelAction,
) -> Result<LocalStatus, String> {
    crate::capture::require_window(&window, "main")?;
    if model_id != "runtime" && !IDS.contains(&model_id.as_str()) {
        return Err("未知语言包。".into());
    }
    let state = app.state::<crate::recognition::local_translation::LocalTranslationState>();
    let mut slot = state
        .0
        .clone()
        .try_lock_owned()
        .map_err(|_| "翻译正在进行，请结束后再管理语言包。")?;
    // 卸载缓存模型后再替换磁盘资源，避免使用已删除的版本。
    if let Some(worker) = slot.take() {
        worker.stop().await;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // guard 随阻塞任务存活，即使调用方关闭窗口，安装中途也不能启动翻译。
        let _guard = slot;
        // 引擎和语言包共用 Worker 互斥锁，替换 EXE/DLL 前已等待旧进程退出。
        if model_id == "runtime" {
            match action {
                ModelAction::Download => crate::recognition::translation_runtime::install(&app, false)?,
                ModelAction::Import => crate::recognition::translation_runtime::install(&app, true)?,
                ModelAction::Delete => return Err("不支持此引擎操作。".into()),
            }
            return status(&app);
        }
        let dir = resources::directory(&app)?;
        match action {
            ModelAction::Download => {
                let pack = core::downloadable(&model_id)?;
                core::install(&dir, &pack, None, &mut |name, received, total| {
                    resources::emit_progress(&app, name, received, total)
                })?;
            }
            ModelAction::Import => {
                if let Some(folder) = app
                    .dialog()
                    .file()
                    .set_title(crate::shared::native_i18n::text(
                        &app,
                        "选择可信的离线语言包目录（含 pack.json）",
                    ))
                    .blocking_pick_folder()
                {
                    let source = folder.into_path().map_err(|e| e.to_string())?;
                    let pack = core::read_pack(&source)?;
                    if pack.model_id != model_id {
                        return Err("所选语言包方向不匹配。".into());
                    }
                    core::install(&dir, &pack, Some(&source), &mut |_, _, _| {})?;
                }
            }
            ModelAction::Delete => core::delete(&dir, &model_id)?,
        }
        status(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 不导出安装包自带的 revision 文本或任何路径，仅保留固定模型 ID 与校验状态。
pub(crate) fn diagnostic_summary(app: &AppHandle) -> serde_json::Value {
    match status(app) {
        Ok(value) => serde_json::json!({"provider":"local-ctranslate2", "runtimeReady":value.runtime_ready, "models":value.models.iter().map(|m| serde_json::json!({"model":m.model_id,"ready":m.ready})).collect::<Vec<_>>()}),
        Err(_) => serde_json::json!({"provider":"local-ctranslate2", "state":"unavailable"}),
    }
}
