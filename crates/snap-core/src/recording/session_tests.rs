//! 使用可控的 QPC 数值覆盖到期、取消和恢复，不依赖真实等待或窗口环境。
use super::*;

fn session() -> Session {
    Session {
        hide_main_window: true,
        view: RecordingView {
            id: 1,
            revision: 1,
            phase: Phase::Ready,
            seconds: 0.0,
            frames: 0,
            dropped_frames: 0,
            target_fps: 30,
            system_audio: false,
            microphone: false,
            encoding_millis: 0.0,
            stats: stats::RecordingStats::default(),
            target_bitrate: 8_087_040,
            encoder_kind: encoder::EncoderKind::System,
            encoder_fallback: None,
            countdown_remaining: 0,
            message: String::new(),
            path: None,
        },
        timeline: Timeline::default(),
        fps_window: stats::FpsWindow::default(),
        queue: std::sync::Arc::default(),
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
fn stop_is_visible_immediately_and_cancel_during_encoding_stays_cancelled() {
    let mut s = session();
    s.begin(0, 0);
    let generation = s.timeline.generation;
    s.stop_requested(false);
    let elapsed = s.timeline.elapsed(i64::MAX);
    assert_eq!(s.view.phase, Phase::Encoding);
    assert!(!s.cancel);
    assert_eq!(s.timeline.generation, generation);
    s.stop_requested(true);
    s.stop_requested(false);
    assert!(s.cancel);
    assert_eq!(s.timeline.elapsed(i64::MAX), elapsed);
    assert!(s.view.message.contains("取消"));
}

#[test]
fn resume_has_no_countdown_and_preserves_elapsed_time() {
    let mut s = session();
    s.begin(100, 0);
    s.fps_window.submitted(5_000_000);
    s.view.stats.captured_frames = 12;
    assert_eq!(s.fps_window.fps(10_000_000), 1.0);
    s.timeline.pause(10_000_100);
    s.view.phase = Phase::Paused;
    s.begin(50_000_100, 0);
    assert_eq!(s.view.phase, Phase::Recording);
    assert_eq!(s.countdown_until, None);
    assert_eq!(s.timeline.project(50_000_100), Some((3, 10_000_000)));
    assert_eq!(s.fps_window.fps(10_000_000), 0.0);
    assert_eq!(s.view.stats.captured_frames, 12);
}

/// 假采集器依次返回空取样、真实帧和设备错误，验证统计不会把空取样算成丢帧。
#[test]
fn capture_statistics_count_polls_separately_and_preserve_errors() {
    struct FakeCapture(usize);
    impl capture::CaptureBackend for FakeCapture {
        fn next_frame(&mut self) -> Result<Option<frame::VideoFrame>, String> {
            self.0 += 1;
            match self.0 {
                1 => Ok(None),
                2 => Ok(Some(frame::VideoFrame {
                    source_time: 123,
                    image: frame::BgraImage::new(2, 2),
                })),
                _ => Err("设备丢失".into()),
            }
        }
        fn wait_for_frame(&self, _: Duration) {}
        fn refresh(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
    let state = RecordingState::default();
    *state.0.lock().unwrap() = Some(session());
    let mut backend = FakeCapture(0);
    assert!(measured_capture(&state, 1, &mut backend).unwrap().is_none());
    assert_eq!(
        measured_capture(&state, 1, &mut backend)
            .unwrap()
            .unwrap()
            .source_time,
        123
    );
    assert!(matches!(measured_capture(&state, 1, &mut backend), Err(e) if e == "设备丢失"));
    let guard = state.0.lock().unwrap();
    let view = &guard.as_ref().unwrap().view;
    assert_eq!(view.stats.capture_polls, 3);
    assert_eq!(view.stats.captured_frames, 1);
    assert_eq!(view.stats.capture_copy_bytes, 16);
    assert_eq!(view.dropped_frames, 0);
    assert_eq!(view.frames, 0);
    assert_eq!(view.revision, 4);
}

#[test]
fn audio_backend_reflects_compiled_worker() {
    let summary = diagnostic_summary();
    let expected = if cfg!(target_os = "windows") {
        "wasapi"
    } else if cfg!(target_os = "macos") {
        "screencapturekit+avfoundation"
    } else {
        "pulseaudio"
    };
    assert_eq!(summary["audioBackend"], expected);
    assert_eq!(summary["audioBackendSupported"], true);
    // 未打开设备检测时不能把设备报告成不可用。
    assert_eq!(summary["audioProbed"], false);
    assert!(summary["microphonePermission"].is_string());
}

#[test]
fn control_pauses_resumes_and_closes_completed_sessions() {
    let state = RecordingState::default();
    *state.0.lock().unwrap() = Some(session());
    let mut hidden = None;
    assert_eq!(
        state
            .control(Some(1), RecordingAction::Start, |hide| {
                hidden = Some(hide);
                Ok(())
            })
            .unwrap(),
        ControlOutcome::Started
    );
    assert_eq!(hidden, Some(true));
    // 倒计时 3 秒；暂停只在录制中可用。
    assert!(state
        .control(Some(1), RecordingAction::Pause, |_| Ok(()))
        .is_err());
    assert!(state
        .control(Some(2), RecordingAction::Stop, |_| Ok(()))
        .is_err());
    assert_eq!(
        state
            .control(None, RecordingAction::Stop, |_| Ok(()))
            .unwrap(),
        ControlOutcome::Stopping
    );
    state.0.lock().unwrap().as_mut().unwrap().view.phase = Phase::Cancelled;
    assert!(!state.is_active());
    assert_eq!(state.completed_id(), Some(1));
    assert_eq!(
        state
            .control(Some(1), RecordingAction::Stop, |_| Ok(()))
            .unwrap(),
        ControlOutcome::Unchanged
    );
    assert_eq!(
        state
            .control(Some(1), RecordingAction::Cancel, |_| Ok(()))
            .unwrap(),
        ControlOutcome::Closed
    );
    assert!(state
        .control(Some(1), RecordingAction::Resume, |_| Ok(()))
        .is_err());
}
