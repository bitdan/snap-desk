//! Wayland 会话的协议探测：列出合成器公布的全局对象接口名，并查询 KWin 与 xdg-desktop-portal 的 D-Bus 接口。
//! 只读取接口是否存在，不发起截图、授权或绑定任何协议对象；任一步失败都按“不可用”处理。
use super::desktop_support::{compositor, Probe};
use std::time::Duration;
use wayland_client::{protocol::wl_registry, Connection, Dispatch, QueueHandle};

struct Globals(Vec<String>);

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { interface, .. } = event {
            // 恶意或异常合成器可能公布大量对象；只保留前 512 个接口名。
            if state.0.len() < 512 {
                state.0.push(interface);
            }
        }
    }
}

fn globals() -> Result<Vec<String>, String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let handle = queue.handle();
    let _registry = conn.display().get_registry(&handle, ());
    let mut found = Globals(Vec::new());
    queue.roundtrip(&mut found).map_err(|e| e.to_string())?;
    Ok(found.0)
}

/// 对象实现了接口且能读出属性即视为存在；门户前端只导出有后端实现的接口。
fn has_interface(
    conn: &zbus::blocking::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> bool {
    let Ok(proxy) =
        zbus::blocking::Proxy::new(conn, destination, path, "org.freedesktop.DBus.Properties")
    else {
        return false;
    };
    proxy.call_method("Get", &(interface, property)).is_ok()
}

pub(crate) fn probe(desktop: &str) -> Probe {
    let mut found = Probe {
        compositor: compositor(
            desktop,
            std::env::var_os("SWAYSOCK").is_some(),
            std::env::var_os("NIRI_SOCKET").is_some(),
            std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some(),
        ),
        xwayland: std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()),
        ..Probe::default()
    };
    match globals() {
        Ok(names) => found.globals = names,
        Err(error) => {
            log::warn!("event=app.wayland_probe_failed module=app stage=registry error={error}")
        }
    }
    // 门户首次访问会被 D-Bus 激活，给足启动时间但不无限等待。
    let conn = zbus::blocking::connection::Builder::session()
        .map(|builder| builder.method_timeout(Duration::from_secs(5)))
        .and_then(|builder| builder.build());
    let conn = match conn {
        Ok(conn) => conn,
        Err(error) => {
            log::warn!("event=app.wayland_probe_failed module=app stage=dbus error={error}");
            return found;
        }
    };
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    found.kwin_screenshot = has_interface(
        &conn,
        "org.kde.KWin",
        "/org/kde/KWin/ScreenShot2",
        "org.kde.KWin.ScreenShot2",
        "Version",
    );
    found.portal_screenshot = has_interface(
        &conn,
        PORTAL,
        PORTAL_PATH,
        "org.freedesktop.portal.Screenshot",
        "version",
    );
    found.portal_screencast = has_interface(
        &conn,
        PORTAL,
        PORTAL_PATH,
        "org.freedesktop.portal.ScreenCast",
        "version",
    );
    found.portal_global_shortcuts = has_interface(
        &conn,
        PORTAL,
        PORTAL_PATH,
        "org.freedesktop.portal.GlobalShortcuts",
        "version",
    );
    found.portal_remote_desktop = has_interface(
        &conn,
        PORTAL,
        PORTAL_PATH,
        "org.freedesktop.portal.RemoteDesktop",
        "version",
    );
    found
}
