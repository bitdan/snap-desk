//! 识别与翻译的配置模型：服务目录、翻译与 OCR 设置及其地址白名单。
//! 实际的网络请求、本地引擎与凭据库读写仍在 `src-tauri/src/recognition`。
pub mod ocr_config;
pub mod services;
pub mod translation;
