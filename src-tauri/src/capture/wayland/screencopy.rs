//! wlr-screencopy（Sway、niri、Hyprland 等 wlroots 系合成器）：复用 xcap 已依赖的 libwayshot，
//! 它负责共享内存缓冲、像素格式与显示器旋转。
use super::{outputs, Output, Shot};

pub(super) fn capture(layout: &[Output], target: usize, cursor: bool) -> Result<Shot, String> {
    let wanted = layout.get(target).ok_or("未找到目标显示器。")?;
    let wayshot = outputs::connect()?;
    let info = wayshot
        .get_all_outputs()
        .iter()
        .find(|info| info.name == wanted.name)
        .ok_or("目标显示器已断开。")?;
    let image = wayshot
        .screenshot_single_output(info, cursor)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    Ok(Shot {
        output: outputs::from_wayshot(info),
        image,
    })
}
