//! 顶部暂存架：剪贴板历史的轻量视图，只持久化设置和固定/隐藏的条目 ID（`shelf.json`），不复制素材。
//! 窗口、悬停唤出与截图期间的隐藏由外壳负责（`apps/desktop-gpui/src/shelf.rs`）。
use crate::clipboard::{
    model::{Item, Kind, Query, TextCategory},
    service::Service,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 架上最多显示的卡片数，也是固定项上限。
pub const MAX_ITEMS: usize = 30;
const MAX_HIDDEN: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShelfCategory {
    Text,
    Image,
    File,
    Link,
    Path,
    Color,
    Favorites,
}

/// 分类取并集；文本分类与历史页一致，包含链接、路径和颜色。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ShelfSettings {
    pub enabled: bool,
    pub top_hover: bool,
    pub categories: Vec<ShelfCategory>,
}
impl Default for ShelfSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            top_hover: false,
            categories: vec![ShelfCategory::Image],
        }
    }
}
impl ShelfSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.categories.len() > 7
            || self
                .categories
                .iter()
                .enumerate()
                .any(|(i, c)| self.categories[..i].contains(c))
        {
            return Err("暂存架类别无效。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShelfAction {
    Keep,
    Unkeep,
    Remove,
    Restore,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Preferences {
    pub settings: ShelfSettings,
    /// 只固定展示顺序，不改变历史收藏和清理规则。
    pub kept_ids: Vec<i64>,
    pub hidden_ids: Vec<i64>,
}

impl Preferences {
    /// 读取 `shelf.json`；文件不存在时为默认值，过大、损坏或条目无效时报错（外壳保留错误并拒绝修改）。
    pub fn load(path: &Path) -> Result<Self, String> {
        let value: Preferences = if path.exists() {
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 65536 {
                return Err("暂存架配置过大。".into());
            }
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|_| "暂存架配置损坏，请检查 shelf.json。".to_string())?
        } else {
            Preferences::default()
        };
        value.settings.validate()?;
        if value.kept_ids.len() > MAX_ITEMS
            || value.hidden_ids.len() > MAX_HIDDEN
            || value
                .kept_ids
                .iter()
                .chain(&value.hidden_ids)
                .any(|id| *id <= 0)
        {
            return Err("暂存架条目配置无效。".into());
        }
        Ok(value)
    }

    /// 临时文件与目标同目录，完整写入并同步后替换；失败时原文件不变。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        self.settings.validate()?;
        let dir = path.parent().ok_or("配置路径无效。")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 放上（固定）、取消固定、从架上移除（隐藏，保留历史）或恢复。`exists` 判断历史中是否仍有该条目，
    /// 先清理已被历史删除的固定引用，避免失效固定项占满位置。
    pub fn apply(
        &mut self,
        id: i64,
        action: ShelfAction,
        exists: impl Fn(i64) -> bool,
    ) -> Result<(), String> {
        self.kept_ids.retain(|value| exists(*value));
        match action {
            ShelfAction::Keep => {
                if !self.kept_ids.contains(&id) {
                    if self.kept_ids.len() >= MAX_ITEMS {
                        return Err("暂存架最多固定 30 项。".into());
                    }
                    self.kept_ids.push(id);
                }
                self.hidden_ids.retain(|value| *value != id);
            }
            ShelfAction::Unkeep => self.kept_ids.retain(|value| *value != id),
            ShelfAction::Remove => {
                self.kept_ids.retain(|value| *value != id);
                self.hidden_ids.retain(|value| *value != id);
                self.hidden_ids.push(id);
                if self.hidden_ids.len() > MAX_HIDDEN {
                    self.hidden_ids.remove(0);
                }
            }
            ShelfAction::Restore => self.hidden_ids.retain(|value| *value != id),
        }
        Ok(())
    }
}

/// 按所选类别合并历史，固定项排在普通项前面。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfSnapshot {
    pub settings: ShelfSettings,
    pub kept_ids: Vec<i64>,
    pub items: Vec<Item>,
    pub recording_enabled: bool,
}

fn query(category: ShelfCategory) -> Query {
    Query {
        kind: match category {
            ShelfCategory::Text => Some(Kind::Text),
            ShelfCategory::Image => Some(Kind::Image),
            ShelfCategory::File => Some(Kind::File),
            _ => None,
        },
        text_category: match category {
            ShelfCategory::Link => Some(TextCategory::Link),
            ShelfCategory::Path => Some(TextCategory::Path),
            ShelfCategory::Color => Some(TextCategory::Color),
            _ => None,
        },
        favorites_only: category == ShelfCategory::Favorites,
        limit: Some(100),
        ..Query::default()
    }
}

/// 同一轮读取持历史操作锁，避免清理素材时拼出不一致的卡片列表。
pub fn snapshot(prefs: &Preferences, service: &Service) -> Result<ShelfSnapshot, String> {
    let recording_enabled = service.status().enabled;
    let data = service.data()?;
    let _guard = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
    let mut items = Vec::new();
    for category in &prefs.settings.categories {
        let mut query = query(*category);
        // 已隐藏条目不占展示名额；必要时翻页，避免移除前 100 项后误报空架。
        let mut visible_count = 0;
        loop {
            let page = data.repo.list(&query, 0)?;
            for item in page.items {
                if !prefs.hidden_ids.contains(&item.id) {
                    items.push(item);
                    visible_count += 1;
                }
            }
            if visible_count >= MAX_ITEMS || page.next_cursor.is_none() {
                break;
            }
            query.cursor = page.next_cursor;
        }
    }
    items.sort_by_key(|item| {
        (
            std::cmp::Reverse(item.last_seen_at),
            std::cmp::Reverse(item.id),
        )
    });
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.id) && !prefs.hidden_ids.contains(&item.id));
    // 固定项在切换类别后继续留在架上；历史删除/清理后自然消失。
    let mut kept = Vec::new();
    for id in &prefs.kept_ids {
        if let Ok(stored) = data.repo.get(*id) {
            kept.push(stored.item);
        }
    }
    items.retain(|item| !prefs.kept_ids.contains(&item.id));
    items.truncate(MAX_ITEMS.saturating_sub(kept.len()));
    kept.extend(items);
    Ok(ShelfSnapshot {
        settings: prefs.settings.clone(),
        kept_ids: prefs.kept_ids.clone(),
        items: kept,
        recording_enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_reject_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shelf.json");
        assert_eq!(Preferences::load(&path).unwrap(), Preferences::default());
        let mut prefs = Preferences::default();
        prefs.settings.enabled = true;
        prefs.settings.categories = vec![ShelfCategory::Text, ShelfCategory::Color];
        prefs.kept_ids = vec![3];
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), prefs);
        std::fs::write(&path, "{").unwrap();
        assert!(Preferences::load(&path).unwrap_err().contains("损坏"));
        std::fs::write(&path, r#"{"keptIds":[0]}"#).unwrap();
        assert!(Preferences::load(&path).unwrap_err().contains("条目"));
        std::fs::write(&path, r#"{"settings":{"categories":["text","text"]}}"#).unwrap();
        assert!(Preferences::load(&path).unwrap_err().contains("类别"));
    }

    #[test]
    fn item_actions_keep_remove_and_restore() {
        let mut prefs = Preferences {
            kept_ids: vec![1, 9],
            ..Default::default()
        };
        // 9 已被历史删除：任何操作都会顺带清理。
        let exists = |id: i64| id != 9;
        prefs.apply(2, ShelfAction::Keep, exists).unwrap();
        assert_eq!(prefs.kept_ids, vec![1, 2]);
        prefs.apply(2, ShelfAction::Remove, exists).unwrap();
        assert_eq!(prefs.kept_ids, vec![1]);
        assert_eq!(prefs.hidden_ids, vec![2]);
        prefs.apply(2, ShelfAction::Restore, exists).unwrap();
        assert!(prefs.hidden_ids.is_empty());
        prefs.apply(1, ShelfAction::Unkeep, exists).unwrap();
        assert!(prefs.kept_ids.is_empty());
        prefs.kept_ids = (100..130).collect();
        assert!(prefs
            .apply(5, ShelfAction::Keep, |_| true)
            .unwrap_err()
            .contains("30"));
    }
}
