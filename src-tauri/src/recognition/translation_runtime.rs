//! 翻译引擎按需安装：校验与安装在 core `snap_core::recognition::local_runtime`；
//! 这里负责导入时的系统文件选择与设置页状态缓存。
use snap_core::recognition::local_runtime as core;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

static STATUS: crate::resources::status_cache::StatusCache<bool> =
    crate::resources::status_cache::StatusCache::new();

fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(core::root(&crate::resources::directory(app)?))
}

pub fn supported() -> bool {
    core::supported()
}

/// 仅设置页复用引擎目录的近期校验；启动执行仍直接调用 executable 完整验证。
pub(super) fn status_ready(app: &AppHandle) -> Result<bool, String> {
    if !supported() {
        return Ok(false);
    }
    STATUS.read(&[root(app)?], || Ok(executable(app).is_ok()))
}

pub fn executable(app: &AppHandle) -> Result<PathBuf, String> {
    core::executable(&crate::resources::directory(app)?)
}

/// `import` 时先弹出系统文件选择器；取消选择不做任何改动。
pub fn install(app: &AppHandle, import: bool) -> Result<(), String> {
    let resources = crate::resources::directory(app)?;
    if !import {
        return core::install(&resources, None);
    }
    if !supported() {
        return core::install(&resources, None);
    }
    let Some(file) = app
        .dialog()
        .file()
        .set_title(crate::shared::native_i18n::text(app, "选择翻译引擎 ZIP"))
        .add_filter("ZIP", &["zip"])
        .blocking_pick_file()
    else {
        return Ok(());
    };
    let source = file.into_path().map_err(|e| e.to_string())?;
    core::install(&resources, Some(&source))
}
