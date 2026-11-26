//! 自动保存与定期清理：设置开启“自动保存”后，复制、贴图或滚动截图的成图另存一份到截图保存目录
//! （[`export::save_directory`]），文件名与格式同普通保存，重名时加编号。
//!
//! 清理只删除这里自动保存的文件：每次保存把路径、大小与修改时间记进数据目录下的 `auto-saved.json`，
//! 到期时文件仍与记录一致才删除；用户自己放进该目录的文件、被改过或移走的自动保存文件都不会被删，
//! 后两种直接从记录中移除。记录损坏时当作空记录，最多是旧文件不再被清理。
use crate::{export, settings::ExportSettings};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// 保存与清理都要读改写同一份记录；同一目录内的取名与写入也需要串行，避免同一秒的两次保存撞名。
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    path: PathBuf,
    size: u64,
    /// 保存后文件的修改时间（Unix 秒），用来判断文件之后有没有被改过。
    modified: u64,
    /// 保存时间（Unix 秒），清理按它计算保留天数。
    saved: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Record {
    version: u32,
    entries: Vec<Entry>,
}

pub fn record_path() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("auto-saved.json"))
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn load(record: &Path) -> Record {
    fs::read(record)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
        .filter(|record| record.version == 1)
        .unwrap_or_default()
}

fn store(record: &Path, entries: Vec<Entry>) -> Result<(), String> {
    let parent = record.parent().ok_or("自动保存记录路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(file.as_file_mut(), &Record { version: 1, entries }).map_err(|e| e.to_string())?;
    file.persist(record).map_err(|e| e.error.to_string())?;
    Ok(())
}

/// 自动保存一份到 `directory` 并登记；返回保存路径。登记失败只影响以后的清理，不把保存报告为失败。
pub fn save(image: &RgbaImage, settings: &ExportSettings, directory: &Path) -> Result<PathBuf, String> {
    save_with_record(record_path().as_deref(), image, settings, directory)
}

fn save_with_record(
    record: Option<&Path>,
    image: &RgbaImage,
    settings: &ExportSettings,
    directory: &Path,
) -> Result<PathBuf, String> {
    let _guard = LOCK.lock().map_err(|_| "自动保存忙碌。")?;
    let path = export::quick_save(image, settings, directory)?;
    if let Some(record) = record {
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        let entry = Entry {
            path: path.clone(),
            size: metadata.len(),
            modified: metadata.modified().map(seconds).unwrap_or(0),
            saved: seconds(SystemTime::now()),
        };
        let mut entries = load(record).entries;
        entries.push(entry);
        if let Err(error) = store(record, entries) {
            log::warn!("event=auto_save.record_failed message={error}");
        }
    }
    Ok(path)
}

/// 删除保存超过 `days` 天、且保存后未被改动的自动保存文件；`days` 为 0 时不清理。返回删除的个数。
pub fn clean(days: u16) -> Result<usize, String> {
    match record_path() {
        Some(record) => clean_record(&record, days, SystemTime::now()),
        None => Ok(0),
    }
}

fn clean_record(record: &Path, days: u16, now: SystemTime) -> Result<usize, String> {
    if days == 0 {
        return Ok(0);
    }
    let _guard = LOCK.lock().map_err(|_| "自动保存忙碌。")?;
    let before = load(record).entries;
    if before.is_empty() {
        return Ok(0);
    }
    let cutoff = seconds(now.checked_sub(Duration::from_secs(u64::from(days) * 86_400)).unwrap_or(UNIX_EPOCH));
    let mut removed = 0;
    let mut kept = Vec::new();
    for entry in before.iter() {
        // 文件已不在或被改过：不再属于自动保存，移出记录，不删除。
        let Ok(metadata) = fs::metadata(&entry.path) else {
            continue;
        };
        let unchanged = metadata.is_file()
            && metadata.len() == entry.size
            && metadata.modified().map(seconds).unwrap_or(0) == entry.modified;
        if !unchanged {
            continue;
        }
        if entry.saved < cutoff {
            match fs::remove_file(&entry.path) {
                Ok(()) => removed += 1,
                // 删不掉（被占用等）：保留记录，下次再试。
                Err(_) => kept.push(entry.clone()),
            }
        } else {
            kept.push(entry.clone());
        }
    }
    if kept != before {
        store(record, kept)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_into_the_directory_and_cleans_only_unchanged_expired_files() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("data").join("auto-saved.json");
        let directory = root.path().join("shots");
        let image = RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]));
        let settings = ExportSettings::default();
        let kept = save_with_record(Some(&record), &image, &settings, &directory).unwrap();
        let edited = save_with_record(Some(&record), &image, &settings, &directory).unwrap();
        let expired = save_with_record(Some(&record), &image, &settings, &directory).unwrap();
        assert_ne!(kept, edited);
        // 用户自己放进目录的文件不在记录里，永远不会被清理。
        let own = directory.join("mine.png");
        fs::write(&own, b"mine").unwrap();
        fs::write(&edited, b"changed by the user").unwrap();
        // 第一、三张的保存时间改成 10 天前（到期）；第二张被用户改过，不再算自动保存的文件。
        let mut entries = load(&record).entries;
        for entry in entries.iter_mut() {
            if entry.path != edited {
                entry.saved -= 10 * 86_400;
            }
        }
        store(&record, entries).unwrap();
        assert_eq!(clean_record(&record, 0, SystemTime::now()).unwrap(), 0);
        assert_eq!(clean_record(&record, 30, SystemTime::now()).unwrap(), 0);
        assert_eq!(clean_record(&record, 7, SystemTime::now()).unwrap(), 2);
        assert!(!kept.exists() && !expired.exists());
        assert!(edited.exists() && own.exists());
        // 被改过的文件已移出记录，以后也不会再被清理。
        assert!(load(&record).entries.is_empty());
    }

    #[test]
    fn corrupt_record_is_treated_as_empty() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("auto-saved.json");
        fs::write(&record, b"broken").unwrap();
        assert_eq!(clean_record(&record, 7, SystemTime::now()).unwrap(), 0);
    }
}
