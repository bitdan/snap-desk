//! X11 键鼠监听：XInput2 原始事件（Raw*）在根窗口上接收，不抓取输入、不影响其他程序。
//! 键码先按当前键盘映射转为 keysym，再归一化为与 Windows 相同的虚拟键编号交给共用的 [`Keys`]，
//! 因此同样只显示命令快捷键，不记录普通文字。鼠标位置取自 QueryPointer（根窗口物理像素）。
use super::super::{capture, config::InputEffects};
use super::{Clicks, EventBuffer, InputEvent, InputKind, Keys};
use crate::x11::{self, keysym, X11};
use std::{
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use xcb::{x, xinput};

/// 与 GTK 默认值一致：400 ms、5 像素内的同键再次按下算双击（时间单位 100 ns）。
const DOUBLE_CLICK: (i64, i32, i32) = (4_000_000, 5, 5);
/// 原始移动事件可达每秒上千次，按约 120 Hz 查询指针位置即可画出平滑轨迹。
const MOVE_INTERVAL: Duration = Duration::from_millis(8);

/// keysym → Windows 虚拟键编号；只映射 [`Keys`] 会显示或用作修饰的键。
fn virtual_key(sym: u32) -> Option<u16> {
    Some(match sym {
        keysym::SHIFT_L => 0xa0,
        keysym::SHIFT_R => 0xa1,
        keysym::CONTROL_L => 0xa2,
        keysym::CONTROL_R => 0xa3,
        keysym::ALT_L => 0xa4,
        keysym::ALT_R => 0xa5,
        keysym::SUPER_L | keysym::META_L => 0x5b,
        keysym::SUPER_R | keysym::META_R => 0x5c,
        0xff08 => 0x08,
        // Tab 与 Shift+Tab 产生的 ISO_Left_Tab。
        0xff09 | 0xfe20 => 0x09,
        keysym::RETURN | keysym::KP_ENTER => 0x0d,
        0xff1b => 0x1b,
        0xff55 => 0x21,
        0xff56 => 0x22,
        0xff57 => 0x23,
        0xff50 => 0x24,
        0xff51 => 0x25,
        0xff52 => 0x26,
        0xff53 => 0x27,
        0xff54 => 0x28,
        0xff63 => 0x2d,
        0xffff => 0x2e,
        // F1–F24。
        0xffbe..=0xffd5 => 0x70 + (sym - 0xffbe) as u16,
        // 字母键不论大小写都对应 A–Z。
        0x61..=0x7a => (sym - 0x20) as u16,
        0x41..=0x5a => sym as u16,
        _ => return None,
    })
}

/// 每个键码取无修饰列的 keysym 并换算为虚拟键；与当前布局一致，非拉丁布局的字母键不显示。
fn keymap(table: &[u32], per_keycode: u8, min: u8) -> Vec<Option<u16>> {
    let mut map = vec![None; 256];
    let per = usize::from(per_keycode);
    if per == 0 {
        return map;
    }
    for (index, row) in table.chunks(per).enumerate() {
        let code = usize::from(min) + index;
        if let (Some(slot), Some(sym)) = (map.get_mut(code), row.first()) {
            *slot = virtual_key(*sym);
        }
    }
    map
}

/// X 按键编号：1 左键、3 右键、4 / 5 滚轮上 / 下（与 Windows 一致，向上为正）。
fn wheel_delta(button: u32) -> Option<i32> {
    match button {
        4 => Some(120),
        5 => Some(-120),
        _ => None,
    }
}

pub struct Listener {
    events: Arc<Mutex<EventBuffer>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl Listener {
    pub fn start(config: InputEffects) -> Result<Self, String> {
        let events = Arc::new(Mutex::new(EventBuffer::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (buffer, stopping) = (events.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("recording-input".into())
            .spawn(move || {
                let prepared = Session::new(config);
                let mut session = match prepared {
                    Ok(session) => {
                        let _ = ready_tx.send(Ok(()));
                        session
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                while !stopping.load(Ordering::Acquire) {
                    session.wait(Duration::from_millis(50));
                    session.receive(&buffer)?;
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let mut listener = Self {
            events,
            stop,
            thread: Some(thread),
        };
        match ready_rx.recv().map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(()) => Ok(listener),
            Err(error) => {
                listener.shutdown();
                Err(error)
            }
        }
    }

    pub fn drain(&self) -> Result<Vec<InputEvent>, String> {
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            return Err("键鼠监听已停止，请检查 X 服务器连接。".into());
        }
        Ok(self
            .events
            .lock()
            .map_err(|_| "键鼠事件缓冲不可用。")?
            .drain())
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Session {
    x11: X11,
    config: InputEffects,
    keymap: Vec<Option<u16>>,
    keys: Keys,
    clicks: Clicks,
    left: bool,
    right: bool,
    moved: Option<Instant>,
    last_move: Option<Instant>,
}

impl Session {
    fn new(config: InputEffects) -> Result<Self, String> {
        if x11::wayland() {
            return Err(x11::WAYLAND_HINT.into());
        }
        let (conn, index) = xcb::Connection::connect_with_extensions(
            None,
            &[],
            &[xcb::Extension::Input],
        )
        .map_err(|e| format!("无法连接 X 服务器：{e}"))?;
        let root = conn
            .get_setup()
            .roots()
            .nth(usize::try_from(index).unwrap_or(0))
            .ok_or("未找到 X 屏幕。")?
            .root();
        let x11 = X11 { conn, root };
        const MISSING: &str = "X 服务器缺少 XInput 2.2 扩展，无法显示录屏键鼠特效。";
        if !x11
            .conn
            .active_extensions()
            .any(|extension| extension == xcb::Extension::Input)
        {
            return Err(MISSING.into());
        }
        // 原始事件在 2.1 起始终发往根窗口（即使其他程序抓取了输入）。
        let version = x11
            .conn
            .wait_for_reply(x11.conn.send_request(&xinput::XiQueryVersion {
                major_version: 2,
                minor_version: 2,
            }))
            .map_err(|_| MISSING)?;
        if (version.major_version(), version.minor_version()) < (2, 1) {
            return Err(MISSING.into());
        }
        let mut mask = xinput::XiEventMask::empty();
        if config.keyboard {
            mask |= xinput::XiEventMask::RAW_KEY_PRESS | xinput::XiEventMask::RAW_KEY_RELEASE;
        }
        if config.clicks || config.movement || config.drag || config.wheel {
            mask |= xinput::XiEventMask::RAW_BUTTON_PRESS
                | xinput::XiEventMask::RAW_BUTTON_RELEASE
                | xinput::XiEventMask::RAW_MOTION;
        }
        x11.conn
            .send_and_check_request(&xinput::XiSelectEvents {
                window: x11.root,
                masks: &[xinput::EventMaskBuf::new(
                    xinput::Device::AllMaster,
                    &[mask],
                )],
            })
            .map_err(|_| MISSING)?;
        let setup = x11.conn.get_setup();
        let (min, max) = (setup.min_keycode(), setup.max_keycode());
        let keymap = x11
            .conn
            .wait_for_reply(x11.conn.send_request(&x::GetKeyboardMapping {
                first_keycode: min,
                count: max.saturating_sub(min).saturating_add(1),
            }))
            .map(|reply| keymap(reply.keysyms(), reply.keysyms_per_keycode(), min))
            .map_err(|_| "无法读取键盘映射。")?;
        Ok(Self {
            x11,
            config,
            keymap,
            keys: Keys::default(),
            clicks: Clicks::default(),
            left: false,
            right: false,
            moved: None,
            last_move: None,
        })
    }

    /// 等待连接上有数据或超时；超时用于检查停止标志。
    fn wait(&self, timeout: Duration) {
        let mut descriptor = libc::pollfd {
            fd: self.x11.conn.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // 移动事件被节流时最多再等一个节流间隔，保证最后的位置也会提交。
        let timeout = if self.moved.is_some() {
            MOVE_INTERVAL
        } else {
            timeout
        };
        unsafe { libc::poll(&mut descriptor, 1, timeout.as_millis() as i32) };
    }

    fn pointer(&self) -> Option<(i32, i32)> {
        self.x11
            .conn
            .wait_for_reply(self.x11.conn.send_request(&x::QueryPointer {
                window: self.x11.root,
            }))
            .ok()
            .filter(|reply| reply.same_screen())
            .map(|reply| (i32::from(reply.root_x()), i32::from(reply.root_y())))
    }

    fn receive(&mut self, buffer: &Mutex<EventBuffer>) -> Result<(), String> {
        let mut output = Vec::new();
        loop {
            let event = match self.x11.conn.poll_for_event() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(xcb::Error::Connection(error)) => {
                    return Err(format!("X 服务器连接已断开：{error}"));
                }
                Err(xcb::Error::Protocol(_)) => continue,
            };
            let xcb::Event::Input(event) = event else {
                continue;
            };
            match event {
                xinput::Event::RawKeyPress(event) => self.key(event.detail(), true, &mut output),
                xinput::Event::RawKeyRelease(event) => {
                    self.key(event.detail(), false, &mut output)
                }
                xinput::Event::RawButtonPress(event) => self.button(event.detail(), true, &mut output),
                xinput::Event::RawButtonRelease(event) => {
                    self.button(event.detail(), false, &mut output)
                }
                xinput::Event::RawMotion(_) => {
                    self.moved.get_or_insert_with(Instant::now);
                }
                _ => {}
            }
        }
        if self.moved.is_some()
            && self
                .last_move
                .is_none_or(|last| last.elapsed() >= MOVE_INTERVAL)
        {
            self.moved = None;
            self.movement(&mut output);
        }
        if !output.is_empty() {
            let source_time = capture::now();
            let mut buffer = buffer.lock().map_err(|_| "键鼠事件缓冲不可用。")?;
            for kind in output {
                buffer.push(InputEvent { source_time, kind });
            }
        }
        Ok(())
    }

    fn key(&mut self, code: u32, down: bool, output: &mut Vec<InputKind>) {
        let Some(key) = usize::try_from(code)
            .ok()
            .and_then(|code| self.keymap.get(code).copied().flatten())
        else {
            return;
        };
        if let Some(label) = self.keys.update(key, down) {
            output.push(InputKind::Key(label));
        }
    }

    fn movement(&mut self, output: &mut Vec<InputKind>) {
        if !(self.config.movement || self.config.drag) {
            return;
        }
        self.last_move = Some(Instant::now());
        if let Some((x, y)) = self.pointer() {
            output.push(InputKind::Move {
                x,
                y,
                dragging: self.left || self.right,
            });
        }
    }

    fn button(&mut self, button: u32, down: bool, output: &mut Vec<InputKind>) {
        if let Some(delta) = wheel_delta(button) {
            // 滚轮每格产生一次按下、一次松开，只在按下时计数。
            if down && self.config.wheel {
                if let Some((x, y)) = self.pointer() {
                    output.push(InputKind::Wheel { x, y, delta });
                }
            }
            return;
        }
        let right = match button {
            1 => false,
            3 => true,
            _ => return,
        };
        if right {
            self.right = down;
        } else {
            self.left = down;
        }
        let Some((x, y)) = self.pointer() else {
            return;
        };
        if down && self.config.clicks {
            let double = self
                .clicks
                .press(capture::now(), x, y, right, DOUBLE_CLICK);
            output.push(InputKind::Click {
                x,
                y,
                right,
                double,
            });
        }
        // 按下与松开都更新一次位置，拖动轨迹的起止状态准确。
        if self.config.movement || self.config.drag {
            output.push(InputKind::Move {
                x,
                y,
                dragging: self.left || self.right,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_map_to_the_shared_virtual_keys() {
        assert_eq!(virtual_key(0x63), Some(0x43));
        assert_eq!(virtual_key(0x43), Some(0x43));
        assert_eq!(virtual_key(keysym::CONTROL_R), Some(0xa3));
        assert_eq!(virtual_key(0xffbe), Some(0x70));
        assert_eq!(virtual_key(0xffd5), Some(0x87));
        assert_eq!(virtual_key(0xfe20), Some(0x09));
        assert_eq!(virtual_key(0x31), None);
        assert_eq!(virtual_key(keysym::ISO_LEVEL3_SHIFT), None);
    }

    #[test]
    fn keymap_uses_the_unshifted_column_from_the_minimum_keycode() {
        // 键码 8：a / A；键码 9：Control_L。
        let table = [0x61, 0x41, keysym::CONTROL_L, 0];
        let map = keymap(&table, 2, 8);
        assert_eq!(map[8], Some(0x41));
        assert_eq!(map[9], Some(0xa2));
        assert_eq!(map[10], None);
        assert!(keymap(&table, 0, 8).iter().all(Option::is_none));
    }

    #[test]
    fn ctrl_c_from_raw_keycodes_is_labelled_like_windows() {
        let map = keymap(&[0x63, keysym::CONTROL_L], 1, 8);
        let mut keys = Keys::default();
        assert_eq!(keys.update(map[9].unwrap(), true), None);
        assert_eq!(keys.update(map[8].unwrap(), true).as_deref(), Some("CTRL+C"));
    }

    #[test]
    fn wheel_buttons_point_up_positive_like_windows() {
        assert_eq!(wheel_delta(4), Some(120));
        assert_eq!(wheel_delta(5), Some(-120));
        assert_eq!(wheel_delta(1), None);
    }
}
