//! 截图会话回放：只在内存保留成功截图的压缩底图、选区和标注，独立于剪贴板历史。
use super::*;
use std::collections::VecDeque;

const MAX_ENTRIES: usize = 20;
const MAX_BYTES: usize = 128 * 1024 * 1024;

struct Snapshot {
    frame: Frame,
    preview: Arc<Vec<u8>>,
    rect: Rect,
    annotations: Vec<crate::shared::annotations::Annotation>,
}

#[derive(Default)]
pub(super) struct History {
    entries: VecDeque<Snapshot>,
    cursor: Option<usize>,
    // 回放过程中保留本次现场；向前越过最新历史时可返回现场，不重新抓屏。
    live: Option<Box<Session>>,
}
impl History {
    /// 始终保留较新的条目；容量按压缩字节计，绝不靠条数掩盖大图内存。
    fn trim(&mut self, max_entries: usize, max_bytes: usize) {
        while self.entries.len() > max_entries
            || self.entries.iter().map(|s| s.preview.len()).sum::<usize>() > max_bytes
        {
            self.entries.pop_front();
        }
    }
    fn destination(&self, direction: &Direction) -> Option<usize> {
        match (direction, self.cursor) {
            (Direction::Previous, None) => self.entries.len().checked_sub(1),
            (Direction::Previous, Some(index)) => Some(index.saturating_sub(1)),
            (Direction::Next, Some(index)) if index + 1 < self.entries.len() => Some(index + 1),
            (Direction::Next, _) => None,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Replay {
    index: usize,
    total: usize,
    rect: Rect,
    annotations: Vec<crate::shared::annotations::Annotation>,
}

pub(super) fn remember(
    app: &AppHandle,
    id: u64,
    rect: Rect,
    annotations: &[crate::shared::annotations::Annotation],
) {
    let state = app.state::<CaptureState>();
    let Ok(session) = state.session.lock() else {
        return;
    };
    let Some(session) = session
        .as_ref()
        .filter(|s| s.frame.id == id && !s.frame.recording && s.frame.replay.is_none())
    else {
        return;
    };
    // 超过压缩预算的单张图片不加入；不因历史记录失败改变已经完成的复制/保存结果。
    if session.preview.len() > MAX_BYTES
        || u64::from(session.image.width()) * u64::from(session.image.height()) > 32_000_000
    {
        return;
    }
    let Ok(mut history) = state.history.lock() else {
        return;
    };
    let mut frame = session.frame.clone();
    frame.targets.clear();
    history.entries.push_back(Snapshot {
        frame,
        preview: session.preview.clone(),
        rect,
        annotations: annotations.to_vec(),
    });
    history.trim(MAX_ENTRIES, MAX_BYTES);
}

pub(super) fn reset(state: &CaptureState) {
    if let Ok(mut history) = state.history.lock() {
        history.cursor = None;
        history.live = None;
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Previous,
    Next,
}

/// 只允许截图窗口在空闲的静态会话中切换；生成新 ID，令在途识别和图片请求自动过期。
#[tauri::command]
pub async fn replay_capture(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    direction: Direction,
) -> Result<Frame, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        let mut current = state.session.lock().map_err(err)?;
        let session = current
            .as_ref()
            .filter(|s| s.frame.id == id && !s.frame.recording)
            .ok_or("当前截图不能回放。")?;
        let mut history = state.history.lock().map_err(err)?;
        if history.entries.is_empty() {
            return Err("还没有截图历史。".into());
        }
        let next = history.destination(&direction);
        if next == history.cursor {
            return Ok(session.frame.clone());
        }
        let mut restored = if let Some(index) = next {
            let saved = &history.entries[index];
            let image = image::load_from_memory(saved.preview.as_ref())
                .map_err(err)?
                .to_rgba8();
            let mut frame = saved.frame.clone();
            frame.targets.clear(); // 历史 HWND 已失效，不允许对今天的窗口执行“原生重抓”。
            frame.replay = Some(Replay {
                index: index + 1,
                total: history.entries.len(),
                rect: saved.rect,
                annotations: saved.annotations.clone(),
            });
            let desktop = desktop::Desktop {
                image: RgbaImage::new(0, 0),
                x: frame.origin_x,
                y: frame.origin_y,
                displays: frame.displays.clone(),
                targets: Vec::new(),
                windows: Vec::new(),
                include_cursor: false,
            };
            Session {
                frame,
                image,
                shown: true,
                desktop,
                preview: saved.preview.clone(),
                latency: Latency::new(Instant::now()),
            }
        } else {
            *history.live.take().ok_or("当前截图现场不可用。")?
        };
        if history.cursor.is_none() {
            history.live = current.take().map(Box::new);
        }
        restored.frame.id = state.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        restored.frame.preview_revision = 0;
        let frame = restored.frame.clone();
        *current = Some(restored);
        history.cursor = next;
        crate::recognition::translation::cancel_window(&app, "capture");
        Ok(frame)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(id: u64) -> Snapshot {
        Snapshot {
            frame: Frame {
                replay: None,
                id,
                width: 1,
                height: 1,
                preview_revision: 0,
                recording: false,
                snap_enabled: false,
                origin_x: 0,
                origin_y: 0,
                displays: vec![],
                targets: vec![],
                layout_key: String::new(),
            },
            preview: Arc::new(vec![0; 4]),
            rect: Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            annotations: vec![],
        }
    }
    #[test]
    fn bounded_history_keeps_newest_and_returns_to_live_after_last() {
        let mut history = History::default();
        for id in 1..=21 {
            history.entries.push_back(snapshot(id));
        }
        history.trim(20, MAX_BYTES);
        assert_eq!(history.entries.len(), 20);
        assert_eq!(history.entries.front().unwrap().frame.id, 2);
        assert_eq!(history.destination(&Direction::Previous), Some(19));
        history.cursor = Some(0);
        assert_eq!(history.destination(&Direction::Previous), Some(0));
        assert_eq!(history.destination(&Direction::Next), Some(1));
        history.cursor = Some(19);
        assert_eq!(history.destination(&Direction::Next), None);
        history.trim(20, 8);
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries.front().unwrap().frame.id, 20);
    }
}
