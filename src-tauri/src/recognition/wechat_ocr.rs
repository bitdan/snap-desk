//! 微信 4.x 本机 OCR 的 Tauri 外壳：托管连接状态与设置页检测命令；发现、识别与回收在
//! `snap_core::recognition::wechat_ocr`。
use snap_core::recognition::wechat_ocr::{self, WechatOcr, WechatOcrStatus};
use tauri::{Manager, WebviewWindow};

/// 连续识别复用同一连接；空闲五分钟后断开，微信 OCR 子进程随之退出。
#[derive(Default)]
pub struct WechatOcrState(WechatOcr);

/// 与本地 OCR 相同：从最后一次识别结束计时；正在识别时跳过，下一轮再检查。
pub fn release_idle(app: &tauri::AppHandle) {
    app.state::<WechatOcrState>()
        .0
        .release_idle(wechat_ocr::IDLE);
}

#[tauri::command]
pub async fn wechat_ocr_status(window: WebviewWindow) -> Result<WechatOcrStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(wechat_ocr::status)
        .await
        .map_err(|e| e.to_string())
}

/// 截图、贴图和截图翻译复用此入口；失败交给用户处理，不切换引擎或上传图片。
pub fn recognize(
    app: &tauri::AppHandle,
    image: &image::RgbaImage,
) -> Result<super::OcrResult, String> {
    app.state::<WechatOcrState>().0.recognize(image)
}
