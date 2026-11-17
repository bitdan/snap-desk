//! X11 共用设施：EWMH 前台窗口、XTest 按键注入与有界的选区读取。
//! 剪贴板历史、划词翻译和快捷键排查共用；截图仍使用 `capture::desktop_linux` 自己的连接。
//! Wayland 会话下 XWayland 只能看到 X 客户端，前台窗口与按键注入都不可靠，因此直接报错。
use std::time::{Duration, Instant};
use xcb::{randr, x, xfixes, Xid, XidNew};

pub(crate) const WAYLAND_HINT: &str = "暂不支持 Wayland 会话，请在登录界面选择 X11（Xorg）会话。";

pub(crate) fn wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// X keysym（与键盘布局无关的按键符号）。
pub(crate) mod keysym {
    pub const C: u32 = 0x63;
    pub const V: u32 = 0x76;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const SHIFT_L: u32 = 0xffe1;
    pub const SHIFT_R: u32 = 0xffe2;
    pub const CONTROL_L: u32 = 0xffe3;
    pub const CONTROL_R: u32 = 0xffe4;
    pub const META_L: u32 = 0xffe7;
    pub const META_R: u32 = 0xffe8;
    pub const ALT_L: u32 = 0xffe9;
    pub const ALT_R: u32 = 0xffea;
    pub const SUPER_L: u32 = 0xffeb;
    pub const SUPER_R: u32 = 0xffec;
    pub const ISO_LEVEL3_SHIFT: u32 = 0xfe03;
}

pub(crate) struct X11 {
    pub conn: xcb::Connection,
    pub root: x::Window,
}

impl X11 {
    pub fn connect() -> Result<Self, String> {
        if wayland() {
            return Err(WAYLAND_HINT.into());
        }
        let (conn, index) = xcb::Connection::connect_with_extensions(
            None,
            &[],
            &[
                xcb::Extension::XFixes,
                xcb::Extension::Test,
                xcb::Extension::RandR,
            ],
        )
        .map_err(|e| format!("无法连接 X 服务器：{e}"))?;
        let root = conn
            .get_setup()
            .roots()
            .nth(usize::try_from(index).unwrap_or(0))
            .ok_or("未找到 X 屏幕。")?
            .root();
        Ok(Self { conn, root })
    }

    /// 不存在时创建；选区目标名和自定义属性名都走这里。
    pub fn intern(&self, name: &str) -> Result<x::Atom, String> {
        self.conn
            .wait_for_reply(self.conn.send_request(&x::InternAtom {
                only_if_exists: false,
                name: name.as_bytes(),
            }))
            .map(|reply| reply.atom())
            .map_err(|_| "X 服务器无响应。".into())
    }

    /// 只查询已存在的原子；不存在说明没有任何程序使用过它。
    pub fn existing(&self, name: &str) -> Option<x::Atom> {
        let atom = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::InternAtom {
                only_if_exists: true,
                name: name.as_bytes(),
            }))
            .ok()?
            .atom();
        (atom != x::ATOM_NONE).then_some(atom)
    }

    /// 32 位属性的第一个值；格式不符或为空时返回 None，不信任其他程序写入的类型。
    pub fn first_u32(&self, window: x::Window, property: x::Atom, r#type: x::Atom) -> Option<u32> {
        let reply = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type,
                long_offset: 0,
                long_length: 1,
            }))
            .ok()?;
        if reply.format() != 32 {
            return None;
        }
        reply.value::<u32>().first().copied()
    }

    /// 8 位字符串属性，最多读取 `limit` 字节。
    pub fn string(&self, window: x::Window, property: x::Atom, limit: u32) -> Option<Vec<u8>> {
        let reply = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type: x::ATOM_ANY,
                long_offset: 0,
                long_length: limit.div_ceil(4),
            }))
            .ok()?;
        (reply.format() == 8).then(|| reply.value::<u8>().to_vec())
    }

    /// EWMH `_NET_ACTIVE_WINDOW`；窗口管理器不支持或没有焦点窗口时返回 None。
    pub fn active_window(&self) -> Option<x::Window> {
        let property = self.existing("_NET_ACTIVE_WINDOW")?;
        let id = self.first_u32(self.root, property, x::ATOM_WINDOW)?;
        (id != 0).then(|| x::Window::new(id))
    }

    /// `_NET_WM_PID` 由客户端自行设置；远程（SSH 转发）窗口的进程号不属于本机，调用方只用于比较与展示。
    pub fn window_pid(&self, window: x::Window) -> Option<u32> {
        let property = self.existing("_NET_WM_PID")?;
        self.first_u32(window, property, x::ATOM_CARDINAL)
            .filter(|pid| *pid != 0)
    }

    pub fn window_exists(&self, window: x::Window) -> bool {
        self.conn
            .wait_for_reply(self.conn.send_request(&x::GetWindowAttributes { window }))
            .is_ok()
    }

    /// 请求窗口管理器激活窗口。来源标为 2（分页器 / 用户操作），避免被防抢焦点策略忽略。
    pub fn activate(&self, window: x::Window) -> Result<(), String> {
        let property = self
            .existing("_NET_ACTIVE_WINDOW")
            .ok_or("窗口管理器不支持切换窗口，内容已复制，可手动粘贴。")?;
        let event = x::ClientMessageEvent::new(
            window,
            property,
            x::ClientMessageData::Data32([2, x::CURRENT_TIME, 0, 0, 0]),
        );
        self.conn
            .send_and_check_request(&x::SendEvent {
                propagate: false,
                destination: x::SendEventDest::Window(self.root),
                event_mask: x::EventMask::SUBSTRUCTURE_REDIRECT | x::EventMask::SUBSTRUCTURE_NOTIFY,
                event: &event,
            })
            .map_err(|_| "未能回到原窗口，内容已复制，可手动粘贴。".into())
    }

    /// 当前键盘映射里产生各 keysym 的键码；找不到的 keysym 不出现在结果里。
    pub fn keycodes(&self, keysyms: &[u32]) -> Vec<(u32, u8)> {
        let setup = self.conn.get_setup();
        let (min, max) = (setup.min_keycode(), setup.max_keycode());
        let Ok(reply) = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetKeyboardMapping {
                first_keycode: min,
                count: max.saturating_sub(min).saturating_add(1),
            }))
        else {
            return Vec::new();
        };
        keysyms
            .iter()
            .filter_map(|sym| {
                keycode_for(reply.keysyms(), reply.keysyms_per_keycode(), min, *sym)
                    .map(|code| (*sym, code))
            })
            .collect()
    }

    /// 任一键码当前处于按下状态；查询失败按“按着”处理，宁可不注入按键。
    pub fn any_pressed(&self, keycodes: &[u8]) -> bool {
        match self
            .conn
            .wait_for_reply(self.conn.send_request(&x::QueryKeymap {}))
        {
            Ok(reply) => keycodes.iter().any(|code| pressed(reply.keys(), *code)),
            Err(_) => true,
        }
    }

    /// 通过 XTest 发送 Ctrl+字母：按下、松开成对发送，最后同步等待服务器处理完毕。
    pub fn ctrl_chord(&self, letter: u32) -> Result<(), String> {
        if !self.has(xcb::Extension::Test) {
            return Err("X 服务器不支持 XTest 按键注入。".into());
        }
        let codes = self.keycodes(&[keysym::CONTROL_L, letter]);
        let find = |sym| codes.iter().find(|(s, _)| *s == sym).map(|(_, code)| *code);
        let (Some(ctrl), Some(key)) = (find(keysym::CONTROL_L), find(letter)) else {
            return Err("当前键盘布局缺少所需按键。".into());
        };
        // KeyPress = 2，KeyRelease = 3。
        for (kind, code) in [(2u8, ctrl), (2, key), (3, key), (3, ctrl)] {
            self.conn.send_request(&xcb::xtest::FakeInput {
                r#type: kind,
                detail: code,
                time: x::CURRENT_TIME,
                root: self.root,
                root_x: 0,
                root_y: 0,
                deviceid: 0,
            });
        }
        self.sync()
            .map_err(|_| "X 服务器不支持 XTest 按键注入。".to_string())
    }

    /// 向未启用的扩展发请求会让 libxcb 关闭整条连接，使用可选扩展前必须先确认。
    fn has(&self, extension: xcb::Extension) -> bool {
        self.conn.active_extensions().any(|ext| ext == extension)
    }

    /// 一次往返：返回时此前发出的请求都已处理，之前产生的事件也已进入本连接的队列。
    pub fn sync(&self) -> Result<(), String> {
        self.conn
            .wait_for_reply(self.conn.send_request(&x::GetInputFocus {}))
            .map(|_| ())
            .map_err(|_| "X 服务器无响应。".into())
    }

    /// XFixes 必须先协商版本才能使用；服务器缺少扩展时返回错误。
    pub fn xfixes(&self) -> Result<(), String> {
        if !self.has(xcb::Extension::XFixes) {
            return Err("X 服务器缺少 XFixes 扩展，无法监听剪贴板。".into());
        }
        self.conn
            .wait_for_reply(self.conn.send_request(&xfixes::QueryVersion {
                client_major_version: 5,
                client_minor_version: 0,
            }))
            .map(|_| ())
            .map_err(|_| "X 服务器缺少 XFixes 扩展，无法监听剪贴板。".into())
    }

    /// 窗口在根坐标系中的矩形（不含窗口管理器边框）。
    pub fn window_rect(&self, window: x::Window) -> Option<(i32, i32, u32, u32)> {
        let geometry = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetGeometry {
                drawable: x::Drawable::Window(window),
            }))
            .ok()?;
        let origin = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::TranslateCoordinates {
                src_window: window,
                dst_window: self.root,
                src_x: 0,
                src_y: 0,
            }))
            .ok()?;
        Some((
            i32::from(origin.dst_x()),
            i32::from(origin.dst_y()),
            u32::from(geometry.width()),
            u32::from(geometry.height()),
        ))
    }

    /// RandR 1.5 活动显示器（物理像素）。
    pub fn monitors(&self) -> Vec<(i32, i32, u32, u32)> {
        if !self.has(xcb::Extension::RandR) {
            return Vec::new();
        }
        self.conn
            .wait_for_reply(self.conn.send_request(&randr::GetMonitors {
                window: self.root,
                get_active: true,
            }))
            .map(|reply| {
                reply
                    .monitors()
                    .filter(|m| m.width() > 0 && m.height() > 0)
                    .map(|m| {
                        (
                            i32::from(m.x()),
                            i32::from(m.y()),
                            u32::from(m.width()),
                            u32::from(m.height()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 创建只接收属性变化的隐藏窗口，用来接收选区数据。
    pub fn hidden_window(&self, events: x::EventMask) -> Result<x::Window, String> {
        let window: x::Window = self.conn.generate_id();
        self.conn
            .send_and_check_request(&x::CreateWindow {
                depth: 0,
                wid: window,
                parent: self.root,
                x: -10,
                y: -10,
                width: 1,
                height: 1,
                border_width: 0,
                class: x::WindowClass::InputOnly,
                visual: x::COPY_FROM_PARENT,
                value_list: &[x::Cw::EventMask(events)],
            })
            .map_err(|_| "无法创建 X11 辅助窗口。")?;
        Ok(window)
    }
}

/// 在 GetKeyboardMapping 的扁平表里找第一个产生 `sym` 的键码；只看前两列（无修饰、Shift）。
fn keycode_for(table: &[u32], per_keycode: u8, min: u8, sym: u32) -> Option<u8> {
    let per = usize::from(per_keycode);
    if per == 0 {
        return None;
    }
    table.chunks(per).enumerate().find_map(|(index, row)| {
        row.iter()
            .take(2)
            .any(|value| *value == sym)
            .then(|| u8::try_from(usize::from(min) + index).ok())
            .flatten()
    })
}

/// 只接受 8 位格式；`value::<u8>()` 遇到 16/32 位数据会断言失败，不能信任所有者写入的格式。
/// 空属性的格式可能为 0，此时按空数据处理。
fn bytes(reply: &x::GetPropertyReply) -> Result<&[u8], ReadError> {
    match reply.format() {
        8 => Ok(reply.value::<u8>()),
        0 => Ok(&[]),
        _ => Err(ReadError::Failed),
    }
}

fn pressed(keys: &[u8], code: u8) -> bool {
    keys.get(usize::from(code / 8))
        .is_some_and(|byte| byte & (1 << (code % 8)) != 0)
}

/// `_XKB_RULES_NAMES` 为 NUL 分隔的 规则、型号、布局、变体、选项；返回布局（如 `us,ru`）。
pub(crate) fn xkb_layout(rules: &[u8]) -> Option<String> {
    let layout = rules.split(|b| *b == 0).nth(2)?;
    let layout = std::str::from_utf8(layout).ok()?.trim();
    (!layout.is_empty()).then(|| layout.to_string())
}

/// 窗口中心所在显示器是否被窗口完全覆盖；中心不在任何显示器上时为 None。
pub(crate) fn covers_monitor(
    window: (i32, i32, u32, u32),
    monitors: &[(i32, i32, u32, u32)],
) -> Option<bool> {
    let (x, y, w, h) = window;
    let (right, bottom) = (i64::from(x) + i64::from(w), i64::from(y) + i64::from(h));
    let (cx, cy) = ((i64::from(x) + right) / 2, (i64::from(y) + bottom) / 2);
    let &(mx, my, mw, mh) = monitors.iter().find(|(mx, my, mw, mh)| {
        cx >= i64::from(*mx)
            && cy >= i64::from(*my)
            && cx < i64::from(*mx) + i64::from(*mw)
            && cy < i64::from(*my) + i64::from(*mh)
    })?;
    Some(
        i64::from(x) <= i64::from(mx)
            && i64::from(y) <= i64::from(my)
            && right >= i64::from(mx) + i64::from(mw)
            && bottom >= i64::from(my) + i64::from(mh),
    )
}

/// 进程可执行文件名（不含目录），用于剪贴板排除名单；读不到 exe 链接时退回 comm。
pub(crate) fn process_name(pid: u32) -> Option<String> {
    if let Ok(path) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // 被替换的可执行文件会带“ (deleted)”后缀，名单比较时去掉。
            return Some(name.trim_end_matches(" (deleted)").to_string());
        }
    }
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let comm = comm.trim();
    (!comm.is_empty()).then(|| comm.to_string())
}

/// 选区读取失败的原因：拒绝转换、超限或超时分开报告，调用方据此决定提示文字。
#[derive(Debug, PartialEq)]
pub(crate) enum ReadError {
    TooLarge,
    Timeout,
    Failed,
}

/// 选区请求方：自己的隐藏窗口接收数据，属性每次读取后删除。大数据按 ICCCM INCR 分段接收，累计超过上限立即放弃。
pub(crate) struct SelectionReader<'a> {
    x: &'a X11,
    window: x::Window,
    property: x::Atom,
    incr: x::Atom,
}

/// 等待单个事件（选区通知或分段数据）的最长时间。
const STEP: Duration = Duration::from_millis(800);

impl<'a> SelectionReader<'a> {
    pub fn new(x: &'a X11) -> Result<Self, String> {
        let window = x.hidden_window(x::EventMask::PROPERTY_CHANGE)?;
        Ok(Self {
            x,
            window,
            property: x.intern("SNAP_DESK_SELECTION")?,
            incr: x.intern("INCR")?,
        })
    }

    /// 选区当前的所有者窗口；没有所有者时为 None。
    pub fn owner(&self, selection: x::Atom) -> Option<x::Window> {
        let owner = self
            .x
            .conn
            .wait_for_reply(
                self.x
                    .conn
                    .send_request(&x::GetSelectionOwner { selection }),
            )
            .ok()?
            .owner();
        (owner.resource_id() != 0).then_some(owner)
    }

    /// 所有者支持的目标格式；所有者拒绝或无响应时为空。
    pub fn targets(&self, selection: x::Atom) -> Result<Vec<x::Atom>, ReadError> {
        let targets = self.x.intern("TARGETS").map_err(|_| ReadError::Failed)?;
        if !self.convert(selection, targets)? {
            return Ok(Vec::new());
        }
        let reply = self.get(true, 0, 4096)?;
        if reply.format() != 32 {
            return Ok(Vec::new());
        }
        Ok(reply.value::<x::Atom>().to_vec())
    }

    /// 读取一种目标格式的 8 位数据；所有者不提供该格式时返回 Ok(None)。
    pub fn read(
        &self,
        selection: x::Atom,
        target: x::Atom,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ReadError> {
        if !self.convert(selection, target)? {
            return Ok(None);
        }
        // 先只取类型与长度，超限时不让服务器把整块数据发过来。
        let head = self.get(false, 0, 0)?;
        if head.r#type() == self.incr {
            return self.read_incr(limit).map(Some);
        }
        let size = head.bytes_after() as usize;
        if size > limit {
            self.delete();
            return Err(ReadError::TooLarge);
        }
        let reply = self.get(true, 0, u32::try_from(size.div_ceil(4)).unwrap_or(u32::MAX))?;
        bytes(&reply).map(|data| Some(data.to_vec()))
    }

    fn read_incr(&self, limit: usize) -> Result<Vec<u8>, ReadError> {
        // 删除 INCR 属性即通知所有者开始发送第一段。
        self.delete();
        let mut data = Vec::new();
        loop {
            self.wait(|event| match event {
                xcb::Event::X(x::Event::PropertyNotify(ev)) => {
                    ev.window() == self.window
                        && ev.atom() == self.property
                        && ev.state() == x::Property::NewValue
                }
                _ => false,
            })?;
            let head = self.get(false, 0, 0)?;
            let size = head.bytes_after() as usize;
            if data.len() + size > limit {
                self.delete();
                return Err(ReadError::TooLarge);
            }
            let chunk = self.get(true, 0, u32::try_from(size.div_ceil(4)).unwrap_or(u32::MAX))?;
            let chunk = bytes(&chunk)?;
            // 零长度的一段表示传输结束。
            if chunk.is_empty() {
                return Ok(data);
            }
            data.extend_from_slice(chunk);
        }
    }

    /// 请求所有者把选区转换到本窗口的属性；返回 false 表示所有者拒绝了该格式。
    fn convert(&self, selection: x::Atom, target: x::Atom) -> Result<bool, ReadError> {
        self.delete();
        self.x.conn.send_request(&x::ConvertSelection {
            requestor: self.window,
            selection,
            target,
            property: self.property,
            time: x::CURRENT_TIME,
        });
        self.x.conn.flush().map_err(|_| ReadError::Failed)?;
        let mut accepted = false;
        self.wait(|event| match event {
            xcb::Event::X(x::Event::SelectionNotify(ev))
                if ev.requestor() == self.window && ev.selection() == selection =>
            {
                accepted = ev.property() != x::ATOM_NONE;
                true
            }
            _ => false,
        })?;
        Ok(accepted)
    }

    fn get(
        &self,
        delete: bool,
        offset: u32,
        length: u32,
    ) -> Result<x::GetPropertyReply, ReadError> {
        self.x
            .conn
            .wait_for_reply(self.x.conn.send_request(&x::GetProperty {
                delete,
                window: self.window,
                property: self.property,
                r#type: x::ATOM_ANY,
                long_offset: offset,
                long_length: length,
            }))
            .map_err(|_| ReadError::Failed)
    }

    fn delete(&self) {
        self.x.conn.send_request(&x::DeleteProperty {
            window: self.window,
            property: self.property,
        });
    }

    /// 本连接只服务一次读取，其他事件直接丢弃；协议错误不影响等待，连接断开则失败。
    fn wait(&self, mut matches: impl FnMut(&xcb::Event) -> bool) -> Result<(), ReadError> {
        let deadline = Instant::now() + STEP;
        loop {
            match self.x.conn.poll_for_event() {
                Ok(Some(event)) => {
                    if matches(&event) {
                        return Ok(());
                    }
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return Err(ReadError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
                Err(xcb::Error::Protocol(_)) => {}
                Err(xcb::Error::Connection(_)) => return Err(ReadError::Failed),
            }
        }
    }
}

impl Drop for SelectionReader<'_> {
    fn drop(&mut self) {
        self.x.conn.send_request(&x::DestroyWindow {
            window: self.window,
        });
        let _ = self.x.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keycode_lookup_uses_unshifted_and_shifted_columns_only() {
        // 键码 8 起，每个键码 3 列：第三列（AltGr 等）不参与匹配。
        let table = [0, 0, keysym::V, keysym::C, 0x43, 0, 0x76 + 1, keysym::V, 0];
        assert_eq!(keycode_for(&table, 3, 8, keysym::C), Some(9));
        assert_eq!(keycode_for(&table, 3, 8, keysym::V), Some(10));
        assert_eq!(keycode_for(&table, 3, 8, 0x1234), None);
        assert_eq!(keycode_for(&table, 0, 8, keysym::C), None);
    }
    #[test]
    fn keymap_bits_follow_keycode_order() {
        let mut keys = [0u8; 32];
        keys[4] = 0b0000_0100;
        assert!(pressed(&keys, 34));
        assert!(!pressed(&keys, 35));
        assert!(!pressed(&keys[..2], 34));
    }
    #[test]
    fn xkb_rules_names_yield_the_layout_field() {
        assert_eq!(
            xkb_layout(b"evdev\0pc105\0us,ru\0,\0grp:alt_shift_toggle\0").as_deref(),
            Some("us,ru")
        );
        assert_eq!(xkb_layout(b"evdev\0pc105\0\0"), None);
        assert_eq!(xkb_layout(b"evdev"), None);
    }
    #[test]
    fn fullscreen_detection_uses_the_monitor_under_the_window_centre() {
        let monitors = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        assert_eq!(covers_monitor((1920, 0, 2560, 1440), &monitors), Some(true));
        assert_eq!(covers_monitor((100, 100, 800, 600), &monitors), Some(false));
        assert_eq!(covers_monitor((-1, -1, 1922, 1082), &monitors), Some(true));
        assert_eq!(covers_monitor((9000, 9000, 10, 10), &monitors), None);
    }
}
