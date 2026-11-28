//! 完整配置备份格式：固定版本、完整结构校验、大小上限、可选密钥，以及导入失败时的配置文件快照回滚。
//! 选择文件、快捷键事务与运行时状态切换由外壳完成。
use super::{proxy_without_credentials, Platform, Settings};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const VERSION: u32 = 1;
const MAX_BYTES: u64 = 512 * 1024;
/// 应用自己的三个固定配置文件名（配置目录下）。
pub const CONFIG_FILES: [&str; 3] = [
    "settings.json",
    "export-directory.json",
    "recording-directory.json",
];

/// null 表示使用默认位置；路径只备份目录偏好，不携带其中的图片或视频。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Directories {
    pub captures: Option<PathBuf>,
    pub recordings: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Backup {
    pub schema_version: u32,
    /// 导出时的平台（`std::env::consts::OS`）。旧备份没有该字段，导入时不换算快捷键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub settings: Settings,
    pub directories: Directories,
    /// 仅在用户导出时明确勾选才写入的服务密钥（明文）；默认导出不含此字段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretEntry>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretEntry {
    service_id: String,
    key_id: String,
    secret: String,
}
/// 读取已保存服务的全部密钥；读取失败直接报错，避免导出一份缺密钥却显示成功的备份。
fn collect_secrets(settings: &Settings) -> Result<Vec<SecretEntry>, String> {
    let mut entries = Vec::new();
    for service in &settings.recognition_services {
        for key in &service.keys {
            let name = crate::recognition::services::secret_key(&service.id, &service.endpoint, key);
            if let Some(secret) = crate::recognition::ai_credentials::read(&name)? {
                entries.push(SecretEntry { service_id: service.id.clone(), key_id: key.clone(), secret });
            }
        }
    }
    Ok(entries)
}
/// 导入后把备份中的密钥写回系统凭据库；只写入导入配置中存在的服务与密钥标识。
pub fn restore_secrets(settings: &Settings, secrets: Vec<SecretEntry>) -> Result<(), String> {
    let mut failed = 0;
    for entry in secrets {
        let Some(service) = settings.recognition_services.iter().find(|service| service.id == entry.service_id) else { continue };
        if !service.keys.contains(&entry.key_id) || entry.secret.is_empty() || entry.secret.len() > 2048 || entry.secret.chars().any(char::is_control) {
            continue;
        }
        let name = crate::recognition::services::secret_key(&service.id, &service.endpoint, &entry.key_id);
        if crate::recognition::ai_credentials::write(&name, Some(entry.secret)).is_err() {
            failed += 1;
        }
    }
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("配置已导入，但有 {failed} 个密钥未能写入系统凭据，请在服务管理中重新填写。"))
    }
}

/// 生成导出文件内容。不含密钥时同样去掉更新代理里的账号密码，只保留地址。
pub fn export_document(
    mut settings: Settings,
    directories: Directories,
    include_secrets: bool,
) -> Result<Vec<u8>, String> {
    if !include_secrets {
        settings.update_network.proxy = proxy_without_credentials(&settings.update_network.proxy);
    }
    let value = Backup {
        schema_version: VERSION,
        platform: Some(Platform::current().id().into()),
        directories,
        secrets: if include_secrets { collect_secrets(&settings)? } else { Vec::new() },
        settings,
    };
    serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())
}

/// 导出目标不能是正在使用的配置文件（`active` 为这些文件的路径）。
pub fn guard_destination(destination: &Path, active: &[PathBuf]) -> Result<(), String> {
    if let Ok(canonical) = destination.canonicalize() {
        if active
            .iter()
            .filter_map(|path| path.canonicalize().ok())
            .any(|path| path == canonical)
        {
            return Err("不能覆盖正在使用的配置文件，请另选导出位置。".into());
        }
    }
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Result<Backup, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("配置文件超过 512 KiB，请检查所选文件。".into());
    }
    let mut json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "配置格式无效，请选择当前版本导出的配置文件。".to_string())?;
    // 新增更新偏好不使旧版完整备份失效；只为这个新增字段补默认值，其他字段仍须完整。
    if let Some(settings) = json.get_mut("settings").and_then(|value| value.as_object_mut()) {
        settings.entry("receivePrereleaseUpdates").or_insert(false.into());
        settings.entry("updateCheckMinutes").or_insert(60.into());
        settings.entry("recognitionServices").or_insert(serde_json::json!([]));
        // 后续版本新增的行为开关同样补默认值：旧备份导入后保持原有行为（不自动复制、不直接执行，剪贴板默认直接粘贴）。
        settings.entry("screenshotAfter").or_insert("none".into());
        for (field, additions) in [
            ("ai", serde_json::json!({"serviceId": "", "autoCopy": false})),
            ("export", serde_json::json!({"includeApp": false, "includeTitle": false})),
            ("ocr", serde_json::json!({"serviceId": "", "autoCopy": false, "mergeLines": false})),
            ("clipboard", serde_json::json!({"pasteOnActivate": true, "excludedApps": []})),
        ] {
            if let Some(object) = settings.get_mut(field).and_then(|value| value.as_object_mut()) {
                for (key, value) in additions.as_object().unwrap() { object.entry(key.clone()).or_insert(value.clone()); }
            }
        }
        // 旧备份没有 OCR 引擎偏好时保留原 PaddleOCR 行为，不擅自切换新引擎。
        settings.entry("ocr").or_insert(serde_json::to_value(crate::recognition::ocr_config::OcrSettings::default()).map_err(|e| e.to_string())?);
    }
    // 导入要求当前完整结构，不借用启动配置的缺省补齐来悄悄重置遗漏字段。
    let shape = serde_json::to_value(Backup {
        schema_version: VERSION,
        platform: None,
        settings: Settings::default(),
        directories: Directories {
            captures: None,
            recordings: None,
        },
        secrets: Vec::new(),
    })
    .map_err(|e| e.to_string())?;
    if !complete_shape(&json, &shape) {
        return Err("配置格式无效，请选择当前版本导出的配置文件。".into());
    }
    let mut value: Backup = serde_json::from_value(json)
        .map_err(|_| "配置格式无效，请选择当前版本导出的配置文件。".to_string())?;
    if value.schema_version != VERSION {
        return Err("配置版本不支持，请使用当前版本重新导出。".into());
    }
    // 跨平台导入：仍是来源平台默认值的快捷键换成本机默认值（如 Windows 的 Alt+V → macOS 的 ⌃⌥V）。
    if let Some(from) = value.platform.as_deref().map(Platform::from_id) {
        if from != Platform::current() {
            value.settings.shortcuts.adopt_platform_defaults(from);
        }
    }
    value.settings.validate()?;
    Ok(value)
}
fn complete_shape(value: &serde_json::Value, shape: &serde_json::Value) -> bool {
    match shape.as_object() {
        Some(fields) => value.as_object().is_some_and(|object| {
            fields.iter().all(|(key, expected)| {
                object
                    .get(key)
                    .is_some_and(|value| complete_shape(value, expected))
            })
        }),
        None => true,
    }
}
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("配置文件超过 512 KiB，请检查所选文件。".into());
    }
    Ok(bytes)
}
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// 仅快照应用自己的固定配置文件；回滚尝试恢复全部文件，绝不删除用户媒体。
pub struct Files(Vec<(PathBuf, Option<Vec<u8>>)>);
impl Files {
    /// `root` 下的 [`CONFIG_FILES`]。
    pub fn snapshot(root: &Path) -> Result<Self, String> {
        Self::snapshot_paths(CONFIG_FILES.iter().map(|name| root.join(name)))
    }
    /// 指定的配置文件（设置文件可能被调试环境变量移到别处时使用）。
    pub fn snapshot_paths(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let mut files = Vec::new();
        for path in paths {
            let bytes = match read_bounded(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if path.try_exists().map_err(|e| e.to_string())? => return Err(error),
                Err(_) => None,
            };
            files.push((path, bytes));
        }
        Ok(Self(files))
    }
    pub fn restore(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        for (path, bytes) in &self.0 {
            let result = if let Some(bytes) = bytes {
                write_bytes(path, bytes)
            } else {
                match fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                }
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }
}

pub fn with_rollback(error: String, failures: Vec<String>) -> String {
    if failures.is_empty() {
        error
    } else {
        format!("配置导入失败：{error}；回滚失败：{}", failures.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn backup() -> Backup {
        Backup {
            schema_version: VERSION,
            platform: Some(Platform::current().id().into()),
            settings: Settings::default(),
            directories: Directories {
                captures: None,
                recordings: None,
            },
            secrets: Vec::new(),
        }
    }
    #[test]
    fn secrets_are_optional_and_round_trip_only_when_present() {
        let plain = serde_json::to_value(backup()).unwrap();
        assert!(plain.get("secrets").is_none());
        let mut value = plain.clone();
        value["secrets"] = serde_json::json!([{"serviceId": "a", "keyId": "default", "secret": "test-key"}]);
        let imported = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(imported.secrets.len(), 1);
        assert_eq!(imported.secrets[0].secret, "test-key");
        value["secrets"] = serde_json::json!([{"serviceId": "a", "keyId": "default"}]);
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn complete_backup_round_trips_shortcuts_and_directories() {
        let mut value = backup();
        value.settings.shortcuts.capture = "Ctrl+Shift+A".into();
        value.settings.shortcuts_enabled = false;
        value.directories.captures = Some(PathBuf::from("C:/Pictures"));
        value.directories.recordings = Some(PathBuf::from("D:/Videos"));
        let imported = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(imported.settings, value.settings);
        assert_eq!(imported.directories.captures, value.directories.captures);
        assert_eq!(
            imported.directories.recordings,
            value.directories.recordings
        );
    }
    #[test]
    fn exported_documents_parse_back_and_strip_proxy_credentials() {
        let mut settings = Settings::default();
        settings.update_network.proxy = "http://user:secret@proxy.example:8080".into();
        let bytes = export_document(settings.clone(), Directories { captures: None, recordings: None }, false).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("secret@"));
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.settings.shortcuts, settings.shortcuts);
        assert!(parsed.secrets.is_empty());
    }
    #[test]
    fn active_config_files_cannot_be_overwritten_by_export() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("settings.json");
        fs::write(&active, b"{}").unwrap();
        assert!(guard_destination(&active, std::slice::from_ref(&active)).is_err());
        assert!(guard_destination(&root.path().join("backup.json"), &[active]).is_ok());
    }
    #[test]
    fn rejects_old_versions_unknown_fields_and_invalid_settings() {
        assert!(parse(&serde_json::to_vec(&Settings::default()).unwrap()).is_err());
        let mut value = serde_json::to_value(backup()).unwrap();
        value["schemaVersion"] = 99.into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value["schemaVersion"] = VERSION.into();
        value["unexpected"] = true.into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value.as_object_mut().unwrap().remove("unexpected");
        // 与截图键重复即无效（默认键因平台而异，取实际值）。
        value["settings"]["shortcuts"]["pin"] = value["settings"]["shortcuts"]["capture"].clone();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(parse(&vec![b' '; MAX_BYTES as usize + 1]).is_err());
    }
    #[test]
    fn incomplete_backups_cannot_reset_missing_fields_to_defaults() {
        let mut value = serde_json::to_value(backup()).unwrap();
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("shortcuts");
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(backup()).unwrap();
        value["directories"]
            .as_object_mut()
            .unwrap()
            .remove("captures");
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn old_backups_default_to_stable_updates() {
        let mut value = serde_json::to_value(backup()).unwrap();
        value["settings"].as_object_mut().unwrap().remove("receivePrereleaseUpdates");
        assert!(!parse(&serde_json::to_vec(&value).unwrap()).unwrap().settings.receive_prerelease_updates);
        value["settings"]["receivePrereleaseUpdates"] = true.into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).unwrap().settings.receive_prerelease_updates);
    }
    #[test]
    fn old_backups_preserve_paddle_and_new_backups_keep_explicit_system_selection() {
        let mut value = serde_json::to_value(backup()).unwrap();
        value["settings"].as_object_mut().unwrap().remove("ocr");
        assert_eq!(parse(&serde_json::to_vec(&value).unwrap()).unwrap().settings.ocr.provider, "local-ppocr");
        value["settings"]["ocr"] = serde_json::json!({"provider":"windows-media", "language":"en-US"});
        let restored = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(restored.settings.ocr.provider, "windows-media");
        assert_eq!(restored.settings.ocr.language, "en-US");
        value["settings"]["ocr"].as_object_mut().unwrap().remove("language");
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn backups_from_before_behavior_switches_import_with_defaults() {
        let mut value = serde_json::to_value(backup()).unwrap();
        let settings = value["settings"].as_object_mut().unwrap();
        settings.remove("screenshotAfter");
        settings["ai"].as_object_mut().unwrap().remove("autoCopy");
        for key in ["autoCopy", "mergeLines"] {
            settings["ocr"].as_object_mut().unwrap().remove(key);
        }
        for key in ["pasteOnActivate", "excludedApps"] {
            settings["clipboard"].as_object_mut().unwrap().remove(key);
        }
        let restored = parse(&serde_json::to_vec(&value).unwrap()).unwrap().settings;
        assert_eq!(restored.screenshot_after, "none");
        assert!(!restored.ai.auto_copy && !restored.ocr.auto_copy && !restored.ocr.merge_lines);
        assert!(restored.clipboard.paste_on_activate && restored.clipboard.excluded_apps.is_empty());
    }
    #[test]
    fn rollback_restores_existing_preferences_and_removes_only_new_records() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("settings.json"), b"original").unwrap();
        let snapshot = Files::snapshot(root.path()).unwrap();
        fs::write(root.path().join("settings.json"), b"changed").unwrap();
        fs::write(root.path().join("export-directory.json"), b"new").unwrap();
        fs::write(root.path().join("image.png"), b"keep").unwrap();
        snapshot.restore().unwrap();
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            b"original"
        );
        assert!(!root.path().join("export-directory.json").exists());
        assert_eq!(fs::read(root.path().join("image.png")).unwrap(), b"keep");
    }

    #[test]
    fn cross_platform_backups_map_default_shortcuts_and_keep_custom_ones() {
        let other = if Platform::current() == Platform::Macos { Platform::Windows } else { Platform::Macos };
        let mut value = backup();
        value.platform = Some(other.id().into());
        value.settings.shortcuts = super::super::Shortcuts {
            pin: "Ctrl+F8".into(),
            ..super::super::Shortcuts::for_platform(other)
        };
        let imported = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            imported.settings.shortcuts,
            super::super::Shortcuts { pin: "Ctrl+F8".into(), ..super::super::Shortcuts::default() }
        );
        // 旧备份没有平台字段：照原样导入。
        let mut json = serde_json::to_value(&value).unwrap();
        json.as_object_mut().unwrap().remove("platform");
        let legacy = parse(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert_eq!(legacy.settings.shortcuts, value.settings.shortcuts);
    }
}
