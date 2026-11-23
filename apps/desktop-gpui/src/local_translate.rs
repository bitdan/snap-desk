//! 本地 CTranslate2 翻译（对照 Tauri `recognition/local_translation.rs`）。
//!
//! 协议、校验、worker 生命周期与语言包安装都在 core（`snap_core::recognition::local_*`）。缓存的 worker 进程
//! 绑定创建它的 tokio 运行时，因此这里持有一个常驻运行时，所有本地翻译与语言包管理都在其上执行；
//! GPUI 后台线程以 `block_on` 等待结果。闲置回收每分钟检查一次（秒数取设置）；退出时终止缓存进程。
use gpui_kit::*;
use snap_core::recognition::{
    local_models,
    local_worker::{self, LocalTranslationState},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::Duration,
};

static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
static STATE: OnceLock<LocalTranslationState> = OnceLock::new();
static STOPPING: AtomicBool = AtomicBool::new(false);

fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("local-translation")
                .enable_all()
                .build()
                .map_err(|_| "无法初始化本地翻译运行时。".to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn state() -> &'static LocalTranslationState {
    STATE.get_or_init(LocalTranslationState::default)
}

pub fn resources() -> Result<PathBuf, String> {
    snap_core::resources::directory().ok_or_else(|| "应用数据目录不可用。".into())
}

/// 逐段翻译（阻塞，在后台线程调用）。`timeout` 为每段上限秒数。
pub fn translate(texts: &[String], target: &str, timeout: u64) -> Result<Vec<String>, String> {
    let runtime = runtime()?;
    let dir = resources()?;
    let stopping = || STOPPING.load(Ordering::Relaxed);
    runtime.block_on(async {
        let mut output = Vec::with_capacity(texts.len());
        for (index, text) in texts.iter().enumerate() {
            let id = format!("gpui-{index}");
            let value =
                local_worker::translate(state(), &dir, &stopping, &id, target, text, timeout)
                    .await
                    .map_err(|error| {
                        log::warn!("event=gpui.local_translation_failed code={}", error.code);
                        error.message
                    })?;
            output.push(value);
        }
        Ok(output)
    })
}

/// 停止缓存的 worker 并持有互斥锁执行 `action`（替换引擎或语言包时调用，阻塞）。
pub fn with_worker_stopped<T>(
    action: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let runtime = runtime()?;
    let dir = resources()?;
    let mut slot = state()
        .0
        .clone()
        .try_lock_owned()
        .map_err(|_| "翻译正在进行，请结束后再管理语言包。".to_string())?;
    if let Some(worker) = slot.take() {
        runtime.block_on(worker.stop());
    }
    let result = action(&dir);
    drop(slot);
    result
}

/// 引擎与语言包状态（完整校验，阻塞）。
pub fn status() -> Result<local_models::LocalStatus, String> {
    local_models::status(&resources()?)
}

/// 启动闲置回收：每分钟按设置的闲置秒数停止长时间未用的 worker。
pub fn init(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        cx.background_executor()
            .timer(Duration::from_secs(60))
            .await;
        let seconds = cx.update(|cx| crate::shell::settings(cx).ai.local_idle_seconds);
        if STATE.get().is_some() {
            if let Ok(runtime) = runtime() {
                runtime.spawn(async move { local_worker::release_idle(state(), seconds).await });
            }
        }
    })
    .detach();
}

/// 退出时终止缓存的 worker，之后不再缓存新进程。
pub fn shutdown() {
    STOPPING.store(true, Ordering::Relaxed);
    if let Some(state) = STATE.get() {
        local_worker::shutdown(state);
    }
}
