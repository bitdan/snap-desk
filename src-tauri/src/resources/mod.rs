//! 用户主动下载 OCR 引擎、模型和字体：下载、校验与安装在 core `snap_core::resources`；
//! 这里负责 Tauri 命令、进度事件与状态缓存。
pub(crate) mod lifecycle;
pub(crate) mod status_cache;

use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

static OPERATION: Mutex<()> = Mutex::new(());
// OCR 与字体分开观察依赖，替换字体不会重新校验模型/DLL。
static OCR_STATUS: status_cache::StatusCache<(bool, bool, u64)> = status_cache::StatusCache::new();
static FONT_STATUS: status_cache::StatusCache<bool> = status_cache::StatusCache::new();

/// 资源清单、类别与摘要校验在 core，GPUI 版与本外壳共用。
pub use snap_core::resources::ResourceKind;
pub(crate) use snap_core::resources::Asset;
use snap_core::resources::{manifest, read_font, FONT_FILE};

/// 设置页展示的实际可用状态，不把“文件存在”直接当作“资源已就绪”。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    directory: String,
    ocr_ready: bool,
    models_ready: bool,
    runtime_ready: bool,
    runtime_download_mib: u64,
    font_ready: bool,
}

/// 下载进度通过主窗口事件发送；received 和 total 均以字节为单位。
#[derive(Clone, Serialize)]
pub struct Progress {
    name: String,
    received: u64,
    total: u64,
}

pub fn directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("resources"))
        .map_err(|e| e.to_string())
}

/// 每次导出取得独立字节快照，导出中途替换字体不会改变换行和排版。
pub fn font_bytes(app: &AppHandle) -> Result<Vec<u8>, String> {
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| "资源正在更新，请稍后导出。")?;
    read_font(&directory(app)?.join(FONT_FILE))
}

pub fn ocr_paths(app: &AppHandle) -> Result<PathBuf, String> {
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| "资源正在更新，请稍后识别。")?;
    snap_core::resources::ocr_models(&directory(app)?)
}

/// 只观察实际校验的 OCR 文件和当前架构运行库；根目录及其他资源不属于本分区。
fn ocr_status_paths(dir: &Path, assets: &[Asset]) -> Result<Vec<PathBuf>, String> {
    let paths: Vec<_> = assets
        .iter()
        .filter(|a| a.kind == ResourceKind::Ocr)
        .map(|a| dir.join(&a.name))
        .collect();
    #[cfg(windows)]
    let paths = {
        let mut paths = paths;
        #[derive(serde::Deserialize)]
        struct RuntimeDirectory {
            arch: String,
            directory: String,
        }
        // 与 ocr_runtime 使用同一份内置清单，避免在缓存层另写一份版本/目录常量。
        let runtimes: Vec<RuntimeDirectory> =
            serde_json::from_str(include_str!("../../assets/onnxruntime.json"))
                .map_err(|e| e.to_string())?;
        if let Some(runtime) = runtimes
            .iter()
            .find(|runtime| runtime.arch == std::env::consts::ARCH)
        {
            paths.push(dir.join(&runtime.directory));
        }
        paths
    };
    Ok(paths)
}

fn status(app: &AppHandle) -> Result<ResourceStatus, String> {
    let dir = directory(app)?;
    let assets = manifest()?;
    let (models_ready, runtime_ready, runtime_download_mib) =
        OCR_STATUS.read(&ocr_status_paths(&dir, &assets)?, || {
            let models_ready = snap_core::resources::models_ready(&dir)?;
            let (runtime_ready, runtime_download_mib) = snap_core::resources::runtime_status(&dir);
            Ok((models_ready, runtime_ready, runtime_download_mib))
        })?;
    let font = dir.join(FONT_FILE);
    let font_ready = FONT_STATUS.read(std::slice::from_ref(&font), || {
        Ok(snap_core::resources::font_ready(&dir))
    })?;
    Ok(ResourceStatus {
        models_ready,
        runtime_ready,
        runtime_download_mib,
        font_ready,
        ocr_ready: models_ready && runtime_ready,
        directory: dir.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn resource_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ResourceStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || status(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// 下载仅使用编译时白名单地址，不接受来自 WebView 的 URL 或文件名。
#[tauri::command]
pub async fn download_resources(
    app: AppHandle,
    window: WebviewWindow,
    kind: ResourceKind,
) -> Result<ResourceStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION.try_lock().map_err(|_| "已有资源操作正在进行。")?;
        let dir = directory(&app)?;
        snap_core::resources::download(&dir, kind, &mut |name, received, total| {
            emit_progress(&app, name, received, total)
        })?;
        status(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn emit_progress(app: &AppHandle, name: &str, received: u64, total: u64) {
    let _ = app.emit_to(
        "main",
        "resource-download-progress",
        Progress {
            name: name.to_owned(),
            received,
            total,
        },
    );
}


/// 由系统对话框授权读取文件，并复制到应用目录；原文件移走后仍可导出。
#[tauri::command]
pub async fn choose_export_font(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ResourceStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATION.try_lock().map_err(|_| "已有资源操作正在进行。")?;
        if let Some(path) = app
            .dialog()
            .file()
            .set_title(crate::shared::native_i18n::text(&app, "选择静态 TTF 导出字体"))
            .add_filter(crate::shared::native_i18n::text(&app, "TrueType 字体"), &["ttf"])
            .blocking_pick_file()
        {
            let bytes = read_font(&path.into_path().map_err(|e| e.to_string())?)?;
            snap_core::resources::install_font(&directory(&app)?, &bytes)?;
        }
        status(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}


/// 导出只投影实际校验结果，不序列化含绝对目录的 ResourceStatus。
pub(crate) fn diagnostic_summary(app: &AppHandle) -> serde_json::Value {
    match crate::recognition::ocr_config::snapshot(app) {
        Ok((selected, service)) => {
            if let Some(service) = service {
                // 在线服务不依赖本地模型；诊断只记录协议和模型，不导出地址、账号或凭据。
                return serde_json::json!({"provider": service.protocol, "model": service.model});
            }
            if selected.provider == "windows-media" {
                return serde_json::json!({"provider": selected.provider, "language": selected.language});
            }
        }
        Err(_) => return serde_json::json!({"state":"unavailable"}),
    }
    match status(app) {
        Ok(value) => serde_json::json!({"provider":"local-ppocr", "model":"pp-ocrv6-small", "ready":value.ocr_ready, "runtimeReady":value.runtime_ready, "modelsReady":value.models_ready}),
        Err(_) => serde_json::json!({"provider":"local-ppocr", "state":"unavailable"}),
    }
}
