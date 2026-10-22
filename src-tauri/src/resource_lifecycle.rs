//! 定期回收空闲模型；不等待忙碌任务，不中断正在执行的识别或翻译。
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Default)]
/// 应用退出时置位，阻止回收循环继续运行及翻译重新缓存进程。
pub struct ResourceLifecycle(pub AtomicBool);

/// 启动时仅安装一个回收循环。OCR 析构可能较慢，放到阻塞线程池执行。
pub fn initialize(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            if app.state::<ResourceLifecycle>().0.load(Ordering::Acquire) {
                break;
            }
            let handle = app.clone();
            if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
                crate::ocr::release_idle(&handle);
            })
            .await
            {
                log::error!("event=resources.cleanup_failed module=ocr error={error}");
            }
            crate::local_translation::release_idle(&app).await;
        }
    });
}

/// 退出信号先于 Worker 清理发布；在途翻译不得在退出后重新缓存子进程。
pub fn shutdown(app: &AppHandle) {
    app.state::<ResourceLifecycle>()
        .0
        .store(true, Ordering::Release);
}

pub fn stopping(app: &AppHandle) -> bool {
    app.state::<ResourceLifecycle>().0.load(Ordering::Acquire)
}
