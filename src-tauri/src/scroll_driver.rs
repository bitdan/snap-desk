//! 自动滚动控制：仅在鼠标留在选区时发送滚轮，位移由拼接结果反馈。
use enigo::{Axis, Enigo, Mouse, Settings};

pub struct Driver {
    input: Enigo,
    anchor: Option<(i32, i32)>,
    pub step: i32,
    pub unchanged: u32,
    pub retries: u32,
    pub moving: u32,
    pub scrolled: bool,
}
impl Driver {
    /// 平台输入句柄在采集线程内创建，不跨线程移动。
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            input: Enigo::new(&Settings::default()).map_err(|e| e.to_string())?,
            anchor: None,
            step: 1,
            unchanged: 0,
            retries: 0,
            moving: 0,
            scrolled: false,
        })
    }
    pub fn check(&mut self, bounds: (f64, f64, f64, f64)) -> Result<(), String> {
        let point = self.input.location().map_err(|e| e.to_string())?;
        let (x, y) = (f64::from(point.0), f64::from(point.1));
        if x < bounds.0
            || y < bounds.1
            || x >= bounds.2
            || y >= bounds.3
            || self
                .anchor
                .is_some_and(|p| p.0.abs_diff(point.0) > 8 || p.1.abs_diff(point.1) > 8)
        {
            return Err("鼠标移动或离开选区，已结束采集。".into());
        }
        self.anchor = Some(point);
        Ok(())
    }
    /// 每次只发一格，由采集线程控制间隔；避免一次多格让页面突然跳动。
    pub fn scroll_tick(&mut self) -> Result<(), String> {
        self.input
            .scroll(1, Axis::Vertical)
            .map_err(|e| e.to_string())?;
        self.scrolled = true;
        Ok(())
    }
    /// 滚轮单位不是像素，按实际位移逐步调节，保留至少半屏重叠。
    pub fn adapt(&mut self, shift: u32, height: u32) {
        if shift > height / 2 {
            self.step = (self.step / 2).max(1);
        } else if shift < height / 4 {
            self.step = (self.step + 1).min(6);
        }
        self.unchanged = 0;
    }
}
