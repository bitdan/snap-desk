//! 录屏目录偏好：沿用图片导出的独立目录记录方式，避免覆盖快捷键设置草稿。
//! 路径只能由原生目录选择器产生；每次准备录屏取快照，整段视频在同一目录写入和发布。
use crate::capture::require_window;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

// 防止多个目录对话框或“恢复默认”同时修改目录记录；不与采集/编码锁相互等待。
static CHANGING: Mutex<()> = Mutex::new(());

/// 本地日期时间便于排序和查找；末尾保留后台随机标识，区分同毫秒导出。
pub(super) fn filename(token: &str, extension: &str, recovered: bool) -> String {
    let now = chrono::Local::now();
    let stamp = now.format("%Y%m%d-%H%M%S-%3f");
    let kind = if recovered { "Recovered" } else { "Recording" };
    format!("SnapDesk-{kind}-{stamp}-{token}.{extension}")
}

/// 无覆盖发布现有视频，失败时关闭 TempPath 自动删除，由 Pending 保留恢复源。
pub(super) fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    let mut temporary = tempfile::TempPath::try_from_path(source).map_err(|e| e.to_string())?;
    temporary.disable_cleanup(true);
    temporary
        .persist_noclobber(destination)
        .map_err(|e| format!("发布录屏失败，源文件已保留：{e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDirectory {
    pub path: String,
    pub is_default: bool,
}

fn record(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("recording-directory.json"))
}

fn read(record: &Path) -> Result<Option<PathBuf>, String> {
    match fs::read(record) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("录屏保存目录配置损坏，请重新选择目录或恢复默认：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("无法读取录屏保存目录：{e}")),
    }
}

/// 与图片目录记录相同：写入并同步临时文件后替换，失败保持原配置。
fn persist(record: &Path, directory: Option<&Path>) -> Result<(), String> {
    let parent = record.parent().ok_or("录屏目录配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(file.as_file_mut(), &directory).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(record)
        .map_err(|e| format!("保存录屏目录失败，原配置保持不变：{e}"))?;
    Ok(())
}

pub(crate) fn resolve(app: &AppHandle) -> Result<(PathBuf, bool), String> {
    match read(&record(app)?)? {
        Some(path) => Ok((path, false)),
        None => Ok((
            app.path()
                .app_local_data_dir()
                .map_err(|e| e.to_string())?
                .join("recordings"),
            true,
        )),
    }
}

fn view(path: &Path, is_default: bool) -> RecordingDirectory {
    RecordingDirectory {
        path: path.to_string_lossy().into_owned(),
        is_default,
    }
}

fn validate_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!(
            "录屏保存目录不可用：{}。请重新选择目录或恢复默认。",
            path.display()
        ));
    }
    // 实际试建文件检查写权限；系统 ACL、只读磁盘无法仅靠 metadata 判断。
    let probe = tempfile::NamedTempFile::new_in(path)
        .map_err(|e| format!("录屏保存目录无法写入：{}：{e}", path.display()))?;
    probe
        .close()
        .map_err(|e| format!("录屏保存目录无法清理临时文件：{e}"))
}

pub fn prepare(app: &AppHandle) -> Result<PathBuf, String> {
    let (path, is_default) = resolve(app)?;
    // 自定义目录被删除/移动或外置磁盘掉线时，不偷偷改存其他位置或重建目录。
    if is_default {
        fs::create_dir_all(&path).map_err(|e| format!("无法创建默认录屏目录：{e}"))?;
    }
    validate_directory(&path)?;
    Ok(path)
}

#[tauri::command]
pub async fn current_recording_directory(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<RecordingDirectory, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (path, is_default) = resolve(&app)?;
        Ok(view(&path, is_default))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取消对话框返回 None，不更新 UI 或配置；reset 只恢复偏好，不移动或删除已有视频。
#[tauri::command]
pub async fn choose_recording_directory(
    app: AppHandle,
    window: WebviewWindow,
    reset: bool,
) -> Result<Option<RecordingDirectory>, String> {
    require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _changing = CHANGING
            .try_lock()
            .map_err(|_| "请先完成当前目录选择。".to_string())?;
        let record = record(&app)?;
        if reset {
            persist(&record, None)?;
            let (path, is_default) = resolve(&app)?;
            return Ok(Some(view(&path, is_default)));
        }
        let mut dialog = app
            .dialog()
            .file()
            .set_title("选择录屏保存文件夹")
            .set_parent(&window);
        if let Ok((path, _)) = resolve(&app) {
            if path.is_dir() {
                dialog = dialog.set_directory(path);
            }
        }
        let Some(selected) = dialog.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(|e| e.to_string())?;
        validate_directory(&path)?;
        persist(&record, Some(&path))?;
        Ok(Some(view(&path, false)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_does_not_overwrite_or_delete_source_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("pending.mp4");
        let destination = root.path().join("existing.mp4");
        fs::write(&source, b"new video").unwrap();
        fs::write(&destination, b"old video").unwrap();
        assert!(publish(&source, &destination).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"new video");
        assert_eq!(fs::read(&destination).unwrap(), b"old video");
        let target = root.path().join("finished.mp4");
        publish(&source, &target).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new video");
    }

    #[test]
    fn directory_survives_reload_and_reset_does_not_delete_videos() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("config/recording-directory.json");
        assert_eq!(read(&record).unwrap(), None);
        let custom = root.path().join("自定义 videos");
        fs::create_dir(&custom).unwrap();
        fs::write(custom.join("existing.mp4"), b"existing video").unwrap();
        validate_directory(&custom).unwrap();
        persist(&record, Some(&custom)).unwrap();
        assert_eq!(read(&record).unwrap(), Some(custom.clone()));
        persist(&record, None).unwrap();
        assert_eq!(read(&record).unwrap(), None);
        assert_eq!(
            fs::read(custom.join("existing.mp4")).unwrap(),
            b"existing video"
        );
    }

    #[test]
    fn missing_directory_and_corrupt_record_fail_without_fallback() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing");
        assert!(validate_directory(&path).is_err());
        assert!(!path.exists());
        assert!(validate_directory(Path::new("relative")).is_err());
        let record = root.path().join("record.json");
        fs::write(&record, "invalid json").unwrap();
        assert!(read(&record).is_err());
        assert_eq!(fs::read_to_string(&record).unwrap(), "invalid json");
    }
}
