//! 串行素材操作锁协调文件与数据库；连接锁只在 Repository 方法内短暂持有。
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
use tauri::{AppHandle, Emitter, Manager};

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
/// state 总是注册，即使数据库损坏，页面仍能查询错误且 F2 完全独立。
pub struct Service {
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
    pub fn new(app: &AppHandle) -> Self {
        let data = (|| {
            let root = app
                .path()
                .app_local_data_dir()
                .map_err(|_| "应用数据目录不可用。")?
                .join("clipboard");
            let blobs = BlobStore::new(root.clone())?;
            let repo = Repository::open(&root.join("history.db"))?;
            blobs.repair(&repo)?;
            Ok(Arc::new(Data {
                repo,
                blobs,
                operation: Mutex::new(()),
            }))
        })();
        Self {
            data,
            running: Mutex::new(None),
            settings: Mutex::new(ClipboardSettings::default()),
            error: Mutex::new(None),
            capture_error: Mutex::new(None),
            revision: AtomicU64::new(0),
            restored: Mutex::new(None),
            write_gate: Mutex::new(()),
            exporting: AtomicBool::new(false),
        }
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
            supported: cfg!(target_os = "windows"),
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
        }
    }
    pub fn changed(&self, app: &AppHandle) {
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = app.emit_to(
            "main",
            "clipboard-changed",
            serde_json::json!({"revision":revision}),
        );
    }
    pub fn report(&self, app: &AppHandle, error: String) {
        if let Ok(mut e) = self.error.lock() {
            *e = Some(error.clone());
        }
        let _ = app.emit_to("main", "clipboard-error", error);
    }
    fn report_capture(&self, app: &AppHandle, error: String) {
        if let Ok(mut current) = self.capture_error.lock() {
            *current = Some(error.clone());
        }
        let _ = app.emit_to("main", "clipboard-error", error);
    }
    /// 保持既有字符串事件契约；有持续故障就继续显示，否则空字符串撤下单次错误。
    /// 不调用 status()，避免 worker 被 configure 等待退出时重入 running 锁。
    fn recovered_capture(&self, app: &AppHandle) {
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
            let _ = app.emit_to("main", "clipboard-error", message);
        }
    }
    /// 只由后台命令或 setup 调用。先停旧监听并等待在途写入，再替换配置。
    pub fn configure(&self, app: &AppHandle, settings: ClipboardSettings) -> Result<(), String> {
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
        let handle = app.clone();
        let worker = std::thread::Builder::new()
            .name("clipboard-storage".into())
            .spawn(move || {
                let mut last = baseline;
                let interval = std::time::Duration::from_secs(3600);
                let mut maintenance = std::time::Instant::now();
                loop {
                    let notification =
                        rx.recv_timeout(interval.saturating_sub(maintenance.elapsed()));
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let service = handle.state::<Service>();
                    // 持续复制也按时维护，不能每次通知都重新开始一小时倒计时。
                    if maintenance.elapsed() >= interval {
                        if let Err(e) = service.maintain(&handle) {
                            service.report(&handle, e);
                        }
                        maintenance = std::time::Instant::now();
                    }
                    match notification {
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            continue;
                        }
                        Ok(()) => {}
                    }
                    let seq = platform::sequence();
                    if seq == last {
                        continue;
                    }
                    match service.capture(&handle, &worker_stop) {
                        Ok(()) => last = seq,
                        Err(e) => service.report_capture(&handle, e),
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
    pub(super) fn maintain(&self, app: &AppHandle) -> Result<(), String> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| "历史设置不可用。")?
            .clone();
        let data = self.data()?;
        let _guard = data.operation.lock().map_err(|_| "历史存储忙碌。")?;
        data.repo.prune(&settings, now())?;
        data.blobs.repair(&data.repo)?;
        self.changed(app);
        Ok(())
    }
    fn capture(&self, app: &AppHandle, stop: &AtomicBool) -> Result<(), String> {
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
                    self.changed(app);
                    return Err(error);
                }
            }
            self.recovered_capture(app);
            self.changed(app);
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
}
