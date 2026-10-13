//! 输入边界：原始键码只在监听线程内归一化，队列仅保存可展示的快捷键和鼠标效果。
use std::collections::VecDeque;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::Listener;

/// 所有位置均为虚拟桌面的物理像素；源时间与 WGC 相同，单位 100 ns。
#[derive(Clone, Debug)]
pub struct InputEvent {
    pub source_time: i64,
    pub kind: InputKind,
}

#[derive(Clone, Debug)]
pub enum InputKind {
    Key(String),
    Click {
        x: i32,
        y: i32,
        right: bool,
        double: bool,
    },
    Move {
        x: i32,
        y: i32,
        dragging: bool,
    },
    Wheel {
        x: i32,
        y: i32,
        delta: i32,
    },
}

/// 状态在生产端已归一化：队列不依赖按键释放事件，因此溢出不会留下卡住的按键。
#[derive(Default)]
pub struct EventBuffer(VecDeque<InputEvent>);
impl EventBuffer {
    pub fn push(&mut self, event: InputEvent) {
        if let Some(last) = self.0.back_mut() {
            match (&mut last.kind, &event.kind) {
                (InputKind::Move { dragging: a, .. }, InputKind::Move { dragging: b, .. })
                    if a == b =>
                {
                    *last = event;
                    return;
                }
                (
                    InputKind::Wheel { x, y, delta },
                    InputKind::Wheel {
                        x: nx,
                        y: ny,
                        delta: nd,
                    },
                ) if delta.signum() == nd.signum()
                    && event.source_time - last.source_time <= 1_000_000 =>
                {
                    *x = *nx;
                    *y = *ny;
                    *delta = delta.saturating_add(*nd);
                    last.source_time = event.source_time;
                    return;
                }
                _ => (),
            }
        }
        if self.0.len() == 512 {
            self.0.pop_front();
        }
        self.0.push_back(event);
    }

    pub fn drain(&mut self) -> Vec<InputEvent> {
        self.0.drain(..).collect()
    }
}

/// 系统阈值由 Windows 适配传入，纯逻辑便于验证双击与三连击边界。
#[derive(Default)]
pub struct Clicks(Option<(i64, i32, i32, bool)>);
impl Clicks {
    pub fn press(
        &mut self,
        time: i64,
        x: i32,
        y: i32,
        right: bool,
        limits: (i64, i32, i32),
    ) -> bool {
        let double = self.0.is_some_and(|(previous, px, py, button)| {
            button == right
                && time >= previous
                && time - previous <= limits.0
                && (i64::from(x) - i64::from(px)).abs() <= i64::from(limits.1)
                && (i64::from(y) - i64::from(py)).abs() <= i64::from(limits.2)
        });
        self.0 = if double {
            None
        } else {
            Some((time, x, y, right))
        };
        double
    }
}

/// 左右修饰键独立跟踪，释放其中一侧不会误清除另一侧；重复 key-down 不重复显示。
pub struct Keys([bool; 256]);
impl Default for Keys {
    fn default() -> Self {
        Self([false; 256])
    }
}
impl Keys {
    pub fn update(&mut self, key: u16, down: bool) -> Option<String> {
        let held = self.0.get_mut(key as usize)?;
        let repeated = *held;
        *held = down;
        if !down || repeated {
            return None;
        }
        let ctrl = self.0[0xa2] || self.0[0xa3];
        let alt = self.0[0xa4] || self.0[0xa5];
        let shift = self.0[0xa0] || self.0[0xa1];
        let win = self.0[0x5b] || self.0[0x5c];
        // AltGr 常表现为 Ctrl+Alt；拒绝该组合，避免把国际键盘文字当快捷键记录。
        if ctrl && alt {
            return None;
        }
        let name = match key {
            0x08 => "BACKSPACE".into(),
            0x09 => "TAB".into(),
            0x0d => "ENTER".into(),
            0x1b => "ESC".into(),
            0x21 => "PAGE UP".into(),
            0x22 => "PAGE DOWN".into(),
            0x23 => "END".into(),
            0x24 => "HOME".into(),
            0x25 => "LEFT".into(),
            0x26 => "UP".into(),
            0x27 => "RIGHT".into(),
            0x28 => "DOWN".into(),
            0x2d => "INSERT".into(),
            0x2e => "DELETE".into(),
            0x70..=0x87 => format!("F{}", key - 0x6f),
            // 不调用 ToUnicode / IME，不生成或保存普通文字；仅允许常用命令键。
            0x41 | 0x43 | 0x46 | 0x50 | 0x53 | 0x56 | 0x58 | 0x59 | 0x5a if ctrl && !win => {
                char::from_u32(u32::from(key))?.to_string()
            }
            0x44 | 0x45 | 0x4c if win && !ctrl && !alt => {
                char::from_u32(u32::from(key))?.to_string()
            }
            _ => return None,
        };
        let mut label = String::new();
        for (active, prefix) in [
            (ctrl, "CTRL+"),
            (alt, "ALT+"),
            (shift, "SHIFT+"),
            (win, "WIN+"),
        ] {
            if active {
                label.push_str(prefix);
            }
        }
        label.push_str(&name);
        Some(label)
    }
}

#[cfg(not(target_os = "windows"))]
pub struct Listener;
#[cfg(not(target_os = "windows"))]
impl Listener {
    pub fn start(_config: super::config::InputEffects) -> Result<Self, String> {
        Err("当前平台不支持录屏键鼠特效。".into())
    }
    pub fn drain(&self) -> Result<Vec<InputEvent>, String> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn double_click_requires_same_button_position_and_time() {
        let mut clicks = Clicks::default();
        let limits = (5_000_000, 2, 2);
        assert!(!clicks.press(0, 10, 10, false, limits));
        assert!(clicks.press(5_000_000, 12, 12, false, limits));
        assert!(!clicks.press(5_000_001, 12, 12, false, limits));
        assert!(!clicks.press(5_000_002, 12, 12, true, limits));
        assert!(!clicks.press(5_000_003, 20, 20, true, limits));
        assert!(!clicks.press(10_000_004, 20, 20, true, limits));
    }

    #[test]
    fn high_frequency_motion_preserves_drag_release_boundary() {
        let mut buffer = EventBuffer::default();
        for x in 0..1000 {
            buffer.push(InputEvent {
                source_time: i64::from(x),
                kind: InputKind::Move {
                    x,
                    y: 0,
                    dragging: true,
                },
            });
        }
        buffer.push(InputEvent {
            source_time: 1000,
            kind: InputKind::Move {
                x: 1000,
                y: 0,
                dragging: false,
            },
        });
        let events = buffer.drain();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].kind,
            InputKind::Move {
                x: 999,
                dragging: true,
                ..
            }
        ));
        assert!(matches!(
            events[1].kind,
            InputKind::Move {
                dragging: false,
                ..
            }
        ));
    }
    #[test]
    fn typing_repeat_release_and_altgr_are_private() {
        let mut keys = Keys::default();
        assert_eq!(keys.update(0x41, true), None);
        keys.update(0x41, false);
        keys.update(0xa2, true);
        assert_eq!(keys.update(0x43, true).as_deref(), Some("CTRL+C"));
        assert_eq!(keys.update(0x43, true), None);
        keys.update(0x43, false);
        keys.update(0xa3, true);
        keys.update(0xa2, false);
        assert_eq!(keys.update(0x43, true).as_deref(), Some("CTRL+C"));
        keys.update(0x43, false);
        keys.update(0xa5, true);
        assert_eq!(keys.update(0x43, true), None);
        let mut keys = Keys::default();
        keys.update(0xa4, true);
        assert_eq!(keys.update(9, true).as_deref(), Some("ALT+TAB"));
    }

    #[test]
    fn buffer_is_bounded_and_merges_wheel_without_losing_direction() {
        let mut buffer = EventBuffer::default();
        for source_time in 0..1000 {
            buffer.push(InputEvent {
                source_time,
                kind: InputKind::Key("ENTER".into()),
            });
        }
        assert_eq!(buffer.drain().len(), 512);
        for (time, delta) in [(1, 120), (2, 120), (3, -120)] {
            buffer.push(InputEvent {
                source_time: time,
                kind: InputKind::Wheel { x: 0, y: 0, delta },
            });
        }
        let events = buffer.drain();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].kind,
            InputKind::Wheel { delta: 240, .. }
        ));
    }
}
