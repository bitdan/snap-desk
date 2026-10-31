//! 录屏流水线：帧采集、有界编码队列、音频提交和最终发布。
use super::*;

/// 已发布的成品始终携带路径；采集异常另附原因，不能把可用片段误报为完整录制。
#[derive(Debug)]
struct CollectedOutput {
    path: String,
    interruption: Option<String>,
}

/// 采集线程绝不等待编码队列；开启特效时额外保留一帧干净底图，动画与事件数量有界。
pub(super) fn collect(
    app: AppHandle,
    id: u64,
    config: RecorderConfig,
    screen: super::scroll::Screen,
    root: PathBuf,
    mut backend: Box<dyn capture::CaptureBackend>,
    audio: Option<audio::Worker>,
) {
    let state = app.state::<RecordingState>();
    let (sender, receiver) = mpsc::sync_channel::<QueuedFrame>(2);
    let abort = std::sync::Arc::new(AtomicBool::new(false));
    let writer_abort = abort.clone();
    let writer_app = app.clone();
    let writer = std::thread::Builder::new()
        .name("recording-encoder".into())
        .spawn(move || {
            write_video(
                &writer_app,
                id,
                config,
                root,
                receiver,
                &writer_abort,
                audio,
            )
        });
    let result = match writer {
        Err(e) => {
            log_step_failure(id, "encoder_thread_start");
            Err(err(e))
        }
        Ok(writer) => {
            let mut last_sample = None;
            let mut rejected_generation = None;
            let mut annotation_revision = live_annotations::snapshot(&app, id).0;
            let mut annotation_editing = false;
            let mut check_screen = Instant::now();
            let mut waiting_since = None;
            let mut listener: Option<input::Listener> = None;
            let mut effects_generation = None;
            let mut compositor = compositor::Compositor::new(config.input_effects, config.region);
            compositor.set_origin((screen.x, screen.y));
            let mut base: Option<frame::BgraImage> = None;
            let mut base_dirty = false;
            let mut annotation_animation = false;
            // Cell 跨即时闭包保留最近步骤；错误返回后只读固定标签，不改变业务错误。
            let capture_stage = std::cell::Cell::new("capture_state");
            let collected = (|| -> Result<(), String> {
                loop {
                    capture_stage.set("capture_state");
                    let (running, generation) = {
                        let mut guard = state.0.lock().map_err(err)?;
                        let s = guard
                            .as_mut()
                            .filter(|s| s.view.id == id)
                            .ok_or("录屏会话已过期。")?;
                        if s.cancel || s.stop {
                            break;
                        }
                        s.advance_countdown(capture::now());
                        // 不按录制时长自动停止；停止、取消和采集/编码异常仍走统一收尾。
                        (s.view.phase == Phase::Recording, s.timeline.generation)
                    };
                    let animated = live_annotations::has_timed_annotations(&app, id);
                    if animated != annotation_animation && !config.input_effects.enabled() {
                        base = None;
                        base_dirty = false;
                        if running && animated {
                            capture_stage.set("capture_refresh");
                            backend.refresh()?;
                        }
                    }
                    annotation_animation = animated;
                    if effects_generation != Some(generation) || !running {
                        // 先停止并 join 消息线程，再清缓存；恢复使用新监听器，不继承按住状态。
                        listener = None;
                        compositor.clear();
                        base = None;
                        base_dirty = false;
                        if running && (config.input_effects.enabled() || annotation_animation) {
                            capture_stage.set("capture_refresh");
                            backend.refresh()?;
                        }
                        effects_generation = Some(generation);
                    }
                    let editing = live_annotations::editing(&app, id);
                    if editing != annotation_editing {
                        compositor.clear();
                        base_dirty = base.is_some();
                        annotation_editing = editing;
                    }
                    let revision = live_annotations::snapshot(&app, id).0;
                    if running && revision != annotation_revision {
                        // 即使桌面静止，确认一笔/撤销后也请求新帧，让标注变化立即进入视频。
                        capture_stage.set("capture_refresh");
                        backend.refresh()?;
                        annotation_revision = revision;
                    }
                    if writer.is_finished() {
                        return Err("录屏编码线程已提前结束。".into());
                    }
                    capture_stage.set("capture_source_check");
                    if check_screen.elapsed() >= Duration::from_millis(500) {
                        backend.check_source()?;
                        live_annotations::update_origin(&app, id, backend.desktop_origin()?);
                        if super::scroll::session::screen_at(&app, screen.x, screen.y)? != screen {
                            capture_stage.set("display_changed");
                            return Err("显示器位置、尺寸或 DPI 改变，请重新录屏。".into());
                        }
                        check_screen = Instant::now();
                    }
                    if running {
                        waiting_since.get_or_insert_with(Instant::now);
                        capture_stage.set("capture_frame");
                        if config.input_effects.enabled() || annotation_animation {
                            if config.input_effects.enabled() && listener.is_none() {
                                capture_stage.set("input_start");
                                listener = Some(input::Listener::start(config.input_effects)?);
                            }
                            capture_stage.set("capture_frame");
                            let frame = measured_capture(&state, id, backend.as_mut())?;
                            compositor.set_origin(backend.desktop_origin()?);
                            let events = if let Some(listener) = listener.as_ref() {
                                capture_stage.set("input_read");
                                listener.drain()?
                            } else {
                                Vec::new()
                            };
                            // 先取完源事件，再取 QPC，避免给未来事件分配更早的视频时间戳。
                            let now = capture::now();
                            let projected = {
                                capture_stage.set("capture_state");
                                let mut guard = state.0.lock().map_err(err)?;
                                let s = guard
                                    .as_mut()
                                    .filter(|s| s.view.id == id)
                                    .ok_or("录屏会话已过期。")?;
                                if s.stop
                                    || s.cancel
                                    || s.view.phase != Phase::Recording
                                    || s.timeline.generation != generation
                                {
                                    s.view.stats.rejected_capture_frames +=
                                        usize::from(frame.is_some());
                                    s.view.revision += 1;
                                    None
                                } else {
                                    let had_frame = frame.is_some();
                                    let frame = frame.filter(|f| {
                                        let accepted = s.timeline.project_composite(f.source_time, now).is_some();
                                        if !accepted {
                                            log_timestamp_rejection(
                                                id, generation, f.source_time, now,
                                                "before_start", &mut rejected_generation,
                                            );
                                        }
                                        accepted
                                    });
                                    if had_frame && frame.is_none() {
                                        s.view.stats.rejected_capture_frames += 1;
                                        s.view.revision += 1;
                                    }
                                    let events = events
                                        .into_iter()
                                        .filter_map(|event| {
                                            s.timeline
                                                .project(event.source_time)
                                                .filter(|(_, pts)| *pts <= s.timeline.elapsed(now))
                                                .map(|(_, pts)| (pts, event.kind))
                                        })
                                        .collect::<Vec<_>>();
                                    s.timeline.project(now).map(|(_, pts)| (pts, frame, events))
                                }
                            };
                            if let Some((pts, frame, events)) = projected {
                                if let Some(frame) = frame {
                                    base = Some(frame.image);
                                    base_dirty = true;
                                    waiting_since = None;
                                }
                                if !annotation_editing {
                                    for (time, event) in events {
                                        compositor.ingest(time, event);
                                    }
                                }
                                if let Some(clean) = base.as_ref() {
                                    // 静态桌面的淡出复用最近一帧干净底图，不重建 WGC 会话。
                                    let needed = base_dirty
                                        || compositor.needs_frame()
                                        || annotation_animation;
                                    let accepted = needed
                                        && timeline::accept_sample(
                                            &mut last_sample,
                                            generation,
                                            pts,
                                            config.fps,
                                        );
                                    if needed && !accepted {
                                        capture_stage.set("capture_state");
                                        let mut guard = state.0.lock().map_err(err)?;
                                        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id)
                                        {
                                            s.view.stats.throttled_samples += 1;
                                            s.view.revision += 1;
                                        }
                                    }
                                    if accepted {
                                        // 合成在锁外；重绘时只读未叠加的底图，帧时间用同源 QPC 的当前媒体时间。
                                        let composition_started = Instant::now();
                                        let image = compositor.render(clean, pts);
                                        let composition_millis =
                                            composition_started.elapsed().as_secs_f64() * 1000.0;
                                        capture_stage.set("capture_state");
                                        let mut guard = state.0.lock().map_err(err)?;
                                        let s = guard
                                            .as_mut()
                                            .filter(|s| s.view.id == id)
                                            .ok_or("录屏会话已过期。")?;
                                        s.view.stats.composed_frames += 1;
                                        s.view.stats.composition_copy_bytes +=
                                            image.data.len() as u64;
                                        s.view.stats.composition_millis += composition_millis;
                                        s.view.revision += 1;
                                        if !s.stop
                                            && !s.cancel
                                            && s.view.phase == Phase::Recording
                                            && s.timeline.generation == generation
                                        {
                                            match sender.try_send(QueuedFrame {
                                                generation,
                                                pts,
                                                annotations: live_annotations::snapshot(&app, id).1,
                                                image,
                                            }) {
                                                Ok(()) => {
                                                    base_dirty = false;
                                                    s.view.stats.queued_frames += 1;
                                                }
                                                Err(TrySendError::Full(_)) => {
                                                    // 包括效果结束后的干净帧，拥堵后也必须重试，防止光圈冻结。
                                                    base_dirty = true;
                                                    s.view.stats.queue_dropped_frames += 1;
                                                    s.view.dropped_frames += 1;
                                                    s.view.revision += 1;
                                                }
                                                Err(TrySendError::Disconnected(_)) => {
                                                    return Err("录屏编码线程已停止。".into())
                                                }
                                            }
                                        } else {
                                            s.view.stats.rejected_composed_frames += 1;
                                        }
                                    }
                                }
                            }
                        } else if let Some(frame) = measured_capture(&state, id, backend.as_mut())?
                        {
                            capture_stage.set("capture_state");
                            let mut guard = state.0.lock().map_err(err)?;
                            let s = guard
                                .as_mut()
                                .filter(|s| s.view.id == id)
                                .ok_or("录屏会话已过期。")?;
                            s.view.revision += 1;
                            let projected = s
                                .timeline
                                .project(frame.source_time)
                                .filter(|(g, _)| !s.stop && !s.cancel && *g == generation);
                            if let Some((generation, pts)) = projected {
                                // 帧池可能残留暂停前的帧；project 会拒绝恢复起点以前的数据。
                                if timeline::accept_sample(
                                    &mut last_sample,
                                    generation,
                                    pts,
                                    config.fps,
                                ) {
                                    waiting_since = None;
                                    match sender.try_send(QueuedFrame {
                                        generation,
                                        pts,
                                        annotations: live_annotations::snapshot(&app, id).1,
                                        image: frame.image,
                                    }) {
                                        Ok(()) => s.view.stats.queued_frames += 1,
                                        Err(TrySendError::Full(_)) => {
                                            s.view.stats.queue_dropped_frames += 1;
                                            s.view.dropped_frames += 1;
                                            s.view.revision += 1;
                                        }
                                        Err(TrySendError::Disconnected(_)) => {
                                            return Err("录屏编码线程已停止。".into())
                                        }
                                    }
                                } else {
                                    s.view.stats.throttled_samples += 1;
                                }
                            } else {
                                if !s.stop && !s.cancel && s.timeline.generation == generation {
                                    log_timestamp_rejection(
                                        id, generation, frame.source_time, capture::now(),
                                        "before_start", &mut rejected_generation,
                                    );
                                }
                                s.view.stats.rejected_capture_frames += 1;
                            }
                        }
                        // 从未收到有效帧时不能无限等待；已有帧的静态桌面允许无新帧。
                        if (if config.input_effects.enabled() || annotation_animation {
                            base.is_none()
                        } else {
                            last_sample.is_none()
                        }) && waiting_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5))
                        {
                            let guard = state.0.lock().map_err(err)?;
                            let received = guard.as_ref().filter(|s| s.view.id == id)
                                .is_some_and(|s| s.view.stats.rejected_capture_frames > 0);
                            if received {
                                capture_stage.set("capture_timestamp_timeout");
                                return Err("已收到屏幕画面，但帧时间戳校验未通过，请查看录屏诊断日志。".into());
                            }
                            capture_stage.set("capture_timeout");
                            return Err("屏幕采集未返回画面，请检查权限或图形驱动。".into());
                        }
                    } else {
                        waiting_since = None;
                    }
                    // 帧事件唤醒采集；超时使静态桌面也能检查停止和设备状态。
                    if running {
                        // Raw Input 独立消息线程不会触发 WGC；特效模式按帧率限制最长等待，
                        // 静态画面也能及时消费输入，并在动画结束后提交一帧干净底图。
                        let timeout = if config.input_effects.enabled() || annotation_animation {
                            Duration::from_nanos(1_000_000_000 / u64::from(config.fps))
                        } else {
                            Duration::from_millis(50)
                        };
                        backend.wait_for_frame(timeout);
                    } else {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                Ok(())
            })();
            // 编码线程退出由写入侧提供具体阶段，避免把连带退出误当成采集根因。
            if collected
                .as_ref()
                .is_err_and(|e| e != "录屏编码线程已提前结束。" && e != "录屏编码线程已停止。")
            {
                log_step_failure(id, capture_stage.get());
            }
            // 来源断开、显示器变化和采集超时不破坏已经提交的媒体样本。
            // 这些故障按停止收尾；状态/编码故障仍立即中止，取消由会话标志裁决。
            let keep_partial = can_finalize_capture(capture_stage.get());
            if collected.is_err() && !keep_partial {
                abort.store(true, Ordering::Release);
            }
            // 先冻结媒体时间，再等待监听器和采集设备释放；驱动清理耗时不计入成品。
            if let Ok(mut guard) = state.0.lock() {
                if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
                    s.freeze();
                    s.stop = true;
                    s.view.phase = Phase::Encoding;
                    s.view.message = if s.cancel {
                        "正在取消录屏并清理临时文件…"
                    } else {
                        "录屏已停止，正在完成文件封装…"
                    }
                    .into();
                    s.view.revision += 1;
                }
            }
            // 无论停止、取消或异常，封装前都释放设备；队列保留冻结前的有效代次。
            drop(listener);
            compositor.clear();
            drop(base);
            drop(backend);
            drop(sender);
            let encoded = writer
                .join()
                .map_err(|_| {
                    log_step_failure(id, "encoder_thread_panic");
                    "录屏编码线程异常退出。".to_string()
                })
                .and_then(|r| r);
            collection_result(capture_stage.get(), collected, encoded)
        }
    };
    live_annotations::clear(&app, id);
    let mut message = String::new();
    if let Ok(mut guard) = state.0.lock() {
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.freeze();
            let (phase, text, path) = match result {
                Ok(CollectedOutput { path, interruption: Some(reason) }) => (
                    Phase::Failed,
                    format!("录屏已中断：{reason} 已保存可用部分：{path}"),
                    Some(path),
                ),
                Ok(CollectedOutput { path, .. }) => (Phase::Completed, format!("录屏已保存：{path}"), Some(path)),
                Err(_) if s.cancel => (
                    Phase::Cancelled,
                    "录屏已取消；未能清理的残留可在录屏页查看并删除。".into(),
                    None,
                ),
                Err(error) => (
                    Phase::Failed,
                    format!(
                        "录屏失败：{error} 请在录屏页查看已保留的数据；未封装数据可能无法播放。"
                    ),
                    None,
                ),
            };
            // 只在统一收尾处记录汇总，不把含完整输出路径的 text/path 写盘。
            if phase == Phase::Failed {
                log::error!("event=recording.failed module=recording op=rec-{id} code=RECORDING_PIPELINE_FAILED");
            }
            let level = if phase == Phase::Failed {
                log::Level::Error
            } else {
                log::Level::Info
            };
            let outcome = match phase {
                Phase::Completed => "success",
                Phase::Cancelled => "cancelled",
                _ => "failed",
            };
            log::log!(level, "event=recording.finished module=recording op=rec-{id} result={outcome} duration_ms={} captured={} encoded={} dropped={} encoding_ms={} queued={} queue_dropped={} throttled={} rejected_capture={} stale={}",
                (s.view.seconds * 1000.0) as u64, s.view.stats.captured_frames, s.view.frames, s.view.dropped_frames, s.view.encoding_millis,
                s.view.stats.queued_frames, s.view.stats.queue_dropped_frames, s.view.stats.throttled_samples,
                s.view.stats.rejected_capture_frames, s.view.stats.stale_queued_frames);
            message = text.clone();
            s.view.phase = phase;
            s.view.message = text;
            s.view.path = path;
            s.view.revision += 1;
        }
    }
    notify_changed(&app);
    // 成功后直接保留已经发布的成品，所有终态都收起录制交互窗口。
    chrome::release_recording(&app);
    app.state::<CaptureState>()
        .active
        .store(false, Ordering::SeqCst);
    let _ = app.emit_to("main", "capture-result", &message);
    // 不主动改变主界面的显示状态；结果通过录屏页的最近完成记录查看。
}

/// 仅采集来源故障允许排空当前代次并封装；内部状态、输入及编码故障不在此回退。
fn can_finalize_capture(stage: &str) -> bool {
    matches!(stage, "capture_source_check" | "display_changed" | "capture_frame" | "capture_refresh" | "capture_timeout")
}

/// 编码故障优先展示具体原生错误；来源中断封装成功时同时保留成品与中断原因。
fn collection_result(stage: &str, collected: Result<(), String>, encoded: Result<String, String>) -> Result<CollectedOutput, String> {
    match (collected, encoded) {
        (Err(reason), Ok(path)) if can_finalize_capture(stage) => Ok(CollectedOutput { path, interruption: Some(reason) }),
        (Err(_), Err(writer_error)) if writer_error != "录屏采集已中止。" => Err(writer_error),
        (Err(error), _) => Err(error),
        (Ok(()), result) => result.map(|path| CollectedOutput { path, interruption: None }),
    }
}

/// Finalize、sync_all 成功才发布；保存失败保留视频与恢复记录，明确取消才清理。
fn write_video(
    app: &AppHandle,
    id: u64,
    config: RecorderConfig,
    root: PathBuf,
    receiver: mpsc::Receiver<QueuedFrame>,
    abort: &AtomicBool,
    mut audio: Option<audio::Worker>,
) -> Result<String, String> {
    // 阶段只来自源码常量；保留 UI 的原生错误，日志不接触路径或自由文本。
    let failure_stage = std::cell::Cell::new("output_config");
    let result = (|| -> Result<String, String> {
        let export_settings = output::preferences(app)?;
        failure_stage.set("output_config");
        export_settings.validate()?;
        failure_stage.set("recovery_create");
        let mut recovery = recovery::Pending::new(app, id, &root, config.format)?;
        let pending = recovery.path();
        progress(app, "recording", Some(&pending));
        failure_stage.set("encoder_init");
        let mut encoder = encoder::create(
            &pending,
            config.region.width,
            config.region.height,
            config.fps,
            config.target_bitrate(),
            config.encoding,
            config.format,
            config.system_audio || config.microphone,
        )?;
        let state = app.state::<RecordingState>();
        let encoder_kind = encoder.kind();
        {
            failure_stage.set("encoder_state");
            let mut guard = state.0.lock().map_err(err)?;
            if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
                s.view.encoder_kind = encoder_kind;
                s.view.revision += 1;
            }
        }
        let mut mixer = audio::Mixer::default();
        loop {
            if abort.load(Ordering::Acquire) {
                return Err("录屏采集已中止。".into());
            }
            failure_stage.set("audio_pump");
            if let Some(worker) = audio.as_mut() {
                // 音频独立于 WGC 新帧；静态桌面和暂停期间仍排空已采集数据。
                pump_audio(worker, &mut mixer, encoder.as_mut(), &state, id, false)?;
            }
            let frame = match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(frame) => frame,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // 没有音轨时也推进静止画面，避免长录屏把全部补帧积压到停止导出。
                    // 只推进已经过的媒体时间，暂停期间 elapsed 保持不变。
                    if audio.is_none() {
                        failure_stage.set("encoder_state");
                        let end = state
                            .0
                            .lock()
                            .map_err(err)?
                            .as_ref()
                            .filter(|s| s.view.id == id && !s.cancel)
                            .map(|s| s.timeline.elapsed(capture::now()));
                        if let Some(end) = end {
                            failure_stage.set("video_advance");
                            encoder.advance_video(end)?;
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            failure_stage.set("frame_state");
            {
                let mut guard = state.0.lock().map_err(err)?;
                let s = guard
                    .as_mut()
                    .filter(|s| s.view.id == id)
                    .ok_or("录屏会话已过期。")?;
                if s.cancel {
                    return Err("已取消录屏。".into());
                }
                if frame.generation != s.timeline.generation {
                    s.view.stats.stale_queued_frames += 1;
                    s.view.dropped_frames += 1;
                    s.view.revision += 1;
                    continue;
                }
                // 在锁内确认归属即视为提交；原生 WriteSample 不持有状态锁，按钮可及时响应。
            }
            let encoding_started = Instant::now();
            // 标注层只绘制一次；每帧在编码前混合，确保准备阶段的标注真正进入成品。
            let mut image = frame.image;
            failure_stage.set("annotation_apply");
            if let Some(layer) = &frame.annotations {
                layer.apply(&mut image, frame.pts)?;
            }
            failure_stage.set("video_write");
            encoder.push(&image, frame.pts)?;
            progress(app, "encoding", Some(&pending));
            let encoding_millis = encoding_started.elapsed().as_secs_f64() * 1000.0;
            failure_stage.set("encoder_state");
            let mut guard = state.0.lock().map_err(err)?;
            if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
                s.view.frames += 1;
                s.view.encoding_millis += encoding_millis;
                s.view.stats.nv12_output_bytes = encoder.converted_bytes();
                // push 期间仍可暂停；旧代次已提交帧计入累计值，但不污染恢复后的窗口。
                if frame.generation == s.timeline.generation {
                    s.fps_window.submitted(frame.pts);
                }
                s.view.revision += 1;
            }
        }
        if abort.load(Ordering::Acquire) {
            return Err("录屏采集已中止。".into());
        }
        failure_stage.set("encoder_state");
        let end = {
            let guard = state.0.lock().map_err(err)?;
            let s = guard
                .as_ref()
                .filter(|s| s.view.id == id && !s.cancel)
                .ok_or("已取消录屏。")?;
            s.timeline.elapsed(capture::now())
        };
        if let Some(worker) = audio.as_mut() {
            // 停止设备并 join 后再取尾包；停止时间之后的样本会被 Timeline 裁掉。
            failure_stage.set("audio_stop");
            worker.stop()?;
            failure_stage.set("audio_finalize");
            pump_audio(worker, &mut mixer, encoder.as_mut(), &state, id, true)?;
        }
        failure_stage.set("recovery_update");
        recovery.stage("finalizing", false)?;
        progress(app, "finalizing", Some(&pending));
        failure_stage.set("encoder_finalize");
        encoder.finish(end)?;
        // 释放 MF 文件句柄后才能同步/重命名；取消期间仍由临时目录负责清理。
        drop(encoder);
        failure_stage.set("recovery_update");
        recovery.stage("syncing", true)?;
        progress(app, "syncing", Some(&pending));
        failure_stage.set("file_open");
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&pending)
            .map_err(err)?;
        failure_stage.set("file_sync");
        file.sync_all().map_err(err)?;
        failure_stage.set("file_validate");
        if file.metadata().map_err(err)?.len() == 0 {
            return Err("编码器未生成视频。".into());
        }
        drop(file);
        failure_stage.set("publish_state");
        let guard = state.0.lock().map_err(err)?;
        guard
            .as_ref()
            .filter(|s| s.view.id == id && !s.cancel)
            .ok_or("已取消录屏。")?;
        // 最后确认取消；发布和取消共用状态锁，失败时 Pending 保留完整视频。
        let output = recovery.output(
            &root,
            &export_settings,
            (config.region.width, config.region.height),
        );
        progress(app, "publishing", Some(&pending));
        failure_stage.set("file_publish");
        output::publish(&pending, &output)?;
        recovery.published(&output);
        result::remember(app, &output);
        progress(app, "completed", Some(&output));
        // 终态与成品路径由 collect 收尾时一起发布，确保完成通知携带已经保存的路径。
        Ok(output.to_string_lossy().into_owned())
    })();
    if result
        .as_ref()
        .is_err_and(|error| error != "已取消录屏。" && error != "录屏采集已中止。")
    {
        log_step_failure(id, failure_stage.get());
    }
    result
}

/// 原生编码放在状态锁外；与视频共用 Timeline，暂停不会在成品中留下音频空洞。
fn pump_audio(
    worker: &mut audio::Worker,
    mixer: &mut audio::Mixer,
    encoder: &mut dyn encoder::Encoder,
    state: &RecordingState,
    id: u64,
    finishing: bool,
) -> Result<(), String> {
    let frames = worker.drain()?;
    let (timeline, end) = {
        let guard = state.0.lock().map_err(err)?;
        let session = guard
            .as_ref()
            .filter(|s| s.view.id == id && !s.cancel)
            .ok_or("已取消录屏。")?;
        let elapsed = session.timeline.elapsed(capture::now());
        let end = if finishing {
            elapsed
        } else {
            elapsed.saturating_sub(audio::MIX_DELAY)
        };
        (session.timeline.clone(), end)
    };
    // 最多 128 个音频包可能累积大量逐样本运算；锁内只复制时间轴快照，
    // 在锁外混音，避免高负载时暂停/停止/取消按钮等待整批音频处理。
    for frame in frames {
        let cancelled = {
            let guard = state.0.lock().map_err(err)?;
            !guard.as_ref().is_some_and(|s| s.view.id == id && !s.cancel)
        };
        if cancelled {
            return Err("已取消录屏。".into());
        }
        mixer.ingest(frame, &timeline)?;
    }
    while let Some(packet) = mixer.next_packet(end) {
        // 长静音段可能需要补交很多包；每 10 ms 媒体数据重新检查取消，
        // 不让一次泵送在停止后继续占满编码线程。原生驱动调用本身仍不可抢占。
        {
            let guard = state.0.lock().map_err(err)?;
            if !guard.as_ref().is_some_and(|s| s.view.id == id && !s.cancel) {
                return Err("已取消录屏。".into());
            }
        }
        encoder.advance_video(packet.pts + packet.duration)?;
        encoder.push_audio(&packet)?;
    }
    Ok(())
}

/// 具体步骤与最终汇总共享 rec 会话号，分析时以步骤事件确定故障环节。
fn log_step_failure(id: u64, stage: &'static str) {
    log::error!("event=recording.step_failed module=recording op=rec-{id} code=RECORDING_STEP_FAILED stage={stage}");
}

/// 每个录制代次仅记录一次异常，只输出固定原因和毫秒差值，不记录画面内容。
fn log_timestamp_rejection(
    id: u64,
    generation: u64,
    source: i64,
    now: i64,
    reason: &'static str,
    logged: &mut Option<u64>,
) {
    if *logged == Some(generation) {
        return;
    }
    *logged = Some(generation);
    log::warn!(
        "event=recording.timestamp_rejected module=recording op=rec-{} generation={} reason={} source_offset_ms={}",
        id, generation, reason, source.saturating_sub(now) / 10_000
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_interruption_keeps_only_successfully_finalized_output() {
        for stage in ["capture_source_check", "display_changed", "capture_frame", "capture_refresh", "capture_timeout"] {
            let output = collection_result(stage, Err("来源已断开".into()), Ok("partial.mp4".into())).unwrap();
            assert_eq!(output.path, "partial.mp4");
            assert_eq!(output.interruption.as_deref(), Some("来源已断开"));
            assert!(matches!(collection_result(stage, Err("来源已断开".into()), Err("封装失败".into())), Err(error) if error == "封装失败"));
        }
        assert!(matches!(collection_result("capture_state", Err("会话已失效".into()), Ok("stale.mp4".into())), Err(error) if error == "会话已失效"));
        assert!(matches!(collection_result("capture_state", Err("会话已失效".into()), Err("录屏采集已中止。".into())), Err(error) if error == "会话已失效"));
    }

    #[test]
    fn normal_stop_and_explicit_cancel_keep_existing_result_semantics() {
        let output = collection_result("capture_state", Ok(()), Ok("complete.mp4".into())).unwrap();
        assert_eq!(output.path, "complete.mp4");
        assert!(output.interruption.is_none());
        assert!(matches!(collection_result("capture_frame", Ok(()), Err("已取消录屏。".into())), Err(error) if error == "已取消录屏。"));
    }
}
