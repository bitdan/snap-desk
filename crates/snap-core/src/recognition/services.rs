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
        if self.keys.len() > 16
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
