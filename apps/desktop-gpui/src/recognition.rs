//! 识别结果窗口：文字识别（可编辑草稿）、图片翻译、二维码与条形码。截图与贴图共用。
//!
//! 截图与贴图的识别结果：
//! - 识别出内容后截图浮层即关闭，结果窗口独立存活，直到用户关闭；
//! - OCR 严格使用设置中所选引擎（本地 PP-OCR、系统 OCR、WeChat OCR（实验性，仅 Windows x64）、在线服务），
//!   设置“自动复制”“自动合并换行”；
//! - 进入翻译视图（截图翻译或点击“翻译文字”）即视为确认，自动用所选服务翻译识别出的文字；
//!   本地 CTranslate2 翻译在本机进程中执行（`local_translate.rs`）；
//! - 设置开启“截图翻译覆盖原文”且原文未改动时按段落翻译，并把译文按原图配色画回原位置
//!   （`snap_core::recognition::overlay`）；
//! - 二维码 / 条形码逐个列出格式与内容，只在用户确认后才打开 HTTP/HTTPS 链接。
//! 识别与翻译在后台线程执行（翻译用一个临时的单线程 tokio 运行时承载异步 HTTP 请求）。
//!
//! 文字翻译窗口复用同一面板、不带图片：“翻译”快捷键先在前台应用
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
        overlay::{self, Paragraph},
        providers::TranslationRequest,
        codes::CodeItem,
    },
    settings::Settings,
};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ocr,
    Translate,
    /// 二维码与条形码（沿用 `Qr` 名称，截图与贴图的入口不变）。
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

/// 贴图“选择文字”：在后台用设置中所选引擎识别图片，与识别结果窗口共用常驻的本地引擎；
/// 返回的任务交给调用方等待并建立文字层（`snap_core::recognition::text_layer`）。
pub fn recognize_text(cx: &mut App, image: Arc<RgbaImage>) -> Task<Result<OcrResult, String>> {
    let settings = crate::shell::settings(cx);
    let local = local_engine(cx);
    cx.background_executor().spawn(async move { recognition::recognize(&image, &settings, &local) })
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
            Mode::Qr => "二维码 / 条形码识别",
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
    codes: Option<Vec<CodeItem>>,
    /// 待确认打开的码内链接（`codes` 的下标）。
    pending_link: Option<usize>,
    busy: Option<&'static str>,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
    /// 识别结果首次就绪时填入草稿（需要窗口，留到下次渲染执行）。
    fill: Option<String>,
    fill_translation: Option<String>,
    /// 窗口置顶；新窗口为普通层级。
    pinned: bool,
    /// 显示左侧原图（“显示/隐藏原图”）。
    show_image: bool,
    /// 识别结果合成的覆盖段落；缺坐标或倾斜时为 None（只能普通翻译）。
    paragraphs: Option<Vec<Paragraph>>,
    /// 最近一次按段落翻译的覆盖块；重新翻译或切换语言即清空。
    overlay: Option<Vec<OverlayItem>>,
    /// 覆盖块显示译文还是露出原文（底栏眼睛按钮切换）。
    show_overlay: bool,
    /// 正在按段落翻译、完成后会覆盖原文；期间就隐藏右侧翻译面板，避免面板出现又消失。
    overlay_pending: bool,
    /// 覆盖原文时手动展开右侧翻译面板（底栏按钮切换）；默认收起，只看原图。
    show_panel: bool,
    /// 当前译文对应的（目标语言, 原文）；回到翻译视图时两者都没变才沿用旧译文。
    translated_for: Option<(String, String)>,
}

/// 一个覆盖块：原图像素坐标下的段落框、行高、译文与取色。
#[derive(Clone)]
struct OverlayItem {
    rect: overlay::Rect,
    line_height: f32,
    text: String,
    colors: overlay::Colors,
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
        // 新窗口没有任何元素持有焦点时按键无处投递，Esc 关不掉窗口；先聚焦面板根节点，
        // 按键沿焦点路径冒泡到 `key`。点进文本框后焦点移入文本框，Esc 仍会冒泡上来。
        let focus = cx.focus_handle();
        focus.focus(window, cx);
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
            codes: None,
            pending_link: None,
            busy: None,
            error: None,
            message: None,
            focus,
            fill: None,
            fill_translation: None,
            pinned: false,
            show_image: true,
            paragraphs: None,
            overlay: None,
            show_overlay: true,
            overlay_pending: false,
            show_panel: false,
            translated_for: None,
        }
    }

    /// 译文覆盖在原图上，右侧翻译面板可以收起（底栏出现展开 / 收起按钮）。
    /// 翻译出错（需要看到原因）、隐藏原图或没有覆盖结果时始终分栏，可在面板里改原文、换语言。
    fn panel_collapsible(&self) -> bool {
        self.mode == Mode::Translate
            && self.show_image
            && self.preview.is_some()
            && self.error.is_none()
            && (self.overlay.is_some() || self.overlay_pending)
    }

    /// 只显示原图：可收起且用户没有展开面板。译文已画在图上，面板默认只会重复一遍。
    fn overlay_only(&self) -> bool {
        self.panel_collapsible() && !self.show_panel
    }

    fn t(&self, key: &'static str) -> String {
        text(&self.settings, key)
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Qr => self.scan_codes(cx),
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
                        panel.paragraphs = overlay::paragraphs(&result.lines);
                        panel.overlay = None;
                        panel.translated_for = None;
                        panel.fill = Some(text.clone());
                        panel.ocr = Some(result);
                        // 截图翻译入口：识别完成即自动翻译。
                        if panel.mode == Mode::Translate && !text.trim().is_empty() {
                            panel.translate_text(text, cx);
                        }
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

    /// 识别二维码 / 条形码；“重新识别”时清掉上次的提示与待确认链接（下标可能对应到新结果里的别的码）。
    fn scan_codes(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(image) = self.image.clone() else {
            return;
        };
        self.busy = Some("正在识别二维码 / 条形码…");
        self.error = None;
        self.message = None;
        self.pending_link = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { recognition::codes::decode(&image) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.busy = None;
                match result {
                    Ok(items) => {
                        log::info!("event=gpui.codes_finished items={}", items.len());
                        if items.is_empty() {
                            panel.message = Some(panel.t("未识别到二维码或条形码。"));
                        }
                        panel.codes = Some(items);
                    }
                    Err(error) => {
                        log::warn!("event=gpui.codes_failed");
                        panel.error = Some(
                            snap_core::i18n::for_language(&panel.settings.appearance.language, &error).to_owned(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 切换到翻译视图；尚无当前语言的译文时自动翻译（进入翻译视图即视为确认发送）。
    fn enter_translate(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Translate;
        let current = (self.target.clone(), self.draft.read(cx).value().to_string());
        if self.busy.is_none() && self.translated_for.as_ref() != Some(&current) {
            self.translate(cx);
        }
        cx.notify();
    }

    /// 切换目标语言并立即重新翻译。
    fn set_target(&mut self, code: &str, cx: &mut Context<Self>) {
        if self.target == code {
            return;
        }
        self.target = code.into();
        self.fill_translation = Some(String::new());
        self.overlay = None;
        self.translated_for = None;
        if self.busy.is_none() {
            self.translate(cx);
        }
        cx.notify();
    }

    /// 点击翻译按钮或进入翻译视图即视为确认上传识别文字。
    fn translate(&mut self, cx: &mut Context<Self>) {
        let source = self.draft.read(cx).value().to_string();
        self.translate_text(source, cx);
    }

    fn translate_text(&mut self, source: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        // 覆盖模式：识别段落可用且原文未改动时按段落翻译，译文才能一一对应原图位置。
        let paragraphs = self
            .paragraphs
            .clone()
            .filter(|list| self.image.is_some() && overlay::matches(&source, list));
        let mut texts = segments(&source);
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
        // 设置关闭“覆盖原文”时始终按空行分段普通翻译。
        let paragraphs = paragraphs.filter(|_| ai.overlay);
        if let Some(list) = &paragraphs {
            texts = list.iter().map(|p| p.text.clone()).collect();
        }
        let request = TranslationRequest { texts, source_language: None, target_language: self.target.clone() };
        if let Err(error) = request.validate() {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let keys = recognition::services::key_ids(&self.settings, &ai.service_id);
        let auto_copy = ai.auto_copy;
        let done_key = (self.target.clone(), source.clone());
        // 取色需要原图与识别坐标的比例；识别引擎可能缩放过输入图片。
        let sample = paragraphs
            .zip(self.image.clone())
            .zip(self.ocr.as_ref().map(|r| (r.image_width, r.image_height)));
        self.overlay_pending = sample.is_some();
        self.busy = Some("正在翻译…");
        self.error = None;
        self.message = None;
        self.overlay = None;
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
            // 覆盖模式在后台线程按段落取色（逐像素扫描），完成后连同译文一起交给界面。
            let result = match (result, sample) {
                (Ok(texts), Some(((list, image), (ocr_width, ocr_height)))) if texts.len() == list.len() => {
                    let items = cx
                        .background_executor()
                        .spawn({
                            let texts = texts.clone();
                            async move { overlay_items(&image, ocr_width, ocr_height, &list, texts) }
                        })
                        .await;
                    Ok((texts, items))
                }
                (result, _) => result.map(|texts| (texts, None)),
            };
            let _ = this.update(cx, |panel, cx| {
                panel.busy = None;
                panel.overlay_pending = false;
                match result {
                    Ok((texts, items)) => {
                        log::info!(
                            "event=gpui.translation_finished segments={} overlay={}",
                            texts.len(),
                            items.is_some()
                        );
                        let text = texts.join("\n\n");
                        panel.overlay = items;
                        panel.show_overlay = true;
                        panel.translated_for = Some(done_key);
                        panel.fill_translation = Some(text.clone());
                        if auto_copy {
                            panel.copy(text, "译文已复制。", cx);
                        }
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

/// 把段落框从识别坐标换算到原图像素后取色；识别坐标与原图宽高比不一致（被裁剪或变形）时不覆盖，避免错位。
fn overlay_items(
    image: &RgbaImage,
    ocr_width: u32,
    ocr_height: u32,
    paragraphs: &[Paragraph],
    texts: Vec<String>,
) -> Option<Vec<OverlayItem>> {
    let sx = image.width() as f32 / ocr_width.max(1) as f32;
    let sy = image.height() as f32 / ocr_height.max(1) as f32;
    if (sx - sy).abs() > sx.max(sy) * 0.02 {
        return None;
    }
    Some(
        paragraphs
            .iter()
            .zip(texts)
            .map(|(paragraph, text)| {
                let r = paragraph.rect;
                let rect = overlay::Rect {
                    x: r.x * sx,
                    y: r.y * sy,
                    width: r.width * sx,
                    height: r.height * sy,
                };
                OverlayItem {
                    rect,
                    line_height: paragraph.line_height * sy,
                    text,
                    colors: overlay::sample_colors(image, rect),
                }
            })
            .collect(),
    )
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
