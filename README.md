# Snap Desk

基于 **Tauri 2 + Rust + Vue 3** 开发的开源桌面工具，集截图、标注、贴图、录屏和文字识别于一体。

目前优先支持 **Windows**，macOS / Linux 仍在适配中。

## 功能

- **截图与标注**：区域 / 全屏截图、窗口识别、快捷键，以及文字、箭头、马赛克等标注工具。
- **滚动截图**：手动滚动、自动拼接，提供二维预览及白底 / 透明 PNG 导出。
- **贴图**：将截图、图片或剪贴板内容固定在桌面，支持缩放和编辑。
- **录屏**：录制无声 MP4 / GIF，支持键鼠操作可视化。
- **OCR 与翻译**：本地文字识别、中英互译，可选在线翻译服务。
- **剪贴板管理**：保存和搜索历史记录，支持贴图及 PDF / 图片导出。
- **外观与引导**：通用设置可保存浅色 / 深色 / 跟随系统主题及中英文偏好。中英文覆盖主导航、截图设置、快捷键、通用设置主体与新手引导，其他功能页和系统错误仍保留原文。帮助页可重新打开新手引导。

## 下载

前往 [Releases](https://github.com/bitdan/snap-desk/releases) 获取 Windows 版本。OCR 和本地翻译所需模型可在应用内按需下载。

## 本地开发

需要 Node.js、Rust 及 [Tauri 2 开发环境](https://v2.tauri.app/start/prerequisites/)。

```bash
git clone https://github.com/bitdan/snap-desk.git
cd snap-desk
npm ci
npm run desktop:dev
```

构建独立可运行的调试程序：`npm run desktop:build:fast`，输出为
`src-tauri/target/debug/snap-desk.exe`；发布优化程序使用 `npm run desktop:build`。
构建前从托盘退出旧程序。缺少本地翻译 Worker 时仍可构建截图工具，本地翻译需另行准备引擎。

## 项目进度

项目仍在开发和测试中，部分功能及跨平台支持尚未完善。详情见 [TODO](Snap_Desk_TODO.md)。

## 许可证

[GPL-3.0-only](LICENSE)
