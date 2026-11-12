//! 应用更新：按设置选择正式版/测试版，校验签名后升级当前分发形式。
//! 每个应用 Release 自带不可变清单，模型/引擎的预发布不会参与应用版本选择。
mod portable;

use crate::{
    capture::{require_window, CaptureState},
    settings::SettingsState,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

const REPOSITORY: &str = "bitdan/snap-desk";
/// 应用常驻托盘，可能连续运行数天；定时复查才能发现运行期间发布的新版本。
/// 每次复查只请求一次发布列表，远低于 GitHub 匿名接口每小时 60 次的限额。
const RECHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// 断网、限流等失败后较快重试，不必等到下一个完整周期。
const RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);
const PUBLIC_KEY: &str = match option_env!("SNAP_DESK_UPDATER_PUBLIC_KEY") {
    Some(key) => key,
    None => "",
};

/// 只暴露更新状态，安装地址和签名由原生端保存，不接受前端传入可执行地址。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppUpdateView {
    current_version: String,
    available_version: Option<String>,
    notes: Option<String>,
    package_kind: String,
    supported: bool,
    configured: bool,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

#[derive(Default)]
struct Session {
    update: Option<Update>,
    bytes: Option<Arc<Vec<u8>>>,
    preview: bool,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

/// 操作锁防止重复检查/下载；短状态锁允许页面在下载过程中查询进度。
#[derive(Default)]
pub(super) struct AppUpdateState {
    installer_prepared: AtomicBool,
    operation: tokio::sync::Mutex<()>,
    session: Mutex<Session>,
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn supported() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
        && !cfg!(debug_assertions)
        && package_kind() != "unsupported"
}

fn package_kind() -> &'static str {
    use tauri::utils::{config::BundleType, platform::bundle_type};
    match bundle_type() {
        Some(BundleType::Nsis) => "nsis",
        None => "standalone",
        _ => "unsupported",
    }
}

/// 是否把测试版纳入候选。当前运行的本身是测试版时始终纳入：
/// 否则测试版用户在开关关闭（默认值）时收不到后续测试版，只能等到正式版发布。
fn preview(app: &AppHandle) -> Result<bool, String> {
    if !app.package_info().version.pre.is_empty() {
        return Ok(true);
    }
    Ok(app
        .state::<SettingsState>()
        .0
        .lock()
        .map_err(error)?
        .value
        .receive_prerelease_updates)
}

fn view(app: &AppHandle) -> Result<AppUpdateView, String> {
    let current_preview = preview(app)?;
    let state = app.state::<AppUpdateState>();
    let session = state.session.lock().map_err(error)?;
    // 导入配置也能改变渠道；尚未重新检查时不能展示旧渠道的下载/安装按钮。
    let same_channel = current_preview == session.preview;
    Ok(AppUpdateView {
        current_version: app.package_info().version.to_string(),
        available_version: session
            .update
            .as_ref()
            .filter(|_| same_channel)
            .map(|u| u.version.clone()),
        notes: session
            .update
            .as_ref()
            .filter(|_| same_channel)
            .and_then(|u| u.body.clone()),
        package_kind: package_kind().into(),
        supported: supported(),
        configured: !PUBLIC_KEY.trim().is_empty(),
        phase: if session.phase.is_empty() || !same_channel {
            "idle".into()
        } else {
            session.phase.clone()
        },
        downloaded: session.downloaded,
        total: session.total,
        error: session.error.clone(),
    })
}

fn ready() -> Result<(), String> {
    if package_kind() == "unsupported" {
        return Err("当前安装方式不支持在线更新，请使用 NSIS 安装包重新安装。".into());
    }
    if !supported() {
        return Err("在线更新仅支持 Windows x64 发布版本。".into());
    }
    if PUBLIC_KEY.trim().is_empty() {
        return Err("此版本未配置更新签名公钥。".into());
    }
    Ok(())
}

fn failed(app: &AppHandle, result: Result<(), String>) -> Result<AppUpdateView, String> {
    let failure = result.err();
    if let Some(message) = &failure {
        let state = app.state::<AppUpdateState>();
        let mut session = state.session.lock().map_err(error)?;
        session.phase = "error".into();
        session.error = Some(message.clone());
    }
    let view = view(app)?;
    let _ = app.emit_to("main", "app-update-changed", &view);
    match failure {
        Some(message) => Err(message),
        None => Ok(view),
    }
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

/// 仅匹配应用 SemVer 标签和固定清单名称，跳过翻译模型/引擎等独立发布。
fn candidate(
    release: &Release,
    current: &Version,
    include_preview: bool,
) -> Option<(Version, String)> {
    if release.draft {
        return None;
    }
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    if version <= *current || (!include_preview && (release.prerelease || !version.pre.is_empty()))
    {
        return None;
    }
    let expected = format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/updater.json",
        release.tag_name
    );
    release
        .assets
        .iter()
        .find(|asset| asset.name == "updater.json" && asset.browser_download_url == expected)
        .map(|_| (version, expected))
}

async fn discover(app: &AppHandle, include_preview: bool) -> Result<Option<Update>, String> {
    let client = reqwest::Client::builder()
        .user_agent("Snap-Desk-Updater")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(error)?;
    let current = &app.package_info().version;
    let mut newest: Option<(Version, String)> = None;
    // GitHub 同仓库还发布模型资源，需要分页，不能依赖 /latest 或列表第一项。
    for page in 1..=20 {
        let response = client
            .get(format!(
                "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100&page={page}"
            ))
            .send()
            .await
            .map_err(error)?
            .error_for_status()
            .map_err(error)?;
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(error)? {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("更新列表过大。".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let releases: Vec<Release> = serde_json::from_slice(&bytes).map_err(error)?;
        for release in &releases {
            if let Some(next) = candidate(release, current, include_preview) {
                if newest.as_ref().is_none_or(|old| next.0 > old.0) {
                    newest = Some(next);
                }
            }
        }
        if releases.len() < 100 {
            break;
        }
        if page == 20 {
            return Err("更新列表超出查询范围，请稍后重试。".into());
        }
    }
    let Some((version, endpoint)) = newest else {
        return Ok(None);
    };
    let target = format!("windows-x86_64-{}", package_kind());
    let updater = app
        .updater_builder()
        .pubkey(PUBLIC_KEY.trim())
        .target(&target)
        .timeout(Duration::from_secs(300))
        .on_before_exit({
            let app = app.clone();
            move || {
                app.state::<AppUpdateState>()
                    .installer_prepared
                    .store(true, Ordering::SeqCst);
                super::prepare_installer_exit(&app);
            }
        })
        .endpoints(vec![endpoint.parse().map_err(error)?])
        .map_err(error)?
        .build()
        .map_err(error)?;
    let mut update = updater
        .check()
        .await
        .map_err(error)?
        .ok_or("更新清单未提供所选版本。")?;
    // 插件的检查超时不自动传递给下载，单独为安装包设置总下载时限。
    update.timeout = Some(Duration::from_secs(300));
    // 清单不能借另一个 tag 发布不同版本，也不能切换包类型或仓库。
    let expected_suffix = match package_kind() {
        "nsis" => "-setup.exe",
        _ => "-standalone.exe",
    };
    let prefix = format!("https://github.com/{REPOSITORY}/releases/download/v{version}/");
    if update.version != version.to_string()
        || !update.download_url.as_str().starts_with(&prefix)
        || !update.download_url.path().ends_with(expected_suffix)
        || update.download_url.query().is_some()
        || update.download_url.fragment().is_some()
    {
        return Err("更新清单的版本或安装包类型不匹配。".into());
    }
    Ok(Some(update))
}

/// `background` 为定时复查：不打扰已有结果，只在查到新结论时才替换会话。
async fn check(app: &AppHandle, background: bool) -> Result<AppUpdateView, String> {
    ready()?;
    let state = app.state::<AppUpdateState>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "更新操作正在进行。")?;
    let include_preview = preview(app)?;
    let quiet = {
        let mut session = state.session.lock().map_err(error)?;
        let settled = background && session.preview == include_preview;
        if settled && session.phase == "ready" {
            // 安装包已下载并校验，复查会清空它；等用户安装或手动重新检查。
            drop(session);
            return view(app);
        }
        // 已有同渠道结论时保留它，界面不会闪回“正在检查”，红点也不会中途消失。
        let quiet = settled && (session.update.is_some() || session.phase == "latest");
        if !quiet {
            *session = Session {
                phase: "checking".into(),
                preview: include_preview,
                ..Default::default()
            };
        }
        quiet
    };
    let result = match discover(app, include_preview).await {
        Ok(update) => {
            let mut session = state.session.lock().map_err(error)?;
            *session = Session {
                phase: if update.is_some() {
                    "available".into()
                } else {
                    "latest".into()
                },
                preview: include_preview,
                update,
                ..Default::default()
            };
            Ok(())
        }
        // 后台复查失败不覆盖上一次的有效结论，由调用方稍后重试。
        Err(message) if quiet => return Err(message),
        Err(message) => Err(message),
    };
    failed(app, result)
}

#[tauri::command]
pub(super) fn app_update_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<AppUpdateView, String> {
    require_window(&window, "main")?;
    view(&app)
}

#[tauri::command]
pub(super) async fn check_app_update(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<AppUpdateView, String> {
    require_window(&window, "main")?;
    check(&app, false).await
}

#[tauri::command]
pub(super) async fn download_app_update(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<AppUpdateView, String> {
    require_window(&window, "main")?;
    ready()?;
    let state = app.state::<AppUpdateState>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "更新操作正在进行。")?;
    let include_preview = preview(&app)?;
    let update = {
        let mut session = state.session.lock().map_err(error)?;
        if session.preview != include_preview {
            return Err("更新渠道已改变，请重新检查更新。".into());
        }
        let update = session.update.clone().ok_or("请先检查更新。")?;
        session.phase = "downloading".into();
        session.error = None;
        session.bytes = None;
        session.downloaded = 0;
        session.total = None;
        update
    };
    let result = async {
        let bytes = update
            .download(
                |chunk, total| {
                    if let Ok(mut session) = state.session.lock() {
                        session.downloaded += chunk as u64;
                        session.total = total;
                    }
                },
                || {},
            )
            .await
            .map_err(error)?;
        let mut session = state.session.lock().map_err(error)?;
        session.bytes = Some(Arc::new(bytes));
        session.phase = "ready".into();
        Ok(())
    }
    .await;
    failed(&app, result)
}

#[tauri::command]
pub(super) async fn install_app_update(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_window(&window, "main")?;
    ready()?;
    let state = app.state::<AppUpdateState>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "更新操作正在进行。")?;
    let include_preview = preview(&app)?;
    let (update, bytes) = {
        let mut session = state.session.lock().map_err(error)?;
        if session.preview != include_preview {
            return Err("更新渠道已改变，请重新检查更新。".into());
        }
        let update = session.update.clone().ok_or("请先检查更新。")?;
        let bytes = session.bytes.clone().ok_or("请先下载更新。")?;
        session.phase = "installing".into();
        session.error = None;
        (update, bytes)
    };
    // 原生入口再次互斥，避免下载期间开始的录屏被安装器直接终止。
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let capture = handle.state::<CaptureState>();
        capture
            .active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "请先完成或取消截图、滚动截图或录屏。".to_string())?;
        let exit = handle.state::<crate::capture::recording::ExitState>();
        let result = exit.begin_update().and_then(|_| {
            if package_kind() == "standalone" {
                portable::install(&handle, &update, &bytes)
            } else {
                update.install(bytes.as_slice()).map_err(error)
            }
        });
        if result.is_err() {
            exit.cancel_update();
            capture.active.store(false, Ordering::SeqCst);
            // ShellExecute/UAC 失败会返回当前进程，恢复被退出钩子停止的剪贴板监听。
            if handle
                .state::<AppUpdateState>()
                .installer_prepared
                .swap(false, Ordering::SeqCst)
            {
                let clipboard_settings = handle
                    .state::<SettingsState>()
                    .0
                    .lock()
                    .map_err(error)?
                    .value
                    .clipboard
                    .clone();
                handle
                    .state::<crate::clipboard::Service>()
                    .configure(&handle, clipboard_settings)?;
            }
        }
        result
    })
    .await
    .map_err(error)
    .and_then(|result| result);
    // 安装失败保留已验证的包，允许用户结束当前任务后重试。
    if let Err(message) = result {
        let mut session = state.session.lock().map_err(error)?;
        session.phase = "ready".into();
        session.error = Some(message.clone());
        return Err(message);
    }
    Ok(())
}

/// 启动后检查一次，此后定时复查；始终不自动下载，页面通过同一状态查询结果。
pub(super) fn initialize(app: &AppHandle) {
    if std::env::args().any(|arg| arg == "--update-failed") {
        if let Ok(mut session) = app.state::<AppUpdateState>().session.lock() {
            session.phase = "error".into();
            session.error = Some("独立 EXE 更新失败，已尝试恢复原版本，请手动下载更新。".into());
        }
        return;
    }
    if !supported() || PUBLIC_KEY.trim().is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            let delay = match check(&app, true).await {
                Ok(_) => RECHECK_INTERVAL,
                Err(_) => RETRY_INTERVAL,
            };
            tokio::time::sleep(delay).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![ReleaseAsset {
                name: "updater.json".into(),
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/{tag}/updater.json"
                ),
            }],
        }
    }

    #[test]
    fn channels_use_semver_and_ignore_resource_releases() {
        let current = Version::parse("0.2.0-beta.2").unwrap();
        assert!(candidate(&release("v0.2.0-beta.10", true), &current, false).is_none());
        assert!(candidate(&release("v0.2.0-beta.10", true), &current, true).is_some());
        assert!(candidate(&release("v0.2.0", false), &current, false).is_some());
        assert!(candidate(&release("v0.1.9", false), &current, false).is_none());
        assert!(candidate(&release("translation-engine-v3", true), &current, true).is_none());
        let mut forged = release("v0.2.1", false);
        forged.assets[0].browser_download_url = "https://example.com/updater.json".into();
        assert!(candidate(&forged, &current, true).is_none());
        forged = release("v0.2.1", false);
        forged.draft = true;
        assert!(candidate(&forged, &current, true).is_none());
    }
}
