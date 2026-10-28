//! 从原生入口到可见蒙层绘制回执的单调时钟计时；不记录屏幕、窗口名或路径。
use serde::Deserialize;
use std::time::Instant;
use tauri::{AppHandle, Manager, WebviewWindow};

/// 各阶段为独立耗时；总耗时从快捷键处理入口计起，包含排队和用户配置的延时。
pub(super) struct Latency {
    pub started: Instant,
    pub hide_ms: f64,
    pub dwm_ms: f64,
    pub capture_ms: f64,
    pub png_ms: f64,
    pub overlay_ready_ms: f64,
    pub delay_ms: f64,
    pub cold: bool,
    pub painted: bool,
}

impl Latency {
    pub fn new(started: Instant) -> Self {
        Self {
            started,
            hide_ms: 0.0,
            dwm_ms: 0.0,
            capture_ms: 0.0,
            png_ms: 0.0,
            overlay_ready_ms: 0.0,
            delay_ms: 0.0,
            cold: false,
            painted: false,
        }
    }
}

pub(super) fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// 前端只报告同一 performance 时钟上的区间，不与 Rust Instant 的绝对值相减。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapturePaint {
    load_ms: f64,
    decode_ms: f64,
    pre_paint_ms: f64,
    post_paint_ms: f64,
    paint_source: PaintSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum PaintSource {
    Raf,
    Timeout,
}

impl CapturePaint {
    fn valid(&self) -> bool {
        [
            self.load_ms,
            self.decode_ms,
            self.pre_paint_ms,
            self.post_paint_ms,
        ]
        .iter()
        .all(|value| value.is_finite() && (0.0..=60_000.0).contains(value))
    }
}

/// 显示后再等待绘制机会才回报；rAF 是可操作首帧的近似值，不宣称测得显示器扫描输出。
/// 超时兜底会单独标记，取消、旧版本和重复回执均不得污染下一次截图的统计。
#[tauri::command]
pub fn capture_painted(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    revision: u64,
    paint: CapturePaint,
) -> Result<(), String> {
    super::require_window(&window, "capture")?;
    if !paint.valid() {
        return Err("截图绘制计时无效。".into());
    }
    let state = app.state::<super::CaptureState>();
    let mut guard = state.session.lock().map_err(super::err)?;
    let Some(session) = guard
        .as_mut()
        .filter(|s| s.frame.id == id && s.frame.preview_revision == revision && s.shown)
    else {
        return Ok(());
    };
    let timing = &mut session.latency;
    if timing.painted {
        return Ok(());
    }
    timing.painted = true;
    let source = match paint.paint_source {
        PaintSource::Raf => "raf",
        PaintSource::Timeout => "timeout",
    };
    log::debug!("event=capture.interactive module=capture op=cap-{id} revision={revision} hide_ms={:.2} dwm_ms={:.2} capture_ms={:.2} png_ms={:.2} overlay_ready_ms={:.2} first_paint_ms={:.2} delay_ms={:.2} load_ms={:.2} decode_ms={:.2} pre_paint_ms={:.2} post_paint_ms={:.2} paint_source={source} width={} height={} monitors={} cold={} png_bytes={}",
        timing.hide_ms, timing.dwm_ms, timing.capture_ms, timing.png_ms, timing.overlay_ready_ms,
        elapsed_ms(timing.started), timing.delay_ms, paint.load_ms, paint.decode_ms, paint.pre_paint_ms, paint.post_paint_ms,
        session.frame.width, session.frame.height, session.frame.displays.len(), u8::from(timing.cold), session.preview.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paint_metrics_reject_unbounded_values_and_unknown_sources() {
        let mut paint = CapturePaint {
            load_ms: 0.0,
            decode_ms: 1.0,
            pre_paint_ms: 100.0,
            post_paint_ms: 16.0,
            paint_source: PaintSource::Raf,
        };
        assert!(paint.valid());
        for value in [f64::NAN, f64::INFINITY, -1.0, 60_001.0] {
            paint.decode_ms = value;
            assert!(!paint.valid());
        }
        assert!(serde_json::from_str::<CapturePaint>(
            r#"{"loadMs":0,"decodeMs":0,"prePaintMs":0,"postPaintMs":0,"paintSource":"untrusted"}"#
        )
        .is_err());
    }
}
