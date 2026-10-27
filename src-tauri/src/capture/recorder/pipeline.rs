//! 录屏流水线：帧采集、有界编码队列、音频提交和最终发布。
use super::*;

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
        Err(e) => Err(err(e)),
        Ok(writer) => {
            let mut last_sample = None;
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
            let collected = (|| -> Result<(), String> {
                loop {
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
                    if effects_generation != Some(generation) || !running {
                        // 先停止并 join 消息线程，再清缓存；恢复使用新监听器，不继承按住状态。
                        listener = None;
                        compositor.clear();
                        base = None;
                        base_dirty = false;
                        if running && config.input_effects.enabled() {
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
                        backend.refresh()?;
                        annotation_revision = revision;
                    }
                    if writer.is_finished() {
                        return Err("录屏编码线程已提前结束。".into());
                    }
                    if check_screen.elapsed() >= Duration::from_millis(500) {
                        backend.check_source()?;
                        live_annotations::update_origin(&app, id, backend.desktop_origin()?);
                        if super::scroll::session::screen_at(&app, screen.x, screen.y)? != screen {
                            return Err("显示器位置、尺寸或 DPI 改变，请重新录屏。".into());
                        }
                        check_screen = Instant::now();
                    }
                    if running {
                        waiting_since.get_or_insert_with(Instant::now);
                        if config.input_effects.enabled() {
                            if listener.is_none() {
                                listener = Some(input::Listener::start(config.input_effects)?);
                            }
                            let frame = measured_capture(&state, id, backend.as_mut())?;
                            compositor.set_origin(backend.desktop_origin()?);
                            let events = listener.as_ref().unwrap().drain()?;
                            // 先取完源事件，再取 QPC，避免给未来事件分配更早的视频时间戳。
                            let now = capture::now();
                            let projected = {
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
                                        s.timeline.project(f.source_time).is_some()
                                            && f.source_time <= now
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
                                    let needed = base_dirty || compositor.needs_frame();
                                    let accepted = needed
                                        && timeline::accept_sample(
                                            &mut last_sample,
                                            generation,
                                            pts,
                                            config.fps,
                                        );
                                    if needed && !accepted {
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
                                s.view.stats.rejected_capture_frames += 1;
                            }
                        }
                        // 从未收到有效帧时不能无限等待；已有帧的静态桌面允许无新帧。
                        if (if config.input_effects.enabled() {
                            base.is_none()
                        } else {
                            last_sample.is_none()
                        }) && waiting_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5))
                        {
                            return Err("屏幕采集未返回画面，请检查权限或图形驱动。".into());
                        }
                    } else {
                        waiting_since = None;
                    }
                    // 帧事件唤醒采集；超时使静态桌面也能检查停止和设备状态。
                    if running {
                        // Raw Input 独立消息线程不会触发 WGC；特效模式按帧率限制最长等待，
                        // 静态画面也能及时消费输入，并在动画结束后提交一帧干净底图。
                        let timeout = if config.input_effects.enabled() {
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
            // 无论停止、取消、采集失败或编码线程提前退出，都在封装 MP4 前停止输入监听。
            drop(listener);
            compositor.clear();
            drop(base);
            if collected.is_err() {
                abort.store(true, Ordering::Release);
            }
            // 先停止捕获，再关闭发送端，编码器按有效代次排空队列后 Finalize。
            drop(backend);
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
            drop(sender);
            let encoded = writer
                .join()
                .map_err(|_| "录屏编码线程异常退出。".to_string())
                .and_then(|r| r);
            // 编码故障优先展示具体原生错误；采集故障不被内部中止提示覆盖。
            match (collected, encoded) {
                (Err(_), Err(writer_error)) if writer_error != "录屏采集已中止。" => {
                    Err(writer_error)
                }
                (Err(e), _) => Err(e),
                (Ok(()), result) => result,
            }
        }
    };
    live_annotations::clear(&app, id);
    let mut message = String::new();
    let mut completed = false;
    if let Ok(mut guard) = state.0.lock() {
        if let Some(s) = guard.as_mut().filter(|s| s.view.id == id) {
            s.freeze();
            let (phase, text, path) = match result {
                Ok(path) => (Phase::Completed, format!("录屏已保存：{path}"), Some(path)),
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
            log::log!(level, "event=recording.finished module=recording op=rec-{id} result={outcome} duration_ms={} captured={} encoded={} dropped={} encoding_ms={}",
                (s.view.seconds * 1000.0) as u64, s.view.stats.captured_frames, s.view.frames, s.view.dropped_frames, s.view.encoding_millis);
            completed = phase == Phase::Completed;
            message = text.clone();
            s.view.phase = phase;
            s.view.message = text;
            s.view.path = path;
            s.view.revision += 1;
        }
    }
    notify_changed(&app);
    chrome::release_recording(&app);
    // 按需创建回放窗口，成功后切换为回放/裁剪窗口，成功后无需到主界面寻找录像。
    if completed {
        if let Err(error) = chrome::show_recording_preview(&app) {
            super::report_error(&app, format!("无法显示录屏回放：{error}"));
        }
    }
    app.state::<CaptureState>()
        .active
        .store(false, Ordering::SeqCst);
    let _ = app.emit_to("main", "capture-result", &message);
    // 主界面保持隐藏；成功由录屏窗口接管编辑，取消或失败沿用现有提示。
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
    let mut recovery = recovery::Pending::new(app, id, &root, config.format)?;
    let pending = recovery.path();
    progress(app, "recording", Some(&pending));
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
        if let Some(worker) = audio.as_mut() {
            // 音频独立于 WGC 新帧；静态桌面和暂停期间仍排空已采集数据。
            pump_audio(worker, &mut mixer, encoder.as_mut(), &state, id, false)?;
        }
        let frame = match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
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
        if let Some(layer) = &frame.annotations {
            layer.apply(&mut image)?;
        }
        encoder.push(&image, frame.pts)?;
        progress(app, "encoding", Some(&pending));
        let encoding_millis = encoding_started.elapsed().as_secs_f64() * 1000.0;
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
        worker.stop()?;
        pump_audio(worker, &mut mixer, encoder.as_mut(), &state, id, true)?;
    }
    recovery.stage("finalizing", false)?;
    progress(app, "finalizing", Some(&pending));
    encoder.finish(end)?;
    // 释放 MF 文件句柄后才能同步/重命名；取消期间仍由临时目录负责清理。
    drop(encoder);
    recovery.stage("syncing", true)?;
    progress(app, "syncing", Some(&pending));
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&pending)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    if file.metadata().map_err(err)?.len() == 0 {
        return Err("编码器未生成视频。".into());
    }
    drop(file);
    let guard = state.0.lock().map_err(err)?;
    guard
        .as_ref()
        .filter(|s| s.view.id == id && !s.cancel)
        .ok_or("已取消录屏。")?;
    // 最后确认取消；发布和取消共用状态锁，失败时 Pending 保留完整视频。
    let output = recovery.output(&root);
    progress(app, "publishing", Some(&pending));
    output::publish(&pending, &output)?;
    recovery.published(&output);
    result::remember_with_layout(app, &output, chrome::recording_preview_layout(app));
    progress(app, "completed", Some(&output));
    // 终态与成品路径由 collect 收尾时一起发布，避免回放读到 Completed 却没有路径。
    Ok(output.to_string_lossy().into_owned())
}

/// 诊断不触发试录/编码器探测，不采集视频或音频。
pub(crate) fn diagnostic_summary() -> serde_json::Value {
    serde_json::json!({"backend":std::env::consts::OS,"captureSupported":capture::supported().is_ok(),"encoderProbed":false,"audioBackend":if cfg!(target_os = "windows") { "wasapi" } else { "unsupported" },"audioProbed":false})
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
