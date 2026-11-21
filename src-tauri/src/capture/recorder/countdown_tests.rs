//! 使用可控的 QPC 数值覆盖到期、取消和恢复，不依赖真实等待或窗口环境。
use super::*;

#[test]
fn exit_waits_for_both_preparation_and_recording_cleanup() {
    let state = ExitState::default();
    state.workers.store(2, Ordering::SeqCst);
    assert!(!state.requested.swap(true, Ordering::SeqCst));
    assert!(state.requested.swap(true, Ordering::SeqCst));
    assert!(!state.release_worker());
    assert!(state.release_worker());
}

#[test]
fn finishing_recording_without_exit_request_keeps_app_alive() {
    let state = ExitState::default();
    state.workers.store(1, Ordering::SeqCst);
    assert!(!state.release_worker());
}

