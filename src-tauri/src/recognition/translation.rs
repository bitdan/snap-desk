//! 翻译路由：默认本地 CTranslate2，仅在线文本服务需要上传授权。
use super::providers::{self, TranslationRequest};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "AiSettingsWire")]
pub struct AiSettings {
    /// 新配置默认本地；旧在线配置保留服务选择，发送前仍需单次授权。
    pub provider: String,
    /// 完整接口地址；DeepL 仅允许官方 Free/Pro 翻译接口。
    pub endpoint: String,
    pub model: String,
    pub target_language: String,
    pub timeout_seconds: u64,
    /// 本地进程空闲回收秒数；0 保持缓存，旧配置默认五分钟。
    pub local_idle_seconds: u64,
}
/// 区分旧字段缺失与显式选择，避免旧日语目标因本地只支持中英而使整个设置失效。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct AiSettingsWire {
    provider: Option<String>,
    endpoint: String,
    model: String,
    target_language: String,
    timeout_seconds: u64,
    local_idle_seconds: u64,
}
impl Default for AiSettingsWire {
    fn default() -> Self {
        let value = AiSettings::default();
        Self {
            provider: None,
            endpoint: value.endpoint,
            model: value.model,
            target_language: value.target_language,
            timeout_seconds: value.timeout_seconds,
            local_idle_seconds: value.local_idle_seconds,
        }
    }
}
impl From<AiSettingsWire> for AiSettings {
    fn from(wire: AiSettingsWire) -> Self {
        let provider = wire.provider.unwrap_or_else(|| {
            if !wire.endpoint.is_empty()
                || !["zh-CN", "en"].contains(&wire.target_language.as_str())
            {
                "online-llm".into()
            } else {
                "local-ctranslate2".into()
            }
        });
        Self {
            provider,
            endpoint: wire.endpoint,
            model: wire.model,
            target_language: wire.target_language,
            timeout_seconds: wire.timeout_seconds,
            local_idle_seconds: wire.local_idle_seconds,
        }
    }
}
impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: "local-ctranslate2".into(),
            endpoint: String::new(),
            model: String::new(),
            target_language: "zh-CN".into(),
            timeout_seconds: 45,
            local_idle_seconds: 300,
        }
    }
}
impl AiSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !["local-ctranslate2", "online-llm", "deepl"].contains(&self.provider.as_str()) {
            return Err("不支持此翻译引擎。".into());
        }
        if self.provider == "local-ctranslate2" {
            crate::recognition::translation_models::model_id(&self.target_language)?;
        }
        if !self.endpoint.is_empty() {
            if self.provider == "deepl" {
                providers::deepl::endpoint(&self.endpoint)?;
            } else {
                endpoint_url(&self.endpoint)?;
            }
        }
        if self.model.len() > 200 || self.model.chars().any(char::is_control) {
            return Err("模型名称无效。".into());
        }
        if !["zh-CN", "en", "ja", "ko", "fr", "de"].contains(&self.target_language.as_str()) {
            return Err("不支持该目标语言。".into());
        }
        if !(5..=120).contains(&self.timeout_seconds) {
            return Err("超时必须为 5～120 秒。".into());
        }
        if self.local_idle_seconds != 0 && !(30..=3600).contains(&self.local_idle_seconds) {
            return Err("闲置回收必须为 0 或 30～3600 秒。".into());
        }
        Ok(())
    }
    fn ready(&self) -> Result<(), String> {
        self.validate()?;
        if self.provider == "local-ctranslate2" {
            return Ok(());
        }
        if self.endpoint.is_empty()
            || (self.provider == "online-llm" && self.model.trim().is_empty())
        {
            return Err("请先在主窗口的识别与翻译页配置服务地址和模型。".into());
        }
        Ok(())
    }
}

/// 禁止 URL 内嵌密码、查询参数和重定向；明文 HTTP 只允许明确的本机地址。
pub(super) fn endpoint_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "服务地址无效。")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if value.len() > 2048
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && local))
    {
        return Err("请使用不含凭证、查询参数的 HTTPS 完整接口地址；HTTP 仅限本机。".into());
    }
    Ok(url)
}

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
/// 截图与仍存在的贴图共享文本翻译；不开放给主页或任意窗口。
fn require_translation_window(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "capture" {
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

fn config(app: &AppHandle) -> Result<AiSettings, String> {
    Ok(app
        .state::<crate::settings::SettingsState>()
        .0
        .lock()
        .map_err(|_| "设置正在使用。")?
        .value
        .ai
        .clone())
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
    let settings = config(&app)?;
    if settings != expected_settings || request.target_language != settings.target_language {
        return Err("翻译配置已变更，请关闭面板重新打开后确认。".into());
    }
    settings.ready()?;
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
    log::info!("event=translation.started module=translation op=tr-{op} provider={provider}");
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
            log::debug!("event=translation.cancelled module=translation op=tr-{op}");
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
            } else { providers::translate(&settings, &request).await.map_err(|error| ("TRANSLATION_REQUEST_FAILED", error)) }
        }) => value.unwrap_or_else(|_| Err(("TRANSLATION_TIMEOUT", "翻译超时，请重试或调整超时时间。".into()))).map_err(|(code, error)| {
            log::error!("event=translation.failed module=translation op=tr-{op} provider={provider} code={code} duration_ms={}", started.elapsed().as_millis());
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
    log::info!("event=translation.finished module=translation op=tr-{op} provider={provider} duration_ms={}", started.elapsed().as_millis());
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
    crate::recognition::ai_credentials::write(&settings.endpoint, secret)?;
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
    Ok(crate::recognition::ai_credentials::read(&settings.endpoint)?.is_some())
}

/// 截图/贴图面板只读取非敏感配置，用于确认发送目的地，不暴露密钥。
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
            log::error!("event=translation.failed module=translation command=ai_provider_test provider=local code={}", error.code);
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
        )
        .await
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
            "http://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/?key=x",
            "file:///tmp/test",
        ] {
            assert!(endpoint_url(url).is_err());
        }
        assert!(endpoint_url("http://127.0.0.1:8080/v1/chat/completions").is_ok());
        assert!(endpoint_url("https://example.com/v1/chat/completions").is_ok());
    }
}
