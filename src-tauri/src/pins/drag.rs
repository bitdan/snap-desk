//! 贴图内容的系统拖出：数据只取当前窗口，图片合成后作为只读临时 PNG 提供。
use super::{composed, err, own, Operation, PinState};
use crate::shared::native_drag::{self, Payload};
use tauri::{AppHandle, Manager, WebviewWindow};

/// 编码在后台完成，系统拖拽在 UI 线程发起；等待期间不持有贴图集合锁。
#[tauri::command]
pub async fn drag_pin_content(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let pin = own(&app, &window)?;
    if pin.view.locked || pin.view.click_through {
        return Err("贴图已锁定或已开启鼠标穿透。".into());
    }
    if !native_drag::SUPPORTED {
        return Err("当前平台尚未支持贴图内容拖出。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let payload = match &pin.drag_text {
            // 文件贴图同样拖出路径文字。
            Some(text) => Payload::Text(text.to_string()),
            None => Payload::Files(vec![native_drag::image_file(&app, &composed(&pin)?)?]),
        };
        native_drag::start(&window, payload)
    })
    .await
    .map_err(err)?
}
