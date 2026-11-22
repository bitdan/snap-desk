//! 识别结果窗口：文字识别（可编辑草稿）、图片翻译、二维码。截图与贴图共用。
//!
//! 对照 Vue 版 `OcrWorkspace.vue`、`TranslationPanel.vue`、`QrResultView.vue`、`PinRecognition.vue`：
//! - 识别出内容后截图浮层即关闭，结果窗口独立存活，直到用户关闭；
//! - OCR 严格使用设置中所选引擎（本地 PP-OCR、系统 OCR、在线服务；微信 OCR 暂不支持），
//!   设置“自动复制”“自动合并换行”与 Tauri 版一致；
//! - 在线翻译只发送识别出的文字，发送前需要用户点击“发送翻译”确认；本地 CTranslate2 翻译在本机进程中执行（`local_translate.rs`）；
//! - 二维码只在用户确认后才打开 HTTP/HTTPS 链接。
//! 识别与翻译在后台线程执行（翻译用一个临时的单线程 tokio 运行时承载异步 HTTP 请求）。
//!
//! 文字翻译窗口（对照 Vue 版 `TranslationWindow.vue`）复用同一面板、不带图片：“翻译”快捷键先在前台应用
//! 复制选中文字（core `recognition::selected_text`；X11 读 PRIMARY 选区），取到时自动翻译，否则打开空白窗口
//! 手动输入；窗口内可“截图翻译”。
#[path = "recognition_ui.rs"]
mod ui;
use gpui_kit::{
    component::input::TextareaState,
    *,
};
use image::RgbaImage;
use snap_core::{
    recognition::{
        self,
        ocr::OcrResult,
        Engines,
        providers::TranslationRequest,
        qr::QrItem,
    },
    settings::Settings,
};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ocr,
    Translate,
    Qr,
}

/// 翻译目标语言（与 `providers::language` 支持的集合一致）。
const TARGETS: [(&str, &str); 6] = [
    ("zh-CN", "简体中文"),
    ("en", "英语"),
    ("ja", "日语"),
    ("ko", "韩语"),
    ("fr", "法语"),
    ("de", "德语"),
];

/// 本地 PP-OCR 与微信 OCR 连接在进程内共享；空闲五分钟后释放模型与连接。
struct SharedEngines(Arc<Engines>);
impl Global for SharedEngines {}

fn local_engine(cx: &mut App) -> Arc<Engines> {
    if let Some(engines) = cx.try_global::<SharedEngines>() {
        return engines.0.clone();
    }
    let local = Arc::new(Engines::default());
    cx.set_global(SharedEngines(local.clone()));
    // 定期检查空闲模型；只持有弱引用不必要：引擎随进程常驻，模型本身按空闲时间释放。
    let sweeper = local.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(60)).await;
            sweeper.release_idle();
        }
    })
    .detach();
    local
}

/// 把识别文字切成翻译段落：按空行分段，最多 50 段（超出部分并入最后一段）。
pub fn segments(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = text
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.len() > 50 {
        let tail = parts.split_off(49).join("\n\n");
        parts.push(tail);
    }
    parts
}

/// “翻译”快捷键：记下前台窗口，后台复制其中选中的文字后打开文字翻译窗口；同一时间只执行一次。
pub fn translate_selection(cx: &mut App) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    // 必须在本程序任何窗口取得焦点之前读取前台窗口。
    let target = recognition::selected_text::foreground();
    cx.spawn(async move |cx| {
        let text = cx
            .background_executor()
            .spawn(async move {
                target
                    .and_then(|target| recognition::selected_text::copy(target, recognition::selected_text::clipboard_text))
                    .and_then(recognition::selected_text::validate)
            })
            .await;
        if let Err(error) = &text {
            // 未取得选区也允许手动输入；不以旧剪贴板内容作为自动翻译原文。
            log::info!("event=gpui.translate_selection_empty message={error}");
        }
        cx.update(|cx| open_text(cx, text.ok()));
        RUNNING.store(false, Ordering::Release);
    })
    .detach();
}

/// 打开文字翻译窗口（`translation_window.rs`）；带原文时立即发送翻译（快捷键划词即视为确认）。
pub fn open_text(cx: &mut App, text: Option<String>) {
    crate::translation_window::open(cx, text);
}

/// 打开识别结果窗口并立即开始识别。
pub fn open(cx: &mut App, image: RgbaImage, mode: Mode) {
    let settings = crate::shell::settings(cx);
    let local = local_engine(cx);
    let size = size(px(920.0), px(580.0));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions {
            title: Some(title(&settings, mode).into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        window_min_size: Some(gpui_kit::size(px(560.0), px(360.0))),
        ..Default::default()
    };
    let image = Arc::new(image);
    let result = cx.open_window(options, |window, cx| {
        cx.new(|cx| {
            let mut panel = Panel::new(Some(image), mode, settings, local, window, cx);
            panel.start(cx);
            panel
        })
    });
    match result {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            log::info!("event=gpui.recognition_opened mode={mode:?}");
        }
        Err(error) => log::error!("event=gpui.recognition_open_failed message={error}"),
    }
}

fn text(settings: &Settings, key: &'static str) -> String {
    snap_core::i18n::for_language(&settings.appearance.language, key).to_owned()
}

fn title(settings: &Settings, mode: Mode) -> String {
    text(
        settings,
        match mode {
            Mode::Ocr => "文字识别结果",
            Mode::Translate => "图片翻译",
            Mode::Qr => "图片二维码识别",
        },
    )
}

struct Panel {
    /// 识别的图片；文字翻译窗口没有图片。
    image: Option<Arc<RgbaImage>>,
    preview: Option<Arc<RenderImage>>,
    mode: Mode,
    settings: Settings,
    local: Arc<Engines>,
    /// 识别结果草稿（可编辑）与原始识别结果。
    draft: Entity<TextareaState>,
    ocr: Option<OcrResult>,
    /// 译文（可编辑、可复制）。
    translated: Entity<TextareaState>,
    target: String,
    qr: Option<Vec<QrItem>>,
    /// 待确认打开的二维码链接。
    pending_link: Option<usize>,
    busy: Option<&'static str>,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
    /// 识别结果首次就绪时填入草稿（需要窗口，留到下次渲染执行）。
    fill: Option<String>,
    fill_translation: Option<String>,
    /// 窗口置顶（对照 Vue 翻译窗口的置顶按钮）；新窗口为普通层级。
    pinned: bool,
    /// 显示左侧原图（Vue “显示/隐藏原图”）。
    show_image: bool,
}

impl Panel {
    fn new(
        image: Option<Arc<RgbaImage>>,
        mode: Mode,
        settings: Settings,
        local: Arc<Engines>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let preview = image.as_ref().map(|image| {
            let mut frame = (**image).clone();
            for pixel in frame.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            Arc::new(RenderImage::new(vec![image::Frame::new(frame)]))
        });
        let target = snap_core::recognition::services::translation_config(&settings)
            .map(|ai| ai.target_language)
            .unwrap_or_else(|_| "zh-CN".into());
        Self {
            image,
            preview,
            mode,
            draft: cx.new(|cx| TextareaState::new(window, cx)),
            translated: cx.new(|cx| TextareaState::new(window, cx)),
            settings,
            local,
            ocr: None,
            target,
            qr: None,
            pending_link: None,
            busy: None,
            error: None,
            message: None,
            focus: cx.focus_handle(),
            fill: None,
            fill_translation: None,
            pinned: false,
            show_image: true,
        }
    }

    fn t(&self, key: &'static str) -> String {
        text(&self.settings, key)
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Qr => self.scan_qr(cx),
            Mode::Ocr | Mode::Translate => self.recognize(cx),
        }
    }

    fn recognize(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(image) = self.image.clone() else {
            return;
        };
        self.busy = Some("正在识别文字…");
        self.error = None;
        self.message = None;
        let (settings, local) = (self.settings.clone(), self.local.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { recognition::recognize(&image, &settings, &local) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.busy = None;
                match result {
                    Ok(result) => {
                        log::info!(
                            "event=gpui.ocr_finished provider={} lines={} elapsed_ms={}",
                            result.provider,
                            result.lines.len(),
                            result.elapsed_ms
                        );
                        let text = recognition::ocr::initial_text(&result);
                        if text.trim().is_empty() {
                            panel.message = Some(panel.t("未识别到文字。"));
                        } else if result.auto_copy {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            panel.message = Some(panel.t("已复制识别文字"));
                        }
                        panel.fill = Some(text);
                        panel.ocr = Some(result);
                    }
                    Err(error) => {
                        log::warn!("event=gpui.ocr_failed message={error}");
                        panel.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scan_qr(&mut self, cx: &mut Context<Self>) {
        let Some(image) = self.image.clone() else {
            return;
        };
        self.busy = Some("正在识别二维码…");
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { recognition::qr::decode(&image) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.busy = None;
                match result {
                    Ok(items) => {
                        log::info!("event=gpui.qr_finished items={}", items.len());
                        if items.is_empty() {
                            panel.message = Some(panel.t("未识别到二维码。"));
                        }
                        panel.qr = Some(items);
                    }
                    Err(error) => panel.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 用户点击“发送翻译”即视为确认上传识别文字（与 Tauri 版 consent 一致）。
    fn translate(&mut self, cx: &mut Context<Self>) {
        let source = self.draft.read(cx).value().to_string();
        self.translate_text(source, cx);
    }

    fn translate_text(&mut self, source: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let texts = segments(&source);
        if texts.is_empty() {
            self.error = Some(self.t("没有可翻译的文字。"));
            cx.notify();
            return;
        }
        let mut ai = match recognition::services::translation_config(&self.settings) {
            Ok(ai) => ai,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        ai.target_language = self.target.clone();
        if let Err(error) = ai.ready() {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let request = TranslationRequest { texts, source_language: None, target_language: self.target.clone() };
        if let Err(error) = request.validate() {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let keys = recognition::services::key_ids(&self.settings, &ai.service_id);
        self.busy = Some("正在翻译…");
        self.error = None;
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // 本地翻译在常驻运行时上复用 worker；在线翻译每次用临时运行时。
                    if ai.provider == "local-ctranslate2" {
                        return crate::local_translate::translate(&request.texts, &request.target_language, ai.timeout_seconds);
                    }
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| "无法初始化翻译网络客户端。".to_string())?;
                    runtime.block_on(recognition::providers::translate(&ai, &request, &keys))
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.busy = None;
                match result {
                    Ok(texts) => {
                        log::info!("event=gpui.translation_finished segments={}", texts.len());
                        panel.fill_translation = Some(texts.join("\n\n"));
                    }
                    Err(error) => {
                        log::warn!("event=gpui.translation_failed");
                        panel.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy(&mut self, value: String, done: &'static str, cx: &mut Context<Self>) {
        if value.trim().is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.message = Some(self.t(done));
        self.error = None;
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            if self.pending_link.take().is_some() {
                cx.notify();
            } else {
                window.remove_window();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::segments;

    #[test]
    fn segments_split_paragraphs_and_cap_at_fifty() {
        assert_eq!(segments("a\nb\n\n\n c \n\n"), vec!["a\nb", "c"]);
        assert!(segments("  \n\n ").is_empty());
        let many = (0..60).map(|i| i.to_string()).collect::<Vec<_>>().join("\n\n");
        let parts = segments(&many);
        assert_eq!(parts.len(), 50);
        assert!(parts[49].starts_with("49") && parts[49].ends_with("59"));
    }
}
