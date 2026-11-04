//! 微信 4.x 本机 OCR：只发现用户已安装组件，不下载或分发微信二进制文件。
use serde::Serialize;
use tauri::WebviewWindow;

/// 检测仅检查组件存在，不启动 OCR 工作进程；实际兼容性在首次识别时确认。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatOcrStatus {
    pub supported: bool,
    pub detected: bool,
    pub error: Option<String>,
}

/// 连接随应用存活，避免每次截图重建微信工作进程及底层回调状态。
#[derive(Default)]
pub struct WechatOcrState {
    #[cfg(all(windows, target_arch = "x86_64"))]
    engine: std::sync::Mutex<Option<wechat_ocr_core::WeChatOcr>>,
}

#[tauri::command]
pub async fn wechat_ocr_status(window: WebviewWindow) -> Result<WechatOcrStatus, String> {
    crate::capture::require_window(&window, "main")?;
    tauri::async_runtime::spawn_blocking(status)
        .await
        .map_err(|e| e.to_string())
}

fn status() -> WechatOcrStatus {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = platform::discover();
        WechatOcrStatus {
            supported: true,
            detected: result.is_ok(),
            error: result.err(),
        }
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    WechatOcrStatus {
        supported: false,
        detected: false,
        error: Some("WeChat OCR 仅支持 Windows x64。".into()),
    }
}

/// 截图、贴图和截图翻译复用此入口；失败交给用户处理，不切换引擎或上传图片。
pub fn recognize(
    app: &tauri::AppHandle,
    image: &image::RgbaImage,
) -> Result<super::OcrResult, String> {
    super::ocr::validate_image(image)?;
    #[cfg(all(windows, target_arch = "x86_64"))]
    return platform::recognize(app, image);
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = app;
        Err("WeChat OCR 仅支持 Windows x64。".into())
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod platform {
    use super::WechatOcrState;
    use crate::recognition::{OcrLine, OcrResult, PolygonPoint};
    use std::{
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };
    use tauri::Manager;
    use wechat_ocr_core::{Config, TextBlock};

    /// 只遍历插件的直接版本目录，按数值版本选择最新可用目录。
    fn plugin_in(root: &Path) -> Result<PathBuf, String> {
        let mut candidates = Vec::new();
        for entry in std::fs::read_dir(root)
            .map_err(|_| "未找到 WeChat OCR 插件。打开微信，对任意图片右键 →「提取文字」，微信会自动下载插件。完成后点击“重新检测”。")?
        {
            let entry = entry.map_err(|e| format!("读取 WeChat OCR 插件目录失败：{e}"))?;
            let name = entry.file_name();
            let Some(version) = name.to_str().and_then(|name| {
                name.split('.')
                    .map(str::parse::<u64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            }) else {
                continue;
            };
            let dir = entry.path().join("extracted");
            if dir.join("wxocr.dll").is_file() {
                candidates.push((version, dir));
            }
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        candidates
            .into_iter()
            .next()
            .map(|(_, path)| path)
            .ok_or_else(|| "未找到 WeChat OCR 插件。打开微信，对任意图片右键 →「提取文字」，微信会自动下载插件。完成后点击“重新检测”。".into())
    }

    /// 只检查明确的安装根目录，不调用 SDK 的全盘符搜索，避免网络盘阻塞设置页。
    fn runtime_in(root: &Path) -> Option<PathBuf> {
        if valid_runtime(root) {
            return Some(root.to_path_buf());
        }
        let mut versions: Vec<_> = std::fs::read_dir(root)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let version = entry
                    .file_name()
                    .to_str()?
                    .split('.')
                    .map(str::parse::<u64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                Some((version, entry.path()))
            })
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        versions
            .into_iter()
            .map(|(_, path)| path)
            .find(|path| valid_runtime(path))
    }

    fn valid_runtime(path: &Path) -> bool {
        path.join("mmmojo_64.dll").is_file()
            && path.parent().is_some_and(|parent| {
                ["Weixin.exe", "WeChat.exe"]
                    .iter()
                    .any(|name| parent.join(name).is_file())
            })
    }

    /// 尊重 SDK 的显式环境变量，其次尝试注册表，最后才检查当前系统的标准安装位置。
    fn runtime_dir() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os(wechat_ocr_core::ENV_WECHAT_DIR) {
            return runtime_in(Path::new(&path))
                .ok_or_else(|| "未找到微信 4.x 运行库：WECHAT_OCR_WECHAT_DIR".into());
        }
        for root in registry_roots().into_iter().chain(
            ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
                .into_iter()
                .filter_map(std::env::var_os)
                .flat_map(|base| {
                    [
                        PathBuf::from(&base).join("Tencent/Weixin"),
                        PathBuf::from(base).join("Tencent/WeChat"),
                    ]
                }),
        ) {
            if let Some(runtime) = runtime_in(&root) {
                return Ok(runtime);
            }
        }
        Err("未找到微信 4.x 运行库，请确认已安装微信 4.x。".into())
    }

    /// 固定注册表键与值，不启动外部命令，也不根据用户输入拼接查询。
    fn registry_roots() -> Vec<PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{
                RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
            },
        };
        let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut roots = Vec::new();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for key in [
                r"Software\Tencent\Weixin",
                r"Software\Tencent\WeChat",
                r"Software\WOW6432Node\Tencent\Weixin",
                r"Software\WOW6432Node\Tencent\WeChat",
            ] {
                for name in ["InstallPath", "InstallLocation"] {
                    let mut buffer = [0u16; 1024];
                    let mut bytes = std::mem::size_of_val(&buffer) as u32;
                    // SAFETY: 固定大小缓冲区与字节长度一致，键和值为有效的 NUL 结尾 UTF-16。
                    let code = unsafe {
                        RegGetValueW(
                            hive,
                            wide(key).as_ptr(),
                            wide(name).as_ptr(),
                            RRF_RT_REG_SZ,
                            std::ptr::null_mut(),
                            buffer.as_mut_ptr().cast(),
                            &mut bytes,
                        )
                    };
                    if code == ERROR_SUCCESS {
                        let length = buffer
                            .iter()
                            .position(|value| *value == 0)
                            .unwrap_or(buffer.len());
                        if length > 0 {
                            let root =
                                PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
                            if !roots.contains(&root) {
                                roots.push(root);
                            }
                        }
                    }
                }
            }
        }
        roots
    }

    pub(super) fn discover() -> Result<(PathBuf, PathBuf), String> {
        let root = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or("无法读取 WeChat OCR 插件目录。")?;
        let plugin = plugin_in(&root.join("Tencent/xwechat/XPlugin/Plugins/WeChatOcr"))?;
        let runtime = runtime_dir()?;
        Ok((plugin, runtime))
    }

    pub(super) fn recognize(
        app: &tauri::AppHandle,
        image: &image::RgbaImage,
    ) -> Result<OcrResult, String> {
        let started = Instant::now();
        let op = super::super::ocr::next_request();
        let result = recognize_inner(app, image, op);
        match &result {
            Ok(value) => log::info!("event=ocr.finished module=ocr engine=wechat-ocr op=ocr-{op} duration_ms={} lines={}", started.elapsed().as_millis(), value.lines.len()),
            Err(_) => log::error!("event=ocr.failed module=ocr engine=wechat-ocr op=ocr-{op} duration_ms={}", started.elapsed().as_millis()),
        }
        result
    }

    fn recognize_inner(
        app: &tauri::AppHandle,
        image: &image::RgbaImage,
        op: u64,
    ) -> Result<OcrResult, String> {
        let started = Instant::now();
        let state = app.state::<WechatOcrState>();
        // 串行请求让单次超时保持有界，且不与另一次初始化或销毁竞争。
        let mut engine = state
            .engine
            .try_lock()
            .map_err(|_| "已有 WeChat OCR 识别正在进行。")?;
        if engine.is_none() {
            let (plugin, runtime) = discover()?;
            *engine = Some(
                Config::new()
                    .ocr_dir(plugin)
                    .wechat_dir(runtime)
                    .connect_timeout(Duration::from_secs(10))
                    .request_timeout(Duration::from_secs(30))
                    .build()
                    .map_err(|e| {
                        format!("WeChat OCR 初始化失败，请切换引擎或重启应用后重试：{e}")
                    })?,
            );
        }
        // SDK 按路径读取图片；文件由后端持有，成功、失败或超时退出时均自动清理。
        let temporary = tempfile::Builder::new()
            .prefix("snap-desk-ocr-")
            .suffix(".png")
            .tempfile()
            .map_err(|e| format!("创建 OCR 临时文件失败：{e}"))?;
        crate::shared::image_io::white_background(image)
            .save_with_format(temporary.path(), image::ImageFormat::Png)
            .map_err(|e| format!("写入 OCR 临时图片失败：{e}"))?;
        let result = engine
            .as_ref()
            .ok_or("WeChat OCR 未初始化。")?
            .recognize(temporary.path())
            .map_err(|e| format!("WeChat OCR 识别失败，请重试或手动切换引擎：{e}"));
        let result = result?;
        let lines: Vec<_> = result
            .blocks
            .into_iter()
            .filter(|block| !block.text.trim().is_empty())
            .enumerate()
            .map(|(order, block)| line(block, order, image.width(), image.height()))
            .collect();
        let elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(OcrResult {
            request_id: format!("ocr-{op}"),
            provider: "wechat-ocr".into(),
            image_width: image.width(),
            image_height: image.height(),
            full_text: lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines,
            elapsed_ms,
            warnings: vec![],
        })
    }

    /// SDK 矩形坐标相对原图；无效框不伪造坐标，置信度缺失时保留文字。
    fn line(block: TextBlock, order: usize, width: u32, height: u32) -> OcrLine {
        let valid = [block.left, block.top, block.right, block.bottom]
            .iter()
            .all(|v| v.is_finite())
            && block.right > block.left
            && block.bottom > block.top;
        let polygon = valid.then(|| {
            [
                (block.left, block.top),
                (block.right, block.top),
                (block.right, block.bottom),
                (block.left, block.bottom),
            ]
            .into_iter()
            .map(|(x, y)| PolygonPoint {
                x: x.clamp(0.0, width as f32),
                y: y.clamp(0.0, height as f32),
            })
            .collect()
        });
        OcrLine {
            id: format!("line-{:04}", order + 1),
            text: block.text,
            confidence: (block.rate.is_finite() && (0.0..=1.0).contains(&block.rate))
                .then_some(block.rate),
            polygon,
            language: None,
            order,
            paragraph_id: None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn discovers_latest_numeric_plugin_and_ignores_incomplete_versions() {
            let root = tempfile::tempdir().unwrap();
            for version in ["9", "10", "11", "invalid"] {
                let dir = root.path().join(version).join("extracted");
                std::fs::create_dir_all(&dir).unwrap();
                if version != "11" {
                    std::fs::write(dir.join("wxocr.dll"), []).unwrap();
                }
            }
            assert_eq!(
                plugin_in(root.path()).unwrap(),
                root.path().join("10/extracted")
            );
        }
        #[test]
        fn runtime_discovery_stays_within_install_root_and_uses_latest_complete_version() {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("Weixin.exe"), []).unwrap();
            for version in ["4.1.9", "4.1.10", "4.1.11"] {
                let dir = root.path().join(version);
                std::fs::create_dir(&dir).unwrap();
                if version != "4.1.11" {
                    std::fs::write(dir.join("mmmojo_64.dll"), []).unwrap();
                }
            }
            assert_eq!(runtime_in(root.path()).unwrap(), root.path().join("4.1.10"));
            std::fs::remove_file(root.path().join("Weixin.exe")).unwrap();
            assert!(runtime_in(root.path()).is_none());
        }
        #[test]
        fn maps_boxes_and_keeps_text_without_invalid_geometry_or_confidence() {
            let block = TextBlock {
                text: "你好".into(),
                left: -2.0,
                top: 3.0,
                right: 120.0,
                bottom: 40.0,
                rate: 0.95,
                ..Default::default()
            };
            let mapped = line(block.clone(), 0, 100, 50);
            assert_eq!(mapped.polygon.unwrap()[1].x, 100.0);
            assert_eq!(mapped.confidence, Some(0.95));
            let mapped = line(
                TextBlock {
                    left: f32::NAN,
                    rate: f32::NAN,
                    ..block
                },
                1,
                100,
                50,
            );
            assert_eq!(mapped.text, "你好");
            assert!(mapped.polygon.is_none());
            assert!(mapped.confidence.is_none());
        }
    }
}
