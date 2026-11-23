//! 翻译路由：默认本地 CTranslate2，仅在线文本服务需要上传授权。
use super::providers::{self, TranslationRequest};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};

/// 配置模型与地址校验在 core；这里保留原路径供翻译调度和服务管理使用。
pub use snap_core::recognition::translation::AiSettings;
#[cfg(test)]
use snap_core::recognition::translation::endpoint_url;

/// Chat Completions 地址补全在 core（在线 OCR 与翻译共用）。
#[cfg(test)]
use snap_core::recognition::translation::chat_endpoint;

/// 全应用最多一个翻译请求；请求 ID 加入原生窗口归属，窗口间不能互相取消。Notify 的 permit 保留取消信号，避免开始前取消丢失。
#[derive(Default)]
pub struct TranslationState(Mutex<Jobs>);
#[derive(Default)]
struct Jobs {
    active: Option<(String, Arc<tokio::sync::Notify>)>,
    // IPC 调度可能让取消先于提交抵达；保留有限的取消记录，避免关闭面板后才开始上传。
    cancelled: VecDeque<String>,
    // 仅进程内保存，最多约 4 MiB 译文，五分钟过期；不落盘。
    cache: VecDeque<CacheEntry>,
    cache_generation: u64,
}
struct CacheEntry {
    key: String,
    texts: Vec<String>,
    created: std::time::Instant,
}
impl Jobs {
    fn start(&mut self, id: &str, notify: Arc<tokio::sync::Notify>) -> Result<(), String> {
        if self.cancelled.iter().any(|cancelled| cancelled == id) {
            return Err("翻译已取消。".into());
        }
        if self.active.is_some() {
            return Err("已有翻译正在进行，请稍候。".into());
        }
        self.active = Some((id.to_string(), notify));
        Ok(())
    }
    fn cancel_window(&self, label: &str) {
        if let Some((id, notify)) = self.active.as_ref() {
            if id.starts_with(&format!("{label}:")) {
                notify.notify_one();
            }
        }
    }

    fn cancel(&mut self, request_id: String) {
        if let Some((id, notify)) = self.active.as_ref() {
            if id == &request_id {
                notify.notify_one();
            }
        }
        if self.cancelled.len() >= 64 {
            self.cancelled.pop_front();
        }
        self.cancelled.push_back(request_id);
    }
}
/// 主窗口、独立翻译、截图与仍存在的贴图共享翻译，其他窗口不能提交或取消任务。
fn require_translation_window(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "capture" | "translation") {
        return Ok(());
    }
    crate::pins::require_pin(app, window)
}

fn owned_job_id(label: &str, request_id: &str) -> String {
    format!("{label}:{request_id}")
}

/// 窗口关闭只中止自己的请求，避免关闭截图时误杀贴图翻译。
pub fn cancel_window(app: &AppHandle, label: &str) {
    if let Ok(guard) = app.state::<TranslationState>().0.lock() {
        guard.cancel_window(label);
    }
}

pub fn cancel_all(app: &AppHandle) {
    if let Ok(guard) = app.state::<TranslationState>().0.lock() {
        if let Some((_, notify)) = guard.active.as_ref() {
            notify.notify_one();
        }
    }
}
struct JobGuard(AppHandle, String);
impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state::<TranslationState>().0.lock() {
            if state.active.as_ref().is_some_and(|(id, _)| id == &self.1) {
                state.active = None;
            }
        }
    }
}

/// 更新任一服务的凭据时清除已有译文，避免跨账号复用缓存。
pub(super) fn clear_cache(app: &AppHandle) {
    let state = app.state::<TranslationState>();
    if let Ok(mut jobs) = state.0.lock() {
        jobs.cache.clear(); jobs.cache_generation = jobs.cache_generation.wrapping_add(1);
    };
}
fn config(app: &AppHandle) -> Result<AiSettings, String> {
    let state = app.state::<crate::settings::SettingsState>();
    let settings = state.0.lock().map_err(|_| "设置正在使用。")?;
    super::services::translation_config(&settings.value)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResult {
    request_id: String,
    translated_text: String,
    target_language: String,
}

/// 在线翻译必须确认上传；本地翻译不需要网络授权。
#[tauri::command]
pub async fn translate_text(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    text: String,
    consent: bool,
    expected_settings: AiSettings,
) -> Result<TranslationResult, String> {
    let target_language = expected_settings.target_language.clone();
    let result = run_translation(
        app,
        window,
        request_id,
        TranslationRequest {
            texts: vec![text],
            source_language: None,
            target_language,
        },
        consent,
        expected_settings,
    )
    .await?;
    Ok(TranslationResult {
        request_id: result.request_id,
        translated_text: result
            .translations
            .into_iter()
            .next()
            .ok_or("服务返回空译文。")?
            .translated_text,
        target_language: result.target_language,
    })
}

/// 批量结果携带原文，前端必须核对顺序与数量后再提交 OCR 单元。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationItem {
    source_text: String,
    translated_text: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationBatchResult {
    request_id: String,
    translations: Vec<TranslationItem>,
    target_language: String,
}

#[tauri::command]
pub async fn translate_texts(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    request: TranslationRequest,
    consent: bool,
    expected_settings: AiSettings,
) -> Result<TranslationBatchResult, String> {
    run_translation(app, window, request_id, request, consent, expected_settings).await
}

/// 单文本和批量请求共享授权、窗口取消、整次超时与短期缓存。
async fn run_translation(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    request: TranslationRequest,
    consent: bool,
    expected_settings: AiSettings,
) -> Result<TranslationBatchResult, String> {
    require_translation_window(&app, &window)?;
    if request_id.is_empty() || request_id.len() > 80 {
        return Err("请求 ID 无效。".into());
    }
    request.validate()?;
    let mut settings = config(&app)?;
    if settings != expected_settings {
        return Err("翻译配置已变更，请关闭面板重新打开后确认。".into());
    }
    // 单次语言选择独立于持久默认值；仍完整核对服务配置，避免向已更换的地址上传。
    settings.target_language = request.target_language.clone();
    settings.ready()?;
    if request.source_language.as_ref() == Some(&request.target_language) {
        return Err("原文语言和目标语言不能相同。".into());
    }
    if settings.provider == "local-ctranslate2"
        && request.source_language.as_ref().is_some_and(|source| !["zh-CN", "en"].contains(&source.as_str()))
    {
        return Err("本地翻译仅支持英译中和中译英。".into());
    }
    if settings.provider != "local-ctranslate2" && !consent {
        return Err("请先确认发送识别文本。".into());
    }
    let job_id = owned_job_id(window.label(), &request_id);
    let notify = Arc::new(tokio::sync::Notify::new());
    {
        let state = app.state::<TranslationState>();
        let mut job = state.0.lock().map_err(|_| "翻译状态不可用。")?;
        job.start(&job_id, notify.clone())?;
    }
    let _guard = JobGuard(app.clone(), job_id.clone());
    static LOG_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let op = LOG_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let started = std::time::Instant::now();
    let provider = if settings.provider == "local-ctranslate2" {
        "local"
    } else {
        "cloud"
    };
    let engine = settings.provider.as_str();
    log::info!("event=translation.started module=translation op=tr-{op} provider={provider} engine={engine}");
    let cache_key =
        blake3::hash(&serde_json::to_vec(&(&settings, &request)).map_err(|_| "请求无效。")?)
            .to_hex()
            .to_string();
    let (cached, generation) = {
        let state = app.state::<TranslationState>();
        let mut jobs = state.0.lock().map_err(|_| "翻译状态不可用。")?;
        jobs.cache
            .retain(|entry| entry.created.elapsed() < Duration::from_secs(300));
        (
            jobs.cache
                .iter()
                .find(|entry| entry.key == cache_key)
                .map(|entry| entry.texts.clone()),
            jobs.cache_generation,
        )
    };
    let translated_texts = tokio::select! {
        biased;
        _ = notify.notified() => {
            log::debug!("event=translation.cancelled module=translation op=tr-{op} provider={provider} engine={engine}");
            return Err("翻译已取消。".into());
        },
        value = tokio::time::timeout(Duration::from_secs(settings.timeout_seconds), async {
            if let Some(texts) = cached { return Ok(texts); }
            if settings.provider == "local-ctranslate2" {
                let mut texts = Vec::new();
                for text in &request.texts {
                    texts.push(crate::recognition::local_translation::translate(&app, &job_id, &settings.target_language, text, settings.timeout_seconds).await.map_err(|error| (error.code, error.message))?);
                }
                Ok(texts)
            } else { providers::translate(&settings, &request, &super::services::key_ids(&app, &settings.service_id)).await.map_err(|error| (failure_code(&error), error)) }
        }) => value.unwrap_or_else(|_| Err(("TRANSLATION_TIMEOUT", "翻译超时，请重试或调整超时时间。".into()))).map_err(|(code, error)| {
            log::error!("event=translation.failed module=translation op=tr-{op} provider={provider} engine={engine} code={code} duration_ms={}", started.elapsed().as_millis());
            error
        })?,
    };
    {
        let state = app.state::<TranslationState>();
        let mut jobs = state.0.lock().map_err(|_| "翻译状态不可用。")?;
        // 命中时不延长有效期，也不重复占用缓存。
        if translated_texts.iter().map(String::len).sum::<usize>() <= 256 * 1024
            && jobs.cache_generation == generation
            && !jobs.cache.iter().any(|entry| entry.key == cache_key)
        {
            while jobs.cache.len() >= 16 {
                jobs.cache.pop_front();
            }
            jobs.cache.push_back(CacheEntry {
                key: cache_key,
                texts: translated_texts.clone(),
                created: std::time::Instant::now(),
            });
        }
    }
    log::info!("event=translation.finished module=translation op=tr-{op} provider={provider} engine={engine} duration_ms={}", started.elapsed().as_millis());
    Ok(TranslationBatchResult {
        request_id,
        translations: request
            .texts
            .into_iter()
            .zip(translated_texts)
            .map(|(source_text, translated_text)| TranslationItem {
                source_text,
                translated_text,
            })
            .collect(),
        target_language: settings.target_language,
    })
}

#[tauri::command]
pub fn cancel_ai_job(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    require_translation_window(&app, &window)?;
    if request_id.is_empty() || request_id.len() > 80 {
        return Err("请求 ID 无效。".into());
    }
    let state = app.state::<TranslationState>();
    let mut guard = state.0.lock().map_err(|_| "翻译状态不可用。")?;
    guard.cancel(owned_job_id(window.label(), &request_id));
    Ok(())
}

#[tauri::command]
pub async fn ai_credential_set(
    app: AppHandle,
    window: WebviewWindow,
    secret: Option<String>,
    expected_endpoint: String,
) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let settings = config(&app)?;
    settings.ready()?;
    if settings.endpoint != expected_endpoint {
        return Err("翻译配置已变更，请重新确认服务地址。".into());
    }
    crate::recognition::ai_credentials::write(&super::services::credential_key(&settings.service_id, &settings.endpoint), secret)?;
    let state = app.state::<TranslationState>();
    let mut jobs = state.0.lock().map_err(|_| "翻译状态不可用。")?;
    jobs.cache.clear();
    jobs.cache_generation = jobs.cache_generation.wrapping_add(1);
    Ok(())
}
#[tauri::command]
pub async fn ai_provider_status(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    crate::capture::require_window(&window, "main")?;
    let settings = config(&app)?;
    if settings.endpoint.is_empty() {
        return Ok(false);
    }
    Ok(!super::services::secrets(&settings.service_id, &settings.endpoint, &super::services::key_ids(&app, &settings.service_id))?.is_empty())
}

/// 翻译界面只读取非敏感配置，用于确认发送目的地，不暴露密钥。
#[tauri::command]
pub async fn ai_translation_config(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<AiSettings, String> {
    require_translation_window(&app, &window)?;
    config(&app)
}
#[tauri::command]
pub async fn ai_provider_test(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    crate::capture::require_window(&window, "main")?;
    let settings = config(&app)?;
    settings.ready()?;
    if settings.provider == "local-ctranslate2" {
        let sample = if settings.target_language == "en" {
            "你好"
        } else {
            "Hello"
        };
        crate::recognition::local_translation::translate(
            &app,
            "settings-test",
            &settings.target_language,
            sample,
            settings.timeout_seconds,
        )
        .await
        .map(|_| ())
        .map_err(|error| {
            log::error!("event=translation.failed module=translation command=ai_provider_test provider=local engine=local-ctranslate2 code={}", error.code);
            error.message
        })
    } else {
        providers::translate(
            &settings,
            &TranslationRequest {
                texts: vec!["Hello".into()],
                source_language: Some("en".into()),
                target_language: settings.target_language.clone(),
            },
            &super::services::key_ids(&app, &settings.service_id),
        ).await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_preserve_online_languages_and_default_to_local() {
        let empty: AiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.provider, "local-ctranslate2");
        assert_eq!(empty.local_idle_seconds, 300);
        let old: AiSettings = serde_json::from_str(
            r#"{"endpoint":"https://example.com/v1/chat/completions","targetLanguage":"ja"}"#,
        )
        .unwrap();
        assert_eq!(old.provider, "online-llm");
        assert!(old.validate().is_ok());
        let invalid: AiSettings =
            serde_json::from_str(r#"{"provider":"local-ctranslate2","targetLanguage":"ja"}"#)
                .unwrap();
        assert!(invalid.validate().is_err());
        assert!(serde_json::from_str::<AiSettings>(r#"{"apiKey":"not-allowed"}"#).is_err());
    }
    #[test]
    fn idle_settings_migrate_and_validate_bounds() {
        for seconds in [0, 30, 300, 3600] {
            let settings: AiSettings =
                serde_json::from_value(serde_json::json!({"localIdleSeconds": seconds})).unwrap();
            assert!(settings.validate().is_ok());
            let saved = serde_json::to_value(&settings).unwrap();
            assert_eq!(saved["localIdleSeconds"], seconds);
        }
        for seconds in [1, 29, 3601, u64::MAX] {
            let settings: AiSettings =
                serde_json::from_value(serde_json::json!({"localIdleSeconds": seconds})).unwrap();
            assert!(settings.validate().is_err());
        }
        for json in [r#"{"localIdleSeconds":-1}"#, r#"{"localIdleSeconds":30.5}"#] {
            assert!(serde_json::from_str::<AiSettings>(json).is_err());
        }
    }
    #[test]
    fn cancellation_before_start_and_concurrent_jobs_are_rejected() {
        let mut jobs = Jobs::default();
        jobs.cancel("early".into());
        assert!(jobs
            .start("early", Arc::new(tokio::sync::Notify::new()))
            .is_err());
        assert!(jobs
            .start("current", Arc::new(tokio::sync::Notify::new()))
            .is_ok());
        assert!(jobs
            .start("next", Arc::new(tokio::sync::Notify::new()))
            .is_err());
        for i in 0..100 {
            jobs.cancel(format!("cancel-{i}"));
        }
        assert_eq!(jobs.cancelled.len(), 64);
        assert_eq!(jobs.active.as_ref().unwrap().0, "current");
    }
    #[test]
    fn cancellations_are_scoped_to_the_native_window() {
        let mut jobs = Jobs::default();
        let capture = owned_job_id("capture", "same-id");
        let pin = owned_job_id("pin-1", "same-id");
        assert_ne!(capture, pin);
        let notify = Arc::new(tokio::sync::Notify::new());
        jobs.start(&pin, notify.clone()).unwrap();
        jobs.cancel(capture.clone());
        jobs.cancel_window("capture");
        jobs.cancel_window("pin-10");
        // enable() 注册等待而不需要启动异步 runtime；其他窗口不能发出 permit。
        let mut pending = Box::pin(notify.notified());
        assert!(!pending.as_mut().enable());
        jobs.cancel_window("pin-1");
        assert!(pending.as_mut().enable());
        jobs.cancel(pin.clone());
        jobs.active = None;
        assert!(jobs
            .start(&capture, Arc::new(tokio::sync::Notify::new()))
            .is_err());
        assert!(jobs
            .start(
                &owned_job_id("pin-2", "same-id"),
                Arc::new(tokio::sync::Notify::new())
            )
            .is_ok());
    }

    #[test]
    fn endpoint_rejects_unsafe_addresses() {
        for url in [
            "ftp://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/?key=x",
            "file:///tmp/test",
        ] {
            assert!(endpoint_url(url).is_err());
        }
        assert!(endpoint_url("http://127.0.0.1:8080/v1/chat/completions").is_ok());
        assert!(endpoint_url("http://intranet.example.com:8089/v1").is_ok());
        assert!(endpoint_url("https://example.com/v1/chat/completions").is_ok());
    }

    #[test]
    fn chat_endpoint_completes_base_urls_and_keeps_custom_paths() {
        for (input, expected) in [
            ("http://intranet.example.com:8089/v1", "http://intranet.example.com:8089/v1/chat/completions"),
            ("http://intranet.example.com:8089/v1/", "http://intranet.example.com:8089/v1/chat/completions"),
            ("https://ark.cn-beijing.volces.com/api/v3", "https://ark.cn-beijing.volces.com/api/v3/chat/completions"),
            ("https://generativelanguage.googleapis.com/v1beta/openai", "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"),
            ("https://api.groq.com/openai/v1", "https://api.groq.com/openai/v1/chat/completions"),
            ("http://127.0.0.1:11434", "http://127.0.0.1:11434/v1/chat/completions"),
            ("https://api.deepseek.com/chat/completions", "https://api.deepseek.com/chat/completions"),
            ("https://example.com/v1/chat/completions/", "https://example.com/v1/chat/completions/"),
            ("https://example.com/proxy/llm", "https://example.com/proxy/llm"),
            ("https://example.com/video", "https://example.com/video"),
        ] {
            assert_eq!(chat_endpoint(input).unwrap().as_str(), expected, "{input}");
        }
        assert!(chat_endpoint("https://example.com/v1?key=x").is_err());
    }
}

/// 按失败阶段给出诊断码：错误正文只送界面，日志只记录固定码，便于区分“未配置密钥 / 钥匙串读取 /
/// 认证 / 额度 / 网络 / 服务返回”，而不必根据耗时猜测原因。
fn failure_code(error: &str) -> &'static str {
    let has = |words: &[&str]| words.iter().any(|word| error.contains(word));
    if has(&["请先在服务管理中保存"]) {
        "TRANSLATION_KEY_MISSING"
    } else if has(&["钥匙串", "凭据", "凭证", "Secret Service", "密钥环"]) {
        "TRANSLATION_CREDENTIAL_READ_FAILED"
    } else if has(&["拒绝认证"]) {
        "TRANSLATION_AUTH_REJECTED"
    } else if has(&["额度", "受限"]) {
        "TRANSLATION_QUOTA_EXCEEDED"
    } else if has(&["连接翻译服务失败", "翻译超时"]) {
        "TRANSLATION_NETWORK_FAILED"
    } else if has(&["HTTP"]) {
        "TRANSLATION_HTTP_ERROR"
    } else if has(&["地址", "语言", "不支持", "无效"]) {
        "TRANSLATION_CONFIG_INVALID"
    } else {
        "TRANSLATION_REQUEST_FAILED"
    }
}

#[cfg(test)]
mod failure_code_tests {
    use super::failure_code;

    #[test]
    fn translation_failures_map_to_stage_codes() {
        for (error, code) in [
            ("请先在服务管理中保存此服务的 API Key。", "TRANSLATION_KEY_MISSING"),
            ("无法读取 macOS 钥匙串，请确认已允许 Snap Desk 访问。", "TRANSLATION_CREDENTIAL_READ_FAILED"),
            ("服务拒绝认证，请检查密钥与模型权限。", "TRANSLATION_AUTH_REJECTED"),
            ("DeepL 翻译额度已用尽，请检查账户用量。", "TRANSLATION_QUOTA_EXCEEDED"),
            ("服务请求受限或额度不足，请稍后重试。", "TRANSLATION_QUOTA_EXCEEDED"),
            ("连接翻译服务失败，请检查网络与服务地址。", "TRANSLATION_NETWORK_FAILED"),
            ("翻译服务返回 HTTP 500，请检查地址和模型。", "TRANSLATION_HTTP_ERROR"),
            ("不支持该目标语言。", "TRANSLATION_CONFIG_INVALID"),
            ("意外错误", "TRANSLATION_REQUEST_FAILED"),
        ] {
            assert_eq!(failure_code(error), code, "{error}");
            assert!(crate::diagnostics::catalog::CODES.contains(&code), "{code}");
        }
    }
}
