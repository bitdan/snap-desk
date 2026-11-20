//! 常驻外壳：托盘菜单、全局快捷键与动作分发，全部运行在 GPUI 主线程。
//!
//! 事件流：托盘、快捷键和单实例转交的回调都在系统线程或消息循环里触发，只做一件事——
//! 把 [`ShellEvent`] 送进无界通道；GPUI 前台任务 `await` 通道并在主线程处理。
//! 这样不需要定时轮询（原型每 50ms 取一次），空闲时进程不被唤醒。
use futures::{StreamExt, channel::mpsc};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::*;
use snap_core::settings::{self, Settings, Shortcut, ShortcutAction};
use std::{collections::HashMap, path::PathBuf};
use tray_icon::{
    MouseButton as TrayButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

/// 外部线程送往主线程的事件。
pub enum ShellEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    HotKey(GlobalHotKeyEvent),
    /// 第二次启动转交的命令行参数。
    Forwarded(Vec<String>),
}

pub type Sender = mpsc::UnboundedSender<ShellEvent>;

/// 托盘菜单项：id 与 Tauri 版一致，文案是词典键（中文原文）。
const ACTIONS: [(&str, &str, ShortcutAction); 5] = [
    ("capture", "截图", ShortcutAction::Capture),
    ("pin", "贴图", ShortcutAction::Pin),
    ("recording", "录屏", ShortcutAction::Record),
    ("history", "剪贴板", ShortcutAction::History),
    ("translation", "翻译", ShortcutAction::Translate),
];
const SHORTCUTS_ID: &str = "shortcuts";
const QUIT_ID: &str = "quit";

/// 主线程独占的外壳状态。托盘与快捷键管理器必须活到进程结束，析构即注销。
struct Shell {
    settings: Settings,
    path: Option<PathBuf>,
    hotkeys: GlobalHotKeyManager,
    /// 当前真正注册成功的按键；与设置中的按键可能不同（被其他程序占用时）。
    registered: Vec<Shortcut>,
    /// 快捷键 id → 动作，由当前设置推导；收到事件时据此分发。
    bindings: HashMap<u32, ShortcutAction>,
    shortcuts_item: CheckMenuItem,
    _tray: TrayIcon,
}
impl Global for Shell {}

/// 在 GPUI 启动回调里调用：创建托盘、注册快捷键，并挂上事件接收任务。
pub fn start(
    cx: &mut App,
    sender: Sender,
    receiver: mpsc::UnboundedReceiver<ShellEvent>,
) -> Result<(), String> {
    // 开发调试用：`SNAP_DESK_SETTINGS` 指向另一份 settings.json，可与已安装的 Tauri 版同时运行而不抢快捷键。
    let path = std::env::var_os("SNAP_DESK_SETTINGS")
        .map(PathBuf::from)
        .or_else(settings::default_path);
    let (settings, warning) = match &path {
        Some(path) => settings::load(path),
        None => (Settings::default(), Some("无法定位设置目录，使用默认设置。".into())),
    };
    if let Some(warning) = warning {
        log::warn!("event=gpui.settings_warning message={warning}");
    }
    log::info!(
        "event=gpui.settings_loaded path={} shortcuts_enabled={}",
        path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        settings.shortcuts_enabled
    );

    // 回调可能在任意线程触发，只转发不处理；通道关闭（主线程退出）时发送失败可忽略。
    let tx = sender.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = tx.unbounded_send(ShellEvent::Menu(event));
    }));
    let tx = sender.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = tx.unbounded_send(ShellEvent::Tray(event));
    }));
    let tx = sender;
    GlobalHotKeyEvent::set_event_handler(Some(move |event| {
        let _ = tx.unbounded_send(ShellEvent::HotKey(event));
    }));

    let language = settings.appearance.language.clone();
    let text = |message: &'static str| snap_core::i18n::for_language(&language, message).to_owned();
    let menu = Menu::new();
    for (id, label, _) in ACTIONS {
        menu.append(&MenuItem::with_id(id, text(label), true, None))
            .map_err(|e| e.to_string())?;
    }
    let shortcuts_item = CheckMenuItem::with_id(
        SHORTCUTS_ID,
        text("启用全局快捷键"),
        true,
        settings.shortcuts_enabled,
        None,
    );
    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &shortcuts_item,
        &MenuItem::with_id(QUIT_ID, text("退出"), true, None),
    ])
    .map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        // 与 Tauri 版一致：左键单击截图，右键弹菜单。
        .with_menu_on_left_click(false)
        .with_tooltip(text("Snap Desk · 截图工具"))
        .with_icon(tray_icon()?)
        .build()
        .map_err(|e| e.to_string())?;

    let hotkeys = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    let mut shell = Shell {
        bindings: bindings(&settings),
        settings,
        path,
        hotkeys,
        registered: Vec::new(),
        shortcuts_item,
        _tray: tray,
    };
    shell.register_startup();
    cx.set_global(shell);

    cx.spawn(async move |cx| {
        let mut receiver = receiver;
        while let Some(event) = receiver.next().await {
            cx.update(|cx| handle(cx, event));
        }
    })
    .detach();
    Ok(())
}

impl Shell {
    /// 启动时与 Tauri 版相同：old 与 next 都是当前设置，已配置但被占用的按键只尽力注册并提示。
    fn register_startup(&mut self) {
        if !self.settings.shortcuts_enabled {
            return;
        }
        let value = self.settings.clone();
        match self.apply(&value, &value, |_| Ok(())) {
            Ok(None) => log::info!(
                "event=gpui.shortcuts_registered count={}",
                self.registered.len()
            ),
            Ok(Some(warning)) => log::warn!("event=gpui.shortcuts_unavailable message={warning}"),
            Err(error) => log::error!("event=gpui.shortcuts_failed message={error}"),
        }
    }

    /// 把 core 的快捷键事务接到 global-hotkey 管理器；失败时 core 负责回滚已注册的按键。
    fn apply(
        &mut self,
        old: &Settings,
        next: &Settings,
        write: impl FnMut(&Settings) -> Result<(), String>,
    ) -> Result<Option<String>, String> {
        let manager = &self.hotkeys;
        settings::apply_settings(
            old,
            next,
            &mut self.registered,
            |key| manager.register(key).map_err(|e| e.to_string()),
            |key| manager.unregister(key).map_err(|e| e.to_string()),
            write,
        )
    }

    /// 托盘“启用全局快捷键”：与 Tauri 版的 `set_shortcuts` 相同，注册结果与磁盘写入是一笔事务。
    /// 设置文件很小，主线程同步写入（含 fsync）只需几毫秒；后续有设置界面时再改为后台写入。
    fn toggle_shortcuts(&mut self) {
        let old = self.settings.clone();
        let mut next = old.clone();
        next.shortcuts_enabled = !old.shortcuts_enabled;
        let path = self.path.clone();
        let result = self.apply(&old, &next, |value| match &path {
            Some(path) => settings::persist(path, value),
            None => Err("无法定位设置目录。".into()),
        });
        match result {
            Ok(warning) => {
                if let Some(warning) = warning {
                    log::warn!("event=gpui.shortcuts_unavailable message={warning}");
                }
                self.settings = next;
                log::info!(
                    "event=gpui.shortcuts_toggled enabled={} registered={}",
                    self.settings.shortcuts_enabled,
                    self.registered.len()
                );
            }
            Err(error) => log::error!("event=gpui.shortcuts_toggle_failed message={error}"),
        }
        // 原生勾选菜单点击时已自行切换外观；以实际生效的设置为准纠正。
        self.shortcuts_item.set_checked(self.settings.shortcuts_enabled);
    }
}

/// 每个已配置按键的 id 对应一个动作；同一按键不会出现两次（设置校验保证无冲突）。
fn bindings(value: &Settings) -> HashMap<u32, ShortcutAction> {
    value
        .shortcuts
        .entries()
        .into_iter()
        .filter_map(|(action, _, key)| {
            let key = settings::parse_shortcut(key).ok().flatten()?;
            Some((key.id(), action))
        })
        .collect()
}

fn handle(cx: &mut App, event: ShellEvent) {
    match event {
        ShellEvent::HotKey(event) => {
            // 只在按下时触发；松开事件忽略，长按不会重复触发（系统只在首次按下发送 Pressed）。
            if event.state != HotKeyState::Pressed {
                return;
            }
            let action = cx.global::<Shell>().bindings.get(&event.id).copied();
            if let Some(action) = action {
                run_action(cx, action, "hotkey");
            }
        }
        ShellEvent::Tray(TrayIconEvent::Click {
            button: TrayButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }) => run_action(cx, ShortcutAction::Capture, "tray"),
        ShellEvent::Tray(_) => {}
        ShellEvent::Menu(event) => match event.id.0.as_str() {
            QUIT_ID => {
                log::info!("event=gpui.quit source=tray");
                cx.quit();
            }
            SHORTCUTS_ID => cx.update_global::<Shell, _>(|shell, _| shell.toggle_shortcuts()),
            id => {
                if let Some((_, _, action)) = ACTIONS.iter().find(|(item, _, _)| *item == id) {
                    run_action(cx, *action, "menu");
                }
            }
        },
        ShellEvent::Forwarded(args) => {
            log::info!("event=gpui.second_instance args={}", args.len());
            if let Some(action) = settings::action_from_args(&args) {
                run_action(cx, action, "cli");
            }
        }
    }
}

/// 托盘、快捷键与命令行共用的动作入口。截图已接入浮层；贴图、录屏等在后续迁移步骤接入，目前只记录触发。
pub fn run_action(cx: &mut App, action: ShortcutAction, source: &str) {
    match action {
        ShortcutAction::Capture => {
            // 截图中再按截图键：与 Tauri 版一致视为取消。
            if crate::capture::active(cx) {
                crate::capture::cancel(cx);
                return;
            }
            log::info!("event=gpui.action action=capture source={source}");
            // 托盘菜单点击后菜单还在淡出，稍等再拍屏，避免把菜单收进截图。
            let delay = if source == "menu" {
                std::time::Duration::from_millis(150)
            } else {
                std::time::Duration::ZERO
            };
            let settings = cx.global::<Shell>().settings.clone();
            crate::capture::start(cx, &settings, delay);
        }
        _ => log::info!(
            "event=gpui.action action={} source={source} status=not_implemented",
            action.id()
        ),
    }
}

/// 与 Tauri 版托盘共用同一张 32×32 图标。
fn tray_icon() -> Result<tray_icon::Icon, String> {
    let image = image::load_from_memory_with_format(
        include_bytes!("../../../src-tauri/icons/32x32.png"),
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?
    .into_rgba8();
    let (width, height) = image.dimensions();
    tray_icon::Icon::from_rgba(image.into_raw(), width, height).map_err(|e| e.to_string())
}
