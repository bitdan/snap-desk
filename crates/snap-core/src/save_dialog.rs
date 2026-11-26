//! Windows 保存对话框（IFileSaveDialog），用于截图、贴图与滚动截图结果的“保存”。
//!
//! rfd 只返回路径，拿不到用户在“保存类型”里选了什么；系统对话框换类型时也不会替换文件名里已带的扩展名，
//! 于是选了 PDF 仍存成 .png。这里监听换类型（`OnTypeChange`），像“画图”一样把文件名里的扩展名换成新类型的，
//! 并更新默认扩展名（用户删掉扩展名时系统按它补上）。覆盖确认仍由系统按最终文件名完成，
//! 调用方再按返回路径的扩展名选择编码格式（`export::write_file`）。
//!
//! 部分第三方文件管理器会接管系统文件对话框，它们不一定发出换类型事件、也不一定允许改文件名，
//! 返回的仍是预填的扩展名。因此关闭后再读一次所选类型（[`resolve`]）：文件名还是预填扩展名、或没有已知扩展名时，
//! 按所选类型换上扩展名；换名后的文件已存在时由这里补一次覆盖确认（系统只确认过原文件名）。
//!
//! 对话框是模态的，会阻塞调用线程直到关闭；回调也在该线程执行。调用方应在单独线程中调用 [`show`]。
use std::path::PathBuf;
use windows::{
    core::{implement, Ref, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::{E_NOTIMPL, ERROR_CANCELLED, HWND},
        UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FileSaveDialog, IFileDialog, IFileDialogEvents,
            IFileDialogEvents_Impl, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName,
            FDE_OVERWRITE_RESPONSE, FDE_SHAREVIOLATION_RESPONSE, FOS_FORCEFILESYSTEM,
            FOS_OVERWRITEPROMPT, SIGDN_FILESYSPATH,
        },
    },
};

/// 一种保存类型：显示名称与扩展名（不带点，第一个为该类型的默认扩展名）。
pub struct FileType {
    pub name: String,
    pub extensions: Vec<String>,
}

pub struct SaveRequest {
    pub title: String,
    /// 默认文件名。可以不带扩展名（关闭后按所选类型补上）；带的话，系统对话框换类型时随之替换。
    pub file_name: String,
    pub directory: Option<PathBuf>,
    /// 保存类型，第一项默认选中。
    pub types: Vec<FileType>,
    /// 所属窗口（HWND）：对话框关闭前禁用它，并显示在它上面。
    pub owner: Option<isize>,
    /// 按所选类型换了扩展名、而新文件已存在时的确认文字，`{0}` 为文件名（调用方已按界面语言翻译）。
    pub replace_prompt: String,
}

/// 显示保存对话框并阻塞到关闭；取消返回 `Ok(None)`。
pub fn show(request: &SaveRequest) -> Result<Option<PathBuf>, String> {
    // 对话框与回调都在本线程：单线程套间。已初始化过（S_FALSE）同样要配对释放；模式冲突时不释放。
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = unsafe { show_dialog(request) }.map_err(|error| error.message());
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

unsafe fn show_dialog(request: &SaveRequest) -> windows::core::Result<Option<PathBuf>> {
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        // 过滤串（如“*.jpg;*.jpeg”）与名称的宽字符缓冲须活到 SetFileTypes 之后。
        let names: Vec<HSTRING> = request.types.iter().map(|t| HSTRING::from(t.name.as_str())).collect();
        let specs: Vec<HSTRING> = request
            .types
            .iter()
            .map(|t| HSTRING::from(t.extensions.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";")))
            .collect();
        let filters: Vec<COMDLG_FILTERSPEC> = names
            .iter()
            .zip(&specs)
            .map(|(name, spec)| COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) })
            .collect();
        if !filters.is_empty() {
            dialog.SetFileTypes(&filters)?;
            dialog.SetFileTypeIndex(1)?;
        }
        if let Some(extension) = request.types.first().and_then(|t| t.extensions.first()) {
            dialog.SetDefaultExtension(&HSTRING::from(extension.as_str()))?;
        }
        dialog.SetOptions(dialog.GetOptions()? | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM)?;
        dialog.SetTitle(&HSTRING::from(request.title.as_str()))?;
        dialog.SetFileName(&HSTRING::from(request.file_name.as_str()))?;
        // 记住的目录已被移走时忽略，交给系统选默认位置。
        if let Some(directory) = &request.directory {
            if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(directory.as_path()), None) {
                let _ = dialog.SetFolder(&folder);
            }
        }
        let events: IFileDialogEvents = TypeSync {
            extensions: request.types.iter().map(|t| t.extensions.clone()).collect(),
        }
        .into();
        let cookie = dialog.Advise(&events)?;
        let shown = dialog.Show(request.owner.map(|hwnd| HWND(hwnd as *mut _)));
        let _ = dialog.Unadvise(cookie);
        match shown {
            Err(error) if error.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
            Err(error) => return Err(error),
            Ok(()) => {}
        }
        let returned = PathBuf::from(take_string(dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?));
        let chosen = dialog.GetFileTypeIndex().ok().map(|index| index as usize);
        let path = resolve(&returned, chosen, &request.types);
        // 只记录类型序号与扩展名，不记录路径；用于判断接管对话框的程序是否如实报告所选类型。
        log::info!(
            "event=save_dialog.result type_index={} extension={} renamed={}",
            chosen.map_or_else(|| "unknown".to_owned(), |index| index.to_string()),
            extension_of(&returned).unwrap_or_default(),
            path != returned
        );
        if path != returned && path.exists() {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let text = HSTRING::from(request.replace_prompt.replace("{0}", &name));
            let answer = MessageBoxW(
                request.owner.map(|hwnd| HWND(hwnd as *mut _)),
                &text,
                &HSTRING::from(request.title.as_str()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            );
            if answer != IDYES {
                return Ok(None);
            }
        }
        Ok(Some(path))
    }
}

fn extension_of(path: &std::path::Path) -> Option<String> {
    path.extension().map(|ext| ext.to_string_lossy().into_owned())
}

/// 按所选类型（从 1 开始的序号）确定最终路径：文件名的扩展名已属于所选类型，或是用户自己写的其他已知类型
/// 扩展名（不是预填的默认扩展名）时保持原样；仍是预填扩展名、或没有已知扩展名时，换成 / 补上所选类型的扩展名。
/// 读不到所选类型时保持原样，由调用方按扩展名处理。
fn resolve(path: &std::path::Path, chosen: Option<usize>, types: &[FileType]) -> PathBuf {
    let Some(target) = chosen.and_then(|index| types.get(index.wrapping_sub(1))) else {
        return path.to_path_buf();
    };
    let Some(wanted) = target.extensions.first() else {
        return path.to_path_buf();
    };
    let current = extension_of(path);
    let is = |list: &[String]| current.as_deref().is_some_and(|ext| list.iter().any(|e| e.eq_ignore_ascii_case(ext)));
    if is(&target.extensions) {
        return path.to_path_buf();
    }
    let known = types.iter().any(|t| is(&t.extensions));
    let prefilled = types.first().is_some_and(|t| is(&t.extensions));
    if known && !prefilled {
        return path.to_path_buf();
    }
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = if known { name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem) } else { name.as_str() };
    path.with_file_name(format!("{stem}.{wanted}"))
}

/// 读出系统分配的字符串并释放。
unsafe fn take_string(text: PWSTR) -> String {
    unsafe {
        let value = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as _));
        value
    }
}

/// 换类型时的新文件名：以任一已知类型的扩展名结尾时换成新类型的扩展名；没有扩展名或是用户自己写的
/// 其他扩展名时返回 None，保持原样（没有扩展名时系统按默认扩展名补上）。
fn swap_extension(name: &str, extension: &str, known: &[Vec<String>]) -> Option<String> {
    let (stem, current) = name.rsplit_once('.')?;
    let recognized = known.iter().flatten().any(|e| e.eq_ignore_ascii_case(current));
    (recognized && !current.eq_ignore_ascii_case(extension) && !stem.is_empty()).then(|| format!("{stem}.{extension}"))
}

/// 对话框事件：只处理换类型，其余返回默认行为（E_NOTIMPL 表示交给系统处理）。
#[implement(IFileDialogEvents)]
struct TypeSync {
    /// 与保存类型一一对应的扩展名。
    extensions: Vec<Vec<String>>,
}

impl IFileDialogEvents_Impl for TypeSync_Impl {
    fn OnFileOk(&self, _: Ref<IFileDialog>) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnFolderChanging(&self, _: Ref<IFileDialog>, _: Ref<IShellItem>) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnFolderChange(&self, _: Ref<IFileDialog>) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnSelectionChange(&self, _: Ref<IFileDialog>) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnShareViolation(&self, _: Ref<IFileDialog>, _: Ref<IShellItem>) -> windows::core::Result<FDE_SHAREVIOLATION_RESPONSE> {
        Err(E_NOTIMPL.into())
    }

    fn OnTypeChange(&self, dialog: Ref<IFileDialog>) -> windows::core::Result<()> {
        let dialog = dialog.ok()?;
        unsafe {
            // 类型序号从 1 开始。
            let index = dialog.GetFileTypeIndex()? as usize;
            log::info!("event=save_dialog.type_changed type_index={index}");
            let Some(extension) = self.extensions.get(index.wrapping_sub(1)).and_then(|e| e.first()) else {
                return Ok(());
            };
            dialog.SetDefaultExtension(&HSTRING::from(extension.as_str()))?;
            let name = take_string(dialog.GetFileName()?);
            if let Some(renamed) = swap_extension(&name, extension, &self.extensions) {
                dialog.SetFileName(&HSTRING::from(renamed))?;
            }
        }
        Ok(())
    }

    fn OnOverwrite(&self, _: Ref<IFileDialog>, _: Ref<IShellItem>) -> windows::core::Result<FDE_OVERWRITE_RESPONSE> {
        Err(E_NOTIMPL.into())
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve, swap_extension, FileType};
    use std::path::Path;

    fn types() -> Vec<FileType> {
        [("PNG", vec!["png"]), ("JPEG", vec!["jpg", "jpeg"]), ("WebP", vec!["webp"]), ("PDF", vec!["pdf"])]
            .into_iter()
            .map(|(name, extensions)| FileType { name: name.into(), extensions: extensions.into_iter().map(String::from).collect() })
            .collect()
    }

    #[test]
    fn chosen_type_wins_over_the_prefilled_extension() {
        let types = types();
        let dir = Path::new(r"C:\shots");
        // 接管的对话框没改文件名：仍是预填的 .png，但选了 PDF。
        assert_eq!(resolve(&dir.join("a.png"), Some(4), &types), dir.join("a.pdf"));
        // 已经是所选类型、读不到所选类型：保持原样。
        assert_eq!(resolve(&dir.join("a.pdf"), Some(4), &types), dir.join("a.pdf"));
        assert_eq!(resolve(&dir.join("a.png"), None, &types), dir.join("a.png"));
        // 用户自己写了另一种已知扩展名：以文件名为准。
        assert_eq!(resolve(&dir.join("a.jpg"), Some(4), &types), dir.join("a.jpg"));
        // 没有扩展名或不是已知扩展名：补上所选类型的扩展名。
        assert_eq!(resolve(&dir.join("notes"), Some(2), &types), dir.join("notes.jpg"));
        assert_eq!(resolve(&dir.join("v1.2 notes"), Some(4), &types), dir.join("v1.2 notes.pdf"));
    }

    #[test]
    fn type_change_swaps_only_known_extensions() {
        let known: Vec<Vec<String>> = [vec!["png"], vec!["jpg", "jpeg"], vec!["webp"], vec!["pdf"]]
            .into_iter()
            .map(|e| e.into_iter().map(String::from).collect())
            .collect();
        assert_eq!(swap_extension("SnapDesk-20261126-111504.png", "pdf", &known).as_deref(), Some("SnapDesk-20261126-111504.pdf"));
        assert_eq!(swap_extension("my.shot.JPEG", "webp", &known).as_deref(), Some("my.shot.webp"));
        // 已是该类型、没有扩展名、用户自己写的扩展名：保持原样。
        assert_eq!(swap_extension("a.pdf", "pdf", &known), None);
        assert_eq!(swap_extension("notes", "pdf", &known), None);
        assert_eq!(swap_extension("v1.2 notes", "pdf", &known), None);
        assert_eq!(swap_extension(".png", "pdf", &known), None);
    }
}
