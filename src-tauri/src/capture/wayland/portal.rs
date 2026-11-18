//! xdg-desktop-portal：ScreenCast 单帧（GNOME 等没有私有截图协议的桌面）与 Screenshot 兜底。
//!
//! ScreenCast 首次截图由用户在系统对话框中选择显示器；授权以 `persist_mode=2` 持久化，
//! 恢复令牌只在真正收到画面后保存，此后不再弹窗。令牌失效时清除并重新请求一次授权。
//! 用户选中的显示器即截图目标，与焦点显示器无关。
use super::{outputs, Output, Shot};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{DeserializeDict, OwnedObjectPath, OwnedValue, Type, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";

/// 授权对话框等待用户操作的上限。
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

fn connect() -> Result<Connection, String> {
    zbus::blocking::connection::Builder::session()
        .map(|builder| builder.method_timeout(Duration::from_secs(30)))
        .and_then(|builder| builder.build())
        .map_err(|e| e.to_string())
}

fn token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "snapdesk{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// 门户请求对象路径由调用方唯一名与 handle_token 决定（`:1.42` → `1_42`），须在调用前订阅其 Response。
pub(super) fn request_path(unique_name: &str, handle: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{handle}")
}

/// 发起一个门户请求并等待 Response；`call` 收到本次的 handle_token。返回值为 Response 的结果字典。
fn request<R>(conn: &Connection, call: impl FnOnce(&str) -> Result<(), String>) -> Result<R, String>
where
    R: for<'d> serde::Deserialize<'d> + Type + Send + 'static,
{
    let handle = token();
    let unique = conn
        .unique_name()
        .ok_or("D-Bus 连接没有唯一名称。")?
        .to_string();
    let path = request_path(&unique, &handle);
    // 路径按值传入，代理与信号迭代器不借用局部变量，可以移入等待线程。
    let proxy = Proxy::new(conn, PORTAL, path, "org.freedesktop.portal.Request")
        .map_err(|e| e.to_string())?;
    let mut signals = proxy
        .receive_signal("Response")
        .map_err(|e| e.to_string())?;
    call(&handle)?;
    let (sender, receiver) = std::sync::mpsc::channel();
    // 信号迭代器没有超时；在独立线程中等待，超时后放弃本次请求。
    std::thread::Builder::new()
        .name("portal-response".into())
        .spawn(move || {
            let message = signals.next();
            let _ = sender.send(message.map(|message| {
                message
                    .body()
                    .deserialize::<(u32, R)>()
                    .map_err(|e| e.to_string())
            }));
        })
        .map_err(|e| e.to_string())?;
    let response = receiver
        .recv_timeout(RESPONSE_TIMEOUT)
        .map_err(|_| "等待屏幕共享授权超时。".to_string())?
        .ok_or("屏幕共享门户没有回应。")??;
    match response.0 {
        0 => Ok(response.1),
        1 => Err(CANCELLED.into()),
        code => Err(format!("门户拒绝了请求（{code}）。")),
    }
}

pub(crate) const CANCELLED: &str = "已取消屏幕共享授权。";

type Results = HashMap<String, OwnedValue>;

/// 规范规定 session_handle 为字符串，部分实现返回对象路径；两种都接受。
fn session_handle(mut results: Results) -> Result<OwnedObjectPath, String> {
    let value = results
        .remove("session_handle")
        .ok_or("屏幕共享门户没有返回会话。")?;
    match &*value {
        Value::Str(text) => OwnedObjectPath::try_from(text.as_str()).map_err(|e| e.to_string()),
        Value::ObjectPath(path) => Ok(path.clone().into()),
        _ => Err("屏幕共享门户返回了无效的会话。".into()),
    }
}

#[derive(DeserializeDict, Type, Debug, Default)]
#[zvariant(signature = "dict")]
struct StreamProperties {
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
}

#[derive(DeserializeDict, Type, Debug, Default)]
#[zvariant(signature = "dict")]
struct Started {
    streams: Option<Vec<(u32, StreamProperties)>>,
    restore_token: Option<String>,
}

#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
struct Screenshot {
    uri: String,
}

/// 授权后的屏幕共享会话；析构时关闭，避免系统继续显示“正在共享屏幕”。
pub(crate) struct Session<'c> {
    conn: &'c Connection,
    pub handle: OwnedObjectPath,
    pub streams: Vec<(u32, Option<(i32, i32)>, Option<(i32, i32)>)>,
    pub restore_token: Option<String>,
}

impl Session<'_> {
    pub fn open_pipewire(&self) -> Result<std::os::fd::OwnedFd, String> {
        let proxy =
            Proxy::new(self.conn, PORTAL, PORTAL_PATH, SCREENCAST).map_err(|e| e.to_string())?;
        let fd: zbus::zvariant::OwnedFd = proxy
            .call(
                "OpenPipeWireRemote",
                &(&*self.handle, HashMap::<&str, Value>::new()),
            )
            .map_err(|e| e.to_string())?;
        Ok(fd.into())
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if let Ok(proxy) = Proxy::new(
            self.conn,
            PORTAL,
            self.handle.as_str(),
            "org.freedesktop.portal.Session",
        ) {
            let _ = proxy.call_method("Close", &());
        }
    }
}

/// 可用的光标模式（1 隐藏、2 嵌入画面）；旧版门户没有该属性时返回 0。
fn cursor_modes(conn: &Connection) -> u32 {
    Proxy::new(conn, PORTAL, PORTAL_PATH, SCREENCAST)
        .ok()
        .and_then(|proxy| proxy.get_property::<u32>("AvailableCursorModes").ok())
        .unwrap_or(0)
}

/// 创建会话、选择显示器并开始共享；`restore` 为上次保存的令牌。
pub(crate) fn start<'c>(
    conn: &'c Connection,
    restore: Option<&str>,
    cursor: bool,
    multiple: bool,
) -> Result<Session<'c>, String> {
    let proxy = Proxy::new(conn, PORTAL, PORTAL_PATH, SCREENCAST).map_err(|e| e.to_string())?;
    let session_token = token();
    let created: Results = request(conn, |handle| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(handle));
        options.insert("session_handle_token", Value::from(session_token.as_str()));
        proxy
            .call_method("CreateSession", &(options,))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let handle = session_handle(created)?;
    let mut session = Session {
        conn,
        handle,
        streams: Vec::new(),
        restore_token: None,
    };
    let modes = cursor_modes(conn);
    let wanted_cursor = if cursor { 2 } else { 1 };
    let _: Results = request(conn, |handle| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(handle));
        options.insert("types", Value::from(1u32));
        options.insert("multiple", Value::from(multiple));
        if modes & wanted_cursor != 0 {
            options.insert("cursor_mode", Value::from(wanted_cursor));
        }
        options.insert("persist_mode", Value::from(2u32));
        if let Some(restore) = restore {
            options.insert("restore_token", Value::from(restore));
        }
        proxy
            .call_method("SelectSources", &(&*session.handle, options))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let started: Started = request(conn, |handle| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(handle));
        proxy
            .call_method("Start", &(&*session.handle, "", options))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    session.streams = started
        .streams
        .unwrap_or_default()
        .into_iter()
        .map(|(node, properties)| (node, properties.position, properties.size))
        .collect();
    session.restore_token = started.restore_token;
    if session.streams.is_empty() {
        return Err("屏幕共享没有返回任何显示器。".into());
    }
    Ok(session)
}

/// 恢复令牌存放在 `$XDG_STATE_HOME/snap-desk/`（默认 `~/.local/state/snap-desk/`），仅当前用户可读写。
pub(crate) fn token_path(name: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("snap-desk").join(name))
}

pub(crate) fn read_token(name: &str) -> Option<String> {
    let value = std::fs::read_to_string(token_path(name)?).ok()?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= 512).then(|| value.to_owned())
}

pub(crate) fn write_token(name: &str, value: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(path) = token_path(name) else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)
        })
        .and_then(|mut file| file.write_all(value.as_bytes()));
    if let Err(error) = written {
        log::warn!("event=capture.portal_token_failed module=capture error={error}");
    }
}

pub(crate) fn forget_token(name: &str) {
    if let Some(path) = token_path(name) {
        let _ = std::fs::remove_file(path);
    }
}

const SCREENSHOT_TOKEN: &str = "screencast-token";

/// 带令牌的授权失败（显示器变化或令牌被撤销）时清除令牌，再请求一次完整授权。
pub(crate) fn start_with_token<'c>(
    conn: &'c Connection,
    name: &str,
    cursor: bool,
    multiple: bool,
) -> Result<Session<'c>, String> {
    let saved = read_token(name);
    match start(conn, saved.as_deref(), cursor, multiple) {
        Err(error) if saved.is_some() && error != CANCELLED => {
            log::info!("event=capture.portal_token_rejected module=capture error={error}");
            forget_token(name);
            start(conn, None, cursor, multiple)
        }
        result => result,
    }
}

pub(super) fn screencast(layout: &[Output], _target: usize, cursor: bool) -> Result<Shot, String> {
    let conn = connect()?;
    let session = start_with_token(&conn, SCREENSHOT_TOKEN, cursor, false)?;
    // 用户在对话框中选中的显示器即截图目标；对应不上时按唯一流处理，几何取流自身的尺寸。
    let (node, position, size) = session.streams[0];
    let fd = session.open_pipewire()?;
    let image = super::pipewire_frame::grab(&fd, node)?;
    let output = match outputs::match_stream(layout, position, size) {
        Some(index) => layout[index].clone(),
        None => {
            let (x, y) = position.unwrap_or((0, 0));
            let (width, height) = size.unwrap_or((image.width() as i32, image.height() as i32));
            Output {
                name: String::new(),
                x,
                y,
                width,
                height,
                physical: image.dimensions(),
            }
        }
    };
    if let Some(value) = &session.restore_token {
        write_token(SCREENSHOT_TOKEN, value);
    }
    drop(session);
    Ok(Shot { output, image })
}

/// `file://` URI 转为本地路径；只接受本机绝对路径。
pub(super) fn uri_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(std::ffi::OsString::from_vec(decoded).into())
}

/// Screenshot 门户返回整个桌面（各显示器拼在一起）的文件；按逻辑布局比例裁出目标显示器。
pub(super) fn screenshot(layout: &[Output], target: usize) -> Result<Shot, String> {
    let output = layout.get(target).ok_or("未找到目标显示器。")?.clone();
    let conn = connect()?;
    let proxy = Proxy::new(
        &conn,
        PORTAL,
        PORTAL_PATH,
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(|e| e.to_string())?;
    let result: Screenshot = request(&conn, |handle| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(handle));
        options.insert("interactive", Value::from(false));
        options.insert("modal", Value::from(false));
        proxy
            .call_method("Screenshot", &("", options))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let path = uri_path(&result.uri).ok_or("截图门户返回了无效的文件地址。")?;
    let loaded = image::open(&path);
    // 门户会把截图保存到用户目录；读完即删除，不留下额外文件。
    let _ = std::fs::remove_file(&path);
    let full = loaded.map_err(|e| e.to_string())?.to_rgba8();
    let (left, top, right, bottom) = bounds(layout);
    let ratio_x = f64::from(full.width()) / f64::from((right - left).max(1));
    let ratio_y = f64::from(full.height()) / f64::from((bottom - top).max(1));
    let crop = |value: i32, origin: i32, ratio: f64| {
        ((f64::from(value - origin)) * ratio).round().max(0.0) as u32
    };
    let x = crop(output.x, left, ratio_x);
    let y = crop(output.y, top, ratio_y);
    let width = crop(output.x + output.width, left, ratio_x)
        .min(full.width())
        .saturating_sub(x);
    let height = crop(output.y + output.height, top, ratio_y)
        .min(full.height())
        .saturating_sub(y);
    if width == 0 || height == 0 {
        return Err("截图门户返回的图片与显示器布局不一致。".into());
    }
    let image = image::imageops::crop_imm(&full, x, y, width, height).to_image();
    Ok(Shot { output, image })
}

fn bounds(layout: &[Output]) -> (i32, i32, i32, i32) {
    let left = layout.iter().map(|o| o.x).min().unwrap_or(0);
    let top = layout.iter().map(|o| o.y).min().unwrap_or(0);
    let right = layout.iter().map(|o| o.x + o.width).max().unwrap_or(1);
    let bottom = layout.iter().map(|o| o.y + o.height).max().unwrap_or(1);
    (left, top, right, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_paths_follow_the_portal_convention() {
        assert_eq!(
            request_path(":1.42", "snapdesk7_0"),
            "/org/freedesktop/portal/desktop/request/1_42/snapdesk7_0"
        );
    }

    #[test]
    fn decodes_local_file_uris_only() {
        assert_eq!(
            uri_path("file:///home/u/Pictures/Screenshot%20from%202026.png"),
            Some("/home/u/Pictures/Screenshot from 2026.png".into())
        );
        assert_eq!(
            uri_path("file://localhost/tmp/a.png"),
            Some("/tmp/a.png".into())
        );
        assert_eq!(uri_path("https://example.com/a.png"), None);
        assert_eq!(uri_path("file://host/tmp/a.png"), None);
        assert_eq!(uri_path("file:///tmp/%zz"), None);
    }
}
