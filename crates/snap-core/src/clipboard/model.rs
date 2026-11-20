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
    /// 回车、双击或 Ctrl+数字后，把内容直接粘贴到唤出前的窗口；关闭时只写回系统剪贴板。
    pub paste_on_activate: bool,
    /// 不记录这些程序复制的内容；只保存可执行文件名（如 `KeePass.exe`），比较时不分大小写。
    pub excluded_apps: Vec<String>,
}
impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            // 默认记录：新安装或配置重置后复制即有历史；暂停入口在剪贴板设置。
            enabled: true,
            max_items: 500,
            max_storage_mb: 200,
            max_text_bytes: 1_048_576,
            max_image_pixels: 16_000_000,
            save_copied_images: true,
            save_copied_text: true,
            save_copied_files: true,
            retain_days: 0,
            paste_on_activate: true,
            excluded_apps: Vec::new(),
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
        // 只接受文件名：不含路径分隔符，避免把整条路径当作进程名而永远匹配不上。
        if self.excluded_apps.len() > 50
            || self.excluded_apps.iter().any(|name| {
                name.is_empty()
                    || name.chars().count() > 100
                    || name.trim() != name
                    || name.contains(['/', '\\', ':'])
            })
        {
            return Err("排除应用无效：最多 50 个程序文件名，每个不超过 100 字且不含路径。".into());
        }
        Ok(())
    }
    /// 进程名与排除名单按不区分大小写比较；名单为空时调用方无需查询进程。
    pub fn excludes(&self, process: &str) -> bool {
        self.excluded_apps
            .iter()
            .any(|name| name.eq_ignore_ascii_case(process))
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

/// 文本的内容分类；不改变复制、预览和导出使用的基础类型。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TextCategory {
    Link,
    Color,
    Path,
}
impl TextCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Color => "color",
            Self::Path => "path",
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Query {
    pub keyword: String,
    pub kind: Option<Kind>,
    pub text_category: Option<TextCategory>,
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
    /// 剪贴板窗口据此决定回车是粘贴还是仅复制，无需读取完整应用设置。
    pub paste_on_activate: bool,
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
