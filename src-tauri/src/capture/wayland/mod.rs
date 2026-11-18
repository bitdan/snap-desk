//! Wayland 截图：冻结一块显示器的画面，再沿用现有的选区覆盖层。
//!
//! Wayland 不允许应用自由定位窗口，覆盖层只能全屏到某一块显示器上，因此每次只抓取一块：
//! 优先取合成器报告的焦点显示器（niri / Sway / Hyprland / KWin），否则取位于原点的显示器；
//! 屏幕共享门户由用户在授权对话框中选择，选中哪块就截哪块。
//!
//! 后端按顺序回退，并记住上次成功的后端，避免每次截图重新尝试不可用的协议：
//! KWin ScreenShot2 → ext-image-copy-capture → wlr-screencopy → ScreenCast 门户（持久化恢复令牌）→ Screenshot 门户。
//! 实现思路参考 mark-shot（MIT / Apache-2.0）的协议选择，代码为独立实现。
mod image_copy;
mod kwin;
mod outputs;
mod pipewire_frame;
mod portal;
mod screencopy;

use image::RgbaImage;
pub(crate) use outputs::Output;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

/// 一次截图的结果：所在显示器的逻辑几何与该显示器的物理像素画面。
pub(crate) struct Shot {
    pub output: Output,
    pub image: RgbaImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Kwin,
    ImageCopy,
    Screencopy,
    ScreenCast,
    Screenshot,
}

impl Backend {
    const ALL: [Self; 5] = [
        Self::Kwin,
        Self::ImageCopy,
        Self::Screencopy,
        Self::ScreenCast,
        Self::Screenshot,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Kwin => "kwin",
            Self::ImageCopy => "ext-image-copy-capture",
            Self::Screencopy => "wlr-screencopy",
            Self::ScreenCast => "portal-screencast",
            Self::Screenshot => "portal-screenshot",
        }
    }

    fn run(self, outputs: &[Output], target: usize, cursor: bool) -> Result<Shot, String> {
        match self {
            Self::Kwin => kwin::capture(outputs, target, cursor),
            Self::ImageCopy => image_copy::capture(outputs, target, cursor),
            Self::Screencopy => screencopy::capture(outputs, target, cursor),
            Self::ScreenCast => portal::screencast(outputs, target, cursor),
            Self::Screenshot => portal::screenshot(outputs, target),
        }
    }
}

/// 当前桌面公布的后端，按优先顺序。
pub(crate) fn available(
    protocols: &crate::shared::desktop_support::DesktopProtocols,
) -> Vec<Backend> {
    Backend::ALL
        .into_iter()
        .filter(|backend| match backend {
            Backend::Kwin => protocols.kwin_screenshot,
            Backend::ImageCopy => protocols.ext_image_copy_capture,
            Backend::Screencopy => protocols.wlr_screencopy,
            Backend::ScreenCast => protocols.portal_screencast,
            Backend::Screenshot => protocols.portal_screenshot,
        })
        .collect()
}

/// 从上次成功的后端开始，依次尝试其余后端；全部失败时返回每个后端的原因。
pub(crate) fn run_chain<T>(
    candidates: &[Backend],
    remembered: &AtomicUsize,
    mut attempt: impl FnMut(Backend) -> Result<T, String>,
) -> Result<T, String> {
    if candidates.is_empty() {
        return Err(NO_BACKEND.into());
    }
    let start = remembered.load(Ordering::Relaxed).min(candidates.len() - 1);
    let mut failures = Vec::new();
    for index in (start..candidates.len()).chain(0..start) {
        let backend = candidates[index];
        match attempt(backend) {
            Ok(value) => {
                remembered.store(index, Ordering::Relaxed);
                return Ok(value);
            }
            // 用户在授权对话框中取消时直接结束，不再换用其他后端弹出第二个对话框。
            Err(error) if error == portal::CANCELLED => return Err(error),
            Err(error) => {
                log::info!(
                    "event=capture.wayland_backend_failed module=capture backend={} error={error}",
                    backend.name()
                );
                failures.push(format!("{}: {error}", backend.name()));
            }
        }
    }
    log::warn!(
        "event=capture.wayland_failed module=capture reasons={}",
        failures.join("; ")
    );
    Err(CAPTURE_FAILED.into())
}

const NO_BACKEND: &str = crate::shared::desktop_support::NO_CAPTURE_BACKEND;
const CAPTURE_FAILED: &str = "Wayland 截图失败，请确认已允许屏幕共享，详情见日志。";

/// 上次截图所在显示器的逻辑几何，覆盖层据此全屏到同一块显示器。
static LAST_OUTPUT: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

pub(crate) fn last_output() -> Option<(i32, i32, i32, i32)> {
    LAST_OUTPUT.lock().ok().and_then(|value| *value)
}

pub(crate) fn capture(include_cursor: bool) -> Result<Shot, String> {
    let support = crate::shared::desktop_support::current();
    let candidates = available(support.protocols());
    let layout = outputs::layout()?;
    let target = outputs::target(&layout, support.compositor());
    static REMEMBERED: AtomicUsize = AtomicUsize::new(0);
    let shot = run_chain(&candidates, &REMEMBERED, |backend| {
        backend.run(&layout, target, include_cursor)
    })?;
    if let Ok(mut last) = LAST_OUTPUT.lock() {
        let o = &shot.output;
        *last = Some((o.x, o.y, o.width, o.height));
    }
    Ok(shot)
}

/// 门户与 KWin 返回的像素格式统一换算为 RGBA；`bgra` 为小端 ARGB32 / XRGB32 的内存顺序。
pub(super) fn to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bgra: bool,
    keep_alpha: bool,
) -> Result<RgbaImage, String> {
    let row = usize::try_from(width).map_err(|_| "截图过大。")? * 4;
    let rows = usize::try_from(height).map_err(|_| "截图过大。")?;
    if width == 0 || height == 0 || stride < row || data.len() < stride * (rows - 1) + row {
        return Err("截图数据不完整，请重新截图。".into());
    }
    if u64::from(width) * u64::from(height) > 64_000_000 {
        return Err("显示器超过 6400 万像素，暂不支持截图。".into());
    }
    let mut rgba = Vec::with_capacity(row * rows);
    for y in 0..rows {
        for p in data[y * stride..y * stride + row].chunks_exact(4) {
            let alpha = if keep_alpha { p[3] } else { 255 };
            if bgra {
                rgba.extend_from_slice(&[p[2], p[1], p[0], alpha]);
            } else {
                rgba.extend_from_slice(&[p[0], p[1], p[2], alpha]);
            }
        }
    }
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "截图数据无效。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_remembers_the_last_backend_that_worked() {
        let remembered = AtomicUsize::new(0);
        let candidates = [Backend::Kwin, Backend::Screencopy, Backend::ScreenCast];
        let mut calls = Vec::new();
        let result = run_chain(&candidates, &remembered, |backend| {
            calls.push(backend);
            if backend == Backend::ScreenCast {
                Ok(7)
            } else {
                Err("no".into())
            }
        });
        assert_eq!(result, Ok(7));
        assert_eq!(remembered.load(Ordering::Relaxed), 2);
        calls.clear();
        // 下一次直接从门户开始；门户失败后回到链首。
        let result = run_chain(&candidates, &remembered, |backend| {
            calls.push(backend);
            if backend == Backend::Kwin {
                Ok(1)
            } else {
                Err("no".into())
            }
        });
        assert_eq!(result, Ok(1));
        assert_eq!(calls, [Backend::ScreenCast, Backend::Kwin]);
        assert_eq!(remembered.load(Ordering::Relaxed), 0);
        assert_eq!(
            run_chain(&[], &remembered, |_| Ok::<_, String>(())),
            Err(NO_BACKEND.into())
        );
        assert_eq!(
            run_chain(&candidates, &remembered, |_| Err::<(), _>("x".into())),
            Err(CAPTURE_FAILED.into())
        );
        calls.clear();
        let cancelled = run_chain(&candidates, &remembered, |backend| {
            calls.push(backend);
            Err::<(), _>(portal::CANCELLED.into())
        });
        assert_eq!(cancelled, Err(portal::CANCELLED.into()));
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn converts_padded_bgra_rows() {
        let data = [1, 2, 3, 0, 9, 9, 9, 9, 4, 5, 6, 7, 9, 9, 9, 9];
        let image = to_rgba(&data, 1, 2, 8, true, false).unwrap();
        assert_eq!(image.as_raw(), &[3, 2, 1, 255, 6, 5, 4, 255]);
        let image = to_rgba(&data, 1, 2, 8, false, true).unwrap();
        assert_eq!(image.as_raw(), &[1, 2, 3, 0, 4, 5, 6, 7]);
        assert!(to_rgba(&data[..10], 1, 2, 8, true, false).is_err());
        assert!(to_rgba(&data, 2, 2, 4, true, false).is_err());
    }
}
