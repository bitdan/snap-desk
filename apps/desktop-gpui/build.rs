//! Windows：把应用图标（resources/icons/icon.ico）嵌入程序资源 ID 1。gpui-pre 注册窗口类时读取这个资源，
//! 标题栏、任务栏与 Alt+Tab 显示 Snap Desk 图标；资源管理器里的程序文件也用它。
//! macOS：显式链接系统 OCR 与录屏声音用到的框架。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        // 系统 OCR 通过 Objective-C 运行时按类名查找 Vision 的类，没有直接引用的 C 符号，
        // 必须显式链接框架，否则运行时找不到 VNRecognizeTextRequest。
        println!("cargo:rustc-link-lib=framework=Vision");
        // 录制系统声音按类名使用 SCStream 等 ScreenCaptureKit 类，同样需要显式链接。
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
    }
    if target_os != "windows" {
        return;
    }
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon = manifest.join("../../resources/icons/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = out.join("snap-desk.rc");
    // .rc 里的路径用正斜杠，避免反斜杠被当成转义。
    let path = icon.canonicalize().unwrap_or(icon).display().to_string().replace('\\', "/");
    let path = path.trim_start_matches("//?/").to_string();
    std::fs::write(&rc, format!("1 ICON \"{path}\"\n")).expect("写入资源脚本失败");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().expect("嵌入应用图标失败");
}
