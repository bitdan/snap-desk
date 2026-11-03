//! 贴图内容的系统拖出：数据只取当前窗口，图片合成后作为只读临时 PNG 提供。
use super::{composed, err, own, Operation, PinState};
use tauri::{AppHandle, Manager, WebviewWindow};

/// 编码在后台完成，OLE 拖拽在 UI 的 STA 线程运行；等待期间不持有贴图集合锁。
#[tauri::command]
pub async fn drag_pin_content(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let pin = own(&app, &window)?;
    if pin.view.locked || pin.view.click_through {
        return Err("贴图已锁定或已开启鼠标穿透。".into());
    }
    #[cfg(not(windows))]
    {
        let _ = (app, pin);
        Err("当前平台尚未支持贴图内容拖出。".into())
    }
    #[cfg(windows)]
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<PinState>();
        let _operation = Operation::acquire(&state)?;
        let (format, bytes) = if let Some(text) = &pin.drag_text {
            (13, wide_bytes(text)) // CF_UNICODETEXT：文件贴图同样拖出路径文字。
        } else {
            let file = tempfile::Builder::new()
                .prefix("snap-desk-drag-")
                .suffix(".png")
                .tempfile()
                .map_err(err)?;
            let path = file.into_temp_path();
            composed(&pin)?
                .save_with_format(&path, image::ImageFormat::Png)
                .map_err(err)?;
            let name = path.to_str().ok_or("临时图片路径编码无效。")?;
            let mut bytes = vec![0u8; 20]; // DROPFILES：20 字节头，Unicode，双 NUL 结束。
            bytes[..4].copy_from_slice(&20u32.to_le_bytes());
            bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
            bytes.extend(wide_bytes(name));
            bytes.extend([0, 0]);
            state.drag_files.lock().map_err(err)?.push(path);
            (15, bytes) // CF_HDROP：图像以 PNG 文件交给微信、资源管理器等应用。
        };
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let _ = sender.send(windows_drag::start(format, bytes));
        })
        .map_err(err)?;
        receiver.recv().map_err(err)?
    })
    .await
    .map_err(err)?
}

/// 包含终止 NUL，避免跨进程接收方越界读取文本。
fn wide_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
mod windows_drag {
    use windows::{
        core::{implement, Ref, HRESULT},
        Win32::{
            Foundation::{
                GlobalFree, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
                DV_E_FORMATETC, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, OLE_E_ADVISENOTSUPPORTED,
                S_OK,
            },
            System::{
                Com::{
                    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA,
                    DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
                },
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
                Ole::{
                    DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize,
                    DROPEFFECT, DROPEFFECT_COPY,
                },
                SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
            },
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
                Shell::SHCreateStdEnumFmtEtc,
            },
        },
    };

    /// 每次 GetData 都给接收方独立的 HGLOBAL，所有权交给 ReleaseStgMedium。
    #[implement(IDataObject)]
    struct Data {
        format: u16,
        bytes: Vec<u8>,
    }
    impl Data {
        fn descriptor(&self) -> FORMATETC {
            FORMATETC {
                cfFormat: self.format,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            }
        }
    }
    #[allow(non_snake_case)]
    impl IDataObject_Impl for Data_Impl {
        fn QueryGetData(&self, request: *const FORMATETC) -> HRESULT {
            let Some(request) = (unsafe { request.as_ref() }) else {
                return E_POINTER;
            };
            if request.cfFormat == self.format
                && request.dwAspect == DVASPECT_CONTENT.0
                && request.lindex == -1
                && request.ptd.is_null()
                && request.tymed & TYMED_HGLOBAL.0 as u32 != 0
            {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }
        fn GetData(&self, request: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
            self.QueryGetData(request).ok()?;
            unsafe {
                let memory = GlobalAlloc(GMEM_MOVEABLE, self.bytes.len())?;
                let pointer = GlobalLock(memory);
                if pointer.is_null() {
                    let _ = GlobalFree(Some(memory));
                    return Err(E_OUTOFMEMORY.into());
                }
                std::ptr::copy_nonoverlapping(
                    self.bytes.as_ptr(),
                    pointer.cast(),
                    self.bytes.len(),
                );
                let _ = GlobalUnlock(memory);
                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: memory },
                    pUnkForRelease: Default::default(),
                })
            }
        }
        fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, output: *mut FORMATETC) -> HRESULT {
            if output.is_null() {
                return E_POINTER;
            }
            unsafe {
                (*output).ptd = std::ptr::null_mut();
            }
            E_NOTIMPL
        }
        fn SetData(
            &self,
            _: *const FORMATETC,
            _: *const STGMEDIUM,
            _: windows::core::BOOL,
        ) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
            if direction != 1 {
                return Err(E_NOTIMPL.into());
            }
            unsafe { SHCreateStdEnumFmtEtc(&[self.descriptor()]) }
        }
        fn DAdvise(
            &self,
            _: *const FORMATETC,
            _: u32,
            _: Ref<IAdviseSink>,
        ) -> windows::core::Result<u32> {
            Err(OLE_E_ADVISENOTSUPPORTED.into())
        }
        fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
            Err(OLE_E_ADVISENOTSUPPORTED.into())
        }
        fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
            Err(OLE_E_ADVISENOTSUPPORTED.into())
        }
    }

    /// Ctrl 仅负责启动；拖拽循环只根据 Esc 和左键判断取消或放下。
    #[implement(IDropSource)]
    struct Source;
    #[allow(non_snake_case)]
    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(
            &self,
            escape: windows::core::BOOL,
            keys: MODIFIERKEYS_FLAGS,
        ) -> HRESULT {
            if escape.as_bool() {
                DRAGDROP_S_CANCEL
            } else if keys.0 & MK_LBUTTON.0 == 0 {
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }
        fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }
    struct OleGuard;
    impl Drop for OleGuard {
        fn drop(&mut self) {
            unsafe { OleUninitialize() }
        }
    }

    pub(super) fn start(format: u16, bytes: Vec<u8>) -> Result<(), String> {
        unsafe {
            // 大图编码期间用户可能已松手，此时取消，不能在晚到的启动请求中意外放下。
            if GetAsyncKeyState(VK_LBUTTON.0 as i32) >= 0 {
                return Ok(());
            }
            OleInitialize(None).map_err(super::err)?;
            let _ole = OleGuard;
            let data: IDataObject = Data { format, bytes }.into();
            let source: IDropSource = Source.into();
            let mut effect = DROPEFFECT::default();
            // 仅允许复制，不让拖入文件管理器的动作删除贴图或临时图像。
            DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect)
                .ok()
                .map_err(super::err)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::System::SystemServices::MK_CONTROL;

        #[test]
        fn releasing_ctrl_keeps_dragging_until_left_button_released() {
            let source: IDropSource = Source.into();
            unsafe {
                assert_eq!(
                    source.QueryContinueDrag(false, MK_LBUTTON | MK_CONTROL),
                    S_OK
                );
                assert_eq!(source.QueryContinueDrag(false, MK_LBUTTON), S_OK);
                assert_eq!(
                    source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(0)),
                    DRAGDROP_S_DROP
                );
                assert_eq!(
                    source.QueryContinueDrag(true, MK_LBUTTON),
                    DRAGDROP_S_CANCEL
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_text_has_a_terminator_and_preserves_paths_and_newlines() {
        let value = "C:\\中文\\a.txt\nD:\\b.txt";
        let bytes = wide_bytes(value);
        assert_eq!(&bytes[bytes.len() - 2..], &[0, 0]);
        let units = bytes
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .take_while(|v| *v != 0)
            .collect::<Vec<_>>();
        assert_eq!(String::from_utf16(&units).unwrap(), value);
    }
}
