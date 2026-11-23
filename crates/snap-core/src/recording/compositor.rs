//! CPU 视频帧特效合成：始终从干净 BGRA 底图重绘，动画由媒体 PTS 驱动。
//! 不创建屏幕浮窗；像素在 NV12 转换前写入，因此 MP4 中真正包含键鼠效果。
use super::{
    config::{InputEffects, KeyPosition},
    frame::BgraImage,
    input::InputKind,
};
use crate::capture::Rect;
use std::collections::VecDeque;

#[derive(Clone)]
enum Visual {
    Click {
        x: i32,
        y: i32,
        color: [u8; 3],
        double: bool,
    },
    Wheel {
        x: i32,
        y: i32,
        delta: i32,
    },
    Trail {
        from: (i32, i32),
        to: (i32, i32),
        drag: bool,
    },
    Key(String),
}

pub struct Compositor {
    config: InputEffects,
    region: Rect,
    visuals: VecDeque<(i64, Visual)>,
    last_point: Option<(i32, i32, bool, i64)>,
    dirty: bool,
    origin: (i32, i32),
}
impl Compositor {
    pub fn new(config: InputEffects, region: Rect) -> Self {
        Self {
            config,
            region,
            visuals: VecDeque::new(),
            last_point: None,
            dirty: false,
            origin: (0, 0),
        }
    }

    /// 输入事件为有符号桌面坐标，先减去显示器原点，再减去来源内选区。
    pub fn set_origin(&mut self, origin: (i32, i32)) {
        self.origin = origin;
    }

    /// 暂停/恢复代次改变时清空提示和轨迹，避免跨暂停连线或重放快捷键。
    pub fn clear(&mut self) {
        self.visuals.clear();
        self.last_point = None;
        self.dirty = false;
    }

    /// 使用 i64 运算，避免副屏负原点与无符号选区相减溢出。
    fn local(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (x, y) = (
            i64::from(x) - i64::from(self.origin.0) - i64::from(self.region.x),
            i64::from(y) - i64::from(self.origin.1) - i64::from(self.region.y),
        );
        (x >= 0 && y >= 0 && x < i64::from(self.region.width) && y < i64::from(self.region.height))
            .then_some((x as i32, y as i32))
    }

    pub fn ingest(&mut self, pts: i64, kind: InputKind) {
        let visual = match kind {
            InputKind::Key(label) if self.config.keyboard => {
                // 最多三条提示；重复快捷键刷新已有提示，防止快速操作遮住整幅画面。
                self.visuals
                    .retain(|(_, v)| !matches!(v, Visual::Key(old) if old == &label));
                if self
                    .visuals
                    .iter()
                    .filter(|(_, v)| matches!(v, Visual::Key(_)))
                    .count()
                    >= 3
                {
                    if let Some(index) = self
                        .visuals
                        .iter()
                        .position(|(_, v)| matches!(v, Visual::Key(_)))
                    {
                        self.visuals.remove(index);
                    }
                }
                Some(Visual::Key(label))
            }
            InputKind::Click {
                x,
                y,
                right,
                double,
            } if self.config.clicks => self.local(x, y).map(|(x, y)| Visual::Click {
                x,
                y,
                double,
                color: if double {
                    self.config.double_color
                } else if right {
                    self.config.right_color
                } else {
                    self.config.left_color
                },
            }),
            InputKind::Wheel { x, y, delta } if self.config.wheel && delta != 0 => {
                let Some((x, y)) = self.local(x, y) else {
                    return;
                };
                // 合并连续滚动，方向变化保留新提示；总数量和动画寿命都有上限。
                if let Some((
                    time,
                    Visual::Wheel {
                        x: px,
                        y: py,
                        delta: pd,
                    },
                )) = self.visuals.back_mut()
                {
                    if pd.signum() == delta.signum() && pts - *time <= 1_000_000 {
                        *time = pts;
                        *px = x;
                        *py = y;
                        *pd = pd.saturating_add(delta);
                        self.dirty = true;
                        return;
                    }
                }
                Some(Visual::Wheel { x, y, delta })
            }
            InputKind::Move { x, y, dragging } => {
                let Some((x, y)) = self.local(x, y) else {
                    self.last_point = None;
                    return;
                };
                let previous = self.last_point.replace((x, y, dragging, pts));
                if !(self.config.movement || (dragging && self.config.drag)) {
                    return;
                }
                let from = previous
                    .filter(|(_, _, was_dragging, time)| {
                        *was_dragging == dragging && pts - *time < 2_000_000
                    })
                    .map(|(x, y, _, _)| (x, y))
                    .unwrap_or((x, y));
                if previous.is_some_and(|(px, py, was_dragging, _)| {
                    px == x && py == y && was_dragging == dragging
                }) {
                    return;
                }
                Some(Visual::Trail {
                    from,
                    to: (x, y),
                    drag: dragging && self.config.drag,
                })
            }
            _ => None,
        };
        if let Some(visual) = visual {
            if self.visuals.len() == 256 {
                self.visuals.pop_front();
            }
            self.visuals.push_back((pts, visual));
            self.dirty = true;
        }
    }

    /// 已过期的效果也要求再绘制一帧干净底图，避免最后一圈冻结在静态视频里。
    pub fn needs_frame(&self) -> bool {
        self.dirty || !self.visuals.is_empty()
    }

    pub fn render(&mut self, base: &BgraImage, pts: i64) -> BgraImage {
        let duration = i64::from(self.config.duration_ms) * 10_000;
        self.visuals.retain(|(time, _)| pts - *time < duration);
        let mut image = base.clone();
        let mut key_row = 0;
        for (time, visual) in &self.visuals {
            if pts < *time {
                continue;
            }
            let progress = (pts - *time) as f32 / duration as f32;
            let alpha = ((1.0 - progress) * 220.0) as u8;
            match visual {
                Visual::Click {
                    x,
                    y,
                    color,
                    double,
                } => {
                    let radius = self.config.size as f32 * (0.35 + 0.65 * progress);
                    ring(&mut image, *x, *y, radius, 2.5, *color, alpha);
                    if *double {
                        ring(&mut image, *x, *y, radius * 0.65, 2.0, *color, alpha);
                    }
                }
                Visual::Trail { from, to, drag } => line(
                    &mut image,
                    *from,
                    *to,
                    if *drag { 3 } else { 1 },
                    if *drag {
                        self.config.right_color
                    } else {
                        self.config.left_color
                    },
                    alpha,
                ),
                Visual::Wheel { x, y, delta } => {
                    let step = (self.config.size / 2) as i32;
                    let direction = if *delta > 0 { -1 } else { 1 };
                    let tip = (*x, *y + direction * step);
                    line(
                        &mut image,
                        (*x, *y - direction * step),
                        tip,
                        2,
                        self.config.left_color,
                        alpha,
                    );
                    line(
                        &mut image,
                        tip,
                        (*x - step / 2, tip.1 - direction * step / 2),
                        2,
                        self.config.left_color,
                        alpha,
                    );
                    line(
                        &mut image,
                        tip,
                        (*x + step / 2, tip.1 - direction * step / 2),
                        2,
                        self.config.left_color,
                        alpha,
                    );
                }
                Visual::Key(label) => {
                    draw_label(&mut image, label, key_row, self.config, alpha);
                    key_row += 1;
                }
            }
        }
        self.dirty = false;
        image
    }
}

fn blend(image: &mut BgraImage, x: i32, y: i32, rgb: [u8; 3], alpha: u8) {
    if x < 0 || y < 0 || x as u32 >= image.width || y as u32 >= image.height {
        return;
    }
    let offset = (y as usize * image.width as usize + x as usize) * 4;
    for (index, color) in [rgb[2], rgb[1], rgb[0]].iter().enumerate() {
        image.data[offset + index] = ((u32::from(image.data[offset + index])
            * (255 - u32::from(alpha))
            + u32::from(*color) * u32::from(alpha)
            + 127)
            / 255) as u8;
    }
    image.data[offset + 3] = 255;
}

fn ring(
    image: &mut BgraImage,
    x: i32,
    y: i32,
    radius: f32,
    thickness: f32,
    rgb: [u8; 3],
    alpha: u8,
) {
    let bound = (radius + thickness).ceil() as i32;
    for py in (y - bound).max(0)..=(y + bound).min(image.height as i32 - 1) {
        for px in (x - bound).max(0)..=(x + bound).min(image.width as i32 - 1) {
            let distance = (((px - x).pow(2) + (py - y).pow(2)) as f32).sqrt();
            if (distance - radius).abs() <= thickness / 2.0 {
                blend(image, px, py, rgb, alpha);
            }
        }
    }
}

fn line(
    image: &mut BgraImage,
    from: (i32, i32),
    to: (i32, i32),
    width: i32,
    rgb: [u8; 3],
    alpha: u8,
) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let steps = dx.abs().max(dy.abs()).max(1);
    for step in 0..=steps {
        let (x, y) = (
            from.0 + (i64::from(dx) * i64::from(step) / i64::from(steps)) as i32,
            from.1 + (i64::from(dy) * i64::from(step) / i64::from(steps)) as i32,
        );
        for py in y - width..=y + width {
            for px in x - width..=x + width {
                blend(image, px, py, rgb, alpha);
            }
        }
    }
}

/// 内置 5×7 英文键名点阵，不依赖系统字体、WebView 或逐帧文字栅格化服务。
fn glyph(c: char) -> [u8; 7] {
    match c {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        _ => [0; 7],
    }
}

fn draw_label(image: &mut BgraImage, label: &str, row: i32, config: InputEffects, alpha: u8) {
    let scale = (config.size / 14).clamp(1, 4) as i32;
    let width = label.len() as i32 * 6 * scale + 12;
    let height = 7 * scale + 12;
    let x = match config.position {
        KeyPosition::TopLeft | KeyPosition::BottomLeft => 8,
        KeyPosition::BottomRight => (image.width as i32 - width - 8).max(0),
    };
    // 顶部提示向下排列，底部提示向上排列，坐标均相对输出视频。
    let y = match config.position {
        KeyPosition::TopLeft => 8 + row * (height + 4),
        KeyPosition::BottomLeft | KeyPosition::BottomRight => {
            (image.height as i32 - 8 - (row + 1) * (height + 4)).max(0)
        }
    };
    for py in y..(y + height).min(image.height as i32) {
        for px in x..(x + width).min(image.width as i32) {
            blend(image, px, py, [20, 24, 32], alpha);
        }
    }
    for (index, c) in label.chars().enumerate() {
        for (gy, bits) in glyph(c).iter().enumerate() {
            for gx in 0..5 {
                if bits & (1 << (4 - gx)) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        blend(
                            image,
                            x + 6 + index as i32 * 6 * scale + gx * scale + sx,
                            y + 6 + gy as i32 * scale + sy,
                            [255; 3],
                            alpha,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_origin_tracks_negative_monitors_and_moving_windows() {
        let mut compositor = Compositor::new(
            InputEffects::default(),
            Rect {
                x: 100,
                y: 40,
                width: 300,
                height: 200,
            },
        );
        compositor.set_origin((-1920, -200));
        assert_eq!(compositor.local(-1820, -160), Some((0, 0)));
        assert_eq!(compositor.local(-1521, 39), Some((299, 199)));
        assert_eq!(compositor.local(-1821, -160), None);
        assert_eq!(compositor.local(-1520, 39), None);
        // 窗口移动后使用最新原点，不在旧位置继续绘制输入特效。
        compositor.set_origin((500, 300));
        assert_eq!(compositor.local(-1820, -160), None);
        assert_eq!(compositor.local(600, 340), Some((0, 0)));
        compositor.clear();
        assert_eq!(compositor.local(610, 350), Some((10, 10)));
    }
    #[test]
    fn effects_are_bounded_and_buttons_have_distinct_pixels() {
        let config = InputEffects {
            clicks: true,
            keyboard: true,
            ..Default::default()
        };
        let region = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let base = BgraImage::new(100, 100);
        let mut frames = Vec::new();
        for (right, double) in [(false, false), (true, false), (false, true)] {
            let mut compositor = Compositor::new(config, region);
            compositor.ingest(
                0,
                InputKind::Click {
                    x: 50,
                    y: 50,
                    right,
                    double,
                },
            );
            frames.push(compositor.render(&base, 1).data);
        }
        assert_ne!(frames[0], frames[1]);
        assert_ne!(frames[0], frames[2]);
        let mut compositor = Compositor::new(config, region);
        for pts in 0..1000 {
            compositor.ingest(
                pts,
                InputKind::Click {
                    x: 50,
                    y: 50,
                    right: false,
                    double: false,
                },
            );
        }
        assert_eq!(compositor.visuals.len(), 256);
        for label in ["CTRL+C", "CTRL+V", "ENTER", "ESC", "ESC"] {
            compositor.ingest(1000, InputKind::Key(label.into()));
        }
        assert_eq!(
            compositor
                .visuals
                .iter()
                .filter(|(_, v)| matches!(v, Visual::Key(_)))
                .count(),
            3
        );
    }

    #[test]
    fn drag_does_not_bridge_release_or_outside_region() {
        let config = InputEffects {
            drag: true,
            ..Default::default()
        };
        let mut compositor = Compositor::new(
            config,
            Rect {
                x: 10,
                y: 10,
                width: 100,
                height: 100,
            },
        );
        for (x, dragging) in [
            (20, true),
            (30, true),
            (40, false),
            (50, true),
            (200, true),
            (60, true),
        ] {
            compositor.ingest(0, InputKind::Move { x, y: 30, dragging });
        }
        assert!(matches!(
            compositor.visuals.back().unwrap().1,
            Visual::Trail {
                from: (50, 20),
                to: (50, 20),
                ..
            }
        ));
        assert_eq!(compositor.visuals.len(), 4);
    }
    #[test]
    fn clean_base_expiry_and_region_clipping() {
        let mut config = InputEffects::default();
        config.clicks = true;
        let mut compositor = Compositor::new(
            config,
            Rect {
                x: 100,
                y: 200,
                width: 80,
                height: 80,
            },
        );
        let base = BgraImage::new(80, 80);
        compositor.ingest(
            0,
            InputKind::Click {
                x: 90,
                y: 210,
                right: false,
                double: false,
            },
        );
        assert!(!compositor.needs_frame());
        compositor.ingest(
            0,
            InputKind::Click {
                x: 140,
                y: 240,
                right: false,
                double: false,
            },
        );
        assert_ne!(compositor.render(&base, 0).data, base.data);
        assert!(base.data.iter().all(|b| *b == 0));
        assert!(compositor.needs_frame());
        assert_eq!(compositor.render(&base, 7_000_000).data, base.data);
        assert!(!compositor.needs_frame());
    }
    #[test]
    fn pause_clear_removes_keys_and_trails_and_tiny_frames_are_safe() {
        let mut config = InputEffects::default();
        config.keyboard = true;
        config.drag = true;
        let mut compositor = Compositor::new(
            config,
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        );
        compositor.ingest(0, InputKind::Key("CTRL+C".into()));
        compositor.ingest(
            0,
            InputKind::Move {
                x: 1,
                y: 1,
                dragging: true,
            },
        );
        assert_eq!(compositor.render(&BgraImage::new(2, 2), 1).data.len(), 16);
        compositor.clear();
        assert!(!compositor.needs_frame());
        assert!(compositor.last_point.is_none());
    }
}
