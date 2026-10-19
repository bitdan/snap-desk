//! Windows CPU 引擎的按需安装与延迟加载；其他功能不触碰 ort API，也不需要 DLL。
//! 清单固定架构、版本与摘要；只提取白名单条目，不解压 SDK、PDB 或任意归档路径。
use crate::resources::{download_file, verified, Asset};
use serde::Deserialize;
use std::{fs, io::Read, path::Path, sync::Mutex};

/// 同时保护安装和加载。只有加载成功才置 true，失败后可下载修复并重试。
static LOADED: Mutex<bool> = Mutex::new(false);

#[derive(Deserialize)]
struct Runtime {
    arch: String,
    directory: String,
    archive: Asset,
    files: Vec<Asset>,
}

fn manifest() -> Result<Runtime, String> {
    let entries: Vec<Runtime> = serde_json::from_str(include_str!("../assets/onnxruntime.json"))
        .map_err(|e| e.to_string())?;
    entries
        .into_iter()
        .find(|r| r.arch == std::env::consts::ARCH)
        .ok_or_else(|| "OCR 引擎下载目前仅支持 Windows x64 / ARM64。".into())
}

/// 每次检查本机文件摘要，不用文件存在或上次下载标记冒充就绪。
pub fn ready(dir: &Path) -> bool {
    manifest().map(|r| installed(dir, &r)).unwrap_or(false)
}

pub fn download_mib() -> u64 {
    manifest()
        .map(|r| r.archive.size.div_ceil(1024 * 1024))
        .unwrap_or(0)
}

fn installed(dir: &Path, runtime: &Runtime) -> bool {
    runtime
        .files
        .iter()
        .all(|f| verified(&dir.join(&runtime.directory).join(&f.name), f))
}

/// 点击“下载识别资源”时调用；已有模型不受影响。加载后的 DLL 由系统锁定，修复需退出应用。
pub fn download(
    app: &tauri::AppHandle,
    client: &reqwest::blocking::Client,
    dir: &Path,
) -> Result<(), String> {
    let loaded = LOADED
        .try_lock()
        .map_err(|_| "OCR 引擎正在加载或更新，请稍后重试。")?;
    let runtime = manifest()?;
    if installed(dir, &runtime) {
        return Ok(());
    }
    if *loaded {
        return Err("OCR 引擎已加载，请退出应用后重新下载修复。".into());
    }
    let archive = download_file(app, client, dir, &runtime.archive)?;
    install(archive.path(), dir, &runtime)
}

/// 所有条目先在临时目录完成校验，再逐个原子替换；中途失败仍为未就绪，重试可修复。
fn install(archive: &Path, dir: &Path, runtime: &Runtime) -> Result<(), String> {
    if !verified(archive, &runtime.archive) {
        return Err("OCR 引擎压缩包校验失败。".into());
    }
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("OCR 引擎压缩包损坏：{e}"))?;
    let staging = tempfile::tempdir_in(dir).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for entry in &runtime.files {
        // 目标路径仅来自内置清单；归档中其他路径（包括 ../、符号链接）均不提取。
        let source = zip
            .by_name(&format!("{}/{}", runtime.directory, entry.name))
            .map_err(|e| format!("OCR 引擎缺少 {}：{e}", entry.name))?;
        if source.size() != entry.size {
            return Err("OCR 引擎文件大小不符。".into());
        }
        let mut target =
            tempfile::NamedTempFile::new_in(staging.path()).map_err(|e| e.to_string())?;
        std::io::copy(&mut source.take(entry.size + 1), &mut target).map_err(|e| e.to_string())?;
        if !verified(target.path(), entry) {
            return Err(format!("{} 校验失败。", entry.name));
        }
        target.as_file().sync_all().map_err(|e| e.to_string())?;
        files.push((target, dir.join(&runtime.directory).join(&entry.name)));
    }
    for (file, path) in files {
        fs::create_dir_all(path.parent().ok_or("OCR 引擎目录无效。")?)
            .map_err(|e| e.to_string())?;
        file.persist(path)
            .map_err(|e| format!("OCR 引擎安装失败，请退出应用后重试：{e}"))?;
    }
    Ok(())
}

/// 首次识别调用。校验后用绝对路径初始化；缺失或系统加载失败返回可操作错误，不影响应用启动。
pub fn initialize(dir: &Path) -> Result<(), String> {
    let mut loaded = LOADED
        .try_lock()
        .map_err(|_| "OCR 引擎正在加载或更新，请稍后重试。")?;
    if *loaded {
        return Ok(());
    }
    let runtime = manifest()?;
    if !installed(dir, &runtime) {
        return Err("OCR 引擎未下载或校验失败，请在“识别与翻译”页下载识别资源。".into());
    }
    let path = dir
        .join(&runtime.directory)
        .join("lib/onnxruntime.dll")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    ort::init_from(&path)
        .map_err(|e| {
            format!("OCR 引擎加载失败，请确认系统的 Microsoft Visual C++ 2015–2022 运行库可用：{e}")
        })?
        .commit();
    *loaded = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn asset(name: &str, bytes: &[u8]) -> Asset {
        Asset {
            kind: crate::resources::ResourceKind::Ocr,
            name: name.into(),
            url: String::new(),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }

    #[test]
    fn install_checks_archive_and_entries_before_replacing_files() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("runtime.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in [
            ("runtime/lib/onnxruntime.dll", b"new"),
            ("runtime/LICENSE", b"MIT"),
            ("../outside", b"bad"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut runtime = Runtime {
            arch: String::new(),
            directory: "runtime".into(),
            archive: asset("runtime.zip", &fs::read(&archive).unwrap()),
            files: vec![
                asset("lib/onnxruntime.dll", b"new"),
                asset("LICENSE", b"BAD"),
            ],
        };
        let dll = dir.path().join("runtime/lib/onnxruntime.dll");
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"old").unwrap();
        assert!(install(&archive, dir.path(), &runtime).is_err());
        assert_eq!(fs::read(&dll).unwrap(), b"old");
        assert!(!installed(dir.path(), &runtime));
        runtime.files[1] = asset("LICENSE", b"MIT");
        install(&archive, dir.path(), &runtime).unwrap();
        assert!(installed(dir.path(), &runtime));
        assert!(!dir.path().join("outside").exists());
        fs::write(&archive, b"corrupt").unwrap();
        assert!(install(&archive, dir.path(), &runtime).is_err());
        assert_eq!(fs::read(&dll).unwrap(), b"new");
    }

    #[test]
    fn missing_runtime_returns_error_without_loading_ort() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!ready(dir.path()));
        // 本用例无需 DLL，也不调用任何可能触发 ort 默认搜索的 API。
        assert!(initialize(dir.path()).unwrap_err().contains("下载"));
        assert!(initialize(dir.path()).is_err(), "失败不能阻止之后重试");
    }

    #[test]
    #[ignore = "需官方 ZIP；仅安装到 target/perf-assets，不修改用户资源目录"]
    fn install_official_runtime_for_ocr_test() {
        let runtime = manifest().unwrap();
        let dir = crate::perf::assets();
        fs::create_dir_all(&dir).unwrap();
        let archive = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(&runtime.archive.name);
        install(&archive, &dir, &runtime).unwrap();
        assert!(ready(&dir));
        initialize(&dir).unwrap();
        // info 是构建信息（分支/提交/编译选项），不保证包含发布版本号。
        // 版本身份由上面的官方 DLL 摘要校验保证，此处验证加载后的 API 确实可调用。
        assert!(!ort::info().is_empty());
        println!("{}", ort::info());
    }
}
