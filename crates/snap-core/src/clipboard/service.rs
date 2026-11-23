//! 剪贴板历史记录服务：平台监听 → 工作线程读取、去重、写入 SQLite 与图片仓库，定时维护。
//! 串行素材操作锁协调文件与数据库；连接锁只在 Repository 方法内短暂持有。
//! 界面通知经构造时注入的回调发出（通知历史窗口），本模块不依赖界面框架。
//! `Service` 是可克隆的句柄（内部共享），工作线程持有一份。
use super::{
    blob_store::BlobStore,
    model::*,
    platform,
    repository::{NewItem, Repository},
};
use image::RgbaImage;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
};

/// 发给界面的通知。
#[derive(Clone, Debug)]
pub enum Event {
    /// 历史或素材有变化；revision 单调递增。
    Changed(u64),
    /// 当前要显示的错误；空字符串表示撤下单次复制错误。
    Error(String),
}

pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;

pub struct Data {
    pub repo: Repository,
    pub blobs: BlobStore,
    pub operation: Mutex<()>,
}
struct Running {
    watcher: Option<platform::Watcher>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.watcher.take(); // 注销并关闭发送端，唤醒等候中的工作线程。
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// 即使数据库损坏也能创建，页面仍能查询错误且贴图入口完全独立。
#[derive(Clone)]
pub struct Service(Arc<Inner>);

impl std::ops::Deref for Service {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

pub struct Inner {
    notify: Notify,
    pub data: Result<Arc<Data>, String>,
    running: Mutex<Option<Running>>,
    pub settings: Mutex<ClipboardSettings>,
    pub error: Mutex<Option<String>>,
    /// 单次复制失败与监听/维护故障独立，正常记录恢复时不能掩盖持续故障。
    capture_error: Mutex<Option<String>>,
    pub revision: AtomicU64,
    /// 恢复操作在写入锁内记录内容摘要，worker 使用同一锁避免先读后标记的竞态。
    pub restored: Mutex<Option<(u32, String)>>,
    pub write_gate: Mutex<()>,
    /// 防止切换页面后再次导出导致多个 PDF/长图编码同时占用大量内存。
    pub exporting: AtomicBool,
}
/// 监听通知之外的兜底检查间隔；只比较系统序列号，不打开剪贴板。
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);
/// 同一份内容连续读取失败的上限，超过后放弃，避免超限内容被无限重试。
const CAPTURE_ATTEMPTS: u8 = 3;
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn image_hash(image: &RgbaImage) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(&image.width().to_le_bytes());
    hash.update(&image.height().to_le_bytes());
    hash.update(image.as_raw());
    hash.finalize().to_hex().to_string()
}

/// 单次复制恢复只撤下自己的错误；持续故障仍作为页面当前错误发布。
fn capture_recovery_message(
    capture_error: &mut Option<String>,
    persistent_error: Option<&str>,
) -> Option<String> {
    capture_error
        .take()
        .map(|_| persistent_error.unwrap_or_default().to_string())
}

impl Service {
    /// `root` 为历史目录（见 `paths::clipboard_dir`）。
    pub fn new(root: Result<std::path::PathBuf, String>, notify: Notify) -> Self {
        let data = (|| {
            let root = root?;
            let blobs = BlobStore::new(root.clone())?;
            let repo = Repository::open(&root.join("history.db"))?;
            // 素材修复交给紧随其后的后台 maintain：这里位于 setup 线程，不逐个核对图片文件。
            Ok(Arc::new(Data {
                repo,
                blobs,
                operation: Mutex::new(()),
            }))
        })();
        Self(Arc::new(Inner {
            notify,
            data,
            running: Mutex::new(None),
            settings: Mutex::new(ClipboardSettings::default()),
            error: Mutex::new(None),
            capture_error: Mutex::new(None),
            revision: AtomicU64::new(0),
            restored: Mutex::new(None),
            write_gate: Mutex::new(()),
            exporting: AtomicBool::new(false),
        }))
    }
    pub fn data(&self) -> Result<&Arc<Data>, String> {
        self.data.as_ref().map_err(Clone::clone)
    }
    pub fn status(&self) -> Status {
        let (count, byte_size) = self
            .data()
            .and_then(|d| d.repo.totals())
            .unwrap_or_default();
        Status {
            enabled: self.running.lock().map(|r| r.is_some()).unwrap_or(false),
            supported: cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux")),
            count,
            byte_size,
            revision: self.revision.load(Ordering::Acquire),
            error: self
                .data
                .as_ref()
                .err()
                .cloned()
                .or_else(|| self.error.lock().ok().and_then(|e| e.clone()))
                .or_else(|| self.capture_error.lock().ok().and_then(|e| e.clone())),
            paste_on_activate: self
                .settings
                .lock()
                .map(|settings| settings.paste_on_activate)
                .unwrap_or(false),
        }
    }
    pub fn changed(&self) {
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        (self.notify)(Event::Changed(revision));
    }
    pub fn report(&self, error: String) {
        if let Ok(mut e) = self.error.lock() {
            *e = Some(error.clone());
        }
        (self.notify)(Event::Error(error));
    }
    fn report_capture(&self, error: String) {
        if let Ok(mut current) = self.capture_error.lock() {
            *current = Some(error.clone());
        }
        (self.notify)(Event::Error(error));
    }
    /// 保持既有字符串事件契约；有持续故障就继续显示，否则空字符串撤下单次错误。
    /// 不调用 status()，避免 worker 被 configure 等待退出时重入 running 锁。
    fn recovered_capture(&self) {
        let Ok(mut capture_error) = self.capture_error.lock() else {
            return;
        };
        if capture_error.is_none() {
            return;
        }
        let Ok(persistent_error) = self.error.lock() else {
            return;
        };
        if let Some(message) =
            capture_recovery_message(&mut capture_error, persistent_error.as_deref())
        {
            (self.notify)(Event::Error(message));
        }
    }
    /// 只由后台命令或 setup 调用。先停旧监听并等待在途写入，再替换配置。
    pub fn configure(&self, settings: ClipboardSettings) -> Result<(), String> {
        settings.validate()?;
        let mut running = self.running.lock().map_err(|_| "监听状态不可用。")?;
        running.take();
        *self.settings.lock().map_err(|_| "历史设置不可用。")? = settings.clone();
        if !settings.enabled {
            *self.error.lock().map_err(|_| "历史状态不可用。")? = None;
            *self.capture_error.lock().map_err(|_| "历史状态不可用。")? = None;
            return Ok(());
        }
        self.data()?;
        let (tx, rx) = mpsc::sync_channel(1);
        // 注册时不主动读取已有内容；只有启用后的系统变化会进入队列。
        let baseline = platform::sequence();
        let watcher = platform::Watcher::start(tx)?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let service = self.clone();
        let worker = std::thread::Builder::new()
            .name("clipboard-storage".into())
            .spawn(move || {
                let mut last = baseline;
                let mut failures = 0u8;
                let interval = std::time::Duration::from_secs(3600);
                let mut maintenance = std::time::Instant::now();
                loop {
                    // 系统通知可能丢失（远程桌面重连、会话切换、其他程序长时间占用剪贴板），
                    // 因此最多等待一个轮询周期就核对一次序列号，漏掉的复制也能补记。
                    let wait = interval
                        .saturating_sub(maintenance.elapsed())
                        .min(POLL_INTERVAL);
                    let notification = rx.recv_timeout(wait);
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    // 持续复制也按时维护，不能每次通知都重新开始一小时倒计时。
                    if maintenance.elapsed() >= interval {
                        if let Err(e) = service.maintain() {
                            service.report(e);
                        }
                        maintenance = std::time::Instant::now();
                    }
                    if matches!(notification, Err(mpsc::RecvTimeoutError::Disconnected)) {
                        break;
                    }
                    // 通知与轮询走同一判断：序列号没变就没有新内容。
                    let seq = platform::sequence();
                    if seq == last {
                        continue;
                    }
                    match service.capture(&worker_stop) {
                        Ok(()) => {
                            last = seq;
                            failures = 0;
                        }
                        Err(e) => {
                            failures += 1;
                            // 首次失败即提示；之后每个轮询周期自动重试，成功时由 capture 撤下提示。
                            if failures == 1 {
                                log::warn!("event=clipboard.capture_failed module=clipboard code=CLIPBOARD_CAPTURE_FAILED");
                                service.report_capture(e);
                            }
                            if failures >= CAPTURE_ATTEMPTS {
                                last = seq;
                                failures = 0;
                            }
                        }
                    }
                }
            })
            .map_err(|_| "无法启动历史存储线程。")?;
        *running = Some(Running {
            watcher: Some(watcher),
            stop,
            worker: Some(worker),
        });
        *self.error.lock().map_err(|_| "历史状态不可用。")? = None;
        *self.capture_error.lock().map_err(|_| "历史状态不可用。")? = None;
        Ok(())
    }
    pub fn shutdown(&self) {
        if let Ok(mut running) = self.running.lock() {
            running.take();
        }
    }
    pub fn maintain(&self) -> Result<(), String> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| "历史设置不可用。")?
            .clone();
        let data = self.data()?;
        let _guard = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
        // 收藏占满配额时淘汰会回滚并报错；素材修复与此无关，仍然执行，
        // 否则启动时丢失文件的记录和孤儿素材要等配额问题解决后才会清理。
        let pruned = data.repo.prune(&settings, now()).map(|_| ());
        data.blobs.repair(&data.repo)?;
        self.changed();
        pruned
    }
    fn capture(&self, stop: &AtomicBool) -> Result<(), String> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| "历史设置不可用。")?
            .clone();
        let data = self.data()?;
        let mut last_error = "剪贴板变化过快，未记录本次内容。".to_string();
        for delay in [0, 30, 80, 160] {
            if delay > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let _write = self
                .write_gate
                .lock()
                .map_err(|_| "剪贴板写入状态不可用。")?;
            let seq = platform::sequence();
            let content = match super::reader::read(&settings) {
                Ok(Some(content)) => content,
                Ok(None) => return Ok(()),
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };
            let kind = match &content {
                Content::Text(_) => Kind::Text,
                Content::Image(_) => Kind::Image,
                Content::Files(_) => Kind::File,
            };
            if seq != platform::sequence() {
                continue;
            }
            let hash = content.hash();
            if self
                .restored
                .lock()
                .map_err(|_| "恢复状态不可用。")?
                .as_ref()
                .is_some_and(|(s, h)| *s == seq && *h == hash)
            {
                return Ok(());
            }
            if let Content::Text(text) = &content {
                if text.is_empty() {
                    return Ok(());
                }
                if text.len() > settings.max_text_bytes as usize {
                    return Err("文本超过历史大小限制，未记录。".into());
                }
            }
            drop(_write);
            let _operation = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            if !data.repo.refresh(kind, &hash, now())? {
                let removed = match content {
                    Content::Files(paths) => {
                        let text = paths.join("\n");
                        data.repo.insert(
                            NewItem {
                                kind,
                                hash: &hash,
                                text: Some(&text),
                                blob: None,
                                thumb: None,
                                width: None,
                                height: None,
                                bytes: text.len() as u64,
                            },
                            &settings,
                            now(),
                        )?
                    }
                    Content::Text(text) => data.repo.insert(
                        NewItem {
                            kind,
                            hash: &hash,
                            text: Some(&text),
                            blob: None,
                            thumb: None,
                            width: None,
                            height: None,
                            bytes: text.len() as u64,
                        },
                        &settings,
                        now(),
                    )?,
                    Content::Image(image) => {
                        let saved = data.blobs.save(&hash, &image)?;
                        let removed = data.repo.insert(
                            NewItem {
                                kind,
                                hash: &hash,
                                text: None,
                                blob: Some(&saved.blob),
                                thumb: Some(&saved.thumb),
                                width: Some(image.width()),
                                height: Some(image.height()),
                                bytes: saved.bytes,
                            },
                            &settings,
                            now(),
                        )?;
                        saved.commit();
                        removed
                    }
                };
                // 数据库提交后只删除本次淘汰的路径，不在每次复制时全目录扫描。
                let cleanup = data.blobs.remove(&removed);
                if let Err(error) = cleanup {
                    self.changed();
                    return Err(error);
                }
            }
            self.recovered_capture();
            self.changed();
            return Ok(());
        }
        Err(last_error)
    }
}
pub enum Content {
    Text(String),
    Image(RgbaImage),
    Files(Vec<String>),
}
impl Content {
    pub fn hash(&self) -> String {
        match self {
            Self::Text(t) => blake3::hash(t.as_bytes()).to_hex().to_string(),
            Self::Image(i) => image_hash(i),
            Self::Files(paths) => blake3::hash(paths.join("\n").as_bytes())
                .to_hex()
                .to_string(),
        }
    }
}
impl Data {
    /// 调用方持 operation 锁取得快照，之后可释放锁再弹窗/写剪贴板/创建贴图。
    pub fn content(&self, id: i64) -> Result<Content, String> {
        let item = self.repo.get(id)?;
        match item.item.kind {
            Kind::File => Ok(Content::Files(super::files::from_stored(
                &item.text.ok_or("文件引用损坏。")?,
            )?)),
            Kind::Text => Ok(Content::Text(item.text.ok_or("历史文本损坏。")?)),
            Kind::Image => Ok(Content::Image(crate::pins::content::read_image(
                &self.blobs.path(&item.blob.ok_or("图片引用损坏。")?)?,
            )?)),
        }
    }
}

impl Service {
    /// 收藏 / 取消收藏。
    pub fn set_favorite(&self, id: i64, favorite: bool) -> Result<(), String> {
        let d = self.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        d.repo.favorite(id, favorite)?;
        self.changed();
        Ok(())
    }

    /// 删除条目及其图片素材。
    pub fn delete(&self, ids: &[i64]) -> Result<(), String> {
        validate_ids(ids)?;
        let d = self.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let removed = d.repo.delete(ids)?;
        let cleanup = d.blobs.remove(&removed);
        self.changed();
        cleanup
    }

    /// 清空历史；`include_favorites` 为假时保留收藏。
    pub fn clear(&self, include_favorites: bool) -> Result<(), String> {
        let d = self.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        let removed = d.repo.clear(include_favorites)?;
        let cleanup = d.blobs.remove(&removed);
        self.changed();
        if cleanup.is_ok() {
            log::info!("event=clipboard.cleared module=clipboard");
        } else {
            log::warn!("event=clipboard.cleanup_failed module=clipboard code=CLIPBOARD_CLEANUP_FAILED");
        }
        cleanup
    }

    /// 图片条目的缩略图（WebP，≤ 256 KiB）。锁内只打开文件，读取在锁外完成。
    pub fn thumbnail(&self, id: i64) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let file = {
            let d = self.data()?;
            let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
            let item = d.repo.get(id)?;
            let path = d.blobs.path(&item.thumb.ok_or("此条目不是图片。")?)?;
            std::fs::File::open(path).map_err(|_| "缩略图不可用。")?
        };
        let limit = 256 * 1024;
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes).map_err(|_| "缩略图读取失败。")?;
        if bytes.len() as u64 > limit {
            return Err("缩略图大小异常。".into());
        }
        Ok(bytes)
    }

    /// 按选择顺序读取多条内容（多选复制/粘贴前合并用）。
    pub fn contents(&self, ids: &[i64]) -> Result<Vec<Content>, String> {
        if ids.is_empty() || ids.len() > MAX_SELECTION {
            return Err("请选择 1～100 条记录。".into());
        }
        let d = self.data()?;
        let _guard = d.operation.lock().map_err(|_| "历史存储忙碌。")?;
        ids.iter().map(|id| d.content(*id)).collect()
    }

    /// 写回系统剪贴板后调用：记下序号与哈希，监听线程据此跳过这次自写入。
    /// 调用方应在写入前取得 `write_gate`，写入与标记之间不让监听线程插入读取。
    pub fn mark_restored(&self, content: &Content) -> Result<(), String> {
        *self.restored.lock().map_err(|_| "恢复状态不可用。")? =
            Some((platform::sequence(), content.hash()));
        Ok(())
    }
}

/// 多选最多 100 条，与前端选择上限一致。
pub const MAX_SELECTION: usize = 100;
pub const MIXED_SELECTION: &str = "文本不能与图片或文件一起复制，请分开选择。";
pub const TOO_MANY_FILES: &str = "一次最多复制 100 个文件或图片，请减少选择。";

/// 多选复制的合并规则：只有一条时原样写回；全是文本时按选择顺序逐行拼接；
/// 全是图片或文件时合并为一组文件引用（重复路径只保留一次，图片由 `image_file` 写成临时 PNG），
/// 因为各平台剪贴板都没有通用的“多张图片”格式，而文件列表在资源管理器、Finder、文件管理器及聊天软件里都能粘贴。
/// 文本与图片/文件混选时拒绝，避免静默丢掉其中一类内容。
pub fn combine(
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
    super::files::validate(&paths)?;
    Ok(Content::Files(paths))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_recovery_preserves_persistent_failure_and_only_clears_a_reported_capture_error() {
        let mut transient = Some("上一次复制失败。".to_string());
        assert_eq!(
            capture_recovery_message(&mut transient, Some("历史素材维护失败。")),
            Some("历史素材维护失败。".to_string())
        );
        assert!(transient.is_none());
        // 未报告新复制故障时不发恢复事件，避免覆盖页面其他操作的错误。
        assert_eq!(
            capture_recovery_message(&mut transient, Some("历史素材维护失败。")),
            None
        );
        transient = Some("文本超过大小限制。".to_string());
        assert_eq!(
            capture_recovery_message(&mut transient, None),
            Some(String::new())
        );
        assert!(transient.is_none());
    }

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

#[cfg(test)]
mod seed {
    /// 开发辅助：向 `SNAP_SEED_DIR` 指定的空目录写入示例历史（文本、链接、颜色、路径、图片），
    /// 用于在不改动真实剪贴板的情况下查看历史窗口。`cargo test -p snap-core seed_history -- --ignored`。
    #[test]
    #[ignore]
    fn seed_history() {
        use super::super::{blob_store::BlobStore, model::*, repository::{NewItem, Repository}};
        let Ok(dir) = std::env::var("SNAP_SEED_DIR") else { return };
        let root = std::path::PathBuf::from(dir);
        let repo = Repository::open(&root.join("history.db")).unwrap();
        let blobs = BlobStore::new(root.clone()).unwrap();
        let settings = ClipboardSettings::default();
        let now = super::now();
        for (i, text) in ["会议纪要：周五前提交迁移清单", "https://example.com/docs/gpui", "#2563eb", r"C:\Users\Public\Documents\report.pdf", "fn main() {}"].iter().enumerate() {
            let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
            repo.insert(NewItem { kind: Kind::Text, hash: &hash, text: Some(text), blob: None, thumb: None, width: None, height: None, bytes: text.len() as u64 }, &settings, now - (i as i64) * 90_000).unwrap();
        }
        let image = image::RgbaImage::from_fn(320, 180, |x, y| image::Rgba([(x % 256) as u8, (y * 255 / 180) as u8, 180, 255]));
        let hash = super::image_hash(&image);
        let saved = blobs.save(&hash, &image).unwrap();
        repo.insert(NewItem { kind: Kind::Image, hash: &hash, text: None, blob: Some(&saved.blob), thumb: Some(&saved.thumb), width: Some(320), height: Some(180), bytes: saved.bytes }, &settings, now - 3_600_000).unwrap();
        saved.commit();
    }
}

