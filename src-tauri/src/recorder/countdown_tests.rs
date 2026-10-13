//! 使用可控的 QPC 数值覆盖到期、取消和恢复，不依赖真实等待或窗口环境。
use super::*;

fn session() -> Session {
    Session {
        view: RecordingView {
            id: 1,
            revision: 1,
            phase: Phase::Ready,
            seconds: 0.0,
            frames: 0,
            dropped_frames: 0,
            target_fps: 30,
            encoder_kind: encoder::EncoderKind::System,
            encoder_fallback: None,
            countdown_remaining: 0,
            message: String::new(),
            path: None,
        },
        timeline: Timeline::default(),
        countdown_seconds: 3,
        countdown_until: None,
        stop: false,
        cancel: false,
    }
}

#[test]
fn countdown_starts_only_at_deadline_and_does_not_add_video_time() {
    let mut s = session();
    s.begin(100, 3);
    assert_eq!(s.view.phase, Phase::Countdown);
    assert_eq!(s.timeline.project(101), None);
    s.advance_countdown(10_000_100);
    assert_eq!(s.view.countdown_remaining, 2);
    s.advance_countdown(30_000_099);
    assert_eq!(s.view.countdown_remaining, 1);
    assert_eq!(s.view.seconds, 0.0);
    s.advance_countdown(30_000_100);
    assert_eq!(s.view.phase, Phase::Recording);
    assert_eq!(s.view.countdown_remaining, 0);
    assert_eq!(s.timeline.project(30_000_100), Some((1, 0)));
}

#[test]
fn stopping_countdown_cancels_and_late_tick_cannot_start_recording() {
    for cancel in [false, true] {
        let mut s = session();
        s.begin(100, 5);
        s.stop_requested(cancel);
        s.stop_requested(cancel);
        s.advance_countdown(60_000_100);
        assert!(s.stop && s.cancel);
        assert_eq!(s.countdown_until, None);
        assert_eq!(s.timeline.project(60_000_100), None);
        assert_eq!(s.view.frames, 0);
        assert_eq!(s.view.seconds, 0.0);
    }
}

#[test]
fn resume_has_no_countdown_and_preserves_elapsed_time() {
    let mut s = session();
    s.begin(100, 0);
    s.timeline.pause(10_000_100);
    s.view.phase = Phase::Paused;
    s.begin(50_000_100, 0);
    assert_eq!(s.view.phase, Phase::Recording);
    assert_eq!(s.countdown_until, None);
    assert_eq!(s.timeline.project(50_000_100), Some((3, 10_000_000)));
}
