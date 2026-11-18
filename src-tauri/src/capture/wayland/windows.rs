//! Wayland 窗口吸附：协议本身不提供其他窗口的位置，只能向合成器询问。
//! Sway（swaymsg）、niri（niri msg）、Hyprland（hyprctl）走各自的命令行 IPC；
//! KDE 加载一次性 KWin 脚本，脚本经 D-Bus 把窗口几何回传给本进程后立即卸载。GNOME 不提供，吸附不可用。
//! 结果只含逻辑坐标下的外框与标题，前者在冻结显示器内换算为物理像素；先返回的窗口位于上层。
use super::Output;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

/// 逻辑坐标（合成器全局布局）中的可吸附窗口。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Window {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub title: String,
    pub app: String,
}

fn json(program: &str, args: &[&str]) -> Option<serde_json::Value> {
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
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}

fn int(value: &serde_json::Value) -> Option<i32> {
    value.as_f64().map(|v| v.round() as i32)
}

pub(crate) fn list(compositor: &str, output: &Output) -> Vec<Window> {
    let windows = match compositor {
        "sway" => json("swaymsg", &["-t", "get_tree", "-r"]).map(|tree| sway(&tree)),
        "niri" => niri(output),
        "hyprland" => hyprland(output),
        "kwin" => kwin().ok(),
        _ => None,
    };
    let mut windows = windows.unwrap_or_default();
    windows.retain(|w| w.width >= 40 && w.height >= 40);
    windows.truncate(256);
    windows
}

/// Sway 树：只取可见的普通与浮动容器；浮动窗口在平铺窗口之上。
pub(crate) fn sway(tree: &serde_json::Value) -> Vec<Window> {
    fn walk(
        node: &serde_json::Value,
        floating: bool,
        tiled: &mut Vec<Window>,
        floats: &mut Vec<Window>,
    ) {
        let is_window = node["pid"].is_number() && node["visible"] == true;
        if is_window {
            let rect = &node["rect"];
            if let (Some(x), Some(y), Some(width), Some(height)) = (
                int(&rect["x"]),
                int(&rect["y"]),
                int(&rect["width"]),
                int(&rect["height"]),
            ) {
                let window = Window {
                    x,
                    y,
                    width,
                    height,
                    title: text(node, "name"),
                    app: if node["app_id"].is_string() {
                        text(node, "app_id")
                    } else {
                        text(&node["window_properties"], "class")
                    },
                };
                if floating {
                    floats.push(window);
                } else {
                    tiled.push(window);
                }
            }
        }
        for child in node["nodes"].as_array().into_iter().flatten() {
            walk(child, floating, tiled, floats);
        }
        for child in node["floating_nodes"].as_array().into_iter().flatten() {
            walk(child, true, tiled, floats);
        }
    }
    let (mut tiled, mut floats) = (Vec::new(), Vec::new());
    walk(tree, false, &mut tiled, &mut floats);
    floats.reverse();
    floats.extend(tiled);
    floats
}

/// niri：只取目标显示器上活动工作区的窗口；位置来自 `layout`（较新的 niri 才提供，旧版本不吸附）。
fn niri(output: &Output) -> Option<Vec<Window>> {
    let workspaces = json("niri", &["msg", "--json", "workspaces"])?;
    let windows = json("niri", &["msg", "--json", "windows"])?;
    Some(niri_windows(&workspaces, &windows, output))
}

pub(crate) fn niri_windows(
    workspaces: &serde_json::Value,
    windows: &serde_json::Value,
    output: &Output,
) -> Vec<Window> {
    let active: Vec<u64> = workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| w["is_active"] == true && w["output"].as_str() == Some(output.name.as_str()))
        .filter_map(|w| w["id"].as_u64())
        .collect();
    let mut found: Vec<(bool, bool, Window)> = Vec::new();
    for window in windows.as_array().into_iter().flatten() {
        if !window["workspace_id"]
            .as_u64()
            .is_some_and(|id| active.contains(&id))
        {
            continue;
        }
        let layout = &window["layout"];
        let (Some(tx), Some(ty)) = (
            int(&layout["tile_pos_in_workspace_view"][0]),
            int(&layout["tile_pos_in_workspace_view"][1]),
        ) else {
            continue;
        };
        let (ox, oy) = (
            int(&layout["window_offset_in_tile"][0]).unwrap_or(0),
            int(&layout["window_offset_in_tile"][1]).unwrap_or(0),
        );
        let (Some(width), Some(height)) = (
            int(&layout["window_size"][0]),
            int(&layout["window_size"][1]),
        ) else {
            continue;
        };
        found.push((
            window["is_focused"] == true,
            window["is_floating"] == true,
            Window {
                x: output.x + tx + ox,
                y: output.y + ty + oy,
                width,
                height,
                title: text(window, "title"),
                app: text(window, "app_id"),
            },
        ));
    }
    // 焦点窗口最上，其次浮动窗口，最后平铺窗口。
    found.sort_by_key(|(focused, floating, _)| (!*focused, !*floating));
    found.into_iter().map(|(_, _, window)| window).collect()
}

/// Hyprland：目标显示器活动工作区上已映射、未隐藏的客户端；`focusHistoryID` 越小越靠上。
fn hyprland(output: &Output) -> Option<Vec<Window>> {
    let monitors = json("hyprctl", &["-j", "monitors"])?;
    let clients = json("hyprctl", &["-j", "clients"])?;
    Some(hyprland_windows(&monitors, &clients, output))
}

pub(crate) fn hyprland_windows(
    monitors: &serde_json::Value,
    clients: &serde_json::Value,
    output: &Output,
) -> Vec<Window> {
    let Some(workspace) = monitors
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["name"].as_str() == Some(output.name.as_str()))
        .and_then(|m| m["activeWorkspace"]["id"].as_i64())
    else {
        return Vec::new();
    };
    let mut found: Vec<(i64, Window)> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            c["mapped"] != false
                && c["hidden"] != true
                && c["workspace"]["id"].as_i64() == Some(workspace)
        })
        .filter_map(|c| {
            Some((
                c["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                Window {
                    x: int(&c["at"][0])?,
                    y: int(&c["at"][1])?,
                    width: int(&c["size"][0])?,
                    height: int(&c["size"][1])?,
                    title: text(c, "title"),
                    app: text(c, "class"),
                },
            ))
        })
        .collect();
    found.sort_by_key(|(order, _)| *order);
    found.into_iter().map(|(_, window)| window).collect()
}

/// 一次性 KWin 脚本：只读当前桌面与活动上可见的普通窗口几何，按层级从上到下回传。
const KWIN_SCRIPT: &str = r#"
const records = [];
for (const window of workspace.windowList().slice(0, 1024)) {
    const desktop = typeof workspace.currentDesktopForScreen === "function"
        ? workspace.currentDesktopForScreen(window.output) : workspace.currentDesktop;
    const visible = !window.minimized && !window.hidden && !window.deleted
        && (window.normalWindow || window.dialog || window.utility)
        && (!window.desktops.length || window.desktops.includes(desktop))
        && (!window.activities.length || window.activities.includes(workspace.currentActivity));
    if (!visible) continue;
    const g = window.frameGeometry;
    records.push({ x: g.x, y: g.y, width: g.width, height: g.height, z: window.stackingOrder,
        title: String(window.caption || ""), app: String(window.resourceClass || "") });
}
callDBus("__DESTINATION__", "/io/snapdesk/WindowSnapshot", "io.snapdesk.WindowSnapshot", "Submit", JSON.stringify(records));
"#;

struct Snapshot(Mutex<Option<mpsc::Sender<String>>>);

#[zbus::interface(name = "io.snapdesk.WindowSnapshot")]
impl Snapshot {
    fn submit(&self, records: String) {
        if let Some(sender) = self.0.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = sender.send(records);
        }
    }
}

pub(crate) fn kwin_records(records: &str) -> Vec<Window> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(records)
    else {
        return Vec::new();
    };
    let mut found: Vec<(i64, Window)> = items
        .iter()
        .filter_map(|item| {
            Some((
                item["z"].as_i64().unwrap_or(0),
                Window {
                    x: int(&item["x"])?,
                    y: int(&item["y"])?,
                    width: int(&item["width"])?,
                    height: int(&item["height"])?,
                    title: text(item, "title"),
                    app: text(item, "app"),
                },
            ))
        })
        .collect();
    // stackingOrder 越大越靠上。
    found.sort_by_key(|(z, _)| std::cmp::Reverse(*z));
    found.into_iter().map(|(_, window)| window).collect()
}

fn kwin() -> Result<Vec<Window>, String> {
    let conn = zbus::blocking::connection::Builder::session()
        .map(|builder| builder.method_timeout(Duration::from_secs(3)))
        .and_then(|builder| builder.build())
        .map_err(|e| e.to_string())?;
    let unique = conn
        .unique_name()
        .ok_or("D-Bus 连接没有唯一名称。")?
        .to_string();
    let (sender, receiver) = mpsc::channel();
    conn.object_server()
        .at(
            "/io/snapdesk/WindowSnapshot",
            Snapshot(Mutex::new(Some(sender))),
        )
        .map_err(|e| e.to_string())?;
    let mut file = tempfile::Builder::new()
        .prefix("snap-desk-windows")
        .suffix(".js")
        .tempfile()
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(
        &mut file,
        KWIN_SCRIPT.replace("__DESTINATION__", &unique).as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    let path = file.path().to_string_lossy().into_owned();
    let plugin = format!("snapdesk_windows_{}", std::process::id());
    let scripting = zbus::blocking::Proxy::new(
        &conn,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )
    .map_err(|e| e.to_string())?;
    let _ = scripting.call_method("unloadScript", &(plugin.as_str(),));
    let id: i32 = scripting
        .call("loadScript", &(path.as_str(), plugin.as_str()))
        .map_err(|e| e.to_string())?;
    let run = (|| {
        // Plasma 6 的脚本对象路径为 /Scripting/Script{id}；Plasma 5 为 /{id}。
        for object in [format!("/Scripting/Script{id}"), format!("/{id}")] {
            if let Ok(script) =
                zbus::blocking::Proxy::new(&conn, "org.kde.KWin", object, "org.kde.kwin.Script")
            {
                if script.call_method("run", &()).is_ok() {
                    return receiver
                        .recv_timeout(Duration::from_millis(800))
                        .map_err(|_| "KWin 脚本没有回传窗口列表。".to_string());
                }
            }
        }
        Err("无法运行 KWin 脚本。".to_string())
    })();
    let _ = scripting.call_method("unloadScript", &(plugin.as_str(),));
    run.map(|records| kwin_records(&records))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Output {
        Output {
            name: "DP-1".into(),
            x: 1920,
            y: 0,
            width: 1280,
            height: 800,
            physical: (2560, 1600),
        }
    }

    #[test]
    fn sway_puts_floating_windows_above_tiled_ones() {
        let tree = serde_json::json!({
            "nodes": [{ "nodes": [
                { "pid": 1, "visible": true, "name": "term", "app_id": "foot", "rect": { "x": 0, "y": 0, "width": 800, "height": 600 }, "nodes": [] },
                { "pid": 2, "visible": false, "name": "hidden", "rect": { "x": 0, "y": 0, "width": 800, "height": 600 }, "nodes": [] }
            ], "floating_nodes": [
                { "pid": 3, "visible": true, "name": "dialog", "window_properties": { "class": "Gimp" }, "rect": { "x": 10, "y": 20, "width": 300, "height": 200 }, "nodes": [] }
            ] }]
        });
        let windows = sway(&tree);
        assert_eq!(windows.len(), 2);
        assert_eq!(
            (windows[0].title.as_str(), windows[0].app.as_str()),
            ("dialog", "Gimp")
        );
        assert_eq!(windows[1].app, "foot");
    }

    #[test]
    fn niri_keeps_the_active_workspace_of_the_output() {
        let workspaces = serde_json::json!([
            { "id": 1, "output": "DP-1", "is_active": true },
            { "id": 2, "output": "DP-1", "is_active": false },
            { "id": 3, "output": "eDP-1", "is_active": true }
        ]);
        let windows = serde_json::json!([
            { "workspace_id": 1, "title": "a", "app_id": "x", "is_focused": false, "is_floating": false,
              "layout": { "tile_pos_in_workspace_view": [16.0, 16.0], "window_offset_in_tile": [2.0, 2.0], "window_size": [600, 700] } },
            { "workspace_id": 1, "title": "b", "app_id": "y", "is_focused": true, "is_floating": false,
              "layout": { "tile_pos_in_workspace_view": [640.0, 16.0], "window_size": [600, 700] } },
            { "workspace_id": 2, "title": "c", "layout": { "tile_pos_in_workspace_view": [0, 0], "window_size": [600, 700] } },
            { "workspace_id": 1, "title": "old niri" }
        ]);
        let found = niri_windows(&workspaces, &windows, &output());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "b");
        assert_eq!((found[1].x, found[1].y), (1920 + 18, 18));
    }

    #[test]
    fn hyprland_orders_by_focus_history() {
        let monitors = serde_json::json!([{ "name": "DP-1", "activeWorkspace": { "id": 4 } }]);
        let clients = serde_json::json!([
            { "at": [1920, 0], "size": [600, 400], "workspace": { "id": 4 }, "focusHistoryID": 2, "title": "back", "class": "a" },
            { "at": [2000, 50], "size": [600, 400], "workspace": { "id": 4 }, "focusHistoryID": 0, "title": "front", "class": "b" },
            { "at": [0, 0], "size": [600, 400], "workspace": { "id": 1 }, "focusHistoryID": 1, "title": "other", "class": "c" },
            { "at": [1920, 0], "size": [600, 400], "workspace": { "id": 4 }, "hidden": true, "title": "hidden" }
        ]);
        let found = hyprland_windows(&monitors, &clients, &output());
        let titles: Vec<_> = found.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["front", "back"]);
    }

    #[test]
    fn kwin_records_sort_by_stacking_order() {
        let found = kwin_records(
            r#"[{"x":0,"y":0,"width":100,"height":100,"z":1,"title":"low"},{"x":5,"y":5,"width":100,"height":100,"z":9,"title":"top"},{"x":"bad"}]"#,
        );
        let titles: Vec<_> = found.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["top", "low"]);
        assert!(kwin_records("not json").is_empty());
    }
}
