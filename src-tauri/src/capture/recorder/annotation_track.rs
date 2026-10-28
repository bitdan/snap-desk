//! 标注的媒体时间轨道：实时展示、逐帧合成和后期编辑共用毫秒时间，不使用墙钟。
use crate::shared::annotations::Annotation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimedAnnotation {
    pub id: String,
    pub shape: Annotation,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub fade_out_ms: u64,
    pub order: u32,
}
impl TimedAnnotation {
    /// 结束时间为半开区间；暂停时媒体时间不变，淡出也保持不变。
    pub fn alpha(&self, ms: u64) -> f64 {
        if ms < self.start_ms || self.end_ms.is_some_and(|end| ms >= end) {
            return 0.0;
        }
        self.end_ms
            .filter(|_| self.fade_out_ms > 0)
            .map(|end| {
                ((end - ms) as f64
                    / self
                        .fade_out_ms
                        .min(end.saturating_sub(self.start_ms))
                        .max(1) as f64)
                    .min(1.0)
            })
            .unwrap_or(1.0)
    }
}

/// 只在图形提交时比较结构化数据；逐帧渲染不重新序列化，也不复制 PNG。
pub fn reconcile(
    history: &mut Vec<TimedAnnotation>,
    current: &[String],
    shapes: &[Annotation],
    ms: u64,
    lifetime: u64,
    fade: u64,
) -> Result<Vec<String>, String> {
    let raster_bytes = history
        .iter()
        .map(|t| t.shape.raster.as_ref().map_or(0, String::len))
        .sum::<usize>()
        + shapes
            .iter()
            .map(|s| s.raster.as_ref().map_or(0, String::len))
            .sum::<usize>();
    if history.len() + shapes.len() > 10_000 || raster_bytes > 60 * 1024 * 1024 {
        return Err("本次录屏标注历史已达上限，请结束后新建录屏。".into());
    }
    let mut ids: Vec<String> = Vec::with_capacity(shapes.len());
    for (order, shape) in shapes.iter().enumerate() {
        let value = serde_json::to_value(shape).map_err(|e| e.to_string())?;
        // filter 接收到 &&String，解引用后再与拥有所有权的 ID 列表比较。
        let previous = current.iter().filter(|id| !ids.contains(*id)).find(|id| {
            history.iter().any(|old| {
                &old.id == *id
                    && old.order == order as u32
                    && serde_json::to_value(&old.shape).ok().as_ref() == Some(&value)
            })
        });
        if let Some(id) = previous {
            ids.push(id.clone());
        } else {
            let id = format!("a-{}", history.len() + 1);
            history.push(TimedAnnotation {
                id: id.clone(),
                shape: shape.clone(),
                start_ms: ms,
                end_ms: (lifetime > 0).then_some(ms.saturating_add(lifetime)),
                fade_out_ms: fade.min(lifetime),
                order: order as u32,
            });
            ids.push(id);
        }
    }
    for old in history
        .iter_mut()
        .filter(|old| current.contains(&old.id) && !ids.contains(&old.id))
    {
        old.end_ms = Some(old.end_ms.unwrap_or(ms).min(ms));
        // 主动清除立即生效，不在清除之前倒推一段新的淡出。
        old.fade_out_ms = 0;
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_shapes_keep_start_and_clear_only_closes_history() {
        let shape:Annotation=serde_json::from_value(serde_json::json!({"kind":"rectangle","start":{"x":0,"y":0},"end":{"x":10,"y":10},"color":"#ff0000","width":2})).unwrap();
        let mut history = Vec::new();
        let ids = reconcile(
            &mut history,
            &[],
            std::slice::from_ref(&shape),
            1000,
            3000,
            500,
        )
        .unwrap();
        let same = reconcile(
            &mut history,
            &ids,
            std::slice::from_ref(&shape),
            2000,
            5000,
            500,
        )
        .unwrap();
        assert_eq!(ids, same);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].start_ms, 1000);
        assert_eq!(history[0].end_ms, Some(4000));
        reconcile(&mut history, &same, &[], 2500, 0, 0).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].end_ms, Some(2500));
        assert_eq!(history[0].fade_out_ms, 0);
    }
    #[test]
    fn fade_uses_media_time_and_excludes_end() {
        let shape = serde_json::from_value(serde_json::json!({"kind":"rectangle","start":{"x":0,"y":0},"end":{"x":10,"y":10},"color":"#ff0000","width":2})).unwrap();
        let item = TimedAnnotation {
            id: "a-1".into(),
            shape,
            start_ms: 1000,
            end_ms: Some(4000),
            fade_out_ms: 500,
            order: 0,
        };
        assert_eq!(item.alpha(999), 0.0);
        assert_eq!(item.alpha(3500), 1.0);
        assert_eq!(item.alpha(3750), 0.5);
        assert_eq!(item.alpha(4000), 0.0);
    }
}
