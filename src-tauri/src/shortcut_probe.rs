//! 用户主动发起的八秒快捷键测试：复用现有系统注册，不安装钩子或记录普通按键。
//! 测试期间只统计已配置动作，避免测试 F1/F3 时意外截图或停止录屏。
use crate::{capture::require_window, settings::SettingsState};
use serde::Serialize;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Default)]
pub struct ProbeState(Mutex<Probe>);
#[derive(Default)]
struct Probe {
    deadline: Option<Instant>,
    received: Vec<Receipt>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    shortcut: String,
    foreground_pid: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Foreground {
    pid: u32,
    elevated: Option<bool>,
    covers_monitor: Option<bool>,
    keyboard_layout: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    enabled: bool,
    editing: bool,
    registration_error: Option<String>,
    registered: Vec<String>,
    received: Vec<Receipt>,
    app_elevated: Option<bool>,
    target: Option<Foreground>,
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
/// 仅由已有全局快捷键的按下回调调用；松键仍走原有去重清理路径。
pub fn receive(app: &AppHandle, shortcut: String) -> bool {
    // 测试中若从托盘开始采集，立即让出热键，尤其不能吞掉停止录屏/滚动截图的动作。
    if app
        .state::<crate::capture::CaptureState>()
        .active
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return false;
    }
    app.state::<ProbeState>()
        .0
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

/// 在后台等待用户切换到 Axure/游戏并按键，避免阻塞 Tauri 系统消息线程。
#[tauri::command]
pub async fn probe_shortcuts(app: AppHandle, window: WebviewWindow) -> Result<ProbeReport, String> {
    require_window(&window, "main")?;
    if !cfg!(windows) {
        return Err("此环境测试仅支持 Windows。".into());
    }
    if app
        .state::<crate::capture::CaptureState>()
        .active
        .load(std::sync::atomic::Ordering::SeqCst)
        || crate::capture::recording::current_recording(app.clone(), window)?.is_some()
    {
        return Err("请先结束截图、滚动截图或录屏，再测试快捷键。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut report = {
            let state = app.state::<SettingsState>();
            let current = state.0.lock().map_err(|e| e.to_string())?;
            ProbeReport {
                enabled: current.value.shortcuts_enabled,
                editing: current.editing,
                registration_error: current.shortcut_error.clone(),
                registered: current.registered.iter().map(ToString::to_string).collect(),
                received: vec![],
                app_elevated: elevation(std::process::id()),
                target: None,
            }
        };
        // 关闭/录入/注册失败直接报告，不把无法测试误报成目标软件拦截。
        if !report.enabled
            || report.editing
            || report.registered.is_empty()
            || report.registration_error.is_some()
        {
            return Ok(report);
        }
        let state = app.state::<ProbeState>();
        let deadline = Instant::now() + Duration::from_secs(8);
        {
            let mut probe = state.0.lock().map_err(|e| e.to_string())?;
            if probe.active(Instant::now()) {
                return Err("快捷键测试正在进行。".into());
            }
            probe.received.clear();
            probe.deadline = Some(deadline);
        }
        let mut interrupted = false;
        while Instant::now() < deadline {
            if app
                .state::<crate::capture::CaptureState>()
                .active
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                interrupted = true;
                break;
            }
            if let Some(target) = foreground() {
                // 返回设置页不会覆盖刚测试的外部窗口；不读取窗口标题和文档内容。
                if target.pid != std::process::id() {
                    report.target = Some(target);
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let mut probe = state.0.lock().map_err(|e| e.to_string())?;
        // 旧线程可能在到期后被调度挂起；不得清空随后发起的新测试。
        report.received = probe.finish(deadline)?;
        drop(probe);
        if interrupted {
            return Err("测试已因截图或录屏开始而结束，功能快捷键继续正常工作。".into());
        }
        let settings = app.state::<SettingsState>();
        let current = settings.0.lock().map_err(|e| e.to_string())?;
        report.enabled = current.value.shortcuts_enabled;
        report.editing = current.editing;
        report.registration_error = current.shortcut_error.clone();
        report.registered = current.registered.iter().map(ToString::to_string).collect();
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
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
#[cfg(not(windows))]
fn foreground_pid() -> u32 {
    0
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
fn foreground() -> Option<Foreground> {
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
#[cfg(not(windows))]
fn foreground() -> Option<Foreground> {
    None
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
