//! 剪贴板配置与 IPC 契约；旧配置缺少整个对象或新字段时补默认值。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ClipboardSettings {
    pub enabled: bool,
    pub max_items: u32,
    pub max_storage_mb: u32,
    pub max_text_bytes: u32,
    pub max_image_pixels: u32,
    pub save_copied_images: bool,
    pub save_copied_text: bool,
    pub save_copied_files: bool,
    pub retain_days: u32,
}
impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_items: 500,
            max_storage_mb: 200,
            max_text_bytes: 1_048_576,
            max_image_pixels: 16_000_000,
            save_copied_images: true,
            save_copied_text: true,
            save_copied_files: true,
            retain_days: 0,
        }
    }
}
impl ClipboardSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(100..=5000).contains(&self.max_items)
            || !(50..=2048).contains(&self.max_storage_mb)
            || !(1..=1_048_576).contains(&self.max_text_bytes)
            || !(1..=16_000_000).contains(&self.max_image_pixels)
            || self.retain_days > 3650
        {
            return Err("历史限制无效：100～5000 条、50～2048 MiB、文本至多 1 MiB、图片至多 1600 万像素、保留期 0～3650 天。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Image,
    File,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::File => "file",
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Query {
    pub keyword: String,
    pub kind: Option<Kind>,
    pub favorites_only: bool,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
impl Query {
    pub fn validate(&self) -> Result<(u32, i64, i64), String> {
        let limit = self.limit.unwrap_or(40);
        if !(1..=100).contains(&limit) || self.keyword.chars().count() > 200 {
            return Err("每页须为 1～100 条，搜索词最多 200 字。".into());
        }
        let (time, id) = match &self.cursor {
            None => (i64::MAX, i64::MAX),
            Some(value) => {
                let (time, id) = value.split_once(':').ok_or("分页游标无效。")?;
                (
                    time.parse::<i64>().map_err(|_| "分页游标无效。")?,
                    id.parse::<i64>().map_err(|_| "分页游标无效。")?,
                )
            }
        };
        if time < 0 || id < 1 {
            return Err("分页游标无效。".into());
        }
        Ok((limit, time, id))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: i64,
    pub kind: Kind,
    pub source: String,
    pub preview: String,
    pub mime_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub byte_size: u64,
    pub favorite: bool,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub seen_count: u64,
    /// 文件条目提供路径供展示；图片与文本不包含此字段。恢复仍只能提交记录 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_paths: Option<Vec<String>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
    pub revision: u64,
}

/// 仅 Rust 内部携带文件引用，前端永远不能提交路径读取素材。
pub struct Stored {
    pub item: Item,
    pub text: Option<String>,
    pub blob: Option<String>,
    pub thumb: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub supported: bool,
    pub count: u64,
    pub byte_size: u64,
    pub revision: u64,
    pub error: Option<String>,
}
pub fn validate_ids(ids: &[i64]) -> Result<(), String> {
    if ids.is_empty()
        || ids.len() > 100
        || ids.iter().any(|id| *id <= 0)
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
    {
        return Err("请选择 1～100 个不同的有效条目。".into());
    }
    Ok(())
}
