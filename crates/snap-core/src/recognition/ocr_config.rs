//! OCR 引擎选择与语言配置；引擎选择独立于文本翻译设置，不做自动回退。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct OcrSettings {
    pub provider: String,
    /// 空值保留旧本地配置；在线识别通过服务 ID 解析连接。
    pub service_id: String,
    /// auto 使用 Windows 用户语言优先级；其他值是系统已安装 OCR 语言的 BCP-47 标签。
    pub language: String,
    /// 识别完成后的整理动作由前端执行；这里只保存开关，随识别结果下发给截图和贴图窗口。
    pub auto_copy: bool,
    pub merge_lines: bool,
}
impl Default for OcrSettings {
    fn default() -> Self {
        Self {
            provider: "local-ppocr".into(),
            service_id: String::new(),
            language: "auto".into(),
            auto_copy: false,
            merge_lines: false,
        }
    }
}
impl OcrSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !["local-ppocr", "windows-media", "wechat-ocr", "service"].contains(&self.provider.as_str()) {
            return Err("此 OCR 引擎尚未接入。".into());
        }
        if self.language.is_empty()
            || self.language.len() > 85
            || !self
                .language
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("OCR 语言标签无效。".into());
        }
        Ok(())
    }
}

/// PaddleOCR 官方异步任务接口路径；VL、结构化模型只走异步任务。
pub const PADDLE_JOB_PATH: &str = "/api/v2/ocr/jobs";

pub fn paddle_is_jobs_endpoint(endpoint: &str) -> bool {
    url::Url::parse(endpoint).is_ok_and(|url| url.path().trim_end_matches('/') == PADDLE_JOB_PATH)
}

/// 服务配置里允许的 Paddle 模型；同步接口只支持通用 OCR 模型。
pub fn paddle_supports_model(endpoint: &str, model: &str) -> bool {
    match model {
        "PP-OCRv5" | "PP-OCRv6" => true,
        "PaddleOCR-VL-1.6" | "PP-StructureV3" => paddle_is_jobs_endpoint(endpoint),
        _ => false,
    }
}

/// 系统 OCR 语言包（BCP 47 标签与显示名）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct OcrLanguage {
    pub tag: String,
    pub name: String,
}

/// 同一请求使用同一份引擎与语言快照，识别期间保存设置只影响下次请求。
pub fn snapshot(
    settings: &crate::settings::Settings,
) -> Result<(OcrSettings, Option<super::services::RecognitionService>), String> {
    let value = settings.ocr.clone();
    value.validate()?;
    let service = if value.provider == "service" {
        Some(super::services::find(settings, &value.service_id)?.clone())
    } else {
        None
    };
    Ok((value, service))
}
