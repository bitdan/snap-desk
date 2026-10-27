fn main() {
    // Tauri 从这个文件读取应用版本；只改 package.json 时也必须刷新内嵌版本与系统资源。
    println!("cargo:rerun-if-changed=../package.json");
    tauri_build::build()
}
