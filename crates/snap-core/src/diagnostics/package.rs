//! 跨平台诊断 ZIP：只写四个固定条目，复用 zip 与 tempfile，失败自动清理临时包。
//! 元数据（版本、平台、OCR/翻译/录屏能力摘要）由外壳生成。
use super::logs;
use std::{io::Write, path::Path};

pub const README: &str = "Snap Desk local diagnostics\nThis package contains a bounded, allowlisted log snapshot and minimal runtime metadata only.\nNo configuration files, credentials, clipboard database, OCR/translation text, screenshots or recordings are included.\nUnknown/legacy messages and unapproved fields were omitted. Review before sharing; no automatic upload occurs.\nThe error count and date range refer only to this bounded snapshot, not the complete history.\n";

/// 诊断包文件名（含本地时间）。
pub fn filename() -> String {
    format!(
        "snap-desk-diagnostics-{}.zip",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )
}

/// 诊断包只能保存为 .zip。
pub fn validate_destination(destination: &Path) -> Result<(), String> {
    if destination
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case("zip"))
    {
        return Err("诊断包必须保存为 .zip 文件。".into());
    }
    Ok(())
}

/// 元数据中的日志统计（与快照一致）。
pub fn log_summary(recent: &logs::Recent) -> serde_json::Value {
    serde_json::json!({
        "entries": recent.entries.len(), "scannedBytes": recent.scanned_bytes,
        "omittedLines": recent.omitted_lines, "truncated": recent.truncated,
        "errors": recent.entries.iter().filter(|e| e.level == logs::Level::Error).count(),
        "from": recent.entries.last().map(|e| &e.timestamp), "to": recent.entries.first().map(|e| &e.timestamp),
        "containsRawMedia": false
    })
}

pub fn write_package(
    destination: &Path,
    metadata: &serde_json::Value,
    recent: &logs::Recent,
) -> Result<(), String> {
    let parent = destination.parent().ok_or("输出目录不可用。")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".snap-desk-diagnostic-")
        .tempfile_in(parent)
        .map_err(|_| "无法创建诊断包，请检查目录权限与磁盘空间。")?;
    let mut zip = zip::ZipWriter::new(temporary.as_file_mut());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut recent_text = String::new();
    let mut errors = String::new();
    for entry in recent.entries.iter().rev() {
        let line = format!(
            "{} [{:?}] [{:?}] {}\n",
            entry.timestamp, entry.level, entry.module, entry.message
        );
        recent_text.push_str(&line);
        if entry.level == logs::Level::Error {
            errors.push_str(&line);
        }
    }
    let json = serde_json::to_string_pretty(metadata).map_err(|_| "无法生成诊断元数据。")?;
    // 即使以后扩充字段，也保留压缩前硬上限。
    if recent_text.len() + errors.len() + json.len() + README.len() > 10 * 1024 * 1024 {
        return Err("诊断包超过大小上限。".into());
    }
    for (name, text) in [
        ("diagnostic.json", json.as_str()),
        ("logs/recent.log", recent_text.as_str()),
        ("logs/errors.log", errors.as_str()),
        ("README.txt", README),
    ] {
        zip.start_file(name, options)
            .map_err(|_| "创建诊断包条目失败。")?;
        zip.write_all(text.as_bytes())
            .map_err(|_| "写入诊断包失败，请检查磁盘空间。")?;
    }
    zip.finish().map_err(|_| "完成诊断包失败。")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "同步诊断包失败。")?;
    // 同目录原子发布；失败保留已有目标文件，临时包由 RAII 清理。
    temporary
        .persist(destination)
        .map_err(|_| "保存诊断包失败，目标文件可能被占用。")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_has_only_fixed_entries_and_failed_commit_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagnostics.zip");
        let recent = logs::Recent {
            entries: vec![],
            scanned_bytes: 0,
            omitted_lines: 0,
            truncated: false,
        };
        write_package(&path, &serde_json::json!({"schemaVersion":1}), &recent).unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut names = zip.file_names().collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "README.txt",
                "diagnostic.json",
                "logs/errors.log",
                "logs/recent.log"
            ]
        );
        let occupied = dir.path().join("occupied.zip");
        std::fs::create_dir(&occupied).unwrap();
        assert!(write_package(&occupied, &serde_json::json!({}), &recent).is_err());
        assert!(occupied.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn only_zip_destinations_are_accepted() {
        assert!(validate_destination(Path::new("C:/out/diag.ZIP")).is_ok());
        assert!(validate_destination(Path::new("C:/out/diag.txt")).is_err());
        assert!(filename().starts_with("snap-desk-diagnostics-"));
    }
}
