//! Wayland 全局快捷键：经 xdg-desktop-portal GlobalShortcuts 绑定（KDE Plasma、GNOME 48+ 等）。
//!
//! Wayland 不允许应用自行抓取全局按键；门户把绑定交给桌面，桌面可能弹窗请用户确认或修改按键，
//! 最终按键以桌面设置为准，设置页填写的按键只作为建议。快捷键配置变化时关闭旧会话重新绑定。
//! 门户不可用时，设置页引导用户在桌面或合成器中把快捷键绑定到 `snap-desk --capture` 等命令。
use super::{Settings, ShortcutAction};
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// 绑定请求：动作名、按界面语言显示的说明与建议按键（XDG 快捷键格式）。
pub(crate) type Binding = (&'static str, String, String);

static WORKER: OnceLock<Mutex<Sender<Vec<Binding>>>> = OnceLock::new();

/// 设置中的按键（如 `Ctrl+Shift+A`）转为门户的建议格式（如 `CTRL+SHIFT+a`）；主键用 X keysym 名称。
pub(crate) fn trigger(value: &str) -> Option<String> {
    let parts: Vec<&str> = value.split('+').filter(|part| !part.is_empty()).collect();
    let (key, modifiers) = parts.split_last()?;
    let mut out: Vec<String> = modifiers
        .iter()
        .map(|modifier| match *modifier {
            "Ctrl" => "CTRL",
            "Alt" => "ALT",
            "Shift" => "SHIFT",
            "Super" => "LOGO",
            other => other,
        })
        .map(str::to_owned)
        .collect();
    let key = match *key {
        "PrintScreen" => "Print".to_owned(),
        "Space" => "space".to_owned(),
        "Enter" => "Return".to_owned(),
        "Backspace" => "BackSpace".to_owned(),
        "PageUp" => "Prior".to_owned(),
        "PageDown" => "Next".to_owned(),
        "ArrowUp" | "Up" => "Up".to_owned(),
        "ArrowDown" | "Down" => "Down".to_owned(),
        "ArrowLeft" | "Left" => "Left".to_owned(),
        "ArrowRight" | "Right" => "Right".to_owned(),
        single if single.chars().count() == 1 => single.to_lowercase(),
        other => other.to_owned(),
    };
    out.push(key);
    Some(out.join("+"))
}

/// 总开关关闭时不绑定任何快捷键；未设置按键的动作不提交。
pub(crate) fn bindings(settings: &Settings) -> Vec<Binding> {
    if !settings.shortcuts_enabled {
        return Vec::new();
    }
    settings
        .shortcuts
        .entries()
        .into_iter()
        .filter_map(|(action, label, value)| {
            let description =
                crate::shared::native_i18n::for_language(&settings.appearance.language, label);
            Some((action.id(), description.to_owned(), trigger(value)?))
        })
        .collect()
}

/// 设置提交后调用；只把新配置交给后台线程，不阻塞持有设置锁的调用方。
pub(crate) fn sync(settings: &Settings) {
    if let Some(worker) = WORKER.get() {
        if let Ok(sender) = worker.lock() {
            let _ = sender.send(bindings(settings));
        }
    }
}

/// 启动绑定线程；门户不可用时线程直接结束。探测含 D-Bus 往返，放在线程里不拖慢启动。
/// `dispatch` 在主线程执行触发的动作。
pub(crate) fn start(app: AppHandle, initial: &Settings, dispatch: fn(&AppHandle, ShortcutAction)) {
    if !crate::shared::x11::wayland() {
        return;
    }
    let (sender, receiver) = mpsc::channel::<Vec<Binding>>();
    if WORKER.set(Mutex::new(sender)).is_err() {
        return;
    }
    let first = bindings(initial);
    let spawned = std::thread::Builder::new()
        .name("portal-shortcuts".into())
        .spawn(move || run(app, first, receiver, dispatch));
    if let Err(error) = spawned {
        log::warn!("event=settings.portal_shortcuts_failed module=settings error={error}");
    }
}

fn connect() -> Result<Connection, String> {
    zbus::blocking::connection::Builder::session()
        .map(|builder| builder.method_timeout(Duration::from_secs(30)))
        .and_then(|builder| builder.build())
        .map_err(|e| e.to_string())
}

fn run(
    app: AppHandle,
    first: Vec<Binding>,
    receiver: mpsc::Receiver<Vec<Binding>>,
    dispatch: fn(&AppHandle, ShortcutAction),
) {
    if !crate::shared::desktop_support::current()
        .protocols()
        .portal_global_shortcuts
    {
        return;
    }
    let conn = match connect() {
        Ok(conn) => conn,
        Err(error) => {
            log::warn!("event=settings.portal_shortcuts_failed module=settings stage=connect error={error}");
            return;
        }
    };
    let current: std::sync::Arc<Mutex<Option<OwnedObjectPath>>> = Default::default();
    listen(&conn, app, current.clone(), dispatch);
    let mut bound: Option<Vec<Binding>> = None;
    let mut wanted = Some(first);
    loop {
        // 连续多次保存只绑定最后一次的配置，避免桌面反复弹出确认对话框。
        while let Ok(next) = receiver.try_recv() {
            wanted = Some(next);
        }
        if let Some(next) = wanted.take() {
            if bound.as_ref() != Some(&next) {
                if let Some(old) = current.lock().ok().and_then(|mut handle| handle.take()) {
                    close(&conn, &old);
                }
                if !next.is_empty() {
                    match bind(&conn, &next) {
                        Ok(handle) => {
                            if let Ok(mut slot) = current.lock() {
                                *slot = Some(handle);
                            }
                            log::info!(
                                "event=settings.portal_shortcuts_bound module=settings count={}",
                                next.len()
                            );
                        }
                        Err(error) => log::warn!(
                            "event=settings.portal_shortcuts_failed module=settings stage=bind error={error}"
                        ),
                    }
                }
                bound = Some(next);
            }
        }
        match receiver.recv() {
            Ok(next) => wanted = Some(next),
            Err(_) => return,
        }
    }
}

/// 与截图门户相同的请求/响应约定：先订阅 Request 对象的 Response，再发起调用。
fn request(
    conn: &Connection,
    call: impl FnOnce(&str) -> Result<(), String>,
) -> Result<HashMap<String, OwnedValue>, String> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let handle = format!(
        "snapdeskkeys{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let unique = conn
        .unique_name()
        .ok_or("D-Bus 连接没有唯一名称。")?
        .to_string();
    let sender = unique.trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{handle}");
    let proxy = Proxy::new(conn, PORTAL, path, "org.freedesktop.portal.Request")
        .map_err(|e| e.to_string())?;
    let mut signals = proxy
        .receive_signal("Response")
        .map_err(|e| e.to_string())?;
    call(&handle)?;
    // 绑定可能等待用户在桌面对话框中确认；本线程专门处理绑定，可以一直等到回应。
    let message = signals.next().ok_or("快捷键门户没有回应。")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        message.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => Ok(results),
        1 => Err("已取消绑定全局快捷键。".into()),
        code => Err(format!("门户拒绝了请求（{code}）。")),
    }
}

fn bind(conn: &Connection, list: &[Binding]) -> Result<OwnedObjectPath, String> {
    static SESSIONS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let proxy = Proxy::new(conn, PORTAL, PORTAL_PATH, INTERFACE).map_err(|e| e.to_string())?;
    let session_token = format!(
        "snapdeskkeys{}s{}",
        std::process::id(),
        SESSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let mut created = request(conn, |handle| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(handle));
        options.insert("session_handle_token", Value::from(session_token.as_str()));
        proxy
            .call_method("CreateSession", &(options,))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let value = created
        .remove("session_handle")
        .ok_or("快捷键门户没有返回会话。")?;
    let session = match &*value {
        Value::Str(text) => OwnedObjectPath::try_from(text.as_str()).map_err(|e| e.to_string())?,
        Value::ObjectPath(path) => path.clone().into(),
        _ => return Err("快捷键门户返回了无效的会话。".into()),
    };
    let shortcuts: Vec<(&str, HashMap<&str, Value>)> = list
        .iter()
        .map(|(id, description, preferred)| {
            let mut properties: HashMap<&str, Value> = HashMap::new();
            properties.insert("description", Value::from(description.as_str()));
            properties.insert("preferred_trigger", Value::from(preferred.as_str()));
            (*id, properties)
        })
        .collect();
    let result = request(conn, |handle| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(handle));
        proxy
            .call_method("BindShortcuts", &(&*session, shortcuts, "", options))
            .map(drop)
            .map_err(|e| e.to_string())
    });
    if let Err(error) = result {
        close(conn, &session);
        return Err(error);
    }
    Ok(session)
}

fn close(conn: &Connection, session: &OwnedObjectPath) {
    if let Ok(proxy) = Proxy::new(
        conn,
        PORTAL,
        session.as_str(),
        "org.freedesktop.portal.Session",
    ) {
        let _ = proxy.call_method("Close", &());
    }
}

/// 监听 Activated 信号；只响应当前会话，旧会话关闭前的迟到信号被忽略。
fn listen(
    conn: &Connection,
    app: AppHandle,
    current: std::sync::Arc<Mutex<Option<OwnedObjectPath>>>,
    dispatch: fn(&AppHandle, ShortcutAction),
) {
    let signals = Proxy::new(conn, PORTAL, PORTAL_PATH, INTERFACE)
        .and_then(|proxy| proxy.receive_signal("Activated"));
    let mut signals = match signals {
        Ok(signals) => signals,
        Err(error) => {
            log::warn!(
                "event=settings.portal_shortcuts_failed module=settings stage=listen error={error}"
            );
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("portal-shortcut-events".into())
        .spawn(move || {
            while let Some(message) = signals.next() {
                let Ok((session, id, _, _)) =
                    message
                        .body()
                        .deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>(
                        )
                else {
                    continue;
                };
                let active = current
                    .lock()
                    .ok()
                    .is_some_and(|handle| handle.as_ref() == Some(&session));
                let Some(action) = active.then(|| ShortcutAction::from_id(&id)).flatten() else {
                    continue;
                };
                let target = app.clone();
                if app
                    .run_on_main_thread(move || dispatch(&target, action))
                    .is_err()
                {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("event=settings.portal_shortcuts_failed module=settings error={error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_settings_keys_to_portal_triggers() {
        assert_eq!(trigger("F1").as_deref(), Some("F1"));
        assert_eq!(trigger("Ctrl+Shift+A").as_deref(), Some("CTRL+SHIFT+a"));
        assert_eq!(trigger("Super+PrintScreen").as_deref(), Some("LOGO+Print"));
        assert_eq!(trigger("Alt+Space").as_deref(), Some("ALT+space"));
        assert_eq!(trigger("").as_deref(), None);
    }

    #[test]
    fn disabled_or_empty_shortcuts_are_not_bound() {
        let mut settings = Settings::default();
        settings.shortcuts_enabled = false;
        assert!(bindings(&settings).is_empty());
        settings.shortcuts_enabled = true;
        settings.shortcuts.history = String::new();
        let bound = bindings(&settings);
        assert!(bound.iter().all(|(id, _, _)| *id != "history"));
        assert!(bound
            .iter()
            .any(|(id, _, trigger)| *id == "capture" && trigger == "F1"));
    }
}
