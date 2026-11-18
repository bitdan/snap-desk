//! X11 剪贴板：XFixes 订阅 CLIPBOARD 所有者变化。监听线程只发送有界唤醒，
//! 读取、解码和 SQL 仍由存储工作线程完成，与 Windows / macOS 分工一致。
//! X11 没有全局序列号，`sequence` 由一条常驻连接统计 XFixes 通知，读取前先同步一次往返，
//! 因此刚写入的内容（包括本程序自己的写回）在返回前已被计入。
use super::super::{model::ClipboardSettings, reader::Snapshot};
use crate::shared::x11::{keysym, process_name, ReadError, SelectionReader, X11};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex, OnceLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use xcb::{x, xfixes, Xid, XidNew};

/// 监听线程检查停止标记的间隔；事件本身不经轮询，到达后在一个间隔内转发。
const TICK: Duration = Duration::from_millis(50);
/// KeePassXC、KDE 等写入密码时附带的标记目标，存在即不记录。
const SECRET_HINT: &str = "x-kde-passwordManagerHint";
const URI_LIST: &str = "text/uri-list";
const GNOME_FILES: &str = "x-special/gnome-copied-files";
const IMAGE_LIMIT: usize = 64 * 1024 * 1024;

fn selection_mask() -> xfixes::SelectionEventMask {
    xfixes::SelectionEventMask::SET_SELECTION_OWNER
        | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
        | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE
}

/// 打开连接并订阅 CLIPBOARD 变化；返回连接与 CLIPBOARD 原子。
fn subscribe() -> Result<(X11, x::Atom), String> {
    let x11 = X11::connect_selections()?;
    x11.xfixes()?;
    let clipboard = x11.intern("CLIPBOARD")?;
    let window = x11.hidden_window(x::EventMask::empty())?;
    x11.conn
        .send_and_check_request(&xfixes::SelectSelectionInput {
            window,
            selection: clipboard,
            event_mask: selection_mask(),
        })
        .map_err(|_| "无法监听系统剪贴板，可在历史页重试。")?;
    Ok((x11, clipboard))
}

fn is_change(event: &xcb::Event, clipboard: x::Atom) -> bool {
    matches!(event, xcb::Event::XFixes(xfixes::Event::SelectionNotify(ev)) if ev.selection() == clipboard)
}

pub struct Watcher {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Watcher {
    pub fn start(tx: SyncSender<()>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (ready, result) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("clipboard-xfixes".into())
            .spawn(move || {
                let (x11, clipboard) = match subscribe() {
                    Ok(v) => {
                        let _ = ready.send(Ok(()));
                        v
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                while !stopped.load(Ordering::Acquire) {
                    match x11.conn.poll_for_event() {
                        Ok(Some(event)) => {
                            if is_change(&event, clipboard) {
                                // 队列已满说明工作线程尚未处理上一次唤醒，它会读到最新内容，丢弃即可。
                                let _ = tx.try_send(());
                            }
                        }
                        Ok(None) => std::thread::sleep(TICK),
                        Err(xcb::Error::Protocol(_)) => {}
                        // 连接断开（X 服务器退出）后不再有通知；工作线程仍会按周期核对序列号。
                        Err(xcb::Error::Connection(_)) => break,
                    }
                }
            })
            .map_err(|_| "无法启动剪贴板监听线程。")?;
        match result.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                worker: Some(worker),
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err("剪贴板监听线程提前结束。".into())
            }
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Tracker {
    x11: X11,
    clipboard: x::Atom,
    count: u32,
}

/// 常驻的计数连接；X 服务器不可用时每次调用重试连接，断开后自动重建。
pub fn sequence() -> u32 {
    static TRACKER: Mutex<Option<Tracker>> = Mutex::new(None);
    let Ok(mut guard) = TRACKER.lock() else {
        return 0;
    };
    if guard.is_none() {
        *guard = subscribe().ok().map(|(x11, clipboard)| Tracker {
            x11,
            clipboard,
            count: 0,
        });
    }
    let Some(tracker) = guard.as_mut() else {
        return 0;
    };
    // 往返完成时，服务器在此之前产生的所有者变化通知都已进入本连接的队列。
    if tracker.x11.sync().is_err() {
        *guard = None;
        return 0;
    }
    loop {
        match tracker.x11.conn.poll_for_event() {
            Ok(Some(event)) => {
                if is_change(&event, tracker.clipboard) {
                    tracker.count = tracker.count.wrapping_add(1);
                }
            }
            Ok(None) => return tracker.count,
            Err(xcb::Error::Protocol(_)) => {}
            Err(xcb::Error::Connection(_)) => {
                *guard = None;
                return 0;
            }
        }
    }
}

/// 读取顺序与 Windows / macOS 一致：文件引用优先，其次 PNG 图片，最后 UTF-8 文本。
pub fn snapshot(settings: &ClipboardSettings) -> Result<Option<Snapshot>, String> {
    let x11 = X11::connect_selections()?;
    let clipboard = x11.intern("CLIPBOARD")?;
    let reader = SelectionReader::new(&x11)?;
    let Some(owner) = reader.owner(clipboard) else {
        return Ok(None);
    };
    // 名单为空时不查询；所有者窗口通常是程序的隐藏窗口，取不到进程号时按前台窗口判断。
    if !settings.excluded_apps.is_empty() {
        let pid = x11
            .window_pid(owner)
            .or_else(|| x11.window_pid(x11.active_window()?));
        if pid
            .and_then(process_name)
            .is_some_and(|name| settings.excludes(&name))
        {
            return Ok(None);
        }
    }
    let targets = reader.targets(clipboard).map_err(busy)?;
    let has = |name: &str| x11.existing(name).filter(|atom| targets.contains(atom));
    if has(SECRET_HINT).is_some() {
        return Ok(None);
    }
    for (name, gnome) in [(URI_LIST, false), (GNOME_FILES, true)] {
        let Some(target) = has(name) else { continue };
        if !settings.save_copied_files {
            return Ok(None);
        }
        let Some(bytes) = reader
            .read(clipboard, target, super::super::files::MAX_BYTES)
            .map_err(|e| limit_error(e, "文件引用超过历史大小限制，未记录。"))?
        else {
            continue;
        };
        let paths = if gnome {
            parse_gnome_copied_files(&bytes)
        } else {
            parse_uri_list(&bytes)
        };
        // 只有 http 等非本地 URI 时不算文件，继续尝试图片和文本。
        if paths.is_empty() {
            continue;
        }
        super::super::files::validate(&paths)?;
        return Ok(Some(Snapshot::Paths(paths)));
    }
    if let Some(target) = has("image/png") {
        if !settings.save_copied_images {
            return Ok(None);
        }
        if let Some(png) = reader
            .read(clipboard, target, IMAGE_LIMIT)
            .map_err(|e| limit_error(e, "剪贴板内容超过单项大小限制，未记录。"))?
        {
            let (width, height) = png_dimensions(&png)?;
            check_pixels(width, height, settings)?;
            return Ok(Some(Snapshot::Png(png)));
        }
    }
    let Some(target) = has("UTF8_STRING").or_else(|| has("text/plain;charset=utf-8")) else {
        return Ok(None);
    };
    if !settings.save_copied_text {
        return Ok(None);
    }
    let Some(bytes) = reader
        .read(clipboard, target, settings.max_text_bytes as usize)
        .map_err(|e| limit_error(e, "文本超过历史大小限制，未记录。"))?
    else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(Snapshot::Utf8(
        String::from_utf8_lossy(&bytes).into_owned(),
    )))
}

fn busy(_: ReadError) -> String {
    "剪贴板所有者未响应，请稍后重试。".into()
}

fn limit_error(error: ReadError, too_large: &str) -> String {
    match error {
        ReadError::TooLarge => too_large.into(),
        e => busy(e),
    }
}

fn check_pixels(width: u32, height: u32, settings: &ClipboardSettings) -> Result<(), String> {
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > u64::from(settings.max_image_pixels)
    {
        return Err("图片超过历史像素限制，未记录。".into());
    }
    Ok(())
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" && &bytes[12..16] == b"IHDR" {
        return Ok((
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        ));
    }
    Err("剪贴板图片头无效，未记录。".into())
}

/// RFC 3986 非保留字符与路径分隔符原样保留，其余字节（含中文的 UTF-8 字节）一律百分号编码。
fn file_uri(path: &str) -> String {
    let mut uri = String::with_capacity(path.len() + 7);
    uri.push_str("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `text/uri-list`（RFC 2483）：每行一个 URI，行尾 CRLF。
fn uri_list(paths: &[String]) -> Vec<u8> {
    paths
        .iter()
        .flat_map(|path| [file_uri(path), "\r\n".into()])
        .collect::<String>()
        .into_bytes()
}

/// Nautilus / Nemo / Caja 的格式：首行 `copy`（不是 `cut`，粘贴时复制而非移动），其后每行一个 URI。
fn gnome_copied_files(paths: &[String]) -> Vec<u8> {
    std::iter::once("copy".to_string())
        .chain(paths.iter().map(|path| file_uri(path)))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

/// 只接受本机 `file://` URI（主机为空或 localhost）；解码后不是 UTF-8 或含 NUL 的条目跳过。
fn file_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        rest.strip_prefix("localhost")?
    };
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    if decoded.contains(&0) {
        return None;
    }
    String::from_utf8(decoded).ok()
}

fn parse_uri_list(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(file_path)
        .collect()
}

/// 首行是操作（copy / cut），无论哪种都只记录路径；恢复时一律按复制写回。
fn parse_gnome_copied_files(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    match lines.next().map(str::trim) {
        Some("copy" | "cut") => lines.map(str::trim).filter_map(file_path).collect(),
        _ => Vec::new(),
    }
}

/// 本程序持有 CLIPBOARD 时提供的数据。
struct Offer {
    uri_list: Vec<u8>,
    gnome: Vec<u8>,
    text: Vec<u8>,
}

/// X11 剪贴板没有中心存储：写入方必须持有选区并应答每次粘贴请求，
/// 因此文件引用由一条常驻连接上的线程提供，直到其他程序取得剪贴板。
struct Owner {
    x11: X11,
    window: x::Window,
    clipboard: x::Atom,
    offer: Mutex<Option<Arc<Offer>>>,
}

impl Owner {
    fn get() -> Result<Arc<Owner>, String> {
        static OWNER: OnceLock<Mutex<Option<Arc<Owner>>>> = OnceLock::new();
        let mut slot = OWNER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "剪贴板写入状态不可用。")?;
        if let Some(owner) = slot.as_ref().filter(|o| o.x11.conn.has_error().is_ok()) {
            return Ok(owner.clone());
        }
        let x11 = X11::connect_selections()?;
        let owner = Arc::new(Owner {
            window: x11.hidden_window(x::EventMask::empty())?,
            clipboard: x11.intern("CLIPBOARD")?,
            x11,
            offer: Mutex::new(None),
        });
        let atoms = Atoms::new(&owner.x11)?;
        let serving = owner.clone();
        std::thread::Builder::new()
            .name("clipboard-owner".into())
            .spawn(move || serving.serve(&atoms))
            .map_err(|_| "无法启动剪贴板写入线程。")?;
        *slot = Some(owner.clone());
        Ok(owner)
    }

    fn serve(&self, atoms: &Atoms) {
        loop {
            match self.x11.conn.wait_for_event() {
                Ok(xcb::Event::X(x::Event::SelectionRequest(ev))) => self.answer(&ev, atoms),
                Ok(xcb::Event::X(x::Event::SelectionClear(ev)))
                    if ev.selection() == self.clipboard =>
                {
                    if let Ok(mut offer) = self.offer.lock() {
                        offer.take();
                    }
                }
                Ok(_) | Err(xcb::Error::Protocol(_)) => {}
                Err(xcb::Error::Connection(_)) => return,
            }
        }
    }

    /// 按 ICCCM 应答：把数据写到请求方窗口的属性，再发送 SelectionNotify；不支持的目标以 None 拒绝。
    fn answer(&self, ev: &x::SelectionRequestEvent, atoms: &Atoms) {
        // 旧客户端可能不给属性名，按规范改用目标名。
        let property = if ev.property() == x::ATOM_NONE {
            ev.target()
        } else {
            ev.property()
        };
        let offer = self.offer.lock().ok().and_then(|o| o.clone());
        let accepted = match offer {
            Some(offer) if ev.selection() == self.clipboard => {
                let target = ev.target();
                if target == atoms.targets {
                    self.put(ev.requestor(), property, x::ATOM_ATOM, &atoms.offered()[..])
                } else if target == atoms.uri_list {
                    self.put(ev.requestor(), property, target, offer.uri_list.as_slice())
                } else if target == atoms.gnome {
                    self.put(ev.requestor(), property, target, offer.gnome.as_slice())
                } else if target == atoms.utf8 || target == atoms.plain {
                    self.put(ev.requestor(), property, target, offer.text.as_slice())
                } else {
                    false
                }
            }
            _ => false,
        };
        let event = x::SelectionNotifyEvent::new(
            ev.time(),
            ev.requestor(),
            ev.selection(),
            ev.target(),
            if accepted { property } else { x::ATOM_NONE },
        );
        self.x11.conn.send_request(&x::SendEvent {
            propagate: false,
            destination: x::SendEventDest::Window(ev.requestor()),
            event_mask: x::EventMask::empty(),
            event: &event,
        });
        let _ = self.x11.conn.flush();
    }

    /// 文件列表最多 1 MiB，远小于常见的最大请求长度，不需要 INCR 分段；超出时拒绝而不是截断。
    fn put<P: x::PropEl>(
        &self,
        window: x::Window,
        property: x::Atom,
        r#type: x::Atom,
        data: &[P],
    ) -> bool {
        let max = (self.x11.conn.get_maximum_request_length() as usize).saturating_mul(4);
        if std::mem::size_of_val(data) + 64 > max {
            return false;
        }
        self.x11.conn.send_request(&x::ChangeProperty {
            mode: x::PropMode::Replace,
            window,
            property,
            r#type,
            data,
        });
        true
    }
}

struct Atoms {
    targets: x::Atom,
    uri_list: x::Atom,
    gnome: x::Atom,
    utf8: x::Atom,
    plain: x::Atom,
}
impl Atoms {
    fn new(x11: &X11) -> Result<Self, String> {
        Ok(Self {
            targets: x11.intern("TARGETS")?,
            uri_list: x11.intern(URI_LIST)?,
            gnome: x11.intern(GNOME_FILES)?,
            utf8: x11.intern("UTF8_STRING")?,
            plain: x11.intern("text/plain;charset=utf-8")?,
        })
    }
    fn offered(&self) -> [x::Atom; 5] {
        [
            self.targets,
            self.uri_list,
            self.gnome,
            self.utf8,
            self.plain,
        ]
    }
}

/// 写回文件引用：`text/uri-list` 供 KDE / 浏览器等使用，`x-special/gnome-copied-files` 供 GNOME 系文件管理器
/// 识别为“复制”；纯文本目标给出换行分隔的路径，便于粘贴到终端。
pub fn write_files(paths: &[String]) -> Result<(), String> {
    super::super::files::validate(paths)?;
    for path in paths {
        if std::fs::metadata(path).is_err() {
            return Err(format!("原文件已移动、删除或不可访问，无法复制：{path}"));
        }
    }
    let owner = Owner::get()?;
    *owner.offer.lock().map_err(|_| "剪贴板写入状态不可用。")? = Some(Arc::new(Offer {
        uri_list: uri_list(paths),
        gnome: gnome_copied_files(paths),
        text: paths.join("\n").into_bytes(),
    }));
    owner.x11.conn.send_request(&x::SetSelectionOwner {
        owner: owner.window,
        selection: owner.clipboard,
        time: x::CURRENT_TIME,
    });
    let current = owner
        .x11
        .conn
        .wait_for_reply(owner.x11.conn.send_request(&x::GetSelectionOwner {
            selection: owner.clipboard,
        }))
        .map_err(|_| "复制文件引用失败，请重试。")?
        .owner();
    if current != owner.window {
        return Err("复制文件引用失败，请重试。".into());
    }
    Ok(())
}

/// 唤出剪贴板窗口前的前台窗口；属于本程序或读取失败时返回 0。
pub fn foreground_window() -> usize {
    let Ok(x11) = X11::connect() else {
        return 0;
    };
    match x11.active_window() {
        Some(window) if x11.window_pid(window) != Some(std::process::id()) => {
            window.resource_id() as usize
        }
        _ => 0,
    }
}

fn window(target: usize) -> Option<x::Window> {
    u32::try_from(target)
        .ok()
        .filter(|id| *id != 0)
        .map(x::Window::new)
}

pub fn activate(target: usize) -> Result<(), String> {
    let x11 = X11::connect()?;
    let window = window(target).filter(|w| x11.window_exists(*w));
    let Some(window) = window else {
        return Err("原窗口已关闭，内容已复制，可手动粘贴。".into());
    };
    x11.activate(window)
}

/// 等原窗口回到前台、且 Shift/Alt/Super 与回车、V 已松开，再经 XTest 注入 Ctrl+V。
/// Ctrl 允许按着：Ctrl+数字触发时用户的 Ctrl 与注入的 Ctrl 效果相同。
pub fn send_paste(target: usize) -> Result<(), String> {
    let x11 = X11::connect()?;
    let window = window(target).ok_or("未能回到原窗口，内容已复制，可手动粘贴。")?;
    let held: Vec<u8> = x11
        .keycodes(&[
            keysym::RETURN,
            keysym::KP_ENTER,
            keysym::SHIFT_L,
            keysym::SHIFT_R,
            keysym::ALT_L,
            keysym::ALT_R,
            keysym::META_L,
            keysym::META_R,
            keysym::SUPER_L,
            keysym::SUPER_R,
            keysym::ISO_LEVEL3_SHIFT,
            keysym::V,
        ])
        .into_iter()
        .map(|(_, code)| code)
        .collect();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let front = x11.active_window() == Some(window);
        if front && !x11.any_pressed(&held) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(if front {
                "请松开按键后重试，内容已复制，可手动粘贴。".into()
            } else {
                "未能回到原窗口，内容已复制，可手动粘贴。".into()
            });
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    x11.ctrl_chord(keysym::V)
        .map_err(|_| "无法粘贴到原窗口，内容已复制，可手动粘贴。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_references_are_percent_encoded_for_both_targets() {
        let paths = vec!["/home/用户/a b#1.txt".to_string(), "/tmp/x%y".to_string()];
        let list = String::from_utf8(uri_list(&paths)).unwrap();
        assert_eq!(
            list,
            "file:///home/%E7%94%A8%E6%88%B7/a%20b%231.txt\r\nfile:///tmp/x%25y\r\n"
        );
        let gnome = String::from_utf8(gnome_copied_files(&paths)).unwrap();
        assert_eq!(
            gnome,
            "copy\nfile:///home/%E7%94%A8%E6%88%B7/a%20b%231.txt\nfile:///tmp/x%25y"
        );
        assert_eq!(parse_uri_list(list.as_bytes()), paths);
        assert_eq!(parse_gnome_copied_files(gnome.as_bytes()), paths);
    }
    #[test]
    fn only_local_file_uris_become_paths() {
        let list = b"# comment\r\nfile://localhost/etc/hosts\r\nhttps://example.com/a\r\nfile://server/share/a\r\nfile:///bad%2\r\nfile:///nul%00\r\nfile:///ok\n";
        assert_eq!(parse_uri_list(list), vec!["/etc/hosts", "/ok"]);
        assert_eq!(
            parse_gnome_copied_files(b"cut\nfile:///tmp/moved"),
            vec!["/tmp/moved"]
        );
        assert!(parse_gnome_copied_files(b"file:///tmp/a").is_empty());
        assert!(parse_gnome_copied_files(b"").is_empty());
    }
    #[test]
    fn png_header_is_checked_before_decoding() {
        assert!(png_dimensions(b"PNG").is_err());
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend_from_slice(&640u32.to_be_bytes());
        header.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_dimensions(&header).unwrap(), (640, 480));
        let settings = ClipboardSettings {
            max_image_pixels: 100,
            ..ClipboardSettings::default()
        };
        assert!(check_pixels(640, 480, &settings).is_err());
        assert!(check_pixels(10, 10, &settings).is_ok());
    }
}
