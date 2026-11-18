//! 桌面会话与功能可用性报告。前端据此在入口处隐藏或标注不可用的功能，而不是点击后才报错。
//! Linux 按桌面实际提供的 Wayland 全局对象与 D-Bus 门户判断，而不是按发行版或桌面名称；
//! 判断逻辑与探测分开，便于在任意平台上测试各种桌面组合。
use serde::Serialize;
use std::sync::OnceLock;

/// 会话类型；`SNAP_DESK_SESSION=x11|wayland` 可覆盖自动识别（环境变量从别的会话泄漏时使用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Session {
    Windows,
    Macos,
    X11,
    Wayland,
}

impl Session {
    fn id(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
        }
    }
}

/// Linux 会话识别：Wayland 会话即使有 `DISPLAY`（Xwayland）也按 Wayland 处理，Xwayland 拿不到 Wayland 画面。
pub(crate) fn linux_session(
    override_value: Option<&str>,
    session_type: Option<&str>,
    wayland_display: bool,
) -> Session {
    match override_value.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("x11") => return Session::X11,
        Some(value) if value.eq_ignore_ascii_case("wayland") => return Session::Wayland,
        _ => {}
    }
    if session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland")) || wayland_display {
        Session::Wayland
    } else {
        Session::X11
    }
}

pub(crate) fn session() -> Session {
    if cfg!(windows) {
        return Session::Windows;
    }
    if cfg!(target_os = "macos") {
        return Session::Macos;
    }
    linux_session(
        std::env::var("SNAP_DESK_SESSION").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}

/// 探测结果：Wayland 全局对象接口名与门户/KWin 接口是否存在。X11、Windows、macOS 下全部为默认值。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Probe {
    /// `XDG_CURRENT_DESKTOP` 等环境变量识别出的合成器。
    pub compositor: Compositor,
    pub globals: Vec<String>,
    pub kwin_screenshot: bool,
    pub portal_screenshot: bool,
    pub portal_screencast: bool,
    pub portal_global_shortcuts: bool,
    pub portal_remote_desktop: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Compositor {
    Kwin,
    Gnome,
    Sway,
    Niri,
    Hyprland,
    /// 其他提供 wlr 协议的合成器（如 river、labwc、Wayfire）。
    Wlroots,
    #[default]
    Other,
}

impl Compositor {
    fn id(self) -> &'static str {
        match self {
            Self::Kwin => "kwin",
            Self::Gnome => "gnome",
            Self::Sway => "sway",
            Self::Niri => "niri",
            Self::Hyprland => "hyprland",
            Self::Wlroots => "wlroots",
            Self::Other => "other",
        }
    }
}

/// `XDG_CURRENT_DESKTOP` 是冒号分隔列表（如 `ubuntu:GNOME`）；合成器专用套接字变量优先。
pub(crate) fn compositor(desktop: &str, sway: bool, niri: bool, hyprland: bool) -> Compositor {
    if niri {
        return Compositor::Niri;
    }
    if hyprland {
        return Compositor::Hyprland;
    }
    if sway {
        return Compositor::Sway;
    }
    for name in desktop.split(':').map(str::trim) {
        match name.to_ascii_lowercase().as_str() {
            "kde" | "plasma" => return Compositor::Kwin,
            "gnome" | "gnome-classic" | "gnome-flashback" => return Compositor::Gnome,
            "sway" => return Compositor::Sway,
            "niri" => return Compositor::Niri,
            "hyprland" => return Compositor::Hyprland,
            "river" | "labwc" | "wayfire" | "wlroots" => return Compositor::Wlroots,
            _ => {}
        }
    }
    Compositor::Other
}

const EXT_IMAGE_COPY: &str = "ext_image_copy_capture_manager_v1";
const EXT_CAPTURE_SOURCE: &str = "ext_output_image_capture_source_manager_v1";
const WLR_SCREENCOPY: &str = "zwlr_screencopy_manager_v1";
const LAYER_SHELL: &str = "zwlr_layer_shell_v1";
const EXT_DATA_CONTROL: &str = "ext_data_control_manager_v1";
const WLR_DATA_CONTROL: &str = "zwlr_data_control_manager_v1";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopProtocols {
    kwin_screenshot: bool,
    ext_image_copy_capture: bool,
    wlr_screencopy: bool,
    layer_shell: bool,
    ext_data_control: bool,
    wlr_data_control: bool,
    portal_screenshot: bool,
    portal_screencast: bool,
    portal_global_shortcuts: bool,
    portal_remote_desktop: bool,
}

/// 单项功能是否可用；不可用时 `reason` 是中文消息键，前端经词典显示。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopFeature {
    available: bool,
    reason: Option<&'static str>,
}

impl DesktopFeature {
    const fn yes() -> Self {
        Self {
            available: true,
            reason: None,
        }
    }
    const fn no(reason: &'static str) -> Self {
        Self {
            available: false,
            reason: Some(reason),
        }
    }
    #[cfg(test)]
    fn available(&self) -> bool {
        self.available
    }
    #[cfg(test)]
    fn reason(&self) -> &'static str {
        self.reason.unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopFeatures {
    screenshot: DesktopFeature,
    screenshot_cursor: DesktopFeature,
    window_snap: DesktopFeature,
    scroll_capture: DesktopFeature,
    recording: DesktopFeature,
    recording_effects: DesktopFeature,
    global_shortcuts: DesktopFeature,
    clipboard_history: DesktopFeature,
    selected_text: DesktopFeature,
    direct_paste: DesktopFeature,
    pin_on_top: DesktopFeature,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSupport {
    session: &'static str,
    /// `XDG_CURRENT_DESKTOP` 原值（截断），仅用于展示与诊断。
    desktop: String,
    compositor: &'static str,
    protocols: DesktopProtocols,
    features: DesktopFeatures,
    /// 在桌面或合成器里绑定快捷键时使用的启动命令（AppImage 为镜像本身的路径）。
    command_line: String,
}

pub(crate) const WAYLAND_PENDING: &str =
    "Wayland 会话暂不支持此功能，请在登录界面选择 X11（Xorg）会话后使用。";
pub(crate) const WAYLAND_SHORTCUTS: &str =
    "Wayland 下应用无法直接注册全局快捷键，请在桌面或合成器的快捷键设置中绑定下方命令。";
const WAYLAND_INPUT: &str = "Wayland 不允许应用监听全局键鼠，录屏键鼠特效不可用。";
const WAYLAND_PASTE: &str =
    "Wayland 不允许应用向其他窗口发送按键，选择条目后只写入剪贴板，请手动粘贴。";
const WAYLAND_SNAP: &str = "当前 Wayland 桌面不提供窗口位置，截图时无法吸附窗口。";
const WAYLAND_ON_TOP: &str = "当前 Wayland 桌面不允许应用置顶窗口，贴图可能被其他窗口遮挡。";
const WAYLAND_CURSOR: &str = "Wayland 截图暂不包含光标。";
const NO_CAPTURE_BACKEND: &str =
    "当前 Wayland 桌面没有可用的截图接口（KWin、ext-image-copy-capture、wlr-screencopy 或屏幕共享门户）。";

pub(crate) fn report(
    session: Session,
    desktop: &str,
    probe: &Probe,
    command_line: String,
) -> DesktopSupport {
    let has = |name: &str| probe.globals.iter().any(|global| global == name);
    let protocols = DesktopProtocols {
        kwin_screenshot: probe.kwin_screenshot,
        ext_image_copy_capture: has(EXT_IMAGE_COPY) && has(EXT_CAPTURE_SOURCE),
        wlr_screencopy: has(WLR_SCREENCOPY),
        layer_shell: has(LAYER_SHELL),
        ext_data_control: has(EXT_DATA_CONTROL),
        wlr_data_control: has(WLR_DATA_CONTROL),
        portal_screenshot: probe.portal_screenshot,
        portal_screencast: probe.portal_screencast,
        portal_global_shortcuts: probe.portal_global_shortcuts,
        portal_remote_desktop: probe.portal_remote_desktop,
    };
    let features = if session == Session::Wayland {
        wayland_features(&protocols)
    } else {
        DesktopFeatures {
            screenshot: DesktopFeature::yes(),
            screenshot_cursor: DesktopFeature::yes(),
            window_snap: DesktopFeature::yes(),
            scroll_capture: DesktopFeature::yes(),
            recording: DesktopFeature::yes(),
            recording_effects: DesktopFeature::yes(),
            global_shortcuts: DesktopFeature::yes(),
            clipboard_history: DesktopFeature::yes(),
            selected_text: DesktopFeature::yes(),
            direct_paste: DesktopFeature::yes(),
            pin_on_top: DesktopFeature::yes(),
        }
    };
    DesktopSupport {
        session: session.id(),
        desktop: desktop.chars().take(64).collect(),
        compositor: if session == Session::Wayland {
            match probe.compositor {
                Compositor::Other if protocols.wlr_screencopy => Compositor::Wlroots.id(),
                other => other.id(),
            }
        } else {
            ""
        },
        protocols,
        features,
        command_line,
    }
}

/// Wayland 各功能尚未接入时统一标注；有截图接口的桌面提示先使用 X11，没有接口的桌面说明原因。
fn wayland_features(protocols: &DesktopProtocols) -> DesktopFeatures {
    let capture_backend = protocols.kwin_screenshot
        || protocols.ext_image_copy_capture
        || protocols.wlr_screencopy
        || protocols.portal_screencast
        || protocols.portal_screenshot;
    let capture = DesktopFeature::no(if capture_backend {
        WAYLAND_PENDING
    } else {
        NO_CAPTURE_BACKEND
    });
    DesktopFeatures {
        screenshot: capture.clone(),
        screenshot_cursor: DesktopFeature::no(WAYLAND_CURSOR),
        window_snap: DesktopFeature::no(WAYLAND_SNAP),
        scroll_capture: capture.clone(),
        recording: capture,
        recording_effects: DesktopFeature::no(WAYLAND_INPUT),
        global_shortcuts: DesktopFeature::no(WAYLAND_SHORTCUTS),
        clipboard_history: DesktopFeature::no(WAYLAND_PENDING),
        selected_text: DesktopFeature::no(WAYLAND_PENDING),
        direct_paste: DesktopFeature::no(WAYLAND_PASTE),
        pin_on_top: if protocols.layer_shell {
            DesktopFeature::yes()
        } else {
            DesktopFeature::no(WAYLAND_ON_TOP)
        },
    }
}

/// 绑定到桌面快捷键的可执行文件：AppImage 必须写镜像路径（`$APPIMAGE`），挂载点每次启动都会变。
pub(crate) fn command_line(
    appimage: Option<std::path::PathBuf>,
    exe: Option<std::path::PathBuf>,
) -> String {
    let path = appimage
        .filter(|path| path.has_root())
        .or(exe)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "snap-desk".into());
    if path
        .chars()
        .any(|c| c.is_whitespace() || "'\"\\$`".contains(c))
    {
        format!("'{}'", path.replace('\'', r"'\''"))
    } else {
        path
    }
}

fn detect() -> DesktopSupport {
    let kind = session();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    #[cfg(target_os = "linux")]
    let found = if kind == Session::Wayland {
        super::wayland_probe::probe(&desktop)
    } else {
        Probe::default()
    };
    #[cfg(not(target_os = "linux"))]
    let found = Probe::default();
    let support = report(
        kind,
        &desktop,
        &found,
        command_line(
            std::env::var_os("APPIMAGE").map(Into::into),
            std::env::current_exe().ok(),
        ),
    );
    log::info!(
        "event=app.desktop_support module=app session={} compositor={} globals={} kwin={} portal_screenshot={} portal_screencast={} portal_shortcuts={}",
        support.session,
        support.compositor,
        found.globals.len(),
        found.kwin_screenshot,
        found.portal_screenshot,
        found.portal_screencast,
        found.portal_global_shortcuts,
    );
    support
}

/// 首次调用时探测并缓存；探测含 D-Bus 往返，调用方应在阻塞线程中调用（启动时已在后台预热）。
pub(crate) fn current() -> &'static DesktopSupport {
    static SUPPORT: OnceLock<DesktopSupport> = OnceLock::new();
    SUPPORT.get_or_init(detect)
}

/// 启动时在后台预热，主窗口首次读取状态时无需等待 D-Bus。
pub(crate) fn warm_up() {
    if let Err(error) = std::thread::Builder::new()
        .name("desktop-support".into())
        .spawn(|| {
            current();
        })
    {
        log::warn!("event=app.desktop_support_failed module=app error={error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(compositor: Compositor, globals: &[&str]) -> Probe {
        Probe {
            compositor,
            globals: globals.iter().map(|g| g.to_string()).collect(),
            ..Probe::default()
        }
    }

    #[test]
    fn wayland_wins_over_xwayland_and_override_is_respected() {
        assert_eq!(
            linux_session(None, Some("wayland"), false),
            Session::Wayland
        );
        assert_eq!(linux_session(None, Some("x11"), true), Session::Wayland);
        assert_eq!(linux_session(None, Some("x11"), false), Session::X11);
        assert_eq!(linux_session(None, None, false), Session::X11);
        assert_eq!(
            linux_session(Some(" X11 "), Some("wayland"), true),
            Session::X11
        );
        assert_eq!(
            linux_session(Some("wayland"), Some("x11"), false),
            Session::Wayland
        );
        assert_eq!(
            linux_session(Some("auto"), Some("x11"), false),
            Session::X11
        );
    }

    #[test]
    fn compositor_prefers_socket_variables_then_desktop_names() {
        assert_eq!(
            compositor("ubuntu:GNOME", false, false, false),
            Compositor::Gnome
        );
        assert_eq!(compositor("KDE", false, false, false), Compositor::Kwin);
        assert_eq!(compositor("", true, false, false), Compositor::Sway);
        assert_eq!(compositor("GNOME", false, true, false), Compositor::Niri);
        assert_eq!(
            compositor("Hyprland", false, false, false),
            Compositor::Hyprland
        );
        assert_eq!(
            compositor("labwc:wlroots", false, false, false),
            Compositor::Wlroots
        );
        assert_eq!(compositor("XFCE", false, false, false), Compositor::Other);
    }

    #[test]
    fn x11_and_native_sessions_report_everything_available() {
        let support = report(Session::X11, "XFCE", &Probe::default(), "snap-desk".into());
        let value = serde_json::to_value(&support).unwrap();
        assert_eq!(value["session"], "x11");
        assert_eq!(value["compositor"], "");
        for (_, feature) in value["features"].as_object().unwrap() {
            assert_eq!(feature["available"], true);
            assert!(feature["reason"].is_null());
        }
    }

    #[test]
    fn wayland_reports_protocols_and_labels_features_up_front() {
        let globals = [
            EXT_IMAGE_COPY,
            EXT_CAPTURE_SOURCE,
            WLR_SCREENCOPY,
            LAYER_SHELL,
            EXT_DATA_CONTROL,
        ];
        let support = report(
            Session::Wayland,
            "niri",
            &probe(Compositor::Niri, &globals),
            "snap-desk".into(),
        );
        assert_eq!(support.compositor, "niri");
        assert!(support.protocols.ext_image_copy_capture && support.protocols.wlr_screencopy);
        assert!(support.protocols.layer_shell && support.protocols.ext_data_control);
        assert!(!support.protocols.wlr_data_control);
        assert_eq!(support.features.screenshot.reason(), WAYLAND_PENDING);
        assert_eq!(
            support.features.global_shortcuts.reason(),
            WAYLAND_SHORTCUTS
        );
        assert!(support.features.pin_on_top.available());
        assert!(!support.features.recording_effects.available());

        // 只有源管理器而没有拷贝管理器时不算可用。
        let partial = report(
            Session::Wayland,
            "",
            &probe(Compositor::Other, &[EXT_CAPTURE_SOURCE]),
            "x".into(),
        );
        assert!(!partial.protocols.ext_image_copy_capture);
        assert_eq!(partial.features.screenshot.reason(), NO_CAPTURE_BACKEND);
        assert!(!partial.features.pin_on_top.available());

        // 未知合成器提供 wlr-screencopy 时归为 wlroots 系。
        let wlr = report(
            Session::Wayland,
            "",
            &probe(Compositor::Other, &[WLR_SCREENCOPY]),
            "x".into(),
        );
        assert_eq!(wlr.compositor, "wlroots");

        let gnome = Probe {
            compositor: Compositor::Gnome,
            portal_screencast: true,
            ..Probe::default()
        };
        let gnome = report(Session::Wayland, "GNOME", &gnome, "x".into());
        assert_eq!(gnome.features.screenshot.reason(), WAYLAND_PENDING);
    }

    #[test]
    fn command_line_prefers_appimage_and_quotes_for_shells() {
        use std::path::PathBuf;
        let exe = Some(PathBuf::from("/tmp/.mount_SnapXYZ/usr/bin/snap-desk"));
        assert_eq!(
            command_line(
                Some(PathBuf::from("/home/u/Apps/Snap.AppImage")),
                exe.clone()
            ),
            "/home/u/Apps/Snap.AppImage"
        );
        assert_eq!(
            command_line(Some(PathBuf::from("relative.AppImage")), exe.clone()),
            "/tmp/.mount_SnapXYZ/usr/bin/snap-desk"
        );
        assert_eq!(
            command_line(Some(PathBuf::from("/home/u/My Apps/it's.AppImage")), None),
            r"'/home/u/My Apps/it'\''s.AppImage'"
        );
        assert_eq!(command_line(None, None), "snap-desk");
    }

    #[test]
    fn user_facing_reasons_have_english_entries() {
        let catalog: std::collections::HashMap<String, String> = serde_json::from_str(
            include_str!("../../../src/shared/i18n/messages.native.en.json"),
        )
        .unwrap();
        for message in [
            WAYLAND_PENDING,
            WAYLAND_SHORTCUTS,
            WAYLAND_INPUT,
            WAYLAND_PASTE,
            WAYLAND_SNAP,
            WAYLAND_ON_TOP,
            WAYLAND_CURSOR,
            NO_CAPTURE_BACKEND,
        ] {
            assert!(
                catalog.contains_key(message),
                "missing English entry: {message}"
            );
        }
    }
}
