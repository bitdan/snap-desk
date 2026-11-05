//! 顶部暂存架是剪贴板历史的轻量视图，只持久化设置和固定 ID，不复制素材。
pub mod commands;
mod window;
pub(crate) use window::{resume_after_capture, suspend_for_capture};

use crate::clipboard::{
    model::{Item, Kind, Query, TextCategory},
    Service as ClipboardService,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

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
    fn validate(&self) -> Result<(), String> {
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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct Preferences {
    settings: ShelfSettings,
    /// 只固定展示顺序，不改变历史收藏和清理规则。
    kept_ids: Vec<i64>,
    hidden_ids: Vec<i64>,
}

/// 保存视图偏好与悬停采样生命周期；历史服务仍拥有全部内容。
pub struct ShelfState {
    preferences: Mutex<Result<Preferences, String>>,
    pub(crate) window_gate: Mutex<()>,
    /// Some 表示采集临时隐藏期间，值记录采集前是否展开；不持久化到设置。
    capture_visibility: Mutex<Option<bool>>,
    stop: AtomicBool,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// 按所选类别合并历史，固定项排在普通项前面。
pub struct ShelfSnapshot {
    pub settings: ShelfSettings,
    pub kept_ids: Vec<i64>,
    pub items: Vec<Item>,
    pub recording_enabled: bool,
}

impl ShelfState {
    fn preferences(&self) -> Result<Preferences, String> {
        self.preferences.lock().map_err(|_| "暂存架忙碌。")?.clone()
    }
    /// 临时文件与目标同目录，替换失败时保留原配置与内存状态。
    fn update(
        &self,
        app: &AppHandle,
        change: impl FnOnce(&mut Preferences) -> Result<(), String>,
    ) -> Result<(), String> {
        use std::io::Write;
        let mut guard = self.preferences.lock().map_err(|_| "暂存架忙碌。")?;
        let mut next = guard.as_ref().map_err(Clone::clone)?.clone();
        change(&mut next)?;
        next.settings.validate()?;
        let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&next).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(dir.join("shelf.json"))
            .map_err(|e| e.to_string())?;
        *guard = Ok(next);
        drop(guard);
        let _ = app.emit_to("shelf", "shelf-changed", ());
        Ok(())
    }
    /// 同一轮读取持历史操作锁，避免清理素材时拼出不一致的卡片列表。
    pub fn snapshot(&self, app: &AppHandle) -> Result<ShelfSnapshot, String> {
        let prefs = self.preferences()?;
        let clipboard = app.state::<ClipboardService>();
        let recording_enabled = clipboard.status().enabled;
        let data = clipboard.data()?;
        let _guard = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let mut items = Vec::new();
        for category in &prefs.settings.categories {
            let mut query = Query {
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
                favorites_only: *category == ShelfCategory::Favorites,
                limit: Some(100),
                ..Query::default()
            };
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
                if visible_count >= 30 || page.next_cursor.is_none() {
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
        items.truncate(30usize.saturating_sub(kept.len()));
        kept.extend(items);
        Ok(ShelfSnapshot {
            settings: prefs.settings,
            kept_ids: prefs.kept_ids,
            items: kept,
            recording_enabled,
        })
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

/// 位置采样仅在开启顶部悬停时运行；隐藏时没有拦截鼠标的透明窗口。
pub fn initialize(app: &AppHandle) {
    let preferences = (|| {
        let path = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("shelf.json");
        let value: Preferences = if path.exists() {
            if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 65536 {
                return Err("暂存架配置过大。".into());
            }
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|_| "暂存架配置损坏，请检查 shelf.json。".to_string())?
        } else {
            Preferences::default()
        };
        value.settings.validate()?;
        if value.kept_ids.len() > 30
            || value.hidden_ids.len() > 1000
            || value
                .kept_ids
                .iter()
                .chain(&value.hidden_ids)
                .any(|id| *id <= 0)
        {
            return Err("暂存架条目配置无效。".into());
        }
        Ok(value)
    })();
    app.manage(ShelfState {
        preferences: Mutex::new(preferences),
        window_gate: Mutex::new(()),
        capture_visibility: Mutex::new(None),
        stop: AtomicBool::new(false),
        worker: Mutex::new(None),
    });
    let handle = app.clone();
    let worker = std::thread::Builder::new()
        .name("shelf-hover".into())
        .spawn(move || {
            let mut hovering = None;
            let mut armed = true;
            loop {
                std::thread::sleep(Duration::from_millis(100));
                let state = handle.state::<ShelfState>();
                if state.stop.load(Ordering::Acquire) {
                    break;
                }
                if !state
                    .preferences()
                    .is_ok_and(|p| p.settings.enabled && p.settings.top_hover)
                {
                    hovering = None;
                    continue;
                }
                if handle
                    .state::<crate::capture::CaptureState>()
                    .active
                    .load(Ordering::Acquire)
                {
                    hovering = None;
                    continue;
                }
                let Ok(point) = handle.cursor_position() else {
                    continue;
                };
                let Ok(Some(monitor)) = handle.monitor_from_point(point.x, point.y) else {
                    continue;
                };
                let scale = monitor.scale_factor();
                let pos = monitor.position();
                let size = monitor.size();
                let top = (point.y - f64::from(pos.y)).abs() <= 2.0 * scale;
                let center = (point.x - (f64::from(pos.x) + f64::from(size.width) / 2.0)).abs()
                    <= 80.0 * scale;
                if top && center {
                    let since = hovering.get_or_insert_with(std::time::Instant::now);
                    if armed && since.elapsed() >= Duration::from_millis(350) {
                        armed = false;
                        // 窗口创建可能等待 UI 线程；采样线程不能等待它，否则退出时 join 会卡住 UI。
                        let app = handle.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = window::show(&app) {
                                crate::capture::report_error(&app, error);
                            }
                        });
                    }
                } else {
                    hovering = None;
                    armed = true;
                }
            }
        });
    if let Ok(worker) = worker {
        *app.state::<ShelfState>().worker.lock().unwrap() = Some(worker);
    }
}

/// 快捷键沿用应用统一映射、冲突校验和注册生命周期；默认不占用新按键。
pub fn toggle(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !handle
            .state::<ShelfState>()
            .preferences()
            .is_ok_and(|p| p.settings.enabled)
        {
            return;
        }
        let result = if handle
            .get_webview_window("shelf")
            .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
            window::hide(&handle)
        } else {
            window::show(&handle)
        };
        if let Err(error) = result {
            crate::capture::report_error(&handle, error);
        }
    });
}
