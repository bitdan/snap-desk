//! 识别与翻译：配置模型（服务目录、翻译与 OCR 设置及地址白名单）、OCR 引擎（本地 PP-OCR、
//! Windows OCR / Apple Vision、在线 OCR 与 PaddleOCR 服务）、二维码与条形码、在线翻译与系统凭据库。
//! 本地 CTranslate2 翻译（引擎安装、语言包、worker 进程桥）在 local_*；微信 OCR（仅 Windows x64）在 wechat_ocr。
pub mod ai_credentials;
pub mod local_models;
pub mod local_runtime;
pub mod local_worker;
pub mod ocr;
pub mod ocr_config;
pub mod overlay;
#[cfg(windows)]
pub mod ocr_runtime;
pub mod online_ocr;
pub mod paddle_ocr;
pub mod providers;
pub mod codes;
pub mod selected_text;
pub mod services;
pub mod translation;
#[cfg(target_os = "macos")]
pub mod vision_ocr;
pub mod wechat_ocr;
#[cfg(windows)]
pub mod windows_ocr;

use crate::settings::Settings;
use ocr::{LocalOcr, OcrResult};
use wechat_ocr::WechatOcr;

/// 进程内常驻的本地识别引擎（由应用外壳持有）。
#[derive(Default)]
pub struct Engines {
    pub local: LocalOcr,
    pub wechat: WechatOcr,
}

impl Engines {
    /// 空闲五分钟后释放本地模型与微信 OCR 连接；正在识别的引擎跳过。
    pub fn release_idle(&self) {
        self.local.release_idle();
        self.wechat.release_idle(wechat_ocr::IDLE);
    }
}

/// 系统 OCR：Windows 为 Windows.Media.Ocr，macOS 为 Apple Vision（沿用同一提供方标识）。
pub fn system_ocr(image: &image::RgbaImage, language: &str) -> Result<OcrResult, String> {
    #[cfg(windows)]
    return windows_ocr::recognize(image, language);
    #[cfg(target_os = "macos")]
    return vision_ocr::recognize(image, language);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (image, language);
        Err("Windows 系统 OCR 仅支持 Windows。".into())
    }
}

/// 系统 OCR 已安装的识别语言（每次查询，安装语言包后刷新即可看到）。
pub fn system_ocr_languages() -> Result<Vec<ocr_config::OcrLanguage>, String> {
    #[cfg(windows)]
    return windows_ocr::languages();
    #[cfg(target_os = "macos")]
    return vision_ocr::languages();
    #[cfg(not(any(windows, target_os = "macos")))]
    Err("Windows 系统 OCR 仅支持 Windows。".into())
}

/// 按已保存的 OCR 设置识别；严格使用所选引擎，不跨引擎重试或自动联网。
/// 识别后的动作开关（自动复制、合并行）随结果返回。
pub fn recognize(image: &image::RgbaImage, settings: &Settings, engines: &Engines) -> Result<OcrResult, String> {
    let (ocr, service) = ocr_config::snapshot(settings)?;
    let mut result = match (service, ocr.provider.as_str()) {
        (Some(service), _) => online_ocr::recognize(image, &service)?,
        (None, "local-ppocr") => engines.local.recognize(image, || {
            crate::resources::ocr_models(&crate::resources::directory().ok_or("无法定位资源目录。")?)
        })?,
        (None, "windows-media") => system_ocr(image, &ocr.language)?,
        (None, "wechat-ocr") => engines.wechat.recognize(image)?,
        _ => return Err("此 OCR 引擎尚未接入。".into()),
    };
    result.auto_copy = ocr.auto_copy;
    result.merge_lines = ocr.merge_lines;
    Ok(result)
}

#[cfg(test)]
mod tests {
    /// 依赖本机已下载的 OCR 资源与系统字体，CI 不跑：
    /// `cargo test -p snap-core recognizes_rendered_text -- --ignored --nocapture`。
    #[test]
    #[ignore]
    fn recognizes_rendered_text_with_local_and_system_engines() {
        let font = crate::pins::text::font_bytes(None).unwrap();
        let image = crate::text_image::render("Snap Desk 文字识别 2026", &font, 640, 16_000_000).unwrap();
        let mut settings = crate::settings::Settings::default();
        let engines = super::Engines::default();
        let result = super::recognize(&image, &settings, &engines).unwrap();
        eprintln!("local: {:?} ({} ms)", result.full_text, result.elapsed_ms);
        assert!(result.full_text.contains("Snap") && result.full_text.contains("2026"));
        settings.ocr.provider = "windows-media".into();
        let system = super::recognize(&image, &settings, &engines);
        eprintln!("system: {:?}", system.as_ref().map(|r| &r.full_text));
        settings.ocr.provider = "wechat-ocr".into();
        let wechat = super::recognize(&image, &settings, &engines);
        eprintln!("wechat: {:?}", wechat.as_ref().map(|r| &r.full_text));
    }
}
