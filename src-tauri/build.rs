fn main() {
    // 发布构建注入公钥；本地开发未配置时仍可启动，但不能下载或安装更新。
    println!("cargo:rerun-if-env-changed=SNAP_DESK_UPDATER_PUBLIC_KEY");
    // Tauri 从这个文件读取应用版本；只改 package.json 时也必须刷新内嵌版本与系统资源。
    println!("cargo:rerun-if-changed=../package.json");
    // macOS 系统 OCR 通过 Objective-C 运行时按类名查找 Vision 的类，没有直接引用的 C 符号，
    // 必须显式链接框架，否则运行时找不到 VNRecognizeTextRequest。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=Vision");
        // 录制系统声音按类名使用 SCStream 等 ScreenCaptureKit 类，同样需要显式链接。
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
    }
    tauri_build::build()
}
