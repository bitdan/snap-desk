//! 录制中的实时标注会话：外壳提交整份标注，这里对账出媒体时间轨并重建编码用的不可变标注层。
//!
//! 坐标为录制区域的物理像素（左上角为原点）。编码线程只短暂取得 `Arc<Layer>`，
//! 修订号变化时才换用新层；暂停期间媒体时间不推进，时效与淡出也随之冻结。
use super::{
    annotation_layer::{self, Layer},
    annotation_track::{self, TimedAnnotation},
};
use crate::annotations::{self, Annotation};
use std::sync::Arc;

/// 录制中最多同时存在的实时滤镜（马赛克、模糊、放大镜逐帧取样，开销随数量增长）。
pub const MAX_LIVE_FILTERS: usize = 8;
/// 新标注的默认淡出时长（仅在设置了时效时生效）。
pub const DEFAULT_FADE_MS: u64 = 500;
/// 允许的标注时效：0 为常驻，其余为毫秒。
pub const LIFETIMES: [u64; 3] = [0, 3000, 5000];

pub struct Live {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// 每次提交加一；编码线程据此判断是否换层。
    pub revision: u64,
    /// 正在编辑（键鼠特效暂停，避免录下标注工具栏的按键提示）。
    pub editing: bool,
    /// 当前标注（录制区域坐标），即编辑器里的内容。
    pub shapes: Vec<Annotation>,
    /// 当前标注对应的时间轨，按图层顺序排列。
    pub tracks: Vec<TimedAnnotation>,
    layer: Arc<Layer>,
    history: Vec<TimedAnnotation>,
    current: Vec<String>,
}

impl Live {
    /// `shapes` 已是录制区域坐标（见 [`annotation_layer::region_shapes`]），从第 0 毫秒起常驻。
    pub fn new(id: u64, shapes: Vec<Annotation>, width: u32, height: u32) -> Result<Self, String> {
        let (layer, history, current) = annotation_layer::initial_layer(&shapes, width, height)?;
        Ok(Self {
            id,
            width,
            height,
            revision: 0,
            editing: false,
            tracks: history.clone(),
            shapes,
            layer: Arc::new(layer),
            history,
            current,
        })
    }

    /// 编码线程读取的标注层与修订号。
    pub fn snapshot(&self) -> (u64, Arc<Layer>) {
        (self.revision, self.layer.clone())
    }

    /// 是否仍有带时效的标注未结束（含 100 ms 余量），需要逐帧重新合成淡出。
    pub fn has_timed(&self, ms: u64) -> bool {
        self.tracks
            .iter()
            .any(|t| t.end_ms.is_some_and(|end| ms <= end.saturating_add(100)))
    }

    /// 已到时效的当前标注下标（按 `shapes` 顺序）；外壳据此从编辑器移除。
    pub fn expired(&self, ms: u64) -> Vec<usize> {
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.end_ms.is_some_and(|end| ms >= end))
            .map(|(index, _)| index)
            .collect()
    }

    /// 提交整份标注：校验、对账（未变的标注保留起始时间，移除的标注在 `ms` 结束）并重建标注层。
    /// `lifetime` 为 0 时新标注常驻；失败时会话保持原状。
    pub fn commit(
        &mut self,
        shapes: Vec<Annotation>,
        ms: u64,
        lifetime: u64,
        fade: u64,
    ) -> Result<(), String> {
        if !LIFETIMES.contains(&lifetime) || fade > 1000 {
            return Err("标注时效参数无效。".into());
        }
        annotations::validate_live(&shapes, self.width, self.height)?;
        if shapes
            .iter()
            .filter(|s| annotations::dynamic_filter(s))
            .count()
            > MAX_LIVE_FILTERS
        {
            return Err("录制中最多支持 8 个实时滤镜。".into());
        }
        let mut history = self.history.clone();
        let current =
            annotation_track::reconcile(&mut history, &self.current, &shapes, ms, lifetime, fade)?;
        let mut tracks: Vec<TimedAnnotation> = history
            .iter()
            .filter(|t| current.contains(&t.id))
            .cloned()
            .collect();
        tracks.sort_by_key(|t| t.order);
        let layer = Layer::timed(tracks.clone(), self.width, self.height)?;
        self.history = history;
        self.current = current;
        self.tracks = tracks;
        self.shapes = shapes;
        self.layer = Arc::new(layer);
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64) -> Annotation {
        serde_json::from_value(serde_json::json!({
            "kind": "rectangle",
            "start": {"x": x, "y": 0},
            "end": {"x": x + 10.0, "y": 10},
            "color": "#ff0000",
            "width": 2
        }))
        .unwrap()
    }

    #[test]
    fn commits_keep_unchanged_shapes_and_expire_timed_ones() {
        let mut live = Live::new(7, vec![rect(0.0)], 100, 50).unwrap();
        assert_eq!(live.snapshot().0, 0);
        assert!(!live.has_timed(0));
        live.commit(vec![rect(0.0), rect(20.0)], 1000, 3000, DEFAULT_FADE_MS)
            .unwrap();
        assert_eq!(live.revision, 1);
        assert_eq!(live.tracks.len(), 2);
        // 原有标注保留第 0 毫秒起常驻，新标注 1 s 起、4 s 结束。
        assert_eq!((live.tracks[0].start_ms, live.tracks[0].end_ms), (0, None));
        assert_eq!(
            (live.tracks[1].start_ms, live.tracks[1].end_ms),
            (1000, Some(4000))
        );
        assert!(live.has_timed(3000));
        assert!(live.expired(3999).is_empty());
        assert_eq!(live.expired(4000), vec![1]);
        live.commit(vec![rect(0.0)], 4000, 0, 0).unwrap();
        assert_eq!(live.tracks.len(), 1);
        assert!(live.expired(9000).is_empty());
    }

    #[test]
    fn invalid_commits_leave_the_session_unchanged() {
        let mut live = Live::new(1, Vec::new(), 100, 50).unwrap();
        assert!(live.commit(vec![rect(95.0)], 0, 0, 0).is_err());
        assert!(live.commit(vec![rect(0.0)], 0, 1234, 0).is_err());
        assert_eq!((live.revision, live.tracks.len()), (0, 0));
    }
}
