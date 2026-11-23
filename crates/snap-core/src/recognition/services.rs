//! 识别服务目录与能力绑定的配置模型；密钥只存系统凭据库，这里只计算凭据键。
use crate::settings::Settings;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecognitionService {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: String,
    pub model: String,
    /// 兼容协议的文本模型不能仅凭服务商名称推断支持图片。
    pub vision: bool,
    /// 同一服务的多个密钥标识，按顺序使用；认证失败或额度不足时换下一个。
    /// 密钥本身只在系统凭据库，配置里只有标识。旧配置缺省为单个 "default"，沿用原凭据条目。
    #[serde(default = "default_keys")]
    pub keys: Vec<String>,
}
pub const DEFAULT_KEY: &str = "default";
pub fn default_keys() -> Vec<String> {
    vec![DEFAULT_KEY.into()]
}
impl RecognitionService {
    pub fn supports_ocr(&self) -> bool {
        matches!(self.protocol.as_str(), "ocr-space" | "paddle-ocr")
            || (self.protocol == "openai-compatible" && self.vision)
    }
    pub fn supports_translation(&self) -> bool {
        matches!(self.protocol.as_str(), "openai-compatible" | "deepl")
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 80
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("服务名称或标识无效。".into());
        }
        super::translation::endpoint_url(&self.endpoint)?;
        if self.model.len() > 200 || self.model.chars().any(char::is_control) {
            return Err("模型名称无效。".into());
        }
        let mut keys = std::collections::HashSet::new();
        if self.keys.len() > MAX_KEYS
            || !self.keys.iter().all(|key| {
                !key.is_empty()
                    && key.len() <= 40
                    && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    && keys.insert(key)
            })
        {
            return Err("服务密钥列表无效。".into());
        }
        match self.protocol.as_str() {
            "openai-compatible" if !self.model.trim().is_empty() => (),
            "ocr-space" if ["1", "2", "3"].contains(&self.model.as_str()) => (),
            "paddle-ocr" if super::ocr_config::paddle_supports_model(&self.endpoint, &self.model) => (),
            "deepl" => {
                super::translation::deepl_endpoint(&self.endpoint)?;
            }
            _ => return Err("服务协议或模型无效。".into()),
        }
        Ok(())
    }
}

pub fn find<'a>(settings: &'a Settings, id: &str) -> Result<&'a RecognitionService, String> {
    settings
        .recognition_services
        .iter()
        .find(|service| service.id == id)
        .ok_or_else(|| "所选服务不存在，请重新选择。".into())
}
/// 服务和功能偏好分别保存；运行时合并为原有翻译契约，保留单次上传确认。
pub fn translation_config(settings: &Settings) -> Result<super::translation::AiSettings, String> {
    let mut value = settings.ai.clone();
    if value.provider != "local-ctranslate2" && !value.service_id.is_empty() {
        let service = find(settings, &value.service_id)?;
        if !service.supports_translation() {
            return Err("所选服务不支持翻译。".into());
        }
        value.provider = if service.protocol == "deepl" {
            "deepl"
        } else {
            "online-llm"
        }
        .into();
        value.endpoint = service.endpoint.clone();
        value.model = service.model.clone();
    }
    value.validate()?;
    Ok(value)
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    if settings.recognition_services.len() > 64 {
        return Err("最多保存 64 个服务。".into());
    }
    let mut ids = std::collections::HashSet::new();
    for service in &settings.recognition_services {
        service.validate()?;
        if !ids.insert(&service.id) {
            return Err("服务标识重复。".into());
        }
    }
    translation_config(settings)?;
    if settings.ocr.provider == "service"
        && !find(settings, &settings.ocr.service_id)?.supports_ocr()
    {
        return Err("所选服务不支持图片识别。".into());
    }
    Ok(())
}
/// 地址参与凭据键，编辑地址后不会把原账号密钥发送给另一个站点。
pub fn credential_key(id: &str, endpoint: &str) -> String {
    if id.is_empty() {
        endpoint.into()
    } else {
        format!("service:{id}:{endpoint}")
    }
}
/// 单个密钥的凭据键；"default" 与旧版单密钥条目相同，升级后无需搬运。
pub fn secret_key(id: &str, endpoint: &str, key: &str) -> String {
    if key == DEFAULT_KEY {
        credential_key(id, endpoint)
    } else {
        format!("{}#{key}", credential_key(id, endpoint))
    }
}

/// 按顺序读取已保存的密钥（系统凭据库），缺失的条目跳过。
pub fn secrets(id: &str, endpoint: &str, keys: &[String]) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for key in keys {
        if let Some(value) = super::ai_credentials::read(&secret_key(id, endpoint, key))? {
            values.push(value);
        }
    }
    Ok(values)
}

/// 认证失败、额度不足或限流时才换下一个密钥；网络、地址等错误换密钥也无济于事。
pub fn key_rejected(error: &str) -> bool {
    ["认证", "额度", "受限"].iter().any(|word| error.contains(word))
}

/// 运行时按服务标识取密钥顺序；旧版直接配置（无服务标识）只有单个条目。
pub fn key_ids(settings: &Settings, service_id: &str) -> Vec<String> {
    if service_id.is_empty() {
        return default_keys();
    }
    find(settings, service_id).map(|service| service.keys.clone()).unwrap_or_default()
}

/// 每个服务最多保存的密钥数。
pub const MAX_KEYS: usize = 16;

/// 只显示首尾少量字符，界面可区分多个密钥而不暴露完整内容。
pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 10 {
        return "••••".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceKeyInfo {
    pub id: String,
    /// None 表示配置里有此标识但系统凭据库中没有对应密钥（例如修改了地址）。
    pub hint: Option<String>,
}

/// 服务各密钥的掩码提示（读取系统凭据库）。
pub fn key_infos(service: &RecognitionService) -> Result<Vec<ServiceKeyInfo>, String> {
    service
        .keys
        .iter()
        .map(|key| {
            let hint = super::ai_credentials::read(&secret_key(&service.id, &service.endpoint, key))?
                .map(|value| mask(&value));
            Ok(ServiceKeyInfo { id: key.clone(), hint })
        })
        .collect()
}

/// 新密钥标识：`k-` 加 8 位十六进制，满足 [`RecognitionService::validate`] 的字符与长度限制。
pub fn new_key_id(existing: &[String]) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
            ^ NEXT.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
        let id = format!("k-{:08x}", (seed ^ (seed >> 29)) as u32);
        if !existing.contains(&id) {
            return id;
        }
    }
}

/// 新密钥的格式检查（长度与控制字符）。
pub fn validate_secret(secret: &str) -> Result<(), String> {
    if secret.chars().count() > 2048 || secret.chars().any(char::is_control) {
        return Err("密钥过长或含控制字符。".into());
    }
    Ok(())
}

/// 服务管理中的连通性测试：翻译服务发送 "Hello"，仅 OCR 的服务上传一张空白小图；只验证地址、认证和额度可用。
pub enum ServiceTest {
    Translate {
        config: super::translation::AiSettings,
        request: super::providers::TranslationRequest,
        keys: Vec<String>,
    },
    /// 阻塞请求，外壳在后台线程执行 [`ServiceTest::run_ocr`]。
    Ocr(RecognitionService),
}

impl ServiceTest {
    /// 指定 `key` 时只用该密钥测试，不做回退，便于逐个确认哪个密钥可用。
    pub fn new(
        mut service: RecognitionService,
        ai: super::translation::AiSettings,
        key: Option<String>,
    ) -> Result<Self, String> {
        service.validate()?;
        if let Some(key) = key {
            if !service.keys.contains(&key) {
                return Err("密钥不存在。".into());
            }
            service.keys = vec![key];
        }
        if !service.supports_translation() {
            return Ok(Self::Ocr(service));
        }
        let mut config = ai;
        config.service_id = service.id.clone();
        config.provider = if service.protocol == "deepl" { "deepl" } else { "online-llm" }.into();
        config.endpoint = service.endpoint.clone();
        config.model = service.model.clone();
        config.target_language = "zh-CN".into();
        Ok(Self::Translate {
            config,
            request: super::providers::TranslationRequest {
                texts: vec!["Hello".into()],
                source_language: Some("en".into()),
                target_language: "zh-CN".into(),
            },
            keys: service.keys,
        })
    }

    pub fn run_ocr(service: &RecognitionService) -> Result<(), String> {
        let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([255, 255, 255, 255]));
        super::online_ocr::recognize(&image, service).map(|_| ())
    }

    /// 翻译测试异步执行，OCR 测试在当前线程阻塞执行（供已在后台线程的外壳使用）。
    pub async fn run(self) -> Result<(), String> {
        match self {
            Self::Translate { config, request, keys } => {
                super::providers::translate(&config, &request, &keys).await.map(|_| ())
            }
            Self::Ocr(service) => Self::run_ocr(&service),
        }
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn masks_and_new_ids_fit_validation() {
        assert_eq!(mask("short"), "••••");
        assert_eq!(mask("sk-abcdefghijkl"), "sk-a…ijkl");
        let mut keys = vec![DEFAULT_KEY.to_owned()];
        for _ in 0..MAX_KEYS - 1 {
            let id = new_key_id(&keys);
            assert!(!keys.contains(&id));
            keys.push(id);
        }
        let service = RecognitionService {
            id: "svc".into(),
            name: "Test".into(),
            protocol: "deepl".into(),
            endpoint: "https://api-free.deepl.com/v2/translate".into(),
            model: String::new(),
            vision: false,
            keys,
        };
        service.validate().unwrap();
        assert!(validate_secret("a\nb").is_err());
        assert!(validate_secret("sk-ok").is_ok());
    }
}
