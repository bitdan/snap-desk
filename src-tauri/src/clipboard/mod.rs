//! 剪贴板历史的受限入口；历史管理限剪贴板窗口与主窗口，暂存架只复用单条读取、复制、贴图和拖出。
/// 存储模型、分类、文件引用校验与 SQLite 仓库在 core；这里保留原路径供平台读写与命令使用。
use snap_core::clipboard::{classification, files, repository};
mod blob_store;
pub mod export;
pub use snap_core::clipboard::model;
mod platform;

/// X11 图片写入（PNG + BMP 目标），供共享的图片复制入口调用。
#[cfg(target_os = "linux")]
pub(crate) fn write_x11_image(image: &image::RgbaImage) -> Result<(), String> {
    platform::write_image(image)
}
mod reader;
mod service;
#[cfg(test)]
mod transport_bench;
mod window;
use base64::{engine::general_purpose::STANDARD, Engine};
use model::*;
use service::Content;
pub use service::Service;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// 即时文件贴图复用已有有界读取与路径校验，不读取或复制原文件内容。
pub(crate) fn file_paths_for_pin() -> Result<Option<Vec<String>>, String> {
    let mut settings = ClipboardSettings::default();
    settings.save_copied_images = false;
    settings.save_copied_text = false;
    settings.save_copied_files = true;
    match platform::snapshot(&settings)? {
        Some(reader::Snapshot::Files(bytes)) => files::decode(&bytes).map(Some),
        #[cfg(not(windows))]
        Some(reader::Snapshot::Paths(paths)) => Ok(Some(paths)),
        _ => Ok(None),
    }
}

/// 重任务统一移出 UI 线程；此处只验证窗口，不对任意文件路径提供入口。
async fn work<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    f: impl FnOnce(&AppHandle, &Service) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    work_impl(app, window, false, f).await
}

/// 暂存架只复用单条读取、复制、贴图和拖出，不开放历史删除及配置命令。
async fn item_work<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    f: impl FnOnce(&AppHandle, &Service) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    work_impl(app, window, true, f).await
}
async fn work_impl<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    shelf: bool,
    f: impl FnOnce(&AppHandle, &Service) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    // 列表在独立剪贴板窗口，设置入口在主窗口；两者共用同一组历史命令。
    let allowed =
        service::UI_WINDOWS.contains(&window.label()) || shelf && window.label() == "shelf";
    if !allowed {
        return Err("当前窗口无权执行此操作。".into());
    }
    tauri::async_runtime::spawn_blocking(move || f(&app, &app.state::<Service>()))
        .await
        .map_err(|_| "历史任务执行失败。".to_string())?
}
pub fn initialize(app: &AppHandle) {
    app.manage(Service::new(app));
    log::info!("event=clipboard.initialized module=clipboard");
    let settings = app
        .state::<crate::settings::SettingsState>()
        .0
        .lock()
        .map(|s| s.value.clipboard.clone())
        .unwrap_or_default();
    if let Err(e) = app.state::<Service>().configure(app, settings) {
        log::warn!("event=clipboard.initialize_failed module=clipboard code=CLIPBOARD_INIT_FAILED");
        app.state::<Service>().report(app, e);
    }
    // 过期淘汰和素材修复要逐个核对图片文件，历史越多越慢；放到后台线程，
    // 不拖住 setup 线程上的托盘、快捷键与主窗口创建。素材操作锁保证它与新复制互斥。
    let app = app.clone();
    std::thread::spawn(move || {
        let service = app.state::<Service>();
        if let Err(e) = service.maintain(&app) {
            log::warn!(
                "event=clipboard.initialize_failed module=clipboard code=CLIPBOARD_INIT_FAILED"
            );
            service.report(&app, e);
        }
    });
}
#[tauri::command]
pub async fn clipboard_status(app: AppHandle, window: WebviewWindow) -> Result<Status, String> {
    work(app, window, |_, s| Ok(s.status())).await
}
#[tauri::command]
pub async fn clipboard_list(
    app: AppHandle,
    window: WebviewWindow,
    query: Query,
) -> Result<Page, String> {
    work(app, window, move |_, s| {
        s.data()?.repo.list(
            &query,
            s.revision.load(std::sync::atomic::Ordering::Acquire),
        )
    })
    .await
}
#[tauri::command]
pub async fn clipboard_get_text(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<String, String> {
    item_work(app, window, move |_, s| {
        let stored = s.data()?.repo.get(id)?;
        if stored.item.kind != Kind::Text {
            return Err("此条目不是文本。".into());
        }
        stored.text.ok_or("历史文本损坏。".into())
    })
    .await
}
/// 用户点击预览链接后，按记录 ID 读取原文并校验协议，再交给系统浏览器。
#[tauri::command]
pub async fn clipboard_open_link(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    work(app, window, move |_, s| {
        let stored = s.data()?.repo.get(id)?;
        if stored.item.kind != Kind::Text {
            return Err("此条目不是文本。".into());
        }
        let url = classification::link_url(stored.text.as_deref().ok_or("历史文本损坏。")?)
            .ok_or("此条目不是有效链接。")?;
        open::that(url).map_err(|e| format!("打开链接失败：{e}"))
    })
    .await
}
/// 按历史 ID 读取路径，文件只在资源管理器中定位，不执行文件本身。
#[tauri::command]
pub async fn clipboard_open_path(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    work(app, window, move |_, s| {
        let stored = s.data()?.repo.get(id)?;
        if stored.item.kind != Kind::Text {
            return Err("此条目不是文本。".into());
        }
        let path = classification::file_path(stored.text.as_deref().ok_or("历史文本损坏。")?)
            .ok_or("此条目不是有效文件路径。")?;
        crate::shared::platform::reveal_path(std::path::Path::new(path))
    })
    .await
}
#[tauri::command]
pub async fn clipboard_get_thumbnail(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<String, String> {
    item_work(app, window, move |_, s| {
        let file = {
            let d = s.data()?;
            let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
            let item = d.repo.get(id)?;
            let path = d.blobs.path(&item.thumb.ok_or("此条目不是图片。")?)?;
            std::fs::File::open(path).map_err(|_| "缩略图不可用。")?
        };
        data_url(
            file,
            256 * 1024,
            "data:image/webp;base64,",
            "缩略图读取失败。",
            "缩略图大小异常。",
        )
    })
    .await
}
/// 素材操作锁内只定位并打开文件，读取和 Base64 在锁外完成：新图片编码落盘也占这把锁，
/// 否则复制大图期间整屏缩略图都要排队。已打开的句柄不受随后删除影响，读到的仍是完整内容。
/// 直接写入最终字符串，避免 Base64 临时字符串与拼接结果同时驻留。
fn data_url(
    file: std::fs::File,
    limit: u64,
    prefix: &str,
    read_error: &'static str,
    size_error: &'static str,
) -> Result<String, String> {
    let bytes = read_limited(file, limit, read_error, size_error)?;
    let mut url = String::with_capacity(prefix.len() + bytes.len().div_ceil(3) * 4);
    url.push_str(prefix);
    STANDARD.encode_string(&bytes, &mut url);
    Ok(url)
}
fn read_limited(
    file: std::fs::File,
    limit: u64,
    read_error: &'static str,
    size_error: &'static str,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| read_error)?;
    if bytes.len() as u64 > limit {
        return Err(size_error.into());
    }
    Ok(bytes)
}

/// 可经 `snap-image` 协议读取历史原图的窗口（含它们打开的预览子窗口）；暂存架不开放原图。
pub(crate) const IMAGE_WINDOWS: [&str; 2] = service::UI_WINDOWS;

/// 双击预览按历史 ID 读取原始 PNG 字节，由只读协议直接交给 WebView；不接受路径，不改写系统剪贴板。
/// 条目不存在或不是图片时返回 None；锁内只定位并打开文件，读取在锁外完成。
pub(crate) fn protocol_image(app: &AppHandle, id: i64) -> Result<Option<Vec<u8>>, String> {
    let Some(s) = app.try_state::<Service>() else {
        return Ok(None);
    };
    let file = {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let Ok(stored) = d.repo.get(id) else {
            return Ok(None);
        };
        if stored.item.kind != Kind::Image {
            return Ok(None);
        }
        let path = d.blobs.path(&stored.blob.ok_or("历史图片损坏。")?)?;
        std::fs::File::open(path).map_err(|_| "历史图片不可用。")?
    };
    // 1600 万像素的 PNG 允许编码开销，但损坏的文件不能无限读入 WebView。
    read_limited(
        file,
        72 * 1024 * 1024,
        "历史图片读取失败。",
        "历史图片大小异常。",
    )
    .map(Some)
}

#[tauri::command]
pub async fn clipboard_restore(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    item_work(app, window, move |app, s| restore(app, s, id)).await
}
/// 把历史条目写回系统剪贴板，并记下序号与哈希，监听线程据此跳过这次自写入。
fn restore(app: &AppHandle, s: &Service, id: i64) -> Result<(), String> {
    let content = {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        d.content(id)?
    };
    write_back(app, s, &content)
}

/// 多选最多 100 条，与前端选择上限一致。
const MAX_SELECTION: usize = 100;
const MIXED_SELECTION: &str = "文本不能与图片或文件一起复制，请分开选择。";
const TOO_MANY_FILES: &str = "一次最多复制 100 个文件或图片，请减少选择。";

/// 多选复制的合并规则：只有一条时原样写回；全是文本时按选择顺序逐行拼接；
/// 全是图片或文件时合并为一组文件引用（重复路径只保留一次，图片由 `image_file` 写成临时 PNG），
/// 因为各平台剪贴板都没有通用的“多张图片”格式，而文件列表在资源管理器、Finder、文件管理器及聊天软件里都能粘贴。
/// 文本与图片/文件混选时拒绝，避免静默丢掉其中一类内容。
fn combine(
    contents: Vec<Content>,
    mut image_file: impl FnMut(&image::RgbaImage) -> Result<String, String>,
) -> Result<Content, String> {
    if contents.len() <= 1 {
        return contents
            .into_iter()
            .next()
            .ok_or_else(|| "请先选择要复制的条目。".to_string());
    }
    let texts = contents
        .iter()
        .filter(|content| matches!(content, Content::Text(_)))
        .count();
    if texts == contents.len() {
        let separator = if cfg!(windows) { "\r\n" } else { "\n" };
        let lines: Vec<String> = contents
            .into_iter()
            .filter_map(|content| match content {
                Content::Text(text) => Some(text),
                _ => None,
            })
            .collect();
        return Ok(Content::Text(lines.join(separator)));
    }
    if texts > 0 {
        return Err(MIXED_SELECTION.into());
    }
    // 先按条目数核对上限，再生成临时图片，避免超限时白写一批文件。
    let count: usize = contents
        .iter()
        .map(|content| match content {
            Content::Files(paths) => paths.len(),
            _ => 1,
        })
        .sum();
    if count > MAX_SELECTION {
        return Err(TOO_MANY_FILES.into());
    }
    let mut paths: Vec<String> = Vec::with_capacity(count);
    for content in contents {
        match content {
            Content::Files(list) => {
                for path in list {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
            Content::Image(image) => paths.push(image_file(&image)?),
            Content::Text(_) => unreachable!("文本已在上方处理"),
        }
    }
    files::validate(&paths)?;
    Ok(Content::Files(paths))
}

/// 按选择顺序读取多条历史并合并后写回剪贴板。
fn restore_many(app: &AppHandle, s: &Service, ids: &[i64]) -> Result<(), String> {
    if ids.is_empty() || ids.len() > MAX_SELECTION {
        return Err("请选择 1～100 条记录。".into());
    }
    let contents = {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        ids.iter()
            .map(|id| d.content(*id))
            .collect::<Result<Vec<_>, _>>()?
    };
    let content = combine(contents, |image| {
        crate::shared::native_drag::image_file(app, image)
    })?;
    write_back(app, s, &content)
}

/// 写回系统剪贴板，并记下序号与哈希，监听线程据此跳过这次自写入（合并结果同样不再记成新条目）。
fn write_back(app: &AppHandle, s: &Service, content: &Content) -> Result<(), String> {
    let _gate = s.write_gate.lock().map_err(|_| "剪贴板写入状态不可用。")?;
    match content {
        Content::Text(t) => app
            .clipboard()
            .write_text(t.clone())
            .map_err(|_| "复制文本失败，请重试。".to_string())?,
        Content::Image(i) => crate::shared::image_io::copy(app, i)?,
        Content::Files(paths) => platform::write_files(paths)?,
    }
    *s.restored.lock().map_err(|_| "恢复状态不可用。")? =
        Some((platform::sequence(), content.hash()));
    Ok(())
}
/// 回车、双击或 Ctrl+数字：写回系统剪贴板后把焦点交还唤出前的窗口并发送 Ctrl+V。
/// 返回 false 表示没有可粘贴的外部窗口（例如从主窗口唤出），此时内容只是已复制。
/// 仅限剪贴板窗口调用；`keep_open` 为窗口的固定状态，固定时不收起。
#[tauri::command]
pub async fn clipboard_paste(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
    keep_open: bool,
) -> Result<bool, String> {
    crate::capture::require_window(&window, window::LABEL)?;
    work(app, window, move |app, s| {
        restore(app, s, id)?;
        paste_into_target(app, keep_open)
    })
    .await
}

/// 多选复制：按选择顺序合并后写回剪贴板，规则见 [`combine`]。
#[tauri::command]
pub async fn clipboard_restore_many(
    app: AppHandle,
    window: WebviewWindow,
    ids: Vec<i64>,
) -> Result<(), String> {
    crate::capture::require_window(&window, window::LABEL)?;
    work(app, window, move |app, s| restore_many(app, s, &ids)).await
}

/// 多选粘贴：合并写回后与单条一样交还焦点并发送一次粘贴键。
#[tauri::command]
pub async fn clipboard_paste_many(
    app: AppHandle,
    window: WebviewWindow,
    ids: Vec<i64>,
    keep_open: bool,
) -> Result<bool, String> {
    crate::capture::require_window(&window, window::LABEL)?;
    work(app, window, move |app, s| {
        restore_many(app, s, &ids)?;
        paste_into_target(app, keep_open)
    })
    .await
}

/// 返回 false 表示没有可粘贴的外部窗口，内容只是已复制。
fn paste_into_target(app: &AppHandle, keep_open: bool) -> Result<bool, String> {
    let target = window::target();
    if target == 0 {
        return Ok(false);
    }
    // 先交还焦点再收起：收起后本程序不再是前台进程，系统可能拒绝切换前台窗口。
    platform::activate(target)?;
    if !keep_open {
        window::hide(app)?;
    }
    platform::send_paste(target)?;
    Ok(true)
}
#[tauri::command]
pub async fn clipboard_pin_item(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    item_work(app, window, move |app, s| {
        let content = {
            let d = s.data()?;
            let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
            d.content(id)?
        };
        match content {
            Content::Image(i) => crate::pins::create_visible(app, i, vec![]),
            Content::Text(text) => crate::pins::create_text_visible(app, &text),
            Content::Files(paths) => crate::pins::create_text_visible(app, &paths.join("\n")),
        }
    })
    .await
}
#[tauri::command]
pub async fn clipboard_set_favorite(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
    favorite: bool,
) -> Result<(), String> {
    work(app, window, move |app, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        d.repo.favorite(id, favorite)?;
        s.changed(app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn clipboard_delete(
    app: AppHandle,
    window: WebviewWindow,
    ids: Vec<i64>,
) -> Result<(), String> {
    validate_ids(&ids)?;
    work(app, window, move |app, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let removed = d.repo.delete(&ids)?;
        let cleanup = d.blobs.remove(&removed);
        s.changed(app);
        cleanup
    })
    .await
}
#[tauri::command]
pub async fn clipboard_clear(
    app: AppHandle,
    window: WebviewWindow,
    include_favorites: bool,
) -> Result<(), String> {
    work(app, window, move |app, s| {
        let d = s.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let removed = d.repo.clear(include_favorites)?;
        let cleanup = d.blobs.remove(&removed);
        s.changed(app);
        if cleanup.is_ok() {
            log::info!("event=clipboard.cleared module=clipboard");
        } else {
            log::warn!(
                "event=clipboard.cleanup_failed module=clipboard code=CLIPBOARD_CLEANUP_FAILED"
            );
        }
        cleanup
    })
    .await
}
/// 历史设置独立合并，防止快捷键页面的旧草稿重新打开已暂停的监听。
#[tauri::command]
pub async fn clipboard_configure(
    app: AppHandle,
    window: WebviewWindow,
    settings: ClipboardSettings,
) -> Result<Status, String> {
    settings.validate()?;
    work(app, window, move |app, s| {
        apply_settings(app, s, move |_| settings)
    })
    .await
}
/// 剪贴板窗口只切换记录启停；其余字段沿用已提交配置，窗口无需读取完整应用设置。
#[tauri::command]
pub async fn clipboard_set_enabled(
    app: AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> Result<Status, String> {
    work(app, window, move |app, s| {
        apply_settings(app, s, move |mut current| {
            current.enabled = enabled;
            current
        })
    })
    .await
}
/// 在设置写闸门内基于最新已提交配置生成新值，再依次重启监听、落盘和按新配额维护。
fn apply_settings(
    app: &AppHandle,
    s: &Service,
    change: impl FnOnce(ClipboardSettings) -> ClipboardSettings,
) -> Result<Status, String> {
    {
        let state = app.state::<crate::settings::SettingsState>();
        // 与普通设置保存共用写闸门；剪贴板启停和配置落盘都在设置数据锁之外。
        let _gate = state.1.gate().map_err(|_| "设置不可用。")?;
        let (old, mut next) = {
            let current = state.0.lock().map_err(|_| "设置不可用。")?;
            (current.value.clipboard.clone(), current.value.clone())
        };
        let settings = change(old.clone());
        next.clipboard = settings.clone();
        if let Err(error) = s.configure(app, settings) {
            let _ = s.configure(app, old.clone());
            s.report(app, error.clone());
            let status = s.status();
            for label in service::UI_WINDOWS {
                let _ = app.emit_to(label, "clipboard-state", &status);
            }
            return Err(error);
        }
        let path = app
            .path()
            .app_config_dir()
            .map_err(|_| "配置目录不可用。")?
            .join("settings.json");
        if let Err(e) = state.1.persist(&path, &next) {
            if let Err(rollback) = s.configure(app, old) {
                s.report(app, rollback);
            }
            return Err(e);
        }
        {
            let mut current = state.0.lock().map_err(|_| "设置不可用。")?;
            current.value = next.clone();
        }
        // 先保存配置成功再清理；配置写盘失败不能按未提交的配额删除历史。
        if let Err(error) = s.maintain(app) {
            s.report(app, error);
        }
        let _ = app.emit_to("main", "settings-changed", &next);
        let status = s.status();
        for label in service::UI_WINDOWS {
            let _ = app.emit_to(label, "clipboard-state", &status);
        }
        Ok(status)
    }
}
/// 完整配置提交成功后再按新配额维护，失败的导入不能清理历史。
pub(crate) fn settings_imported(app: &AppHandle) {
    let service = app.state::<Service>();
    if let Err(error) = service.maintain(app) {
        service.report(app, error);
    }
    let status = service.status();
    for label in service::UI_WINDOWS {
        let _ = app.emit_to(label, "clipboard-state", &status);
    }
}

/// 快捷键与托盘在主线程回调；建窗可能等待 UI 线程，必须移到阻塞任务中执行。
pub fn open_history(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = window::toggle(&app) {
            crate::capture::report_error(&app, error);
        }
    });
}

/// 由资源回收循环调用：收起较久的剪贴板窗口释放 WebView。
pub fn release_idle_window(app: &AppHandle) {
    window::release_idle(app);
}

/// 窗口失焦只转告前端：是否收起取决于固定状态及是否有预览、导出等子窗口。
/// WebView2 在点击页面、开始拖动时会把焦点在外层窗口与内部 WebView 之间转交，
/// 期间会先报一次失焦再立刻报获得焦点；因此延迟复核，仍未取得焦点才算真正离开。
pub fn window_blurred(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        window::remember_switched_target();
        let Some(window) = app.get_webview_window(window::LABEL) else {
            return;
        };
        if window.is_visible().unwrap_or(false) && !window.is_focused().unwrap_or(true) {
            let _ = app.emit_to(window::LABEL, "clipboard-blur", ());
        }
    });
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowAction {
    Open,
    Hide,
    Drag,
}

/// 主窗口只能唤出；收起与拖动仅限剪贴板窗口自身，其他窗口不能操纵它。
/// 同步命令在主线程执行：系统拖动必须在鼠标仍按下时由 UI 线程发起，否则按下已结束、拖动无效。
/// 显示与收起会等待 UI 线程处理窗口消息，因此移到后台线程，不能在这里阻塞主线程。
#[tauri::command]
pub fn clipboard_window_control(
    app: AppHandle,
    window: WebviewWindow,
    action: WindowAction,
) -> Result<(), String> {
    match action {
        WindowAction::Drag => {
            crate::capture::require_window(&window, window::LABEL)?;
            window.start_dragging().map_err(|e| e.to_string())
        }
        WindowAction::Open => {
            crate::capture::require_window(&window, "main")?;
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = window::show(&app) {
                    crate::capture::report_error(&app, error);
                }
            });
            Ok(())
        }
        WindowAction::Hide => {
            crate::capture::require_window(&window, window::LABEL)?;
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = window::hide(&app) {
                    crate::capture::report_error(&app, error);
                }
            });
            Ok(())
        }
    }
}

/// 按历史 ID 拖出原图、文件或文本；原图临时文件沿用贴图的生命周期管理。
#[tauri::command]
pub async fn clipboard_drag_item(
    app: AppHandle,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    use crate::shared::native_drag::{self, Payload};
    if !native_drag::SUPPORTED {
        return Err("当前平台尚未支持内容拖出。".into());
    }
    let source = window.clone();
    item_work(app, window, move |app, service| {
        let content = {
            let data = service.data()?;
            let _guard = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
            data.content(id)?
        };
        let payload = match content {
            Content::Text(text) => Payload::Text(text),
            Content::Files(paths) => Payload::Files(paths),
            Content::Image(image) => Payload::Files(vec![native_drag::image_file(app, &image)?]),
        };
        native_drag::start(&source, payload)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Content {
        Content::Text(value.into())
    }
    fn files(paths: &[&str]) -> Content {
        Content::Files(paths.iter().map(|p| p.to_string()).collect())
    }
    fn image() -> Content {
        Content::Image(image::RgbaImage::new(1, 1))
    }
    fn no_images(_: &image::RgbaImage) -> Result<String, String> {
        panic!("不应生成临时图片")
    }

    #[test]
    fn single_selection_is_written_back_unchanged() {
        assert!(matches!(combine(vec![image()], no_images), Ok(Content::Image(_))));
        assert!(combine(Vec::new(), no_images).is_err());
    }

    #[test]
    fn texts_join_in_selection_order_with_platform_line_breaks() {
        let separator = if cfg!(windows) { "\r\n" } else { "\n" };
        let Ok(Content::Text(joined)) = combine(vec![text("b"), text("a"), text("c")], no_images)
        else {
            panic!("应合并为文本");
        };
        assert_eq!(joined, ["b", "a", "c"].join(separator));
    }

    #[test]
    fn files_and_images_merge_into_one_deduplicated_file_list() {
        let mut made = 0;
        let result = combine(
            vec![
                files(&["C:\\a.txt", "C:\\b.txt"]),
                image(),
                files(&["C:\\b.txt", "C:\\c.txt"]),
            ],
            |_| {
                made += 1;
                Ok(format!("C:\\tmp\\{made}.png"))
            },
        );
        let Ok(Content::Files(paths)) = result else {
            panic!("应合并为文件列表");
        };
        assert_eq!(
            paths,
            ["C:\\a.txt", "C:\\b.txt", "C:\\tmp\\1.png", "C:\\c.txt"]
        );
    }

    #[test]
    fn mixed_text_and_over_limit_selections_are_rejected_before_writing_images() {
        assert_eq!(
            combine(vec![text("a"), image()], no_images).err().as_deref(),
            Some(MIXED_SELECTION)
        );
        let many: Vec<String> = (0..100).map(|i| format!("C:\\{i}.txt")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            combine(vec![files(&refs), image()], no_images).err().as_deref(),
            Some(TOO_MANY_FILES)
        );
    }
}
