//! 剪贴板组合导出命令：编码与校验在 core（`snap_core::clipboard::export`），这里负责保存对话框、
//! 暂存与恢复中心登记。快照与输出登记到恢复中心，正常失败由 RAII 清理。
use super::service::Service;
pub use snap_core::clipboard::export::*;
use serde::Serialize;
use std::fs;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

struct ExportGuard<'a>(&'a std::sync::atomic::AtomicBool);
impl Drop for ExportGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    status: &'static str,
    output_paths: Vec<String>,
}
#[tauri::command]
pub async fn clipboard_export(
    app: AppHandle,
    window: WebviewWindow,
    options: Options,
) -> Result<ExportResult, String> {
    // 历史列表已移到独立剪贴板窗口，导出由该窗口（及其编辑子窗口的原 WebView）发起。
    if !super::UI_WINDOWS.contains(&window.label()) {
        return Err("当前窗口无权执行此操作。".into());
    }
    options.validate()?;
    // 组合编辑器是剪贴板窗口的子窗口，命令却归属剪贴板窗口；保存对话框挂到当前聚焦的编辑器上，
    // 否则无主的系统对话框会被置顶的剪贴板窗口和编辑器压在下面。
    let parent = app
        .webview_windows()
        .into_values()
        .find(|candidate| candidate.label().starts_with("dialog-") && candidate.is_focused().unwrap_or(false))
        .unwrap_or(window);
    tauri::async_runtime::spawn_blocking(move || {
        let service = app.state::<Service>();
        service
            .exporting
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| "已有素材正在导出，请完成或取消当前保存对话框。")?;
        let _export = ExportGuard(&service.exporting);
        let temp = crate::shared::export_staging::Workspace::temporary(
            &app,
            "剪贴板导出快照",
            (0..options.ids.len()).map(|i| format!("{i}.png")).collect(),
        )?;
        let items = snapshot(
            &app.state::<Service>(),
            &options.ids,
            &options.texts,
            temp.path(),
        )?;
        // TXT 与纯图片不依赖字体；缺字体时在打开保存对话框之前提示用户。
        let font_bytes = if options.mode != Mode::Separate
            && items.iter().any(|item| matches!(item, Snapshot::Text(_)))
        {
            crate::resources::font_bytes(&app)?
        } else {
            Vec::new()
        };
        let cancelled = || ExportResult {
            status: "cancelled",
            output_paths: vec![],
        };
        let remember = app
            .state::<crate::settings::SettingsState>()
            .0
            .lock()
            .map_err(|_| "导出设置不可用。")?
            .value
            .export
            .remember_directory;
        let mut dialog = app
            .dialog()
            .file()
            .set_title(crate::shared::native_i18n::text(&app, "导出剪贴板素材"))
            .set_parent(&parent);
        if remember {
            if let Some(dir) = crate::shared::image_io::current_directory(&app)? {
                dialog = dialog.set_directory(dir);
            }
        }
        let mut outputs = Vec::new();
        let directory;
        if options.mode == Mode::Separate {
            let Some(path) = dialog.blocking_pick_folder() else {
                return Ok(cancelled());
            };
            let parent = path.into_path().map_err(io_error)?;
            directory = parent.clone();
            // 在用户选择的目录内创建独立批次文件夹，不静默覆盖任何已有文件。
            let names = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    format!(
                        "{:03}.{}",
                        index + 1,
                        if matches!(item, Snapshot::Text(_)) {
                            "txt"
                        } else {
                            "png"
                        }
                    )
                })
                .collect();
            let staging =
                crate::shared::export_staging::Workspace::new(&app, &parent, "剪贴板独立文件批次", names)?;
            for (index, item) in items.iter().enumerate() {
                let path = staging.path().join(format!(
                    "{:03}.{}",
                    index + 1,
                    if matches!(item, Snapshot::Text(_)) {
                        "txt"
                    } else {
                        "png"
                    }
                ));
                match item {
                    Snapshot::Text(t) => fs::write(&path, t.as_bytes()).map_err(io_error)?,
                    Snapshot::Image(p, _, _) => {
                        fs::copy(p, &path).map_err(io_error)?;
                    }
                }
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .and_then(|f| f.sync_all())
                    .map_err(io_error)?;
                outputs.push(path.to_string_lossy().into_owned());
            }
            // 全部成功才交付目录；失败仅清理登记成员，崩溃后的半批次可在恢复中心检查。
            staging.publish()?;
        } else {
            let ext = if options.mode == Mode::Pdf {
                "pdf"
            } else {
                "png"
            };
            let Some(path) = dialog
                .add_filter(crate::shared::native_i18n::text(&app, "导出文件"), &[ext])
                .set_file_name(format!("SnapDesk-{}.{}", super::service::now(), ext))
                .blocking_save_file()
            else {
                return Ok(cancelled());
            };
            let path = path.into_path().map_err(io_error)?;
            if !path
                .extension()
                .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case(ext))
            {
                return Err(format!("请选择 .{ext} 扩展名。"));
            }
            directory = path.parent().ok_or("导出路径无效。")?.to_path_buf();
            let mut file =
                crate::shared::export_staging::TrackedFile::new(&app, &directory, ext, "剪贴板组合导出")?;
            if options.mode == Mode::Pdf {
                pdf_with_font(&items, &options, file.file_mut(), &font_bytes)?;
            } else {
                long_png_with_font(&items, &options, file.file_mut(), &font_bytes)?;
            }
            file.finalize()?;
            file.publish(&path, true)?;
            outputs.push(path.to_string_lossy().into_owned());
        }
        if remember {
            let _ = crate::shared::image_io::remember_export_directory(&app, &directory);
        }
        Ok(ExportResult {
            status: "saved",
            output_paths: outputs,
        })
    })
    .await
    .map_err(|_| "导出任务执行失败。".to_string())?
}

