//! 应用内更新（对照 Tauri `app/updater.rs`）：启动 10 秒后检查一次，之后按设置的间隔复查（每分钟重读间隔，
//! 失败 10 分钟后重试）；只提示，不自动下载。发现新版本后读取发布自带的固定清单，下载（镜像优先、断点续传）
//! 并用编译时注入的公钥校验 minisign 签名，通过后才允许安装。
//!
//! 分发形式：程序目录有 NSIS 卸载程序（`uninstall.exe`）即安装版，运行同一清单目标
//! `windows-x86_64-nsis` 的安装器（`/P /UPDATE /R`，与 updater 插件被动模式一致，装完重启）；否则为独立 EXE，
//! 交给与 Tauri 版相同的助手脚本在退出后原子替换。macOS/Linux 的 GPUI 版尚无安装包，只提供发布页。
use gpui_kit::*;
use semver::Version;
use snap_core::updates::{
    self,
    download::{self, Network},
    package::{self, Package},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const PUBLIC_KEY: &str = match option_env!("SNAP_DESK_UPDATER_PUBLIC_KEY") {
    Some(key) => key,
    None => "",
};
/// 启动后首次检查前的等待。
const FIRST_CHECK: Duration = Duration::from_secs(10);
/// 等待期间每分钟重读一次间隔：在设置页改短后不必等旧周期走完。
const INTERVAL_POLL: Duration = Duration::from_secs(60);
/// 断网、限流等失败后较快重试。
const RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// 发布流水线实际产出更新包的平台（与 `scripts/release-assets.mjs` 一致）。
const PLATFORM: Option<&str> = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some("windows-x86_64")
} else {
    None
};

/// 产品版本（与 Tauri 版共用 package.json）。
pub fn current_version() -> String {
    serde_json::from_str::<serde_json::Value>(include_str!("../../../package.json"))
        .ok()
        .and_then(|value| value.get("version")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "0.0.0".into())
}

fn current() -> Version {
    Version::parse(&current_version()).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// 当前进程的分发形式，同时决定清单目标和安装方式。
pub fn package_kind() -> &'static str {
    if !cfg!(windows) {
        return "unsupported";
    }
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe").is_file()))
        .unwrap_or(false);
    if installed {
        "nsis"
    } else {
        "standalone"
    }
}

fn suffix(kind: &str) -> &'static str {
    if kind == "nsis" {
        "-setup.exe"
    } else {
        "-standalone.exe"
    }
}

pub fn supported() -> bool {
    PLATFORM.is_some() && !cfg!(debug_assertions) && package_kind() != "unsupported"
}

pub fn configured() -> bool {
    !PUBLIC_KEY.trim().is_empty()
}

fn ready() -> Result<(), String> {
    if package_kind() == "unsupported" {
        return Err("当前安装方式不支持在线更新，请从发布页下载新版本。".into());
    }
    if !supported() {
        return Err("在线更新仅支持已发布平台的正式构建。".into());
    }
    if !configured() {
        return Err("此版本未配置更新签名公钥。".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Available,
    Latest,
    Downloading,
    Ready,
    Installing,
    Error,
}

#[derive(Default)]
struct Session {
    phase: Phase,
    preview: bool,
    package: Option<Package>,
    bytes: Option<Arc<Vec<u8>>>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

/// 设置页与提示读取的状态。
#[derive(Clone)]
pub struct View {
    pub current: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub kind: &'static str,
    pub supported: bool,
    pub configured: bool,
    pub phase: Phase,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}

impl View {
    pub fn operating(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking | Phase::Downloading | Phase::Installing
        )
    }
    /// 顶部“有更新”与导航红点。
    pub fn has_update(&self) -> bool {
        self.version.is_some() && self.supported && self.configured
    }
}

#[derive(Default)]
struct Updater {
    session: Arc<Mutex<Session>>,
    /// 防止重复检查/下载/安装。
    busy: Arc<AtomicBool>,
}

impl Global for Updater {}

/// 是否把测试版纳入候选：当前运行的本身是测试版时始终纳入。
fn preview(cx: &App) -> bool {
    !current().pre.is_empty() || crate::shell::settings(cx).receive_prerelease_updates
}

pub fn view(cx: &App) -> View {
    let include_preview = preview(cx);
    let mut view = View {
        current: current_version(),
        version: None,
        notes: None,
        kind: package_kind(),
        supported: supported(),
        configured: configured(),
        phase: Phase::Idle,
        downloaded: 0,
        total: None,
        error: None,
    };
    let Some(updater) = cx.try_global::<Updater>() else {
        return view;
    };
    let Ok(session) = updater.session.lock() else {
        return view;
    };
    // 渠道改变（切换测试版开关、导入配置）后，旧渠道的结果不再展示。
    if session.preview == include_preview {
        view.version = session.package.as_ref().map(|p| p.version.to_string());
        view.notes = session.package.as_ref().and_then(|p| p.notes.clone());
        view.phase = session.phase;
    }
    view.downloaded = session.downloaded;
    view.total = session.total;
    view.error = session.error.clone();
    view
}

fn with_session(cx: &mut App, f: impl FnOnce(&mut Session)) {
    if let Ok(mut session) = cx.default_global::<Updater>().session.lock() {
        f(&mut session);
    }
    cx.refresh_windows();
}

/// 查询结果：(新版本的更新包；None 为已是最新)。
fn discover(
    settings: &snap_core::settings::Settings,
    include_preview: bool,
) -> Result<Option<Package>, String> {
    let network = Network::from_settings(&settings.update_network)?;
    let Some(found) = updates::latest(
        &current(),
        include_preview,
        Some(&settings.update_network.proxy),
    )?
    else {
        return Ok(None);
    };
    let platform = PLATFORM.ok_or("在线更新仅支持已发布平台的正式构建。")?;
    let kind = package_kind();
    package::fetch(
        &network,
        &found,
        &format!("{platform}-{kind}"),
        suffix(kind),
    )
    .map(Some)
}

/// 检查更新。`background` 为定时复查：已有同渠道结论时不闪回“正在检查”，失败也不覆盖上一次的结论。
/// `done` 收到是否失败（定时复查据此决定重试间隔）。
pub fn check(cx: &mut App, background: bool, done: impl FnOnce(bool) + 'static) {
    if let Err(error) = ready() {
        if !background {
            with_session(cx, |s| {
                s.phase = Phase::Error;
                s.error = Some(error);
            });
        }
        done(true);
        return;
    }
    let include_preview = preview(cx);
    let updater = cx.default_global::<Updater>();
    if updater.busy.swap(true, Ordering::SeqCst) {
        done(false);
        return;
    }
    let busy = updater.busy.clone();
    let quiet = {
        let Ok(mut session) = updater.session.lock() else {
            busy.store(false, Ordering::SeqCst);
            return;
        };
        let settled = background && session.preview == include_preview;
        if settled && session.phase == Phase::Ready {
            // 安装包已下载并校验，复查会清空它；等用户安装或手动重新检查。
            busy.store(false, Ordering::SeqCst);
            drop(session);
            done(false);
            return;
        }
        let quiet = settled && (session.package.is_some() || session.phase == Phase::Latest);
        if !quiet {
            *session = Session {
                phase: Phase::Checking,
                preview: include_preview,
                ..Default::default()
            };
        }
        quiet
    };
    cx.refresh_windows();
    let settings = crate::shell::settings(cx);
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = cx
            .background_executor()
            .spawn(async move { discover(&settings, include_preview) })
            .await;
        busy.store(false, Ordering::SeqCst);
        let failed = result.is_err();
        let _ = cx.update(|cx| {
            match result {
                Ok(package) => {
                    if package.is_some() {
                        log::info!("event=gpui.update_available");
                    }
                    with_session(cx, |s| {
                        *s = Session {
                            phase: if package.is_some() {
                                Phase::Available
                            } else {
                                Phase::Latest
                            },
                            preview: include_preview,
                            package,
                            ..Default::default()
                        }
                    });
                }
                Err(error) => {
                    log::warn!("event=gpui.update_check_failed message={error}");
                    if !quiet {
                        with_session(cx, |s| {
                            s.phase = Phase::Error;
                            s.error = Some(error);
                        });
                    }
                }
            }
            done(failed);
        });
    })
    .detach();
}

/// 下载并校验安装包；下载期间每半秒刷新窗口显示进度。
pub fn download(cx: &mut App) {
    if let Err(error) = ready() {
        with_session(cx, |s| s.error = Some(error));
        return;
    }
    let include_preview = preview(cx);
    let updater = cx.default_global::<Updater>();
    if updater.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let (busy, session) = (updater.busy.clone(), updater.session.clone());
    let package = {
        let Ok(mut s) = session.lock() else {
            busy.store(false, Ordering::SeqCst);
            return;
        };
        let package = s.package.clone().filter(|_| s.preview == include_preview);
        let Some(package) = package else {
            s.error = Some("请先检查更新。".into());
            busy.store(false, Ordering::SeqCst);
            drop(s);
            cx.refresh_windows();
            return;
        };
        s.phase = Phase::Downloading;
        s.error = None;
        s.bytes = None;
        s.downloaded = 0;
        s.total = None;
        package
    };
    cx.refresh_windows();
    let network = crate::shell::settings(cx).update_network;
    let progress = session.clone();
    let work = cx.background_executor().spawn(async move {
        let network = Network::from_settings(&network)?;
        let client = network.client(None)?;
        let routes = network.routes(&package.url);
        let bytes = updates::block_on(download::download(
            &client,
            &routes,
            &download::RETRY_WAITS,
            download::STALL_AFTER,
            |downloaded, total| {
                if let Ok(mut s) = progress.lock() {
                    s.downloaded = downloaded;
                    s.total = total;
                }
            },
        ))??;
        // 镜像与直连下载的字节一样不受信任：必须通过签名与版本校验才保存。
        package::verify(&bytes, &package.signature, PUBLIC_KEY, &package.version)?;
        Ok::<_, String>(bytes)
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut work = std::pin::pin!(work);
        let result = loop {
            let tick = cx.background_executor().timer(Duration::from_millis(500));
            match futures::future::select(work.as_mut(), tick).await {
                futures::future::Either::Left((result, _)) => break result,
                futures::future::Either::Right(_) => {
                    let _ = cx.update(|cx| cx.refresh_windows());
                }
            }
        };
        busy.store(false, Ordering::SeqCst);
        let _ = cx.update(|cx| {
            with_session(cx, |s| match result {
                Ok(bytes) => {
                    log::info!("event=gpui.update_downloaded bytes={}", bytes.len());
                    s.bytes = Some(Arc::new(bytes));
                    s.phase = Phase::Ready;
                }
                Err(error) => {
                    log::warn!("event=gpui.update_download_failed message={error}");
                    s.phase = Phase::Error;
                    s.error = Some(error);
                }
            })
        });
    })
    .detach();
}

/// 退出并安装。截图、滚动截图或录屏进行中时拒绝，避免安装器直接终止它们。
pub fn install(cx: &mut App) {
    if let Err(error) = ready() {
        with_session(cx, |s| s.error = Some(error));
        return;
    }
    if crate::capture::active(cx) || crate::scroll::active(cx) || crate::recording::active(cx) {
        with_session(cx, |s| {
            s.error = Some("请先完成或取消截图、滚动截图或录屏。".into())
        });
        return;
    }
    let updater = cx.default_global::<Updater>();
    if updater.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let busy = updater.busy.clone();
    let prepared = updater.session.lock().ok().and_then(|mut s| {
        let package = s.package.clone()?;
        let bytes = s.bytes.clone()?;
        s.phase = Phase::Installing;
        s.error = None;
        Some((package, bytes))
    });
    let Some((package, bytes)) = prepared else {
        busy.store(false, Ordering::SeqCst);
        with_session(cx, |s| s.error = Some("请先下载更新。".into()));
        return;
    };
    cx.refresh_windows();
    let result = package::verify(&bytes, &package.signature, PUBLIC_KEY, &package.version)
        .and_then(|_| {
            if package_kind() == "nsis" {
                launch_installer(&package, &bytes)
            } else {
                replace_standalone(&bytes)
            }
        });
    busy.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => {
            log::info!("event=gpui.update_installing kind={}", package_kind());
            crate::shell::quit(cx, "update");
        }
        Err(error) => {
            log::warn!("event=gpui.update_install_failed message={error}");
            // 安装失败保留已验证的包，允许用户结束当前任务后重试。
            with_session(cx, |s| {
                s.phase = Phase::Ready;
                s.error = Some(error);
            });
        }
    }
}

/// 在浏览器中打开发布页（有候选版本时打开该版本）。
pub fn open_release_page(cx: &App) -> Result<(), String> {
    let version = view(cx).version;
    updates::open_release_page(version.as_deref())
}

/// NSIS 安装包写入临时目录后以被动模式运行；安装器结束当前进程并在完成后重启。
#[cfg(windows)]
fn launch_installer(package: &Package, bytes: &[u8]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::{
        w,
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW},
    };
    let name = package
        .url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|n| n.ends_with("-setup.exe") && !n.contains(['\\', '/', ':']))
        .ok_or("安装包文件名无效。")?;
    let directory = std::env::temp_dir().join(format!("snap-desk-update-{}", package.version));
    std::fs::create_dir_all(&directory).map_err(|e| format!("无法写入临时目录：{e}"))?;
    let path = directory.join(name);
    std::fs::write(&path, bytes).map_err(|e| format!("无法写入安装包：{e}"))?;
    let file: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            w!("open"),
            file.as_ptr(),
            w!("/P /UPDATE /R"),
            std::ptr::null(),
            SW_SHOW,
        )
    };
    if result as isize <= 32 {
        return Err(format!(
            "无法启动安装程序（代码 {}），请重试或在浏览器中下载。",
            result as isize
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn launch_installer(_: &Package, _: &[u8]) -> Result<(), String> {
    Err("当前平台不支持在线安装。".into())
}

/// 独立 EXE：暂存到程序目录，外部助手（与 Tauri 版同一脚本）等本进程退出后原子替换并重启。
#[cfg(windows)]
fn replace_standalone(bytes: &[u8]) -> Result<(), String> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::{io::Write, os::windows::process::CommandExt, process::Command};

    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = target.parent().ok_or("程序目录无效。")?;
    // 创建失败（只读目录/权限不足）时保持应用运行，不申请提权后悄悄写入其他位置。
    let directory = tempfile::Builder::new()
        .prefix(".snapdesk-update-")
        .tempdir_in(parent)
        .map_err(|_| "程序目录不可写，请手动下载新版本并替换。")?;
    let source = directory.path().join("new.exe");
    let mut file = std::fs::File::create(&source).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let script = include_str!("../../../src-tauri/src/app/updater/portable.ps1");
    let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let system = std::env::var_os("SystemRoot").ok_or("无法定位 Windows 系统目录。")?;
    let mut helper = Command::new(
        std::path::Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-EncodedCommand",
    ])
    .arg(base64::engine::general_purpose::STANDARD.encode(encoded))
    // 路径通过环境变量传入固定脚本，包含引号、空格或 $ 的目录不会成为命令。
    .env("SNAP_UPDATE_TARGET", &target)
    .env("SNAP_UPDATE_SOURCE", &source)
    .env("SNAP_UPDATE_PARENT", std::process::id().to_string())
    .env("SNAP_UPDATE_HASH", format!("{:x}", Sha256::digest(bytes)))
    .creation_flags(0x08000000)
    .spawn()
    .map_err(|e| e.to_string())?;
    let started = Instant::now();
    while !directory.path().join("ready").is_file() {
        if helper.try_wait().map_err(|e| e.to_string())?.is_some()
            || started.elapsed() > Duration::from_secs(5)
        {
            let _ = helper.kill();
            let _ = helper.wait();
            return Err("无法启动更新助手，程序保持运行，请重试或手动更新。".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 只有助手就绪才转移暂存目录所有权；助手仅清理其中的已知文件。
    let _ = directory.keep();
    Ok(())
}

#[cfg(not(windows))]
fn replace_standalone(_: &[u8]) -> Result<(), String> {
    Err("独立 EXE 更新仅适用于 Windows。".into())
}

/// 启动时调用：处理独立 EXE 更新失败后的重启参数，并开始后台检查。
pub fn initialize(cx: &mut App) {
    if std::env::args().any(|arg| arg == "--update-failed") {
        with_session(cx, |s| {
            s.phase = Phase::Error;
            s.error = Some("独立 EXE 更新失败，已尝试恢复原版本，请手动下载更新。".into());
        });
        return;
    }
    if ready().is_err() {
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(FIRST_CHECK).await;
        loop {
            let (sender, receiver) = futures::channel::oneshot::channel();
            cx.update(|cx| {
                check(cx, true, move |failed| {
                    let _ = sender.send(failed);
                })
            });
            let failed = receiver.await.unwrap_or(true);
            let checked = Instant::now();
            loop {
                cx.background_executor().timer(INTERVAL_POLL).await;
                let minutes = cx.update(|cx| crate::shell::settings(cx).update_check_minutes);
                let wait = if failed {
                    RETRY_INTERVAL
                } else {
                    Duration::from_secs(u64::from(minutes) * 60)
                };
                if checked.elapsed() >= wait {
                    break;
                }
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{current_version, suffix};

    #[test]
    fn product_version_is_semver() {
        assert!(semver::Version::parse(&current_version()).is_ok());
    }

    #[test]
    fn package_suffix_follows_the_distribution() {
        assert_eq!(suffix("nsis"), "-setup.exe");
        assert_eq!(suffix("standalone"), "-standalone.exe");
    }
}
