//! 导出本地诊断包：选择保存位置并生成元数据；打包与白名单清洗在 core `snap_core::diagnostics`。
use super::{catalog, location, logs, runtime};
use snap_core::diagnostics::package::{filename, log_summary, validate_destination, write_package};
use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

pub fn save(app: &AppHandle, window: &WebviewWindow) -> Result<bool, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_parent(window)
        .set_title(crate::shared::native_i18n::text(app, "导出本地诊断包"))
        .set_file_name(filename())
        .add_filter("ZIP", &["zip"])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let destination = file.into_path().map_err(|_| "所选位置不是本地文件。")?;
    validate_destination(&destination)?;
    log::logger().flush();
    let (root, base) = location(app)?;
    let recent = logs::recent(
        &logs::files(&root, &base)?,
        &logs::Query {
            limit: 500,
            level: None,
            module: None,
        },
        catalog::COMMANDS,
    )?;
    let metadata = serde_json::json!({
        "schemaVersion": 1,
        "app": "Snap Desk", "appVersion": app.package_info().version.to_string(),
        "buildChannel": if app.package_info().version.pre.is_empty() { "stable" } else { "prerelease" },
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "collectedAt": chrono::Local::now().to_rfc3339(),
        "logLevel": runtime::level(),
        "ocr": crate::resources::diagnostic_summary(app),
        "translation": crate::recognition::translation_models::diagnostic_summary(app),
        // 不为导出试录视频；标明仅为后端能力检查，编码器未探测。
        "recording": crate::capture::recording::diagnostic_summary(),
        "logs": log_summary(&recent)
    });
    write_package(&destination, &metadata, &recent)?;
    log::info!("event=diagnostics.exported module=diagnostics");
    Ok(true)
}
