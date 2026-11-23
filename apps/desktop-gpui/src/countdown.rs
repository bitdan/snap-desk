//! 延时截图倒计时：主屏中央显示剩余秒数，期间再按截图快捷键（或托盘“截图”）取消。
//!
//! Tauri 版在主窗口显示倒计时（`capture-delay` 事件）；GPUI 版没有主窗口，改为一个小的置顶提示窗。
//! 提示窗不抢焦点、鼠标穿透，避免打断用户为截图准备的界面状态（菜单、悬停提示等）；
//! 倒计时结束先关闭提示窗并稍候，再拍屏，提示窗不会出现在截图里。
use gpui_kit::*;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

/// 关闭提示窗后等待系统合成器移除它再拍屏。
const SETTLE: Duration = Duration::from_millis(250);
/// 检查取消的间隔。
const TICK: Duration = Duration::from_millis(100);

/// 剩余整秒（向上取整），与 Tauri 版 `wait_delay` 一致。
pub fn remaining(deadline: Instant, now: Instant) -> u8 {
    if now >= deadline {
        return 0;
    }
    (deadline - now).as_secs_f64().ceil().min(f64::from(u8::MAX)) as u8
}

/// 一次倒计时；`cancelled` 由取消入口设置，等待循环每个 TICK 检查。
#[derive(Clone)]
pub struct Countdown {
    cancelled: Rc<Cell<bool>>,
}

impl Countdown {
    pub fn cancel(&self) {
        self.cancelled.set(true);
    }
}

struct View {
    remaining: u8,
    hint: String,
}

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            // 与截图浮层提示（Vue `.capture-tip`）同一套深色圆角框。
            .rounded(px(10.0))
            .bg(rgba(0x242330ed))
            .text_color(rgb(0xffffff))
            .child(div().text_size(px(56.0)).font_weight(FontWeight::SEMIBOLD).child(self.remaining.to_string()))
            .child(div().px(px(12.0)).text_xs().text_color(rgb(0xb7b4c8)).child(self.hint.clone()))
    }
}

/// 显示倒计时并等待；返回 true 表示倒计时走完（应继续截图），false 表示已取消。
/// `register` 在提示窗打开后被调用一次，宿主据此保存取消句柄。
pub async fn run(
    cx: &mut AsyncApp,
    seconds: u8,
    hint: String,
    register: impl FnOnce(&mut App, Countdown),
) -> bool {
    let countdown = Countdown { cancelled: Rc::new(Cell::new(false)) };
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds));
    let opened = cx.update(|cx| {
        register(cx, countdown.clone());
        let size = size(px(220.0), px(150.0));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        let mut view = None;
        let handle = cx.open_window(options, |window, cx| {
            if let Err(error) = crate::native::set_click_through(window, true) {
                log::warn!("event=gpui.countdown_click_through_failed message={error}");
            }
            let entity = cx.new(|_| View { remaining: seconds, hint });
            view = Some(entity.clone());
            entity
        });
        match handle {
            Ok(handle) => Some((AnyWindowHandle::from(handle), view)),
            Err(error) => {
                // 提示窗失败不影响延时截图本身。
                log::warn!("event=gpui.countdown_window_failed message={error}");
                None
            }
        }
    });
    let (window, view) = match opened {
        Some((window, view)) => (Some(window), view),
        None => (None, None),
    };
    log::info!("event=gpui.capture_countdown seconds={seconds}");
    let mut shown = seconds;
    let elapsed = loop {
        if countdown.cancelled.get() {
            break false;
        }
        let left = remaining(deadline, Instant::now());
        if left == 0 {
            break true;
        }
        if left != shown {
            shown = left;
            if let Some(view) = view.as_ref() {
                cx.update(|cx| {
                    view.update(cx, |v, cx| {
                        v.remaining = left;
                        cx.notify();
                    })
                });
            }
        }
        cx.background_executor().timer(TICK.min(deadline - Instant::now())).await;
    };
    if let Some(window) = window {
        cx.update(|cx| {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        });
    }
    if elapsed {
        cx.background_executor().timer(SETTLE).await;
    } else {
        log::info!("event=gpui.capture_countdown_cancelled");
    }
    elapsed
}

#[cfg(test)]
mod tests {
    use super::remaining;
    use std::time::{Duration, Instant};

    #[test]
    fn remaining_rounds_up_and_stops_at_zero() {
        let now = Instant::now();
        assert_eq!(remaining(now + Duration::from_millis(2100), now), 3);
        assert_eq!(remaining(now + Duration::from_millis(1000), now), 1);
        assert_eq!(remaining(now, now), 0);
        assert_eq!(remaining(now, now + Duration::from_secs(1)), 0);
    }
}
