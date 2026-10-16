//! 本地翻译进程桥：固定安装路径、单任务、受限 JSONL；取消/超时丢弃 Future 即杀死进程。
use serde_json::{json, Value};
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
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
}

/// 只运行随应用发布的程序，绝不从 PATH 或模型包加载可执行文件。
pub fn executable(app: &AppHandle) -> Result<PathBuf, String> {
    let name = if cfg!(windows) {
        "snap-translator-worker.exe"
    } else {
        "snap-translator-worker"
    };
    #[cfg(debug_assertions)]
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("translator-runtime");
    #[cfg(not(debug_assertions))]
    let base = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("translator-runtime");
    let _ = app;
    let path = base.join(name);
    if !path.is_file() {
        return Err("此安装缺少本地翻译引擎，请安装包含原生翻译引擎的版本。".into());
    }
    Ok(path)
}

impl Worker {
    /// 文件管理前等待进程退出，确保 Windows 释放模型文件句柄。
    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
    async fn start(app: &AppHandle) -> Result<Self, String> {
        let path = executable(app)?;
        let mut command = Command::new(&path);
        command
            .current_dir(path.parent().ok_or("翻译引擎路径无效。")?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW，避免截图时弹出控制台。
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动翻译引擎，请检查安装文件及运行库。")?;
        let input = child.stdin.take().ok_or("翻译输入管道不可用。")?;
        let output = BufReader::new(child.stdout.take().ok_or("翻译输出管道不可用。")?);
        let mut worker = Self {
            child,
            input,
            output,
            model: String::new(),
        };
        worker
            .exchange(json!({"id":"health", "protocol":1, "action":"health"}))
            .await?;
        Ok(worker)
    }

    /// 每条消息有独立 ID；限制读取字节数，避免崩溃进程或异常输出耗尽主进程内存。
    async fn exchange(&mut self, request: Value) -> Result<Value, String> {
        let mut bytes = serde_json::to_vec(&request).map_err(|_| "翻译请求编码失败。")?;
        if bytes.len() > 1024 * 1024 {
            return Err("翻译请求过大。".into());
        }
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| "翻译引擎已退出，请重试。")?;
        self.input.flush().await.map_err(|_| "翻译引擎已退出。")?;
        let mut response = Vec::new();
        (&mut self.output)
            .take(256 * 1024 + 1)
            .read_until(b'\n', &mut response)
            .await
            .map_err(|_| "翻译响应读取失败。")?;
        if response.len() > 256 * 1024 || response.last() != Some(&b'\n') {
            return Err("翻译引擎响应不完整或过大，请重试。".into());
        }
        let value: Value = serde_json::from_slice(&response).map_err(|_| "翻译引擎协议错误。")?;
        if value["protocol"] != 1 || value["id"] != request["id"] {
            return Err("翻译响应身份不匹配。".into());
        }
        if value["ok"] != true {
            return Err(match value["error"].as_str() {
                Some("input_too_long") => "单行文字过长，请拆成较短的句子后重试。",
                Some("output_too_long") => "译文达到长度上限，请拆分原文后重试。",
                _ => "本地翻译失败，请检查语言包与引擎版本。",
            }
            .into());
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
) -> Result<String, String> {
    let model = crate::translation_models::model_id(target)?;
    let state = app.state::<LocalTranslationState>();
    let mut slot = state
        .0
        .try_lock()
        .map_err(|_| "本地翻译或语言包操作正在进行。")?;
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
                crate::translation_models::verified_directory(&handle, &model_name)
            })
            .await
            .map_err(|_| "语言包检查任务失败。")??;
            worker.exchange(json!({"id":"load", "protocol":1, "action":"load", "directory":dir, "modelId":model})).await?;
            worker.model = model.into();
        }
        let lines: Vec<&str> = text.split('\n').collect();
        if lines.len() > 64 {
            return Err("一次最多翻译 64 行，请缩小选区。".into());
        }
        let segments: Vec<Value> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| json!({"id":i.to_string(),"text":line}))
            .collect();
        let response = worker
            .exchange(json!({"id":id,"protocol":1,"action":"translate","segments":segments}))
            .await?;
        let items = response["items"].as_array().ok_or("翻译结果缺少段落。")?;
        if items.len() != lines.len() {
            return Err("翻译结果段落数量不匹配。".into());
        }
        let mut output = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if item["id"].as_str() != Some(i.to_string().as_str()) {
                return Err("翻译结果段落顺序不匹配。".into());
            }
            let value = item["text"].as_str().ok_or("翻译结果文本无效。")?;
            if !lines[i].trim().is_empty() && value.trim().is_empty() {
                return Err("翻译结果为空。".into());
            }
            output.push(value.to_string());
        }
        *slot = Some(worker);
        Ok(output.join("\n"))
    };
    tokio::time::timeout(Duration::from_secs(timeout), operation)
        .await
        .map_err(|_| "本地翻译超时，已停止引擎，可重试。")?
}

/// 正常退出显式终止缓存进程；活动任务由现有取消信号清理。
pub fn shutdown(app: &AppHandle) {
    if let Ok(mut slot) = app.state::<LocalTranslationState>().0.try_lock() {
        if let Some(mut worker) = slot.take() {
            let _ = worker.child.start_kill();
        }
    }
}
