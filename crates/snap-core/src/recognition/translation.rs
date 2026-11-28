//! 翻译配置模型与地址校验；网络请求、凭据和本地引擎调度仍在各前端外壳。
use serde::{Deserialize, Serialize};

/// 内置本地语言包方向；下标 0 为英译中，1 为中译英（与语言包清单一致）。
pub const LOCAL_MODEL_IDS: [&str; 2] = ["opus-mt-en-zh-int8", "opus-mt-zh-en-int8"];

/// 目标语言对应的本地语言包；本地引擎只支持中英互译。
pub fn local_model_id(target: &str) -> Result<&'static str, String> {
    match target {
        "zh-CN" => Ok(LOCAL_MODEL_IDS[0]),
        "en" => Ok(LOCAL_MODEL_IDS[1]),
        _ => Err("本地翻译仅支持英译中和中译英。".into()),
    }
}

/// DeepL 只接受官方 Free / Pro 的 `/v2/translate` 地址，避免把密钥发往其他站点。
pub fn deepl_endpoint(value: &str) -> Result<url::Url, String> {
    if ![
        "https://api-free.deepl.com/v2/translate",
        "https://api.deepl.com/v2/translate",
    ]
    .contains(&value)
    {
        return Err("DeepL 仅支持官方 Free 或 Pro 的 /v2/translate 地址。".into());
    }
    url::Url::parse(value).map_err(|_| "服务地址无效。".into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "AiSettingsWire")]
pub struct AiSettings {
    /// 新配置默认本地；旧在线配置保留服务选择，发送前仍需单次授权。
    pub provider: String,
    /// 服务管理连接；空字符串兼容旧的地址配置和凭据。
    pub service_id: String,
    /// 完整接口地址；DeepL 仅允许官方 Free/Pro 翻译接口。
    pub endpoint: String,
    pub model: String,
    pub target_language: String,
    pub timeout_seconds: u64,
    /// 本地进程空闲回收秒数；0 保持缓存，旧配置默认五分钟。
    pub local_idle_seconds: u64,
    /// 翻译完成后自动复制译文；由翻译界面执行。
    pub auto_copy: bool,
}
/// 区分旧字段缺失与显式选择，避免旧日语目标因本地只支持中英而使整个设置失效。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct AiSettingsWire {
    provider: Option<String>,
    service_id: String,
    endpoint: String,
    model: String,
    target_language: String,
    timeout_seconds: u64,
    local_idle_seconds: u64,
    auto_copy: bool,
    /// 兼容旧配置：截图翻译已固定原地覆盖原文，不再保存此开关。
    #[serde(rename = "overlay")]
    _overlay: bool,
    /// 兼容旧配置，右键翻译入口已移除，不再保存此字段。
    #[serde(rename = "selectionContextMenu")]
    _selection_context_menu: bool,
}
impl Default for AiSettingsWire {
    fn default() -> Self {
        let value = AiSettings::default();
        Self {
            provider: None,
            service_id: String::new(),
            endpoint: value.endpoint,
            model: value.model,
            target_language: value.target_language,
            timeout_seconds: value.timeout_seconds,
            local_idle_seconds: value.local_idle_seconds,
            auto_copy: value.auto_copy,
            _overlay: false,
            _selection_context_menu: false,
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
            service_id: wire.service_id,
            endpoint: wire.endpoint,
            model: wire.model,
            target_language: wire.target_language,
            timeout_seconds: wire.timeout_seconds,
            local_idle_seconds: wire.local_idle_seconds,
            auto_copy: wire.auto_copy,
        }
    }
}
impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: "local-ctranslate2".into(),
            service_id: String::new(),
            endpoint: String::new(),
            model: String::new(),
            target_language: "zh-CN".into(),
            timeout_seconds: 45,
            local_idle_seconds: 300,
            auto_copy: false,
        }
    }
}
impl AiSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !["local-ctranslate2", "online-llm", "deepl"].contains(&self.provider.as_str()) {
            return Err("不支持此翻译引擎。".into());
        }
        if self.provider == "local-ctranslate2" {
            local_model_id(&self.target_language)?;
        }
        if !self.endpoint.is_empty() {
            if self.provider == "deepl" {
                deepl_endpoint(&self.endpoint)?;
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
    pub fn ready(&self) -> Result<(), String> {
        self.validate()?;
        if self.provider == "local-ctranslate2" {
            return Ok(());
        }
        if self.endpoint.is_empty()
            || (self.provider == "online-llm" && self.model.trim().is_empty())
        {
            return Err("请先在主窗口的翻译页选择服务，或在服务管理中添加服务。".into());
        }
        Ok(())
    }
}

/// 禁止 URL 内嵌密码、查询参数和重定向；允许 HTTP 以适配内网自建服务（明文传输由用户自行承担）。
pub fn endpoint_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|_| "服务地址无效。")?;
    if value.len() > 2048
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || !matches!(url.scheme(), "https" | "http")
    {
        return Err("请使用不含凭证、查询参数的 HTTP / HTTPS 完整接口地址。".into());
    }
    Ok(url)
}

/// OpenAI 兼容服务的 Chat Completions 地址。很多服务与网关只给出基础地址（如 `https://host/v1`、
/// 火山方舟的 `/api/v3`、Gemini 的 `/v1beta/openai`），这里补上 `/chat/completions`；
/// 只有主机名时按 `/v1/chat/completions`。其他自定义路径原样使用，已保存的地址与密钥索引不变。
pub fn chat_endpoint(value: &str) -> Result<url::Url, String> {
    let mut url = endpoint_url(value)?;
    let path = url.path().trim_end_matches('/').to_string();
    let last = path.rsplit('/').next().unwrap_or_default();
    let version = last.len() > 1
        && last.starts_with('v')
        && last[1..].starts_with(|c: char| c.is_ascii_digit())
        && last[1..].chars().all(|c| c.is_ascii_alphanumeric());
    let next = if path.ends_with("/chat/completions") {
        return Ok(url);
    } else if path.is_empty() {
        "/v1/chat/completions".to_string()
    } else if version || last == "openai" {
        format!("{path}/chat/completions")
    } else {
        return Ok(url);
    };
    url.set_path(&next);
    Ok(url)
}

