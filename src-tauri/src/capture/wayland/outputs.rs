//! 显示器布局：经 xdg-output 读取各显示器的名称、逻辑位置与尺寸（与合成器布局一致）以及物理像素尺寸。
//! 复用 xcap 已依赖的 libwayshot 连接；目标显示器优先取合成器报告的焦点显示器。
use std::process::{Command, Stdio};

/// 逻辑坐标下的显示器；`physical` 为该显示器的像素尺寸（已考虑旋转），用于计算缩放比例。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Output {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub physical: (u32, u32),
}

impl Output {
    /// 物理像素 ÷ 逻辑尺寸；读不到物理尺寸时按 1 处理。
    pub fn scale(&self) -> f64 {
        if self.width <= 0 || self.physical.0 == 0 {
            1.0
        } else {
            f64::from(self.physical.0) / f64::from(self.width)
        }
    }
}

pub(super) fn connect() -> Result<libwayshot_xcap::WayshotConnection, String> {
    // libwayshot 在合成器缺少 xdg-output 时直接 panic；转成普通错误，交给门户后端。
    std::panic::catch_unwind(libwayshot_xcap::WayshotConnection::new)
        .map_err(|_| "合成器不支持 xdg-output。".to_string())?
        .map_err(|e| e.to_string())
}

pub(super) fn from_wayshot(info: &libwayshot_xcap::output::OutputInfo) -> Output {
    use wayland_client::protocol::wl_output::Transform;
    let region = &info.logical_region.inner;
    // wl_output 的模式尺寸不随旋转变化；竖屏显示器交换宽高后才与逻辑尺寸同向。
    let rotated = matches!(
        info.transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    );
    let (pw, ph) = (info.physical_size.width, info.physical_size.height);
    Output {
        name: info.name.clone(),
        x: region.position.x,
        y: region.position.y,
        width: i32::try_from(region.size.width).unwrap_or(i32::MAX),
        height: i32::try_from(region.size.height).unwrap_or(i32::MAX),
        physical: if rotated { (ph, pw) } else { (pw, ph) },
    }
}

pub(super) fn layout() -> Result<Vec<Output>, String> {
    let wayshot = connect()?;
    let outputs: Vec<Output> = wayshot
        .get_all_outputs()
        .iter()
        .map(from_wayshot)
        .filter(|o| o.width > 0 && o.height > 0)
        .collect();
    if outputs.is_empty() {
        return Err("未找到可用显示器，请检查显示器连接。".into());
    }
    Ok(outputs)
}

/// 焦点显示器的名称；只调用合成器自带的命令行或 D-Bus 接口，读不到时返回 None。
fn focused_name(compositor: &str) -> Option<String> {
    let json = |program: &str, args: &[&str]| -> Option<serde_json::Value> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| serde_json::from_slice(&output.stdout).ok())
            .flatten()
    };
    let focused_in = |value: serde_json::Value| -> Option<String> {
        value
            .as_array()?
            .iter()
            .find(|item| item["focused"] == true)?["name"]
            .as_str()
            .map(str::to_owned)
    };
    match compositor {
        "sway" => focused_in(json("swaymsg", &["-t", "get_outputs", "-r"])?),
        "hyprland" => focused_in(json("hyprctl", &["-j", "monitors"])?),
        "niri" => json("niri", &["msg", "--json", "focused-output"])?["name"]
            .as_str()
            .map(str::to_owned),
        "kwin" => {
            let conn = zbus::blocking::Connection::session().ok()?;
            let proxy =
                zbus::blocking::Proxy::new(&conn, "org.kde.KWin", "/KWin", "org.kde.KWin").ok()?;
            proxy.call::<_, _, String>("activeOutputName", &()).ok()
        }
        _ => None,
    }
}

/// 截图的目标显示器下标：焦点显示器，其次位于原点的显示器（多为主屏），最后是第一块。
pub(super) fn target(outputs: &[Output], compositor: &str) -> usize {
    pick(outputs, focused_name(compositor).as_deref())
}

pub(super) fn pick(outputs: &[Output], focused: Option<&str>) -> usize {
    focused
        .and_then(|name| outputs.iter().position(|o| o.name == name))
        .or_else(|| outputs.iter().position(|o| o.x == 0 && o.y == 0))
        .unwrap_or(0)
}

/// 门户返回的流按逻辑位置对应显示器；没有位置时只在尺寸唯一匹配或只有一块显示器时接受。
pub(super) fn match_stream(
    outputs: &[Output],
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
) -> Option<usize> {
    if let Some((x, y)) = position {
        return outputs.iter().position(|o| o.x == x && o.y == y);
    }
    if outputs.len() == 1 {
        return Some(0);
    }
    let (w, h) = size?;
    let mut matches = outputs.iter().enumerate().filter(|(_, o)| {
        (o.width, o.height) == (w, h) || (o.physical.0 as i32, o.physical.1 as i32) == (w, h)
    });
    let first = matches.next()?.0;
    matches.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, x: i32, y: i32, width: i32, height: i32, physical: (u32, u32)) -> Output {
        Output {
            name: name.into(),
            x,
            y,
            width,
            height,
            physical,
        }
    }

    #[test]
    fn picks_focused_then_origin_then_first() {
        let outputs = [
            output("HDMI-A-1", -1920, 0, 1920, 1080, (1920, 1080)),
            output("eDP-1", 0, 0, 1280, 800, (2560, 1600)),
        ];
        assert_eq!(pick(&outputs, Some("HDMI-A-1")), 0);
        assert_eq!(pick(&outputs, Some("DP-9")), 1);
        assert_eq!(pick(&outputs, None), 1);
        assert_eq!(pick(&outputs[..1], None), 0);
        assert_eq!(outputs[1].scale(), 2.0);
        assert_eq!(output("x", 0, 0, 0, 0, (0, 0)).scale(), 1.0);
    }

    #[test]
    fn matches_portal_streams_by_position_or_unique_size() {
        let outputs = [
            output("A", 0, 0, 1920, 1080, (1920, 1080)),
            output("B", 1920, 0, 1280, 800, (2560, 1600)),
        ];
        assert_eq!(match_stream(&outputs, Some((1920, 0)), None), Some(1));
        assert_eq!(match_stream(&outputs, Some((5, 5)), None), None);
        assert_eq!(match_stream(&outputs, None, Some((2560, 1600))), Some(1));
        assert_eq!(match_stream(&outputs, None, Some((1, 1))), None);
        assert_eq!(match_stream(&outputs[..1], None, None), Some(0));
        let twins = [
            outputs[0].clone(),
            output("C", 1920, 0, 1920, 1080, (1920, 1080)),
        ];
        assert_eq!(match_stream(&twins, None, Some((1920, 1080))), None);
    }
}
