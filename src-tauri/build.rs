fn main() {
    // 发布构建注入公钥；本地开发未配置时仍可启动，但不能下载或安装更新。
    println!("cargo:rerun-if-env-changed=SNAP_DESK_UPDATER_PUBLIC_KEY");
    // Tauri 从这个文件读取应用版本；只改 package.json 时也必须刷新内嵌版本与系统资源。
    println!("cargo:rerun-if-changed=../package.json");
    tauri_build::build()
}
