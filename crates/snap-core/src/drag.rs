//! 把内容拖到其他应用（系统拖出）。Windows 使用 OLE DoDragDrop；macOS / Linux 的实现依赖各外壳的
//! 原生窗口对象，仍在 `src-tauri/src/shared/native_drag_*.rs`。
//! 调用方只能提供受信任的内容：文字，或由 [`image_file`] 从像素编码出的临时 PNG。

/// 拖出的内容：文字或本地文件路径（图片先由 [`image_file`] 写成临时 PNG）。
pub enum Payload {
    Text(String),
    Files(Vec<String>),
}

/// 接收方可能在放下后才异步读取文件：临时文件统一保留到进程退出。
static TEMP_FILES: std::sync::Mutex<Vec<tempfile::TempPath>> = std::sync::Mutex::new(Vec::new());

/// 只编码受信任的像素，调用方不能传任意磁盘路径；返回临时 PNG 的路径。
pub fn image_file(image: &image::RgbaImage) -> Result<String, String> {
    let path = tempfile::Builder::new()
        .prefix("snap-desk-drag-")
        .suffix(".png")
        .tempfile()
        .map_err(err)?
        .into_temp_path();
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(err)?;
    let name = path.to_str().ok_or("临时图片路径编码无效。")?.to_owned();
    TEMP_FILES.lock().map_err(err)?.push(path);
    Ok(name)
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Windows：必须在 UI（STA）线程、且不处于界面框架的状态借用中调用；DoDragDrop 自带消息循环，
/// 阻塞到放下或取消。调用时左键已松开（例如大图编码期间用户已松手）则直接返回，不会意外放下。
#[cfg(windows)]
pub fn start(payload: Payload) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    if ACTIVE.swap(true, Ordering::AcqRel) {
        return Err("另一个拖拽正在进行。".into());
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    // CF_UNICODETEXT 与 CF_HDROP。
    let (format, bytes) = match payload {
        Payload::Text(text) => (13, wide_bytes(&text)),
        Payload::Files(paths) => (15, file_bytes(&paths)),
    };
    windows_drag::start(format, bytes)
}

/// CF_HDROP 的 Unicode 路径列表，包含 DROPFILES 头与双 NUL。
#[cfg(windows)]
fn file_bytes(paths: &[String]) -> Vec<u8> {
    let mut bytes = vec![0u8; 20];
    bytes[..4].copy_from_slice(&20u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    for path in paths {
        bytes.extend(wide_bytes(path));
    }
    bytes.extend([0, 0]);
    bytes
}

/// 包含终止 NUL，避免跨进程接收方越界读取文本。
#[cfg_attr(not(windows), allow(dead_code))]
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
