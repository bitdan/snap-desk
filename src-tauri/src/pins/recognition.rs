//! 识别结果临时借用贴图窗口空间；关闭后恢复原位与尺寸，不改变图片缩放和视图设置。
use super::*;

#[derive(Clone, Copy)]
pub(super) struct WindowBounds {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

/// 小贴图也能显示与截图相同的结果面板；仅当前贴图能调整自己的临时窗口布局。
#[tauri::command]
pub async fn set_pin_recognition_open(
    app: AppHandle,
    window: WebviewWindow,
    expanded: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        let _dpi = crate::capture::desktop::PhysicalDpi::enter()?;
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let pin = own(&app, &window)?;
        // 原生窗口调用可能切回 UI 线程；不能持有销毁回调也会获取的窗口几何锁。
        let saved = state
            .recognition_bounds
            .lock()
            .map_err(err)?
            .get(window.label())
            .copied();
        if !expanded {
            if let Some(bounds) = saved {
                window.set_size(bounds.size).map_err(err)?;
                window.set_position(bounds.position).map_err(err)?;
                state
                    .recognition_bounds
                    .lock()
                    .map_err(err)?
                    .remove(window.label());
            }
            return Ok(());
        }
        if pin.view.click_through {
            return Err("请先恢复贴图鼠标操作。".into());
        }
        if saved.is_some() {
            return Ok(());
        }
        let previous = WindowBounds {
            position: window.outer_position().map_err(err)?,
            size: window.inner_size().map_err(err)?,
        };
        let monitor = window
            .current_monitor()
            .map_err(err)?
            .ok_or("无法获取显示器信息。")?;
        let work = monitor.work_area();
        if work.size.width == 0 || work.size.height == 0 {
            return Err("无法获取显示器信息。".into());
        }
        let width = ((1080.0 * monitor.scale_factor()).round() as u32)
            .min(work.size.width)
            .max(1);
        let height = ((f64::from(width) * 0.618).round() as u32)
            .min(work.size.height)
            .max(1);
        let position = PhysicalPosition::new(
            work.position.x + ((work.size.width - width) / 2) as i32,
            work.position.y + ((work.size.height - height) / 2) as i32,
        );
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(err)?;
        if let Err(error) = window.set_position(position) {
            window.set_size(previous.size).map_err(err)?;
            return Err(err(error));
        }
        let mut saved = state.recognition_bounds.lock().map_err(err)?;
        if !state
            .images
            .lock()
            .map_err(err)?
            .contains_key(window.label())
        {
            return Err("贴图已关闭。".into());
        }
        saved.insert(window.label().to_string(), previous);
        Ok(())
    })
    .await
    .map_err(err)?
}
