# Snap Desk

基于 **Tauri 2 + Rust + Vue 3** 开发的开源桌面工具，集截图、标注、贴图、录屏和文字识别于一体。

目前优先支持 **Windows**，macOS / Linux 仍在适配中。

## 功能

- **截图与标注**：区域 / 全屏截图、窗口识别、快捷键，以及文字、箭头、马赛克等标注工具。
- **滚动截图**：在所选显示器区域手动横向或纵向滚动，首次滚轮方向锁定单轴，自动拼接并实时预览；支持同轴折返、白底 / 透明 PNG 导出，以及匹配失败时保存已确认部分。
- **贴图**：将截图、图片或剪贴板内容固定在桌面，支持缩放和编辑。
- **Windows 录屏**：录制所选显示器区域或完整显示器，支持 MP4 30 / 60 FPS、GIF 10 / 20 FPS、倒计时和暂停 / 恢复；MP4 可选系统声音与麦克风，GIF 无声。支持键鼠操作可视化、录制中实时标注，以及标注保留时间和淡出设置。
- **录屏保存**：正常结束后直接保存带标注的 MP4 / GIF，不再进入录后回放和编辑；失败或取消时清理临时文件，不保留恢复副本。
- **OCR、二维码与翻译**：截图、贴图和导入图片可本地识别文字及二维码；支持本地中英互译、OpenAI-compatible 文本服务和 DeepL 官方 API（Free / Pro）。进入翻译面板后按已保存的服务配置自动开始；在线服务仅接收识别文字，不上传图片。图片翻译展示原文 / 译文文本对照，支持复制与取消翻译请求。
- **剪贴板管理**：保存和搜索文本、图片及文件路径历史，支持分类、收藏、贴图，以及文本 / 图片的批量 PDF、长 PNG 或独立文件导出。文件历史只保存路径，不备份原文件。
- **外观与引导**：通用设置可保存浅色 / 深色 / 跟随系统主题及中英文偏好。操作页、辅助窗口、托盘、应用定义的对话框和已知错误提示支持中英文；用户内容、文件路径及未知系统 / 驱动错误详情保留原文。帮助页可重新打开新手引导。

## 下载

前往 [Releases](https://github.com/bitdan/snap-desk/releases) 获取 Windows 版本。OCR 模型与运行库、本地翻译引擎及语言包在应用内按需安装；本地翻译引擎下载 / 导入目前仅支持 Windows x64。

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
构建前从托盘退出旧程序。生成安装包使用 `npm run desktop:package`；Windows 测试版仅生成 NSIS EXE 安装包。
主程序构建不依赖本地翻译引擎，使用本地翻译时在“识别与翻译”设置中下载或导入引擎，并安装对应语言包。

## 代码组织

```text
src/
  main.ts              # WebView 启动与窗口选择
  app/                 # 主界面、引导和全局样式
  features/<功能>/     # 业务逻辑、components、api、events 与通信类型
  shared/              # 通用 UI、几何、集合、语言与窗口标签解析
  services/            # IPC 错误处理、命令白名单、日志与 API 汇总
src-tauri/src/
  lib.rs / main.rs     # 模块地图与原生启动入口
  app/                 # Tauri 组装、应用命令、托盘/快捷键动作
  capture/             # 截图会话、桌面采集、预览；含 recorder/ 和 scroll/
  pins/                # 桌面贴图与内容读取
  clipboard/           # 剪贴板历史、持久化、导出与平台适配
  recognition/         # OCR、二维码、翻译、模型、引擎与凭据
  settings/            # 设置、快捷键、自启与目录访问
  resources/           # 资源下载与空闲资源回收
  diagnostics/         # 日志查询、诊断与导出
  shared/              # 标注渲染、图像导出、安全暂存、窗口和系统基础设施
tests/<功能>/          # 对应功能的前端测试
```

前端功能目录为 `capture`、`annotations`、`scroll`、`recording`、`pins`、`clipboard`、`recognition`、`settings`、`resources` 和 `diagnostics`。新增功能先找所属目录，组件与业务逻辑放在一起；跨功能通用代码才进入 `shared`。通信命令与事件由各功能维护，通过 `services/ipc.ts` 统一处理错误，`services/api.ts` 只汇总导出。

`npm run check` 执行目录边界、IPC 契约和 TypeScript 静态检查；`npm test` 先检查目录边界与 IPC 契约，再运行前端及构建检查脚本测试。具体协作规则见 [AGENTS.md](AGENTS.md)。

## 项目进度

项目仍在开发和测试中，部分功能及跨平台支持尚未完善。详情见 [TODO](Snap_Desk_TODO.md)。

## 许可证

[GPL-3.0-only](LICENSE)
