//! macOS 与 Linux（Wayland）的系统拖出：gpui-pre 的元素拖拽在指针离开窗口时把本地文件交给系统
//! （`on_drag` + `external_drag_payload`）。平台只接受文件，文字先写成临时 .txt；X11 后端不支持外部拖拽，
//! 拖到窗口外没有效果。Windows 仍用 core 的 OLE 拖拽（`snap_core::drag::start`），不经过这里。
use gpui_kit::*;
use std::path::PathBuf;

/// 拖拽期间跟随指针的预览：不显示内容（系统拖拽接管后由系统绘制图标）。
pub struct Ghost;

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// 拖出内容转为平台文件拖拽；准备失败时记日志并放弃这次拖出。
pub fn files(payload: Result<snap_core::drag::Payload, String>) -> Option<ExternalDragPayload> {
    let paths = match payload.and_then(|payload| match payload {
        snap_core::drag::Payload::Text(text) => snap_core::drag::text_file(&text).map(|path| vec![path]),
        snap_core::drag::Payload::Files(paths) => Ok(paths),
    }) {
        Ok(paths) if !paths.is_empty() => paths,
        Ok(_) => return None,
        Err(error) => {
            log::warn!("event=gpui.drag_out_failed message={error}");
            return None;
        }
    };
    let entries = paths.into_iter().map(|path| {
        let path = PathBuf::from(path);
        let directory = path.is_dir();
        (path, directory)
    });
    Some(ExternalDragPayload::Files(FileDragPaths::new(entries)))
}
