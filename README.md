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
  recovery/            # 恢复与清理
  shared/              # 标注渲染、图像导出、窗口和系统基础设施
tests/<功能>/          # 对应功能的前端测试
```

前端功能目录为 `capture`、`annotations`、`scroll`、`recording`、`pins`、`clipboard`、`recognition`、`settings`、`resources`、`diagnostics` 和 `recovery`。新增功能先找所属目录，组件与业务逻辑放在一起；跨功能通用代码才进入 `shared`。通信命令与事件由各功能维护，通过 `services/ipc.ts` 统一处理错误，`services/api.ts` 只汇总导出。

`npm run check` 执行目录边界检查和 TypeScript 静态检查；`npm test` 同样先检查目录边界。具体协作规则见 [AGENTS.md](AGENTS.md)。

## 项目进度

项目仍在开发和测试中，部分功能及跨平台支持尚未完善。详情见 [TODO](Snap_Desk_TODO.md)。

## 许可证

[GPL-3.0-only](LICENSE)
