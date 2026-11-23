//! 用户主动发起的五秒快捷键测试：复用现有系统注册，不安装钩子或记录普通按键。
//! 测试期间只统计已配置动作，避免测试 F1/F3 时意外截图或停止录屏。
//!
//! 外壳负责：在已注册快捷键的按下回调里先调用 [`ProbeState::receive`]（返回 true 时不执行动作）、
//! 提供注册快照、截图开始时中断测试，并在主线程读取 macOS 输入法标识。
use serde::Serialize;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct ProbeState(Mutex<Probe>);
#[derive(Default)]
struct Probe {
    deadline: Option<Instant>,
    received: Vec<Receipt>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub shortcut: String,
    pub foreground_pid: u32,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Foreground {
    pub pid: u32,
    pub elevated: Option<bool>,
    pub covers_monitor: Option<bool>,
    pub keyboard_layout: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    /// 与 `std::env::consts::OS` 一致，前端据此选择适用的排查建议。
    pub platform: &'static str,
    /// 仅 macOS：测试期间是否有应用开启了“安全输入”（密码框、终端的安全键盘输入等）。
    pub secure_input: Option<bool>,
    pub enabled: bool,
    pub editing: bool,
    pub registration_error: Option<String>,
    pub registered: Vec<String>,
    pub received: Vec<Receipt>,
    pub app_elevated: Option<bool>,
    pub target: Option<Foreground>,
}

/// 外壳的快捷键注册快照。
#[derive(Clone, Debug, Default)]
pub struct Registration {
    pub enabled: bool,
    /// 正在录入快捷键（系统注册已临时释放）。
    pub editing: bool,
    pub error: Option<String>,
    pub registered: Vec<String>,
}

impl ProbeReport {
    /// 测试结束后用最新注册状态覆盖（测试期间用户可能修改了设置）。
    pub fn set_registration(&mut self, registration: Registration) {
        self.enabled = registration.enabled;
        self.editing = registration.editing;
        self.registration_error = registration.error;
        self.registered = registration.registered;
    }
}

impl Probe {
    fn active(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now < deadline)
    }
    /// 自动到期是额外保险：即使测试线程异常退出，也不会永久吞掉功能快捷键。
    fn receive(&mut self, shortcut: String, pid: u32, now: Instant) -> bool {
        if !self.active(now) {
            return false;
        }
        if self.received.len() < 32 {
            self.received.push(Receipt {
                shortcut,
                foreground_pid: pid,
            });
        }
        true
    }
    /// 只清理自己拥有的测试；旧后台任务迟到时保留新一轮的接收记录。
    fn finish(&mut self, deadline: Instant) -> Result<Vec<Receipt>, String> {
        if self.deadline != Some(deadline) {
            return Err("此轮测试已被新的测试替代。".into());
        }
        self.deadline = None;
        Ok(std::mem::take(&mut self.received))
    }
}

impl ProbeState {
    /// 仅由已有全局快捷键的按下回调调用；测试中返回 true，外壳不执行功能动作。
    /// 截图进行中时外壳不应调用（立即让出热键，尤其不能吞掉停止录屏/滚动截图的动作）。
    pub fn receive(&self, shortcut: String) -> bool {
        self.0
            .lock()
            .map(|mut probe| {
                let now = Instant::now();
                if !probe.active(now) {
                    return false;
                }
                probe.receive(shortcut, foreground_pid(), now)
            })
            .unwrap_or(false)
    }

    /// 阻塞最长五秒，等待用户切换到目标软件并按键；在后台线程调用。
    /// `interrupted` 在截图或录屏开始时返回 true；`input_source` 读取 macOS 输入法标识（需在主线程，其他平台不调用）。
    pub fn run(
        &self,
        registration: Registration,
        interrupted: &dyn Fn() -> bool,
        input_source: &dyn Fn() -> Option<String>,
    ) -> Result<ProbeReport, String> {
        if !cfg!(any(windows, target_os = "macos")) {
            return Err("此环境测试仅支持 Windows 和 macOS。".into());
        }
        let mut report = ProbeReport {
            platform: std::env::consts::OS,
            secure_input: None,
            enabled: false,
            editing: false,
            registration_error: None,
            registered: vec![],
            received: vec![],
            app_elevated: elevation(std::process::id()),
            target: None,
        };
        report.set_registration(registration);
        // 关闭/录入/注册失败直接报告，不把无法测试误报成目标软件拦截。
        if !report.enabled
            || report.editing
            || report.registered.is_empty()
            || report.registration_error.is_some()
        {
            return Ok(report);
        }
        // 最长等待五秒；在外部窗口收到第一次按键后立即结束，不必等满时长。
        let deadline = Instant::now() + Duration::from_secs(5);
        {
            let mut probe = self.0.lock().map_err(|e| e.to_string())?;
            if probe.active(Instant::now()) {
                return Err("快捷键测试正在进行。".into());
            }
            probe.received.clear();
            probe.deadline = Some(deadline);
        }
        let mut stopped = false;
        while Instant::now() < deadline {
            if interrupted() {
                stopped = true;
                break;
            }
            if let Some(target) = foreground(input_source) {
                // 返回设置页不会覆盖刚测试的外部窗口；不读取窗口标题和文档内容。
                if target.pid != std::process::id() {
                    report.target = Some(target);
                    // 安全输入通常由前台应用开启，只在外部应用位于前台时采样；出现过一次即记为开启。
                    if let Some(secure) = secure_input() {
                        report.secure_input = Some(report.secure_input.unwrap_or(false) || secure);
                    }
                }
            }
            let received_external = self
                .0
                .lock()
                .map(|probe| {
                    probe
                        .received
                        .iter()
                        .any(|receipt| receipt.foreground_pid != std::process::id())
                })
                .unwrap_or(false);
            if received_external {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let mut probe = self.0.lock().map_err(|e| e.to_string())?;
        // 旧线程可能在到期后被调度挂起；不得清空随后发起的新测试。
        report.received = probe.finish(deadline)?;
        drop(probe);
        if stopped {
            return Err("测试已因截图或录屏开始而结束，功能快捷键继续正常工作。".into());
        }
        Ok(report)
    }
}

/// 排查建议：观测事实与建议分开，满屏几何信息不能证明独占，权限差异也不能证明拦截。
/// 每条为词典模板（`{0}`、`{1}` 占位）与参数。
pub fn messages(report: &ProbeReport) -> Vec<(&'static str, Vec<String>)> {
    let mut messages: Vec<(&'static str, Vec<String>)> = Vec::new();
    if !report.enabled {
        messages.push(("全局快捷键已禁用，请先启用后重新测试。", vec![]));
    }
    if report.editing {
        messages.push((
            "正在录入快捷键，系统注册已临时释放；结束录入后重试。",
            vec![],
        ));
    }
    if let Some(error) = &report.registration_error {
        messages.push((
            "系统注册失败：{0}。关闭占用此组合的软件或更换组合，再保存快捷键。",
            vec![error.clone()],
        ));
    }
    if report.registered.is_empty() {
        messages.push(("当前没有已注册的快捷键；请设置至少一个动作并保存。", vec![]));
    }
    if !messages.is_empty() {
        return messages;
    }
    messages.push(("已注册：{0}。", vec![report.registered.join("、")]));
    let Some(target) = &report.target else {
        messages.push((
            "未观察到外部前台窗口；重新测试并在五秒内切换到目标软件按键。",
            vec![],
        ));
        return messages;
    };
    let mut received: Vec<&str> = Vec::new();
    for item in report
        .received
        .iter()
        .filter(|item| item.foreground_pid == target.pid)
    {
        if !received.contains(&item.shortcut.as_str()) {
            received.push(&item.shortcut);
        }
    }
    if received.is_empty() {
        messages.push((
            "目标进程 PID {0} 在前台时未收到快捷键；本次观测无法确定它是否拦截了按键。请确认使用已保存组合并重新测试。",
            vec![target.pid.to_string()],
        ));
    } else {
        messages.push((
            "目标进程 PID {0} 在前台时已收到：{1}。若实际动作仍失败，请查看日志中的截图或录屏错误。",
            vec![target.pid.to_string(), received.join("、")],
        ));
    }
    if report
        .received
        .iter()
        .any(|item| item.foreground_pid != target.pid)
    {
        messages.push((
            "其他前台窗口收到过热键；这不代表目标软件内也能触发。",
            vec![],
        ));
    }
    let macos = report.platform == "macos";
    if macos {
        // macOS 没有管理员提升的区别；常见原因是安全输入和 F 键被当作媒体键。
        if report.secure_input == Some(true) {
            messages.push(("测试期间有应用开启了安全输入（如密码框或终端的“安全键盘输入”），系统可能因此不分发快捷键；离开密码框或关闭该选项后重试。", vec![]));
        }
        let function_key = |key: &str| {
            let last = key.rsplit('+').next().unwrap_or(key);
            last.len() > 1
                && last[..1].eq_ignore_ascii_case("f")
                && last[1..].chars().all(|c| c.is_ascii_digit())
        };
        if received.is_empty() && report.registered.iter().any(|key| function_key(key)) {
            messages.push(("Mac 键盘上的 F 键默认控制亮度、音量等，需要同时按 Fn；也可以在“系统设置 → 键盘”中打开“将 F1、F2 等键用作标准功能键”。", vec![]));
        }
    } else {
        match (target.elevated, report.app_elevated) {
            (None, _) | (_, None) => messages.push((
                "进程权限读取失败，无法比较权限级别；受保护进程可能禁止查询。",
                vec![],
            )),
            (Some(a), Some(b)) if a != b => messages.push((
                "目标软件与 Snap Desk 的管理员提升状态不同；可先将两者以普通权限运行后对比，勿为此关闭系统安全保护。",
                vec![],
            )),
            _ => messages.push((
                "目标软件与 Snap Desk 的管理员提升状态相同；此检查不代表所有进程安全属性一致。",
                vec![],
            )),
        }
    }
    match target.covers_monitor {
        Some(true) if macos => messages.push((
            "目标窗口覆盖显示器（可能处于全屏空间）；尝试退出全屏后对比。",
            vec![],
        )),
        Some(true) => messages.push((
            "目标窗口覆盖显示器，但无法据此判断全屏独占；尝试窗口化或无边框模式后对比。",
            vec![],
        )),
        None => messages.push(("无法读取目标窗口的物理边界，满屏状态未知。", vec![])),
        Some(false) => {}
    }
    let layout = if target.keyboard_layout == "0" {
        "未知".to_owned()
    } else {
        target.keyboard_layout.clone()
    };
    messages.push((
        "目标键盘布局：{0}。若组合被输入法使用，切换英文输入或改用 F 键组合后对比。",
        vec![layout],
    ));
    messages
}

#[cfg(windows)]
fn foreground_pid() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
    }
    pid
}
#[cfg(target_os = "macos")]
fn foreground_pid() -> u32 {
    crate::macos::frontmost_pid()
        .and_then(|pid| u32::try_from(pid).ok())
        .unwrap_or(0)
}
/// X11 读取 EWMH 前台窗口的 `_NET_WM_PID`；Wayland、无窗口管理器或未设置时为 0。
#[cfg(not(any(windows, target_os = "macos")))]
fn foreground_pid() -> u32 {
    use crate::x11::X11;
    X11::connect()
        .ok()
        .and_then(|x| x.window_pid(x.active_window()?))
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn secure_input() -> Option<bool> {
    Some(crate::macos::secure_input_enabled())
}
#[cfg(not(target_os = "macos"))]
fn secure_input() -> Option<bool> {
    None
}

/// 查询失败保留 unknown；高权限/受保护进程不可读并不等于它一定拦截了快捷键。
#[cfg(windows)]
fn elevation(pid: u32) -> Option<bool> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    // RAII 保证 OpenProcessToken/GetTokenInformation 任意一步失败都释放已打开的句柄。
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token).ok()?;
        let token = Handle(token);
        let mut value = TOKEN_ELEVATION::default();
        let mut length = 0;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut value as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
        .ok()?;
        Some(value.TokenIsElevated != 0)
    }
}
#[cfg(not(windows))]
fn elevation(_: u32) -> Option<bool> {
    None
}

#[cfg(windows)]
fn foreground(_: &dyn Fn() -> Option<String>) -> Option<Foreground> {
    use windows::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        UI::{
            HiDpi::{
                SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            },
            Input::KeyboardAndMouse::GetKeyboardLayout,
            WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId},
        },
    };
    // GetWindowRect 可能被 DPI 虚拟化；比较显示器边界时统一为物理像素，离开后恢复线程上下文。
    struct DpiContext {
        previous: DPI_AWARENESS_CONTEXT,
    }
    impl Drop for DpiContext {
        fn drop(&mut self) {
            if !self.previous.0.is_null() {
                unsafe {
                    SetThreadDpiAwarenessContext(self.previous);
                }
            }
        }
    }
    unsafe {
        let dpi = DpiContext {
            previous: SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2),
        };
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if thread == 0 || pid == 0 {
            return None;
        }
        let mut rect = RECT::default();
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let covers_monitor = if !dpi.previous.0.is_null()
            && GetWindowRect(hwnd, &mut rect).is_ok()
            && GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
            .as_bool()
        {
            Some(
                rect.left <= monitor.rcMonitor.left
                    && rect.top <= monitor.rcMonitor.top
                    && rect.right >= monitor.rcMonitor.right
                    && rect.bottom >= monitor.rcMonitor.bottom,
            )
        } else {
            None
        };
        Some(Foreground {
            pid,
            elevated: elevation(pid),
            covers_monitor,
            keyboard_layout: format!("{:X}", GetKeyboardLayout(thread).0 as usize),
        })
    }
}
/// macOS 没有管理员提升的概念，elevated 保持未知；输入法标识需在主线程读取。
#[cfg(target_os = "macos")]
fn foreground(input_source: &dyn Fn() -> Option<String>) -> Option<Foreground> {
    let pid = foreground_pid();
    if pid == 0 {
        return None;
    }
    let keyboard_layout = input_source().unwrap_or_else(|| "0".into());
    Some(Foreground {
        pid,
        elevated: None,
        covers_monitor: covers_monitor(pid),
        keyboard_layout,
    })
}

/// 取该进程最上层的普通窗口，与其所在显示器的点坐标边界比较。
#[cfg(target_os = "macos")]
fn covers_monitor(pid: u32) -> Option<bool> {
    let normal = crate::macos::normal_window_ids();
    let window = xcap::Window::all().ok()?.into_iter().find(|window| {
        window.pid().is_ok_and(|value| value == pid)
            && window.id().is_ok_and(|id| normal.contains(&id))
            && !window.is_minimized().unwrap_or(true)
    })?;
    let (x, y) = (window.x().ok()?, window.y().ok()?);
    let (right, bottom) = (
        i64::from(x) + i64::from(window.width().ok()?),
        i64::from(y) + i64::from(window.height().ok()?),
    );
    let (cx, cy) = ((i64::from(x) + right) / 2, (i64::from(y) + bottom) / 2);
    let monitor = xcap::Monitor::all().ok()?.into_iter().find(|monitor| {
        let (mx, my) = (monitor.x().unwrap_or(0), monitor.y().unwrap_or(0));
        let (mw, mh) = (monitor.width().unwrap_or(0), monitor.height().unwrap_or(0));
        cx >= i64::from(mx)
            && cy >= i64::from(my)
            && cx < i64::from(mx) + i64::from(mw)
            && cy < i64::from(my) + i64::from(mh)
    })?;
    let (mx, my) = (i64::from(monitor.x().ok()?), i64::from(monitor.y().ok()?));
    Some(
        i64::from(x) <= mx
            && i64::from(y) <= my
            && right >= mx + i64::from(monitor.width().ok()?)
            && bottom >= my + i64::from(monitor.height().ok()?),
    )
}

/// X11 同样没有管理员提升的概念；全屏按前台窗口与其中心所在 RandR 显示器比较，
/// 键盘布局取 `_XKB_RULES_NAMES` 中配置的布局列表（如 `us,ru`）。
#[cfg(not(any(windows, target_os = "macos")))]
fn foreground(_: &dyn Fn() -> Option<String>) -> Option<Foreground> {
    use crate::x11::{covers_monitor, xkb_layout, X11};
    let x = X11::connect().ok()?;
    let window = x.active_window()?;
    let pid = x.window_pid(window)?;
    let covers_monitor = x
        .window_rect(window)
        .and_then(|rect| covers_monitor(rect, &x.monitors()));
    let keyboard_layout = x
        .existing("_XKB_RULES_NAMES")
        .and_then(|property| x.string(x.root, property, 1024))
        .and_then(|rules| xkb_layout(&rules))
        .unwrap_or_else(|| "0".into());
    Some(Foreground {
        pid,
        elevated: None,
        covers_monitor,
        keyboard_layout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_expires_and_bounds_only_configured_receipts() {
        let now = Instant::now();
        let mut probe = Probe::default();
        assert!(!probe.receive("F1".into(), 1, now));
        probe.deadline = Some(now + Duration::from_secs(8));
        for _ in 0..100 {
            assert!(probe.receive("F1".into(), 2, now));
        }
        assert_eq!(probe.received.len(), 32);
        assert_eq!(probe.received[0].foreground_pid, 2);
        assert!(!probe.receive("F1".into(), 2, now + Duration::from_secs(8)));
    }
    #[test]
    fn messages_match_frontend_rules() {
        let mut report = ProbeReport {
            platform: "windows",
            secure_input: None,
            enabled: false,
            editing: false,
            registration_error: None,
            registered: vec![],
            received: vec![],
            app_elevated: Some(false),
            target: None,
        };
        assert_eq!(messages(&report).len(), 2);
        report.enabled = true;
        report.registered = vec!["F1".into(), "Ctrl+F2".into()];
        assert_eq!(
            messages(&report)[1].0,
            "未观察到外部前台窗口；重新测试并在五秒内切换到目标软件按键。"
        );
        report.target = Some(Foreground {
            pid: 7,
            elevated: Some(true),
            covers_monitor: None,
            keyboard_layout: "0".into(),
        });
        report.received = vec![
            Receipt {
                shortcut: "F1".into(),
                foreground_pid: 7,
            },
            Receipt {
                shortcut: "F1".into(),
                foreground_pid: 7,
            },
            Receipt {
                shortcut: "F1".into(),
                foreground_pid: 8,
            },
        ];
        let all = messages(&report);
        assert_eq!(all[0].1, vec!["F1、Ctrl+F2".to_owned()]);
        assert_eq!(all[1].1, vec!["7".to_owned(), "F1".to_owned()]);
        assert!(all.iter().any(|(key, _)| key.starts_with("其他前台窗口")));
        assert!(all
            .iter()
            .any(|(key, _)| key.starts_with("目标软件与 Snap Desk 的管理员提升状态不同")));
        assert!(all
            .iter()
            .any(|(key, _)| key.starts_with("无法读取目标窗口")));
        assert_eq!(all.last().unwrap().1, vec!["未知".to_owned()]);
    }
    #[test]
    fn delayed_completion_cannot_clear_new_probe_or_its_receipts() {
        let now = Instant::now();
        let old_deadline = now + Duration::from_secs(8);
        let deadline = now + Duration::from_secs(16);
        let mut probe = Probe {
            deadline: Some(deadline),
            received: vec![],
        };
        assert!(probe.receive("F1".into(), 42, now + Duration::from_secs(9)));
        assert!(probe.finish(old_deadline).is_err());
        assert!(probe.active(now + Duration::from_secs(10)));
        let received = probe.finish(deadline).unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].foreground_pid, 42);
        assert!(probe.received.is_empty());
        assert!(!probe.active(now + Duration::from_secs(10)));
    }
}
