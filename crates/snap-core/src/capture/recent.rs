//! 最近一次成功导出（复制、保存、贴图）的选区，用于“沿用上次选区”与“最近选区 R”。
//!
//! 对应前端 `recent-selection.ts`：只记住范围，不保存截图内容或标注；显示器布局（`layout_key`，
//! 即显示器列表的 JSON）或截图尺寸变化时不恢复，不猜测原来的目标位置。
//! Tauri 版存在 WebView localStorage；GPUI 版没有 WebView，存为数据目录下的 JSON 文件。
use super::{Display, Rect};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSelection {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub layout_key: String,
    pub rect: Rect,
}

/// 显示器布局键：与 Tauri 版 `Frame::layout_key` 相同（显示器列表的 JSON）。
pub fn layout_key(displays: &[Display]) -> String {
    serde_json::to_string(displays).unwrap_or_default()
}

/// 文件可能被旧版本或外部修改：恢复前重新验证完整的像素边界。
fn valid(recent: &RecentSelection) -> bool {
    recent.version == 1
        && recent.width > 0
        && recent.height > 0
        && recent.rect.width > 0
        && recent.rect.height > 0
        && u64::from(recent.rect.x) + u64::from(recent.rect.width) <= u64::from(recent.width)
        && u64::from(recent.rect.y) + u64::from(recent.rect.height) <= u64::from(recent.height)
}

/// 当前截图尺寸与布局一致时返回上次的选区。
pub fn restore(recent: Option<&RecentSelection>, width: u32, height: u32, layout_key: &str) -> Option<Rect> {
    recent
        .filter(|r| valid(r) && r.width == width && r.height == height && r.layout_key == layout_key)
        .map(|r| r.rect)
}

pub fn default_path() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("recent-selection.json"))
}

pub fn load(path: &Path) -> Option<RecentSelection> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<RecentSelection>(&bytes).ok().filter(valid)
}

/// 写入失败只影响下次恢复，不影响本次导出；调用方记录日志即可。
pub fn save(path: &Path, rect: Rect, width: u32, height: u32, layout_key: String) -> Result<RecentSelection, String> {
    let recent = RecentSelection { version: 1, width, height, layout_key, rect };
    if !valid(&recent) {
        return Err("选区超出截图边界。".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(&recent).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    Ok(recent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn saves_and_restores_only_for_same_size_and_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("recent.json");
        save(&path, rect(10, 20, 300, 200), 1920, 1080, "a".into()).unwrap();
        let recent = load(&path);
        assert_eq!(restore(recent.as_ref(), 1920, 1080, "a"), Some(rect(10, 20, 300, 200)));
        assert_eq!(restore(recent.as_ref(), 2560, 1440, "a"), None);
        assert_eq!(restore(recent.as_ref(), 1920, 1080, "b"), None);
        assert_eq!(restore(None, 1920, 1080, "a"), None);
    }

    #[test]
    fn rejects_out_of_bounds_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        assert!(save(&path, rect(1900, 0, 100, 10), 1920, 1080, String::new()).is_err());
        std::fs::write(&path, br#"{"version":1,"width":10,"height":10,"layoutKey":"","rect":{"x":5,"y":5,"width":9,"height":1}}"#).unwrap();
        assert!(load(&path).is_none());
        std::fs::write(&path, b"not json").unwrap();
        assert!(load(&path).is_none());
    }
}
