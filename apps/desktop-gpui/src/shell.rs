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
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem},
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

/// 托盘菜单项：id 文案是词典键（中文原文）。
const ACTIONS: [(&str, &str, ShortcutAction); 5] = [
    ("capture", "截图", ShortcutAction::Capture),
    ("pin", "贴图", ShortcutAction::Pin),
    ("recording", "录屏", ShortcutAction::Record),
    ("history", "剪贴板", ShortcutAction::History),
    ("translation", "翻译", ShortcutAction::Translate),
];
const SHORTCUTS_ID: &str = "shortcuts";
const SETTINGS_ID: &str = "settings";
const RESTART_ID: &str = "restart";
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
    /// 设置界面正在录入快捷键：已注销全部全局快捷键，按键直接送到设置窗口。
    suspended: bool,
    _tray: TrayIcon,
}
impl Global for Shell {}

/// 在 GPUI 启动回调里调用：创建托盘、注册快捷键，并挂上事件接收任务。
pub fn start(
    cx: &mut App,
    sender: Sender,
    receiver: mpsc::UnboundedReceiver<ShellEvent>,
) -> Result<(), String> {
    let path = settings_path();
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
    // 托盘菜单：五个动作、打开主界面（设置窗口）、全局快捷键开关、重启、退出。
    menu.append_items(&[
        &MenuItem::with_id(SETTINGS_ID, text("打开 Snap Desk"), true, None),
        &shortcuts_item,
        &MenuItem::with_id(RESTART_ID, text("重启"), true, None),
        &MenuItem::with_id(QUIT_ID, text("退出"), true, None),
    ])
    .map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        // 左键单击截图，右键弹菜单。
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
        suspended: false,
        _tray: tray,
    };
    shell.register_startup();
    let clipboard = shell.settings.clipboard.clone();
    cx.set_global(shell);
    // 按主题设置与系统外观选定调色板（窗口尚未打开，不需要重绘）。
    crate::theme::refresh(cx);
    // 剪贴板历史记录服务（按设置启用）；历史窗口由托盘“剪贴板”或快捷键打开。
    crate::clipboard::init(cx, &clipboard);
    crate::shelf::init(cx);
    crate::local_translate::init(cx);

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
    /// 启动时 old 与 next 都是当前设置，已配置但被占用的按键只尽力注册并提示。
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

    /// 以 `next` 替换当前设置并写盘；快捷键注册与写盘为一笔事务。录入快捷键期间（已注销）只写盘。
    fn commit(&mut self, path: &std::path::Path, next: Settings) -> Result<Settings, String> {
        let old = self.settings.clone();
        if self.suspended {
            settings::persist(path, &next)?;
        } else {
            let warning = self.apply(&old, &next, |value| settings::persist(path, value))?;
            if let Some(warning) = warning {
                log::warn!("event=gpui.shortcuts_unavailable message={warning}");
            }
        }
        self.bindings = bindings(&next);
        self.settings = next;
        self.shortcuts_item.set_checked(self.settings.shortcuts_enabled);
        log::info!("event=gpui.settings_saved registered={}", self.registered.len());
        Ok(self.settings.clone())
    }

    /// 托盘“启用全局快捷键”：注册结果与磁盘写入是一笔事务。
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
                // 快捷键兼容测试中只记录按键，不执行动作；截图、录屏、滚动截图进行中立即让出热键。
                let busy = crate::capture::active(cx) || crate::recording::active(cx) || crate::scroll::active(cx);
                if !busy {
                    let value = cx.global::<Shell>().settings.shortcuts.entries().into_iter()
                        .find(|(a, _, _)| *a == action)
                        .map(|(_, _, value)| value.to_owned());
                    if let Some(value) = value {
                        if crate::settings_window::probe::receive(value) {
                            return;
                        }
                    }
                }
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
            QUIT_ID => quit(cx, "tray"),
            RESTART_ID => restart(cx),
            SHORTCUTS_ID => cx.update_global::<Shell, _>(|shell, _| shell.toggle_shortcuts()),
            SETTINGS_ID => crate::settings_window::open(cx),
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

/// 设置文件路径。开发调试用：`SNAP_DESK_SETTINGS` 指向另一份 settings.json，可与已安装的版本
/// 同时运行而不抢快捷键。
fn settings_path() -> Option<PathBuf> {
    std::env::var_os("SNAP_DESK_SETTINGS")
        .map(PathBuf::from)
        .or_else(settings::default_path)
}

/// 设置文件所在目录：GPUI 版自己的小偏好（如剪贴板分类顺序）放在这里，随 `SNAP_DESK_SETTINGS` 一起隔离。
pub fn settings_dir() -> Option<PathBuf> {
    settings_path().and_then(|path| path.parent().map(|p| p.to_path_buf()))
}

/// 修改并保存剪贴板设置（只改剪贴板部分，见 [`update_settings`]）。返回保存后的剪贴板设置。
/// 调用方负责按新设置重启记录服务（历史窗口自行在后台执行）。
pub fn update_clipboard(
    cx: &mut App,
    change: impl FnOnce(&mut snap_core::clipboard::model::ClipboardSettings),
) -> Result<snap_core::clipboard::model::ClipboardSettings, String> {
    let saved = save(cx, |s| change(&mut s.clipboard))?;
    log::info!(
        "event=gpui.clipboard_settings_saved enabled={} paste_on_activate={}",
        saved.clipboard.enabled,
        saved.clipboard.paste_on_activate
    );
    Ok(saved.clipboard)
}

/// 修改并保存设置。先从磁盘重新读取再只改 `change` 涉及的部分：开发时另一个实例可能同时写入同一文件，
/// 整份覆盖会丢掉它在本进程启动后保存的其他设置。快捷键变化与写盘是一笔事务（注册失败时回滚、不写盘）；
/// 剪贴板设置变化时在后台按新设置重启记录服务。返回保存后的设置。
pub fn update_settings(cx: &mut App, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let old = cx.global::<Shell>().settings.clone();
    let saved = save(cx, change)?;
    apply_theme(cx);
    if saved.clipboard != old.clipboard {
        if let Some(service) = crate::clipboard::service(cx) {
            let clipboard = saved.clipboard.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = service.configure(clipboard) {
                        log::warn!("event=gpui.clipboard_configure_failed message={error}");
                        service.report(error);
                    }
                })
                .detach();
        }
    }
    Ok(saved)
}

fn save(cx: &mut App, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let path = cx.global::<Shell>().path.clone().ok_or("无法定位设置目录。")?;
    let (mut fresh, warning) = settings::load(&path);
    if let Some(warning) = warning {
        log::warn!("event=gpui.settings_warning message={warning}");
    }
    change(&mut fresh);
    fresh.validate()?;
    cx.update_global::<Shell, _>(|shell, _| shell.commit(&path, fresh))
}

/// 导入完整配置备份：先校验备份中的目录；快照设置文件与两个
/// 目录记录；快捷键注册、目录记录与设置写盘为一笔事务，任一步失败恢复快照与运行时目录；成功后按需重启
/// 剪贴板记录服务并写回备份中的密钥。截图、录屏、滚动截图或快捷键录入进行中时拒绝。
pub fn import_backup(cx: &mut App, mut backup: snap_core::settings::backup::Backup) -> Result<Settings, String> {
    use snap_core::{recording::output, settings::backup};
    if crate::capture::active(cx) || crate::recording::active(cx) || crate::scroll::active(cx) {
        return Err("请先完成或取消截图、录屏，再修改设置。".into());
    }
    if cx.global::<Shell>().suspended {
        return Err("请先结束快捷键录入。".into());
    }
    if let Some(path) = &backup.directories.captures {
        snap_core::export::validate_directory(path)?;
    }
    if let Some(path) = &backup.directories.recordings {
        output::validate_directory(path)?;
    }
    let settings_path = cx.global::<Shell>().path.clone().ok_or("无法定位设置目录。")?;
    let capture_record = snap_core::export::directory_record().ok_or("无法定位配置目录。")?;
    let recording_record = recording_record().ok_or("无法定位配置目录。")?;
    let old = cx.global::<Shell>().settings.clone();
    let old_recording = output::configured_directory(&recording_record, old.recording_export.remember_directory)?;
    let files = backup::Files::snapshot_paths([settings_path.clone(), capture_record.clone(), recording_record.clone()])?;
    let secrets = std::mem::take(&mut backup.secrets);
    let directories = backup.directories;
    let mut next = backup.settings;
    next.onboarding_version = next.onboarding_version.max(old.onboarding_version);
    let result = cx.update_global::<Shell, _>(|shell, _| {
        let target = next.clone();
        let warning = shell.apply(&old, &next, |value| {
            if value == &target {
                snap_core::export::configure_directory_record(&capture_record, directories.captures.as_deref())?;
                output::configure_directory(
                    &recording_record,
                    directories.recordings.as_deref(),
                    target.recording_export.remember_directory,
                )?;
                settings::persist(&settings_path, value)
            } else {
                files.restore()?;
                output::restore_session_directory(old_recording.clone())
            }
        });
        match warning {
            Ok(warning) => {
                if let Some(warning) = warning {
                    log::warn!("event=gpui.shortcuts_unavailable message={warning}");
                }
                shell.bindings = bindings(&next);
                shell.settings = next.clone();
                shell.shortcuts_item.set_checked(next.shortcuts_enabled);
                Ok(())
            }
            Err(error) => {
                let mut errors = Vec::new();
                if let Err(error) = files.restore() {
                    errors.push(error);
                }
                if let Err(error) = output::restore_session_directory(old_recording.clone()) {
                    errors.push(error);
                }
                Err(backup::with_rollback(error, errors))
            }
        }
    });
    result?;
    if next.clipboard != old.clipboard {
        if let Some(service) = crate::clipboard::service(cx) {
            let clipboard = next.clipboard.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = service.configure(clipboard) {
                        service.report(error);
                    }
                })
                .detach();
        }
    }
    apply_theme(cx);
    log::info!("event=gpui.settings_imported schema_version={}", backup::VERSION);
    backup::restore_secrets(&next, secrets)?;
    Ok(next)
}

/// 主题改变时重绘全部窗口（截图工具栏、剪贴板历史、设置窗口等读取同一调色板）。
pub fn apply_theme(cx: &mut App) {
    if crate::theme::refresh(cx) {
        log::info!("event=gpui.theme_changed");
        cx.refresh_windows();
    }
}

/// 录屏保存目录记录（与旧版相同）。
pub fn recording_record() -> Option<PathBuf> {
    snap_core::paths::config_dir().map(|dir| dir.join("recording-directory.json"))
}

/// 导出配置时不能覆盖的正在使用的配置文件。
pub fn active_config_files(cx: &App) -> Vec<PathBuf> {
    [cx.global::<Shell>().path.clone(), snap_core::export::directory_record(), recording_record()]
        .into_iter()
        .flatten()
        .collect()
}

/// 设置界面录入快捷键期间注销全部全局快捷键（否则按下的组合会被系统热键吞掉或直接触发动作）；
/// 结束录入后按当前设置重新注册，被占用的按键只记日志。
pub fn suspend_shortcuts(cx: &mut App, suspend: bool) {
    cx.update_global::<Shell, _>(|shell, _| {
        if shell.suspended == suspend {
            return;
        }
        shell.suspended = suspend;
        if suspend {
            for key in &shell.registered {
                if let Err(error) = shell.hotkeys.unregister(*key) {
                    log::warn!("event=gpui.shortcut_suspend_failed message={error}");
                }
            }
            log::info!("event=gpui.shortcuts_suspended count={}", shell.registered.len());
        } else {
            let keys = std::mem::take(&mut shell.registered);
            for key in keys {
                match shell.hotkeys.register(key) {
                    Ok(()) => shell.registered.push(key),
                    Err(error) => log::warn!("event=gpui.shortcut_resume_failed message={error}"),
                }
            }
            log::info!("event=gpui.shortcuts_resumed count={}", shell.registered.len());
        }
    });
}

/// 快捷键兼容测试的注册快照：已注册的按键（设置中的写法）与被占用的按键。
pub fn probe_registration(cx: &App) -> snap_core::settings::shortcut_probe::Registration {
    let shell = cx.global::<Shell>();
    let mut registered = Vec::new();
    let mut occupied = Vec::new();
    for (_, _, value) in shell.settings.shortcuts.entries() {
        let Some(key) = settings::parse_shortcut(value).ok().flatten() else {
            continue;
        };
        if shell.registered.contains(&key) {
            registered.push(value.to_owned());
        } else if shell.settings.shortcuts_enabled && !shell.suspended {
            occupied.push(value.to_owned());
        }
    }
    snap_core::settings::shortcut_probe::Registration {
        enabled: shell.settings.shortcuts_enabled,
        editing: shell.suspended,
        error: (!occupied.is_empty()).then(|| occupied.join("、")),
        registered,
    }
}

/// 被系统或其他程序占用、暂未生效的动作（设置界面标注“已被占用”）。
pub fn unavailable_shortcuts(cx: &App) -> Vec<ShortcutAction> {
    let shell = cx.global::<Shell>();
    if !shell.settings.shortcuts_enabled || shell.suspended {
        return Vec::new();
    }
    shell
        .settings
        .shortcuts
        .entries()
        .into_iter()
        .filter_map(|(action, _, value)| {
            let key = settings::parse_shortcut(value).ok().flatten()?;
            (!shell.registered.contains(&key)).then_some(action)
        })
        .collect()
}

/// 当前设置快照（截图、贴图读取外观与导出偏好）。
pub fn settings(cx: &App) -> Settings {
    cx.global::<Shell>().settings.clone()
}

/// 托盘、快捷键与命令行共用的动作入口。
pub fn run_action(cx: &mut App, action: ShortcutAction, source: &str) {
    // 托盘或菜单发起的动作结束进行中的快捷键测试（测试中的热键已在上面被记录并拦下）。
    crate::settings_window::probe::interrupt();
    match action {
        ShortcutAction::Capture => {
            // 滚动截图采集中按截图键：结束采集并输出。
            if crate::scroll::stop(cx) {
                log::info!("event=gpui.action action=scroll_stop source={source}");
                return;
            }
            // 截图中再按截图键视为取消。
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
            crate::capture::start(cx, &settings, delay, None);
        }
        ShortcutAction::Ocr => {
            // 截图识字：框选完成即识别；截图中再按视为取消，与截图键一致。
            if crate::capture::active(cx) {
                crate::capture::cancel(cx);
                return;
            }
            log::info!("event=gpui.action action=ocr source={source}");
            let delay = if source == "menu" {
                std::time::Duration::from_millis(150)
            } else {
                std::time::Duration::ZERO
            };
            // 识字不叠加普通截图延时。
            let mut settings = cx.global::<Shell>().settings.clone();
            settings.delay_seconds = 0;
            crate::capture::start(cx, &settings, delay, Some(crate::recognition::Mode::Ocr));
        }
        ShortcutAction::Pin => {
            log::info!("event=gpui.action action=pin source={source}");
            // 截图中贴当前选区；否则先恢复穿透中的贴图，没有时才贴剪贴板。
            if crate::capture::active(cx) {
                crate::capture::pin(cx);
            } else if !crate::pins::restore_click_through(cx) {
                crate::pins::from_clipboard(cx);
            }
        }
        ShortcutAction::Translate => {
            log::info!("event=gpui.action action=translate source={source}");
            crate::recognition::translate_selection(cx);
        }
        ShortcutAction::Shelf => {
            log::info!("event=gpui.action action=shelf source={source}");
            crate::shelf::toggle(cx);
        }
        ShortcutAction::History => {
            log::info!("event=gpui.action action=history source={source}");
            crate::clipboard::open(cx);
        }
        ShortcutAction::Record => {
            log::info!("event=gpui.action action=record source={source}");
            crate::recording::shortcut(cx);
        }
    }
}

/// 托盘图标（32×32，与安装包同一张）。
fn tray_icon() -> Result<tray_icon::Icon, String> {
    let image = image::load_from_memory_with_format(
        include_bytes!("../../../resources/icons/32x32.png"),
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?
    .into_rgba8();
    let (width, height) = image.dimensions();
    tray_icon::Icon::from_rgba(image.into_raw(), width, height).map_err(|e| e.to_string())
}

/// 退出前的资源收尾（本地翻译进程、剪贴板监听）后结束事件循环。
fn finish(cx: &mut App) {
    crate::local_translate::shutdown();
    crate::clipboard::shutdown(cx);
    cx.quit();
}

/// 托盘“退出”与应用内更新：录屏中先停止并等视频保存完成再退出。
pub fn quit(cx: &mut App, source: &str) {
    log::info!("event=gpui.quit source={source}");
    crate::recording::quit_when_idle(cx, finish);
}

/// 托盘“重启”：与退出同一收尾（录屏先保存），再拉起新进程；新进程等本进程结束后再取得单实例锁，
/// 否则会被当作第二个实例把参数转交回来。
pub fn restart(cx: &mut App) {
    log::info!("event=gpui.restart source=tray");
    crate::recording::quit_when_idle(cx, |cx| {
        // AppImage 要重新运行镜像本身，挂载点里的程序路径在退出后失效。
        match crate::updater::appimage().map(Ok).unwrap_or_else(std::env::current_exe) {
            Ok(exe) => {
                if let Err(error) = std::process::Command::new(exe)
                    .arg(format!("{RELAUNCH_ARG}{}", std::process::id()))
                    .spawn()
                {
                    log::error!("event=gpui.restart_failed message={error}");
                }
            }
            Err(error) => log::error!("event=gpui.restart_failed message={error}"),
        }
        finish(cx);
    });
}

/// 重启时传给新进程的参数前缀，后接旧进程 PID。
pub const RELAUNCH_ARG: &str = "--relaunch-after=";
