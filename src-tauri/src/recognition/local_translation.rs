//! 本地翻译进程桥：固定安装路径、单任务、受限 JSONL；取消/超时丢弃 Future 即杀死进程。
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

#[derive(Default)]
pub struct LocalTranslationState(pub Arc<Mutex<Option<Worker>>>);
pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    model: String,
    /// 仅成功完成翻译后刷新；任务持锁期间回收器不会接触进程。
    last_used: Instant,
}

/// 只运行应用数据目录内经过固定摘要校验的引擎，不从 PATH 或语言包加载程序。
pub fn executable(app: &AppHandle) -> Result<PathBuf, String> {
    crate::recognition::translation_runtime::executable(app)
}

/// 诊断码与展示文案分离；下层资源错误保留给界面，不把路径或系统错误写入日志。
#[derive(Debug)]
pub struct LocalError {
    pub code: &'static str,
    pub message: String,
}
impl LocalError {
    fn new(code: &'static str, message: &str) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl From<String> for LocalError {
    fn from(message: String) -> Self {
        Self {
            code: "LOCAL_RESOURCE_INVALID",
            message,
        }
    }
}

impl Worker {
    /// 文件管理前等待进程退出，确保 Windows 释放模型文件句柄。
    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
    async fn start(app: &AppHandle) -> Result<Self, LocalError> {
        // 完整摘要校验涉及磁盘读取，移到阻塞池，避免占用异步运行时线程。
        let handle = app.clone();
        let path = tauri::async_runtime::spawn_blocking(move || executable(&handle))
            .await
            .map_err(|_| LocalError::new("LOCAL_ENGINE_VERIFY_FAILED", "翻译引擎校验任务失败。"))?
            .map_err(|message| LocalError {
                code: "LOCAL_ENGINE_VERIFY_FAILED",
                message,
            })?;
        let mut command = Command::new(&path);
        command
            .current_dir(path.parent().ok_or(LocalError::new(
                "LOCAL_ENGINE_PATH_INVALID",
                "翻译引擎路径无效。",
            ))?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW，避免截图时弹出控制台。
        let mut child = command.spawn().map_err(|_| {
            LocalError::new(
                "LOCAL_ENGINE_START_FAILED",
                "无法启动翻译引擎，请检查安装文件及运行库。",
            )
        })?;
        let input = child.stdin.take().ok_or(LocalError::new(
            "LOCAL_ENGINE_PIPE_UNAVAILABLE",
            "翻译输入管道不可用。",
        ))?;
        let output = BufReader::new(child.stdout.take().ok_or(LocalError::new(
            "LOCAL_ENGINE_PIPE_UNAVAILABLE",
            "翻译输出管道不可用。",
        ))?);
        let mut worker = Self {
            child,
            input,
            output,
            model: String::new(),
            last_used: Instant::now(),
        };
        worker
            .exchange(json!({"id":"health", "protocol":1, "action":"health"}))
            .await?;
        Ok(worker)
    }

    /// 每条消息有独立 ID；限制读取字节数，避免崩溃进程或异常输出耗尽主进程内存。
    async fn exchange(&mut self, request: Value) -> Result<Value, LocalError> {
        let mut bytes = serde_json::to_vec(&request)
            .map_err(|_| LocalError::new("LOCAL_REQUEST_ENCODE_FAILED", "翻译请求编码失败。"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(LocalError::new("LOCAL_REQUEST_TOO_LARGE", "翻译请求过大。"));
        }
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| LocalError::new("LOCAL_ENGINE_EXITED", "翻译引擎已退出，请重试。"))?;
        self.input
            .flush()
            .await
            .map_err(|_| LocalError::new("LOCAL_ENGINE_EXITED", "翻译引擎已退出。"))?;
        let mut response = Vec::new();
        (&mut self.output)
            .take(256 * 1024 + 1)
            .read_until(b'\n', &mut response)
            .await
            .map_err(|_| LocalError::new("LOCAL_RESPONSE_READ_FAILED", "翻译响应读取失败。"))?;
        if response.len() > 256 * 1024 || response.last() != Some(&b'\n') {
            return Err(LocalError::new(
                "LOCAL_RESPONSE_INCOMPLETE",
                "翻译引擎响应不完整或过大，请重试。",
            ));
        }
        let value: Value = serde_json::from_slice(&response)
            .map_err(|_| LocalError::new("LOCAL_PROTOCOL_INVALID", "翻译引擎协议错误。"))?;
        if value["protocol"] != 1 || value["id"] != request["id"] {
            return Err(LocalError::new(
                "LOCAL_RESPONSE_ID_MISMATCH",
                "翻译响应身份不匹配。",
            ));
        }
        if value["ok"] != true {
            return Err(match value["error"].as_str() {
                Some("input_too_long") => LocalError::new(
                    "LOCAL_INPUT_TOO_LONG",
                    "单行文字过长，请拆成较短的句子后重试。",
                ),
                Some("output_too_long") => LocalError::new(
                    "LOCAL_OUTPUT_TOO_LONG",
                    "译文达到长度上限，请拆分原文后重试。",
                ),
                _ => LocalError::new(
                    "LOCAL_ENGINE_REJECTED",
                    "本地翻译失败，请检查语言包与引擎版本。",
                ),
            });
        }
        Ok(value)
    }
}

/// 用目标语言明确选择方向，不对混合文本进行不可靠的自动语言检测。
pub async fn translate(
    app: &AppHandle,
    id: &str,
    target: &str,
    text: &str,
    timeout: u64,
) -> Result<String, LocalError> {
    if crate::resources::lifecycle::stopping(app) {
        return Err(LocalError::new("LOCAL_APP_STOPPING", "应用正在退出。"));
    }
    let model = crate::recognition::translation_models::model_id(target)?;
    let state = app.state::<LocalTranslationState>();
    let mut slot = state
        .0
        .try_lock()
        .map_err(|_| LocalError::new("LOCAL_BUSY", "本地翻译或语言包操作正在进行。"))?;
    // Worker 从缓存取出后由本 Future 独占。任何错误、取消、超时都会 Drop 并终止它。
    let cached = slot.take();
    let operation = async {
        let mut worker = match cached {
            Some(worker) => worker,
            None => Worker::start(app).await?,
        };
        if worker.model != model {
            let handle = app.clone();
            let model_name = model.to_string();
            let dir = tauri::async_runtime::spawn_blocking(move || {
                crate::recognition::translation_models::verified_directory(&handle, &model_name)
            })
            .await
            .map_err(|_| LocalError::new("LOCAL_MODEL_VERIFY_FAILED", "语言包检查任务失败。"))?
            .map_err(|message| LocalError {
                code: "LOCAL_MODEL_VERIFY_FAILED",
                message,
            })?;
            worker.exchange(json!({"id":"load", "protocol":1, "action":"load", "directory":dir, "modelId":model})).await?;
            worker.model = model.into();
        }
        let lines: Vec<&str> = text.split('\n').collect();
        if lines.len() > 64 {
            return Err(LocalError::new(
                "LOCAL_TOO_MANY_LINES",
                "一次最多翻译 64 行，请缩小选区。",
            ));
        }
        let segments: Vec<Value> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| json!({"id":i.to_string(),"text":line}))
            .collect();
        let response = worker
            .exchange(json!({"id":id,"protocol":1,"action":"translate","segments":segments}))
            .await?;
        let items = response["items"].as_array().ok_or(LocalError::new(
            "LOCAL_RESPONSE_SHAPE_INVALID",
            "翻译结果缺少段落。",
        ))?;
        if items.len() != lines.len() {
            return Err(LocalError::new(
                "LOCAL_RESPONSE_SHAPE_INVALID",
                "翻译结果段落数量不匹配。",
            ));
        }
        let mut output = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if item["id"].as_str() != Some(i.to_string().as_str()) {
                return Err(LocalError::new(
                    "LOCAL_RESPONSE_SHAPE_INVALID",
                    "翻译结果段落顺序不匹配。",
                ));
            }
            let value = item["text"].as_str().ok_or(LocalError::new(
                "LOCAL_RESPONSE_SHAPE_INVALID",
                "翻译结果文本无效。",
            ))?;
            if !lines[i].trim().is_empty() && value.trim().is_empty() {
                return Err(LocalError::new("LOCAL_EMPTY_OUTPUT", "翻译结果为空。"));
            }
            output.push(value.to_string());
        }
        worker.last_used = Instant::now();
        if !crate::resources::lifecycle::stopping(app) {
            *slot = Some(worker);
        }
        Ok(output.join("\n"))
    };
    tokio::time::timeout(Duration::from_secs(timeout), operation)
        .await
        .map_err(|_| LocalError::new("LOCAL_TIMEOUT", "本地翻译超时，已停止引擎，可重试。"))?
}

/// 使用最新保存的闲置秒数；0 表示保持缓存。先释放设置锁，再尝试 Worker 锁。
pub async fn release_idle(app: &AppHandle) {
    let seconds = match app.state::<crate::settings::SettingsState>().0.try_lock() {
        Ok(settings) => settings.value.ai.local_idle_seconds,
        Err(_) => return,
    };
    let state = app.state::<LocalTranslationState>();
    // try_lock_owned 避免等待翻译；停止进程期间继续持锁，防止新旧 Worker 重叠驻留。
    let Ok(mut slot) = state.0.clone().try_lock_owned() else {
        return;
    };
    if slot
        .as_ref()
        .is_some_and(|worker| idle_expired(worker.last_used.elapsed(), seconds))
    {
        if let Some(worker) = slot.take() {
            worker.stop().await;
            log::info!("event=translation.worker_stopped module=translation reason=idle");
        }
    }
}

fn idle_expired(elapsed: Duration, seconds: u64) -> bool {
    seconds != 0 && elapsed >= Duration::from_secs(seconds)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn idle_deadline_respects_disable_and_exact_boundary() {
        assert!(!idle_expired(Duration::from_secs(3601), 0));
        assert!(!idle_expired(Duration::from_millis(299_999), 300));
        assert!(idle_expired(Duration::from_secs(300), 300));
        assert!(idle_expired(Duration::from_secs(301), 300));
    }
}

/// 正常退出显式终止缓存进程；活动任务由现有取消信号清理。
pub fn shutdown(app: &AppHandle) {
    if let Ok(mut slot) = app.state::<LocalTranslationState>().0.try_lock() {
        if let Some(mut worker) = slot.take() {
            let _ = worker.child.start_kill();
        }
    }
}
