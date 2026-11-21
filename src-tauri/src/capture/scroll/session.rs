//! Tauri 命令与 [`TauriHost`]：采集循环、匹配与输出在 core `snap_core::scroll::session`。
use super::super::{
    close_session, crop, err, report_error, require_window, CaptureState, Operation, Rect,
};
use super::chrome;
use snap_core::scroll::{self as core, ExportBackground, Host, Screen, ScrollState, ScrollView};
use std::{
    any::Any,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// 通过物理桌面原点重查来源，不因主屏切换而悄悄换到另一块屏幕。
pub(in crate::capture) fn screen_at(app: &AppHandle, x: i32, y: i32) -> Result<Screen, String> {
    let monitors = app.available_monitors().map_err(err)?;
    let mut layout: Vec<_> = monitors
        .iter()
        .map(|m| {
            (
                m.position().x,
                m.position().y,
                m.size().width,
                m.size().height,
                u64::from((m.scale_factor() as f32).to_bits()),
            )
        })
        .collect();
    layout.sort_unstable();
    let monitor = monitors
        .into_iter()
        .find(|m| (m.position().x, m.position().y) == (x, y))
        .ok_or("来源显示器已移除或布局改变。")?;
    Ok(Screen {
        name: monitor.name().cloned(),
        x,
        y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale: monitor.scale_factor().to_bits(),
        layout,
    })
}

/// 预览窗与控制条接收同一版本快照；图片是否渲染由各窗口自行决定。
fn emit_update(app: &AppHandle, update: &ScrollView) {
    for label in ["scroll-preview", "scroll-controls"] {
        let _ = app.emit_to(label, "scroll-updated", update);
    }
}

struct TauriHost {
    app: AppHandle,
}

impl Host for TauriHost {
    fn state(&self) -> &ScrollState {
        &self.app.state::<CaptureState>().inner().scroll
    }
    fn operation(&self) -> &AtomicBool {
        &self.app.state::<CaptureState>().inner().operation
    }
    fn screen(&self, x: i32, y: i32) -> Result<Screen, String> {
        screen_at(&self.app, x, y)
    }
    fn hide_for_sample(&self, id: u64, windows: &[String]) -> Result<Box<dyn Any>, String> {
        Ok(Box::new(chrome::hide_for_sample(&self.app, id, windows)?))
    }
    fn updated(&self, view: &ScrollView) {
        emit_update(&self.app, view);
    }
    fn hide_windows(&self) {
        chrome::hide(&self.app);
    }
    fn show_output(&self) -> Result<(), String> {
        chrome::show_output(&self.app)
    }
    fn report_error(&self, message: String) {
        report_error(&self.app, message);
    }
    fn close_session(&self, id: u64) -> Result<(), String> {
        close_session(&self.app, id)
    }
    fn results_dir(&self) -> Result<PathBuf, String> {
        Ok(self
            .app
            .path()
            .app_local_data_dir()
            .map_err(err)?
            .join("scroll-results"))
    }
    fn publish(&self, root: &Path, png: &[u8]) -> Result<PathBuf, String> {
        use std::io::Write;
        std::fs::create_dir_all(root).map_err(err)?;
        let mut file =
            crate::shared::export_staging::TrackedFile::new(&self.app, root, "png", "滚动截图导出")?;
        file.file_mut().write_all(png).map_err(err)?;
        file.finalize()?;
        let path = root.join(format!("scroll-{}.png", file.id()));
        file.publish(&path, false)?;
        Ok(path)
    }
    fn copy(&self, image: &image::RgbaImage, png: &[u8]) -> Result<(), String> {
        crate::shared::image_io::copy_png(&self.app, image, png)
    }
    fn finished(&self, result: &ScrollView, message: String) {
        let _ = self.app.emit_to("capture", "scroll-updated", result);
        let _ = self.app.emit_to("main", "capture-result", message);
    }
}

#[tauri::command]
pub fn control_scroll(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    cancel: bool,
    confirmed_only: Option<bool>,
) -> Result<(), String> {
    if !matches!(window.label(), "scroll-preview" | "scroll-controls") {
        return Err("窗口无权控制滚动截图。".into());
    }
    if cancel {
        return if core::cancel(&TauriHost { app }, id, "window") {
            Ok(())
        } else {
            Err("滚动截图已结束。".into())
        };
    }
    let state = app.state::<CaptureState>();
    core::finish(&state.scroll, id, Some(confirmed_only.unwrap_or(false)))
}

/// 此函数由托盘和全局快捷键调用，不阻塞 UI；后台完成当前取样后直接输出成图。
pub fn stop(app: &AppHandle) -> bool {
    core::stop(&app.state::<CaptureState>().scroll)
}

#[tauri::command]
pub async fn start_scroll(
    app: AppHandle,
    window: WebviewWindow,
    id: u64,
    rect: Rect,
    save_frames: bool,
    background: Option<ExportBackground>,
) -> Result<ScrollView, String> {
    require_window(&window, "capture")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CaptureState>();
        let _operation = Operation::acquire(&state.operation)?;
        if state.scroll.session.lock().map_err(err)?.is_some() {
            return Err("滚动截图会话已存在。".into());
        }
        let seed = crop(&app, id, rect, &[])?;
        let (rect, screen) = super::super::selected_region(&app, id, rect)?;
        let data = if save_frames {
            app.path().app_local_data_dir().ok()
        } else {
            None
        };
        let placement = screen.clone();
        let mut session = core::new_session(
            id,
            seed,
            rect,
            screen,
            data.as_deref(),
            background.unwrap_or_default(),
        )?;
        let mut pending_windows = chrome::PendingScrollWindows(&app, false);
        session.obscuring_windows = chrome::place(&app, &placement, rect)?;
        super::super::hide_overlay(&app, &window)?;
        let result = core::install(&state.scroll, session)?;
        emit_update(&app, &result);
        let host: Arc<dyn Host> = Arc::new(TauriHost { app: app.clone() });
        if let Err(error) = chrome::show(&app).and_then(|_| core::launch(host, id, 120)) {
            core::abort(&state.scroll)?;
            chrome::hide(&app);
            let _ = window.show();
            return Err(error);
        }
        // 后续匹配仅使用选区帧/图块；不再常驻整桌面的 RGBA 和 Base64。
        // Operation 在本闭包结束前仍被持有，采集线程不会抢先结束会话。
        super::super::release_selection(&app, id)?;
        pending_windows.1 = true;
        log::info!(
            "event=scroll.started module=scroll op=scroll-{id} width={} height={}",
            result.width,
            result.height
        );
        Ok(result)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn current_scroll(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<ScrollView>, String> {
    if !matches!(
        window.label(),
        "capture" | "scroll-preview" | "scroll-controls"
    ) {
        return Err("窗口无权读取滚动截图。".into());
    }
    tauri::async_runtime::spawn_blocking(move || core::current(&app.state::<CaptureState>().scroll))
        .await
        .map_err(err)?
}
