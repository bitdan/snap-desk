//! 服务管理中的多密钥（对照 Vue `ServiceSettings.vue` 的密钥列表）。
//!
//! - 已保存的密钥按顺序列出掩码提示（首个为“首选”），可逐个测试或删除；新密钥逐行填写，最多 16 个。
//! - 修改地址后原密钥不会发往新地址：保存时先按旧地址清除原密钥，需要重新填写。
//! - “测试”使用已保存的连接与密钥；有未保存的修改时先保存（不关闭编辑），再测试。
//!
//! 密钥只写系统凭据库（core `ai_credentials`），配置里只保存标识。
use super::super::{col, row, SettingsView};
use super::{new_id, ServiceEditor};
use gpui_kit::{
    component::input::{Input, InputState},
    *,
};
use snap_core::recognition::{
    ai_credentials,
    services::{self, RecognitionService, ServiceKeyInfo, ServiceTest, MAX_KEYS},
};
use std::collections::HashMap;

/// 单个密钥的测试状态。
pub(in super::super) enum KeyTest {
    Testing,
    Ok,
    Failed(String),
}

/// 编辑中服务的密钥：已保存的掩码提示（None 为读取中）、新填写的输入框与逐个测试结果。
#[derive(Default)]
pub(in super::super) struct Keys {
    saved: Option<Vec<ServiceKeyInfo>>,
    new: Vec<Entity<InputState>>,
    tests: HashMap<String, KeyTest>,
    /// 整体测试进行中与结果。
    testing: bool,
    result: Option<Result<(), String>>,
    /// 后台读取的版本号，过期结果丢弃。
    revision: u64,
}

fn secret_input(window: &mut Window, cx: &mut Context<SettingsView>) -> Entity<InputState> {
    let language = crate::shell::settings(cx).appearance.language;
    let placeholder = snap_core::i18n::for_language(&language, "填写 API Key / Token").to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(placeholder)
    })
}

impl Keys {
    /// 打开编辑器时的初始状态：没有已保存密钥时预留一个输入框。
    pub(in super::super) fn new(
        existing: Option<&RecognitionService>,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> Self {
        let empty = existing.is_none_or(|s| s.keys.is_empty());
        Self {
            saved: existing.is_none().then(Vec::new),
            new: if empty {
                vec![secret_input(window, cx)]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }
}

impl SettingsView {
    fn saved_service(&self, cx: &App) -> Option<RecognitionService> {
        let id = self.service_editor.as_ref()?.existing.clone()?;
        crate::shell::settings(cx)
            .recognition_services
            .into_iter()
            .find(|s| s.id == id)
    }

    fn editor_value(state: &Entity<InputState>, cx: &App) -> String {
        state.read(cx).value().trim().to_owned()
    }

    /// 地址与已保存的不同（保存时清除原密钥）。
    fn endpoint_moved(&self, cx: &App) -> bool {
        let (Some(editor), Some(saved)) = (self.service_editor.as_ref(), self.saved_service(cx))
        else {
            return false;
        };
        Self::editor_value(&editor.endpoint, cx) != saved.endpoint
    }

    /// 草稿与已保存配置不同（含新填的密钥）时，测试前需要先保存。
    fn draft_dirty(&self, cx: &App) -> bool {
        let Some(editor) = self.service_editor.as_ref() else {
            return false;
        };
        let Some(saved) = self.saved_service(cx) else {
            return true;
        };
        let value = |state: &Entity<InputState>| Self::editor_value(state, cx);
        editor.keys.new.iter().any(|state| !value(state).is_empty())
            || value(&editor.name) != saved.name
            || value(&editor.endpoint) != saved.endpoint
            || value(&editor.model) != saved.model
            || (editor.protocol == "openai-compatible" && editor.vision != saved.vision)
    }

    /// 后台读取已保存密钥的掩码提示。
    pub(in super::super) fn load_keys(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.saved_service(cx) else {
            return;
        };
        let Some(editor) = self.service_editor.as_mut() else {
            return;
        };
        editor.keys.revision += 1;
        let revision = editor.keys.revision;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let infos = cx
                .background_executor()
                .spawn(async move { services::key_infos(&service) })
                .await;
            let _ = this.update(cx, |view, cx| {
                let Some(editor) = view.service_editor.as_mut() else {
                    return;
                };
                if editor.keys.revision != revision {
                    return;
                }
                match infos {
                    Ok(infos) => editor.keys.saved = Some(infos),
                    Err(error) => {
                        editor.keys.saved = Some(Vec::new());
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_key_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let moved = self.endpoint_moved(cx);
        let Some(editor) = self.service_editor.as_ref() else {
            return;
        };
        let saved = if moved {
            0
        } else {
            editor.keys.saved.as_ref().map_or(0, Vec::len)
        };
        if saved + editor.keys.new.len() >= MAX_KEYS {
            return;
        }
        let input = secret_input(window, cx);
        if let Some(editor) = self.service_editor.as_mut() {
            editor.keys.new.push(input);
        }
        cx.notify();
    }

    /// 删除一个已保存的密钥：先删凭据再从配置移除标识；失败时标识仍在，可重试。
    fn remove_key(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.saved_service(cx) else {
            return;
        };
        if let Err(error) = ai_credentials::write(
            &services::secret_key(&service.id, &service.endpoint, &id),
            None,
        ) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let (target, key) = (service.id.clone(), id.clone());
        if let Err(error) = crate::shell::update_settings(cx, move |s| {
            if let Some(slot) = s.recognition_services.iter_mut().find(|x| x.id == target) {
                slot.keys.retain(|k| *k != key);
            }
        }) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let needs_input = match self.service_editor.as_mut() {
            Some(editor) => {
                if let Some(saved) = editor.keys.saved.as_mut() {
                    saved.retain(|k| k.id != id);
                }
                editor.keys.tests.remove(&id);
                editor.keys.saved.as_ref().is_some_and(Vec::is_empty) && editor.keys.new.is_empty()
            }
            None => false,
        };
        if needs_input {
            let input = secret_input(window, cx);
            if let Some(editor) = self.service_editor.as_mut() {
                editor.keys.new.push(input);
            }
        }
        self.error = None;
        self.notice = Some("密钥已删除。".into());
        log::info!("event=gpui.recognition_key_deleted");
        self.load_recognition(cx);
        cx.notify();
    }

    /// 校验并保存服务与新填写的密钥。`keep_open` 时保存后留在编辑器（用于“测试”），返回是否已保存。
    pub(in super::super) fn save_service(
        &mut self,
        keep_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let moved = self.endpoint_moved(cx);
        let previous = self.saved_service(cx);
        let Some(editor) = self.service_editor.as_ref() else {
            return false;
        };
        let read = |state: &Entity<InputState>| state.read(cx).value().trim().to_owned();
        let secrets: Vec<String> = editor
            .keys
            .new
            .iter()
            .map(read)
            .filter(|v| !v.is_empty())
            .collect();
        let mut keys: Vec<String> = match (&previous, moved) {
            (Some(p), false) => p.keys.clone(),
            _ => Vec::new(),
        };
        let mut added = Vec::new();
        for secret in secrets {
            let id = services::new_key_id(&keys);
            keys.push(id.clone());
            added.push((id, secret));
        }
        let service = RecognitionService {
            id: editor.existing.clone().unwrap_or_else(new_id),
            name: read(&editor.name),
            protocol: editor.protocol.into(),
            endpoint: read(&editor.endpoint),
            model: read(&editor.model),
            vision: editor.vision || matches!(editor.protocol, "ocr-space" | "paddle-ocr"),
            keys,
        };
        let checked = service.validate().and_then(|_| {
            added
                .iter()
                .try_for_each(|(_, secret)| services::validate_secret(secret))
        });
        if let Err(error) = checked {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        // 地址变更时先按旧地址清除原密钥，避免凭据库中残留无法管理的条目。
        if let (Some(previous), true) = (&previous, moved) {
            for key in &previous.keys {
                if let Err(error) = ai_credentials::write(
                    &services::secret_key(&previous.id, &previous.endpoint, key),
                    None,
                ) {
                    self.error = Some(error);
                    cx.notify();
                    return false;
                }
            }
        }
        let saved = service.clone();
        if let Err(error) = crate::shell::update_settings(cx, move |s| {
            match s.recognition_services.iter_mut().find(|x| x.id == saved.id) {
                Some(slot) => *slot = saved.clone(),
                None => s.recognition_services.push(saved.clone()),
            }
            // 正在使用的翻译服务改了地址或模型：同步运行时配置。
            if s.ai.service_id == saved.id {
                s.ai.provider = if saved.protocol == "deepl" {
                    "deepl"
                } else {
                    "online-llm"
                }
                .into();
                s.ai.endpoint = saved.endpoint.clone();
                s.ai.model = saved.model.clone();
            }
        }) {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        for (id, secret) in added {
            if let Err(error) = ai_credentials::write(
                &services::secret_key(&service.id, &service.endpoint, &id),
                Some(secret),
            ) {
                self.error = Some(error);
                cx.notify();
                return false;
            }
        }
        self.error = None;
        self.saved = true;
        log::info!("event=gpui.recognition_service_saved");
        if keep_open {
            // 草稿换成已保存的配置，重新读取密钥列表（版本号递增，在途测试结果作废）。
            if let Some(editor) = self.service_editor.as_mut() {
                editor.existing = Some(service.id.clone());
                let revision = editor.keys.revision;
                editor.keys = Keys {
                    revision,
                    ..Default::default()
                };
            }
            if service.keys.is_empty() {
                let input = secret_input(window, cx);
                if let Some(editor) = self.service_editor.as_mut() {
                    editor.keys.new.push(input);
                }
            }
            self.load_keys(cx);
        } else {
            self.service_editor = None;
            self.notice = Some("服务已保存。".into());
        }
        self.load_recognition(cx);
        cx.notify();
        true
    }

    /// 测试已保存的连接：`key` 为 None 时按顺序使用全部密钥（与实际调用一致），否则只用该密钥。
    fn test_service(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let busy = self.service_editor.as_ref().is_none_or(|e| {
            e.keys.testing
                || key
                    .as_ref()
                    .is_some_and(|k| matches!(e.keys.tests.get(k), Some(KeyTest::Testing)))
        });
        if busy {
            return;
        }
        if key.is_none() && self.draft_dirty(cx) && !self.save_service(true, window, cx) {
            return;
        }
        let Some(service) = self.saved_service(cx) else {
            return;
        };
        let ai = crate::shell::settings(cx).ai;
        let test = match ServiceTest::new(service, ai, key.clone()) {
            Ok(test) => test,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let Some(editor) = self.service_editor.as_mut() else {
            return;
        };
        let revision = editor.keys.revision;
        match &key {
            Some(id) => {
                editor.keys.tests.insert(id.clone(), KeyTest::Testing);
            }
            None => {
                editor.keys.testing = true;
                editor.keys.result = None;
            }
        }
        self.error = None;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match test {
                        ServiceTest::Ocr(service) => ServiceTest::run_ocr(&service),
                        test => tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|_| "无法初始化翻译网络客户端。".to_string())?
                            .block_on(test.run()),
                    }
                })
                .await;
            log::info!(
                "event=gpui.recognition_service_tested ok={}",
                result.is_ok()
            );
            let _ = this.update(cx, |view, cx| {
                let Some(editor) = view.service_editor.as_mut() else {
                    return;
                };
                match key {
                    // 密钥列表已重新读取（保存或删除后），旧结果丢弃。
                    Some(_) if editor.keys.revision != revision => {}
                    Some(id) => {
                        let state = match result {
                            Ok(()) => KeyTest::Ok,
                            Err(error) => KeyTest::Failed(error),
                        };
                        editor.keys.tests.insert(id, state);
                    }
                    None => {
                        editor.keys.testing = false;
                        editor.keys.result = Some(result);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn link(
        &self,
        id: impl Into<ElementId>,
        label: String,
        danger: bool,
        enabled: bool,
    ) -> Stateful<Div> {
        let link = div()
            .id(id)
            .text_xs()
            .text_color(rgb(if danger { col::danger() } else { col::accent() }))
            .child(label);
        if enabled {
            link.cursor_pointer().hover(|s| s.opacity(0.8))
        } else {
            link.opacity(0.5)
        }
    }

    /// “API 密钥”行：已保存密钥、新密钥输入框与“添加另一个密钥”。
    pub(in super::super) fn key_rows(
        &self,
        editor: &ServiceEditor,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let moved = self.endpoint_moved(cx);
        let stored = editor.existing.is_some();
        let saved: &[ServiceKeyInfo] = if moved {
            &[]
        } else {
            editor.keys.saved.as_deref().unwrap_or(&[])
        };
        let total = saved.len() + editor.keys.new.len();
        let idle = !editor.keys.testing;
        let mut list = div().w(px(300.0)).flex().flex_col().gap(px(6.0));
        if editor.keys.saved.is_none() && stored && !moved {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(rgb(col::muted()))
                    .child(t("正在读取…")),
            );
        }
        for (index, info) in saved.iter().enumerate() {
            let id = info.id.clone();
            let (status, failure) = match editor.keys.tests.get(&info.id) {
                Some(KeyTest::Ok) => (
                    Some(
                        div()
                            .text_xs()
                            .text_color(rgb(crate::theme::c().ok_text))
                            .child(t("可用")),
                    ),
                    None,
                ),
                Some(KeyTest::Failed(error)) => (
                    Some(
                        div()
                            .text_xs()
                            .text_color(rgb(col::danger()))
                            .child(t("不可用")),
                    ),
                    Some(
                        div()
                            .px(px(8.0))
                            .text_xs()
                            .text_color(rgb(col::danger()))
                            .child(t(error)),
                    ),
                ),
                _ => (None, None),
            };
            let testing = matches!(editor.keys.tests.get(&info.id), Some(KeyTest::Testing));
            let test_id = id.clone();
            list = list.child(
                div()
                    .h(px(28.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(6.0))
                    .bg(rgb(col::page_bg()))
                    .child(
                        div()
                            .text_xs()
                            .font_family("Consolas")
                            .text_color(rgb(if info.hint.is_some() {
                                col::text()
                            } else {
                                col::muted()
                            }))
                            .child(info.hint.clone().unwrap_or_else(|| t("未找到密钥"))),
                    )
                    .children((index == 0 && total > 1).then(|| {
                        div()
                            .text_xs()
                            .text_color(rgb(col::muted()))
                            .child(t("首选"))
                    }))
                    .child(div().flex_1())
                    .children(status)
                    .child(
                        self.link(
                            ("key-test", index),
                            t(if testing { "正在测试…" } else { "测试" }),
                            false,
                            idle && !testing,
                        )
                        .on_click(cx.listener(
                            move |view, _: &ClickEvent, window, cx| {
                                view.test_service(Some(test_id.clone()), window, cx)
                            },
                        )),
                    )
                    .child(
                        self.link(("key-remove", index), t("删除"), true, idle)
                            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                                view.remove_key(id.clone(), window, cx)
                            })),
                    ),
            );
            list = list.children(failure);
        }
        for state in &editor.keys.new {
            list = list.child(
                div()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(col::border()))
                    .bg(rgb(col::surface()))
                    .child(Input::new(state)),
            );
        }
        let can_add = idle && total < MAX_KEYS;
        let mut add = self.link(
            "key-add",
            format!("+ {}", t("添加另一个密钥")),
            false,
            can_add,
        );
        if can_add {
            add = add.on_click(
                cx.listener(|view, _: &ClickEvent, window, cx| view.add_key_input(window, cx)),
            );
        }
        list = list.child(add);
        let hint = moved.then(|| t("服务地址已更改，原密钥不会发往新地址，请重新填写密钥。"));
        row("API Key / Token".into(), hint, list)
    }

    /// “测试”按钮与结果（保存、取消旁）。
    pub(in super::super) fn test_controls(
        &self,
        editor: &ServiceEditor,
        t: &dyn Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> (Stateful<Div>, Option<Div>) {
        let testing = editor.keys.testing;
        let label = if testing { "正在测试…" } else { "测试" };
        let mut button = self.action_button("service-test", t(label), !testing);
        if !testing {
            button =
                button.on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                    view.test_service(None, window, cx)
                }));
        }
        let result = editor.keys.result.as_ref().map(|result| match result {
            Ok(()) => div()
                .text_xs()
                .text_color(rgb(crate::theme::c().ok_text))
                .child(t("测试成功，服务可用。")),
            Err(error) => div()
                .text_xs()
                .text_color(rgb(col::danger()))
                .child(t(error)),
        });
        (button, result)
    }
}
