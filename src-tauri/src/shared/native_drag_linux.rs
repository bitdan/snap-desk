//! Linux 拖出：在窗口的 GTK 部件上发起 GTK 拖拽，由 GTK 负责 X11 的 XDND 与 Wayland 的数据设备协议。
//!
//! 请求经前端转发后才到达，此时没有可用的 GDK 按下事件，因此不传事件：GTK 改用默认座位的指针
//! 与当前位置作为起点。会话异步进行，调用在发起后立即返回；只允许“复制”，数据在接收方
//! 请求时才写入，文件以 `text/uri-list` 提供，文字以 UTF-8 文本目标提供。
use super::Payload;
use gtk::{gdk, glib, prelude::*};
use std::cell::RefCell;
use tauri::WebviewWindow;

thread_local! {
    /// 只在 GTK 主线程访问；结束或失败时清空，期间拒绝新的拖拽，避免两次会话争用指针抓取。
    static ACTIVE: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// 本次会话连接的信号；结束时断开，下一次拖拽不会收到上一份数据。
struct Session {
    widget: gtk::ApplicationWindow,
    handlers: Vec<glib::SignalHandlerId>,
}

pub(super) fn start(window: &WebviewWindow, payload: Payload) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = sender.send(begin(&target, payload));
        })
        .map_err(super::err)?;
    receiver.recv().map_err(super::err)?
}

/// 文件路径编码为 `file://` URI；GLib 负责百分号转义非 ASCII 与保留字符。
fn uris(paths: &[String]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| {
            glib::filename_to_uri(path, None)
                .map(|uri| uri.to_string())
                .map_err(|_| "文件路径无法编码为 URI。".to_string())
        })
        .collect()
}

fn begin(window: &WebviewWindow, payload: Payload) -> Result<(), String> {
    if ACTIVE.with(|active| active.borrow().is_some()) {
        return Err("另一个拖拽正在进行。".into());
    }
    let widget = window.gtk_window().map_err(super::err)?;
    let surface = widget.window().ok_or("窗口尚未显示，无法拖出。")?;
    // 大图编码期间用户可能已松手，此时取消，不能在晚到的启动请求中意外放下。
    let pointer = gdk::Display::default()
        .and_then(|display| display.default_seat())
        .and_then(|seat| seat.pointer())
        .ok_or("无法读取指针设备。")?;
    let (_, _, _, mask) = surface.device_position(&pointer);
    if !mask.contains(gdk::ModifierType::BUTTON1_MASK) {
        return Ok(());
    }
    let targets = gtk::TargetList::new(&[]);
    let data = match payload {
        Payload::Text(text) => {
            targets.add_text_targets(0);
            Data::Text(text)
        }
        Payload::Files(paths) => {
            targets.add_uri_targets(0);
            Data::Uris(uris(&paths)?)
        }
    };
    let get = widget.connect_drag_data_get(move |_, _, selection, _, _| {
        // 返回值表示目标格式是否可写；接收方请求了不支持的格式时保持为空即可。
        let _ = match &data {
            Data::Text(text) => selection.set_text(text),
            Data::Uris(uris) => {
                let uris = uris.iter().map(String::as_str).collect::<Vec<_>>();
                selection.set_uris(&uris)
            }
        };
    });
    let end = widget.connect_drag_end(|_, _| finish());
    let failed = widget.connect_drag_failed(|_, _, _| {
        finish();
        glib::Propagation::Proceed
    });
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some(Session {
            widget: widget.clone(),
            handlers: vec![get, end, failed],
        })
    });
    let context = widget.drag_begin_with_coordinates(
        &targets,
        gdk::DragAction::COPY,
        1,
        None,
        -1,
        -1,
    );
    if context.is_none() {
        finish();
        return Err("系统拒绝了拖拽请求。".into());
    }
    Ok(())
}

enum Data {
    Text(String),
    Uris(Vec<String>),
}

/// drag-end 与 drag-failed 可能先后都触发，只有第一次真正断开信号。
fn finish() {
    let Some(session) = ACTIVE.with(|active| active.borrow_mut().take()) else {
        return;
    };
    // 在信号处理函数内断开自身不安全，推迟到下一次主循环迭代。
    glib::idle_add_local_once(move || {
        for handler in session.handlers {
            session.widget.disconnect(handler);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_become_escaped_file_uris() {
        let uris = uris(&["/tmp/a b/报告#1.png".into()]).unwrap();
        assert_eq!(uris.len(), 1);
        assert!(uris[0].starts_with("file:///tmp/a%20b/"));
        assert!(!uris[0].contains(' ') && !uris[0].contains('#'));
        assert!(uris(&["relative.png".into()]).is_err());
    }
}
