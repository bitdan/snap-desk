//! 只从应用日志目录读取有限尾部；拒绝链接和未知文件名，展示与导出共用保守白名单。
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub const MAX_SCAN: u64 = 2 * 1024 * 1024;
pub const MAX_LINES: usize = 500;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    App,
    Capture,
    Scroll,
    Recording,
    Ocr,
    Translation,
    Clipboard,
    Frontend,
    Diagnostics,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub limit: usize,
    pub level: Option<Level>,
    pub module: Option<Module>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub timestamp: String,
    pub level: Level,
    pub module: Module,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recent {
    pub entries: Vec<Entry>,
    pub scanned_bytes: u64,
    pub omitted_lines: usize,
    pub truncated: bool,
}

pub struct LogFile {
    pub path: PathBuf,
    pub active: bool,
    pub size: u64,
}

fn linked(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // 同时拒绝 NTFS junction 与其他重解析点。
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// 验证目录及祖先，避免把固定 LogDir 通过链接指向其他数据。
pub fn safe_directory(root: &Path) -> Result<(), String> {
    for path in root.ancestors() {
        let meta = fs::symlink_metadata(path).map_err(|_| "日志目录不可用或无读取权限。")?;
        if linked(&meta) || !meta.is_dir() {
            return Err("日志目录不允许使用链接。".into());
        }
    }
    Ok(())
}

/// 与插件 2.10 的命名契约一致；不把相似前缀、备份和用户文件当作归档。
pub fn archive_name(name: &str, base: &str) -> bool {
    let Some(date) = name
        .strip_prefix(&format!("{base}_"))
        .and_then(|s| s.strip_suffix(".log"))
    else {
        return false;
    };
    date.len() == 19
        && date.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 | 13 | 16 => byte == b'-',
            10 => byte == b'_',
            _ => byte.is_ascii_digit(),
        })
        && chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%d_%H-%M-%S").is_ok()
}

pub fn files(root: &Path, base: &str) -> Result<Vec<LogFile>, String> {
    safe_directory(root)?;
    let mut files = Vec::new();
    for (index, entry) in fs::read_dir(root)
        .map_err(|_| "无法读取日志目录。")?
        .enumerate()
    {
        if index >= 128 {
            return Err("日志目录文件过多，请手动检查。".into());
        }
        let entry = entry.map_err(|_| "无法读取日志目录项。")?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let active = name == format!("{base}.log");
        if !active && !archive_name(name, base) {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path()).map_err(|_| "日志正在轮转，请刷新重试。")?;
        if linked(&meta) || !meta.is_file() {
            return Err("日志文件不允许使用链接或特殊文件。".into());
        }
        files.push(LogFile {
            path: entry.path(),
            active,
            size: meta.len(),
        });
    }
    files.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| b.path.cmp(&a.path)));
    Ok(files)
}

/// 只能删除已枚举的归档；不递归、不删除活动文件，返回部分失败数量供用户核对。
pub fn clear_archives(root: &Path, base: &str) -> Result<(usize, usize), String> {
    let mut deleted = 0;
    let mut failed = 0;
    for file in files(root, base)?.into_iter().filter(|f| !f.active) {
        safe_directory(root)?;
        // remove_file 删除链接本身，不跟随链接删除目标。
        match fs::remove_file(file.path) {
            Ok(()) => deleted += 1,
            Err(_) => failed += 1,
        }
    }
    Ok((deleted, failed))
}

fn open(file: &LogFile) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 直接打开重解析点本身并检查句柄，防止枚举与打开之间的链接替换。
        options.custom_flags(0x00200000);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // O_NOFOLLOW
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100); // O_NOFOLLOW
    }
    let handle = options
        .open(&file.path)
        .map_err(|_| "日志被占用、已轮转或无读取权限，请重试。")?;
    let meta = handle.metadata().map_err(|_| "无法读取日志属性。")?;
    if linked(&meta) || !meta.is_file() {
        return Err("日志文件类型不安全。".into());
    }
    Ok(handle)
}

/// 从时间最近的文件往前读；预算覆盖所有文件，不逐个扫描完整文件。
pub fn recent(files: &[LogFile], query: &Query) -> Result<Recent, String> {
    if !(1..=MAX_LINES).contains(&query.limit) {
        return Err("日志条数必须在 1–500 之间。".into());
    }
    let mut result = Recent {
        entries: Vec::new(),
        scanned_bytes: 0,
        omitted_lines: 0,
        truncated: false,
    };
    for file in files {
        if result.scanned_bytes >= MAX_SCAN || result.entries.len() >= query.limit {
            result.truncated = true;
            break;
        }
        let mut handle = open(file)?;
        let length = handle.metadata().map_err(|_| "无法读取日志大小。")?.len();
        let count = length.min(MAX_SCAN - result.scanned_bytes);
        let offset = length - count;
        handle
            .seek(SeekFrom::Start(offset))
            .map_err(|_| "无法定位日志尾部。")?;
        let mut bytes = Vec::with_capacity(count as usize);
        handle
            .take(count)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取日志失败。")?;
        result.scanned_bytes += bytes.len() as u64;
        result.truncated |= offset > 0;
        // 尾部 seek 可能落在 UTF-8/行中间；丢弃首个不完整行，不拼出伪日志。
        let start = if offset > 0 {
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map(|p| p + 1)
                .unwrap_or(bytes.len())
        } else {
            0
        };
        // 正在写入的末行也不视为完整记录。
        let end = bytes
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(start);
        let text = String::from_utf8_lossy(&bytes[start..end.max(start)]);
        for line in text.lines().rev() {
            if result.entries.len() >= query.limit {
                result.truncated = true;
                break;
            }
            let Some(entry) = sanitize(line) else {
                result.omitted_lines += 1;
                continue;
            };
            if query.level.is_some_and(|l| l != entry.level)
                || query.module.is_some_and(|m| m != entry.module)
            {
                continue;
            }
            result.entries.push(entry);
        }
    }
    Ok(result)
}

/// 二次隐私保护采取重建而非正则替换：自由文本/未知字段/旧日志一律不输出。
pub fn sanitize(line: &str) -> Option<Entry> {
    if line.len() > 4096 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let timestamp = parts.next()?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .to_rfc3339();
    let level = match parts.next()? {
        "[ERROR]" => Level::Error,
        "[WARN]" => Level::Warn,
        "[INFO]" => Level::Info,
        "[DEBUG]" => Level::Debug,
        _ => return None,
    };
    let _target = parts.next()?;
    let fields: Vec<_> = parts.filter_map(|s| s.split_once('=')).collect();
    let event = fields.iter().find(|(k, _)| *k == "event")?.1;
    if !super::catalog::EVENTS.contains(&event) {
        return None;
    }
    let module = match fields.iter().find(|(k, _)| *k == "module")?.1 {
        "app" => Module::App,
        "capture" => Module::Capture,
        "scroll" => Module::Scroll,
        "recording" => Module::Recording,
        "ocr" => Module::Ocr,
        "translation" => Module::Translation,
        "clipboard" => Module::Clipboard,
        "frontend" => Module::Frontend,
        "diagnostics" => Module::Diagnostics,
        _ => return None,
    };
    let mut safe = vec![format!("event={event}")];
    for (key, value) in fields {
        let allowed = match key {
            "reason" if event == "pins.memory" => {
                ["created", "destroyed", "transformed"].contains(&value)
            }
            "count" | "rgba_bytes" if event == "pins.memory" => value.parse::<u64>().is_ok(),
            "reason" if event == "scroll.rejected" => [
                "none",
                "fixed_bounds_changed",
                "anchor_fixed_bounds_changed",
                "anchor_position_conflict",
                "alignment_missing",
                "axis_changed",
                "stationary_overlay",
                "seam_conflict",
                "canvas_conflict",
            ]
            .contains(&value),
            "axis" if event == "scroll.rejected" => {
                ["Vertical", "Horizontal", "Auto"].contains(&value)
            }
            "error_kind" if module == Module::Frontend => [
                "Error",
                "TypeError",
                "ReferenceError",
                "SyntaxError",
                "RangeError",
                "URIError",
                "EvalError",
                "AggregateError",
                "DOMException",
                "non_error",
                "unknown",
            ]
            .contains(&value),
            "window" if module == Module::Frontend => [
                "main",
                "capture",
                "pin",
                "selection-edge",
                "recording-controls",
                "scroll-preview",
                "scroll-controls",
                "unknown",
            ]
            .contains(&value),
            "previous_started_ms" | "previous_pid" if event == "app.previous_exit_unclean" => {
                value.parse::<u64>().is_ok()
            }
            "code" => super::catalog::CODES.contains(&value),
            "command" => super::catalog::COMMANDS.contains(&value),
            "op" => ["cap-", "rec-", "scroll-", "ocr-", "tr-"]
                .iter()
                .any(|prefix| {
                    value.strip_prefix(*prefix).is_some_and(|n| {
                        !n.is_empty() && n.len() <= 20 && n.bytes().all(|c| c.is_ascii_digit())
                    })
                }),
            "provider" => ["local", "cloud"].contains(&value),
            "queued" | "queue_dropped" | "throttled" | "rejected_capture" | "stale"
                if event == "recording.finished" => value.parse::<u64>().is_ok(),
            "result" => ["success", "cancelled", "failed"].contains(&value),
            "stage" if event == "recording.step_failed" => [
                "encoder_thread_start",
                "encoder_thread_panic",
                "capture_state",
                "capture_refresh",
                "capture_source_check",
                "display_changed",
                "input_start",
                "input_read",
                "capture_frame",
                "capture_timeout",
                "output_config",
                "recovery_create",
                "encoder_init",
                "encoder_state",
                "audio_pump",
                "video_advance",
                "frame_state",
                "annotation_apply",
                "video_write",
                "audio_stop",
                "audio_finalize",
                "recovery_update",
                "encoder_finalize",
                "file_open",
                "file_sync",
                "file_validate",
                "publish_state",
                "file_publish",
            ]
            .contains(&value),
            "stage" | "previous" => [
                "idle",
                "preparing",
                "recording",
                "encoding",
                "finalizing",
                "syncing",
                "publishing",
                "completed",
            ]
            .contains(&value),
            "hide_ms" | "dwm_ms" | "capture_ms" | "png_ms" | "overlay_ready_ms" | "first_paint_ms"
            | "delay_ms" | "load_ms" | "decode_ms" | "pre_paint_ms" | "post_paint_ms" | "clone_ms" | "compose_ms"
                if event.starts_with("capture.") => value.len() <= 20 && value.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.0),
            "compose_ms" | "png_ms" | "clipboard_ms" if event == "scroll.export" => value.len() <= 20 && value.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.0),
            "rgba_bytes" if event == "scroll.export" => value.parse::<u64>().is_ok(),
            "paint_source" if event == "capture.interactive" => ["raf", "timeout"].contains(&value),
            "revision" | "monitors" | "png_bytes" if event.starts_with("capture.") => value.parse::<u64>().is_ok(),
            "cold" if event.starts_with("capture.") => ["0", "1"].contains(&value),
            "duration_ms" | "inference_ms" | "encoding_ms" | "width" | "height" | "frames"
            | "captured" | "encoded" | "dropped" | "fps" | "lines" | "line" | "column" => {
                value.len() <= 20
                    && value
                        .parse::<f64>()
                        .is_ok_and(|n| n.is_finite() && n >= 0.0)
            }
            _ => false,
        };
        if allowed {
            safe.push(format!("{key}={value}"));
        }
    }
    Some(Entry {
        timestamp,
        level,
        module,
        message: safe.join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_details_survive_export_without_free_text() {
        let prefix = "2026-10-29T12:00:00+08:00 [ERROR] [test]";
        for (fields, expected) in [
            ("event=frontend.uncaught_error module=frontend window=pin error_kind=TypeError line=42 column=7 message=secret stack=private", "event=frontend.uncaught_error window=pin error_kind=TypeError line=42 column=7"),
            ("event=translation.failed module=translation op=tr-12 provider=local code=LOCAL_INPUT_TOO_LONG text=secret", "event=translation.failed op=tr-12 provider=local code=LOCAL_INPUT_TOO_LONG"),
            ("event=recording.step_failed module=recording op=rec-7 code=RECORDING_STEP_FAILED stage=encoder_init path=private", "event=recording.step_failed op=rec-7 code=RECORDING_STEP_FAILED stage=encoder_init"),
            ("event=recording.finished module=recording queued=10 queue_dropped=2 throttled=30 rejected_capture=4 stale=1", "event=recording.finished queued=10 queue_dropped=2 throttled=30 rejected_capture=4 stale=1"),
            ("event=recording.finished module=recording queued=-1 queue_dropped=NaN throttled=secret", "event=recording.finished"),
            ("event=scroll.rejected module=scroll reason=anchor_fixed_bounds_changed axis=Vertical frames=9", "event=scroll.rejected reason=anchor_fixed_bounds_changed axis=Vertical frames=9"),
            ("event=frontend.uncaught_error module=frontend window=private error_kind=secret", "event=frontend.uncaught_error"),
            ("event=recording.step_failed module=recording stage=private code=secret", "event=recording.step_failed"),
            ("event=scroll.rejected module=scroll reason=private axis=private", "event=scroll.rejected"),
        ] {
            assert_eq!(sanitize(&format!("{prefix} {fields}")).unwrap().message, expected);
        }
    }

    #[test]
    fn capture_latency_preserves_safe_stages_and_rejects_content_and_invalid_times() {
        let line = "2026-10-28T12:00:00+08:00 [DEBUG] [capture] event=capture.interactive module=capture op=cap-3 hide_ms=0 dwm_ms=1.5 capture_ms=8 png_ms=20 overlay_ready_ms=3 first_paint_ms=140 load_ms=4 decode_ms=2 pre_paint_ms=100 post_paint_ms=32 delay_ms=0 revision=0 width=3840 height=2160 monitors=2 cold=0 png_bytes=123 paint_source=raf image=private";
        let safe = sanitize(line).unwrap().message;
        for field in ["hide_ms=0", "dwm_ms=1.5", "capture_ms=8", "png_ms=20", "overlay_ready_ms=3", "first_paint_ms=140", "monitors=2", "paint_source=raf"] { assert!(safe.contains(field)); }
        assert!(!safe.contains("private"));
        let invalid = line.replace("decode_ms=2", "decode_ms=NaN").replace("paint_source=raf", "paint_source=private");
        let safe = sanitize(&invalid).unwrap().message;
        assert!(!safe.contains("decode_ms"));
        assert!(!safe.contains("paint_source"));
        for event in ["capture.prepared", "capture.visible", "capture.native_replaced"] { assert!(sanitize(&line.replace("capture.interactive", event)).is_some()); }
    }
    #[test]
    fn abnormal_exit_log_preserves_only_numeric_run_identity() {
        let line = "2026-10-23T16:00:00+08:00 [WARN] [app] event=app.previous_exit_unclean module=app previous_started_ms=1792727969001 previous_pid=42 path=private";
        assert_eq!(
            sanitize(line).unwrap().message,
            "event=app.previous_exit_unclean previous_started_ms=1792727969001 previous_pid=42"
        );
        let invalid = line.replace("previous_pid=42", "previous_pid=private");
        assert!(!sanitize(&invalid).unwrap().message.contains("previous_pid"));
    }
    #[test]
    fn pin_memory_diagnostics_keep_only_counts_and_known_reasons() {
        let line = "2026-10-21T09:00:00+08:00 [INFO] [pins] event=pins.memory module=capture reason=destroyed count=0 rgba_bytes=0 image=private";
        assert_eq!(
            sanitize(line).unwrap().message,
            "event=pins.memory reason=destroyed count=0 rgba_bytes=0"
        );
        let invalid = line
            .replace("reason=destroyed", "reason=private")
            .replace("count=0", "count=-1")
            .replace("rgba_bytes=0", "rgba_bytes=NaN");
        assert_eq!(sanitize(&invalid).unwrap().message, "event=pins.memory");
    }
    #[test]
    fn rejects_paths_and_legacy_secrets() {
        for name in [
            "../snap-desk_2026-10-19_12-00-00.log",
            "snap-desk-secret.log",
            "snap-desk_2026-99-19_12-00-00.log",
            "snap-desk.log",
            "snap-desk_2026-10-19_12-00-00.log.bak",
        ] {
            assert!(!archive_name(name, "snap-desk"));
        }
        assert!(archive_name(
            "snap-desk_2026-10-19_12-00-00.log",
            "snap-desk"
        ));
        assert!(sanitize("legacy token=secret OCR正文").is_none());
        let entry = sanitize("2026-10-19T12:00:00+08:00 [ERROR] [private/path] event=ipc.failed module=frontend command=app_status token=secret email=user@example.com path=C:\\Users\\private").unwrap();
        assert_eq!(entry.message, "event=ipc.failed command=app_status");
    }
    #[test]
    fn bounds_reads_and_ignores_partial_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snap-desk.log");
        let line = "2026-10-19T12:00:00+08:00 [INFO] [app] event=app.starting module=app\n";
        fs::write(&path, format!("{}unfinished", line.repeat(40_000))).unwrap();
        let files = vec![LogFile {
            path,
            active: true,
            size: 0,
        }];
        let result = recent(
            &files,
            &Query {
                limit: 500,
                level: None,
                module: None,
            },
        )
        .unwrap();
        assert_eq!(result.entries.len(), 500);
        assert!(result.scanned_bytes <= MAX_SCAN && result.truncated);
        assert!(recent(
            &files,
            &Query {
                limit: 501,
                level: None,
                module: None
            }
        )
        .is_err());
    }

    #[test]
    fn clearing_keeps_active_log_and_unknown_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for name in [
            "snap-desk.log",
            "snap-desk_2026-10-19_12-00-00.log",
            "other.log",
            "snap-desk_settings.json",
        ] {
            fs::write(root.join(name), "unchanged").unwrap();
        }
        assert_eq!(clear_archives(&root, "snap-desk").unwrap(), (1, 0));
        assert_eq!(
            fs::read_to_string(root.join("snap-desk.log")).unwrap(),
            "unchanged"
        );
        assert!(root.join("other.log").exists());
        assert!(root.join("snap-desk_settings.json").exists());
    }

    #[cfg(windows)]
    #[test]
    fn locked_logs_report_failure_without_removing_active_log() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("snap-desk_2026-10-19_12-00-00.log");
        fs::write(&path, "unchanged").unwrap();
        let _locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(open(&LogFile {
            path: path.clone(),
            active: false,
            size: 9
        })
        .is_err());
        assert_eq!(clear_archives(&root, "snap-desk").unwrap(), (0, 1));
        assert!(path.exists());
    }

    #[test]
    fn query_rejects_unknown_fields_and_filters_across_archives() {
        for query in [
            r#"{"limit":500,"path":"../secret"}"#,
            r#"{"limit":500,"module":"../../etc"}"#,
            r#"{"limit":500,"level":"TRACE"}"#,
        ] {
            assert!(serde_json::from_str::<Query>(query).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join("snap-desk.log");
        let archive = dir.path().join("snap-desk_2026-10-19_12-00-00.log");
        fs::write(
            &active,
            "2026-10-19T12:00:01+08:00 [INFO] [app] event=app.starting module=app\n",
        )
        .unwrap();
        fs::write(
            &archive,
            "2026-10-19T12:00:00+08:00 [ERROR] [ocr] event=ocr.failed module=ocr code=OCR_BUSY\n",
        )
        .unwrap();
        let files = vec![
            LogFile {
                path: active,
                active: true,
                size: 0,
            },
            LogFile {
                path: archive,
                active: false,
                size: 0,
            },
        ];
        let result = recent(
            &files,
            &Query {
                limit: 10,
                level: Some(Level::Error),
                module: Some(Module::Ocr),
            },
        )
        .unwrap();
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].message.contains("OCR_BUSY"));
        fs::remove_file(&files[0].path).unwrap();
        assert!(recent(
            &files,
            &Query {
                limit: 10,
                level: None,
                module: None
            }
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_files() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let path = dir.path().join("snap-desk.log");
        symlink(outside.path(), &path).unwrap();
        assert!(open(&LogFile {
            path,
            active: true,
            size: 0
        })
        .is_err());
    }
}
