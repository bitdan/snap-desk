# Snap Desk

基于 **Tauri 2 + Rust + Vue 3** 的开源桌面工具，目前优先支持 **Windows**。

## 功能

- **截图与标注**：区域 / 全屏截图、窗口吸附、多显示器、延时截图、取色及常用标注工具。
- **滚动截图**：手动横向或纵向滚动、自动拼接、实时预览，支持同轴折返和白底 / 透明 PNG 导出。
- **贴图**：将截图、图片或剪贴板内容固定在桌面，支持分组、缩放、图像调整与继续标注。
- **录屏**：Windows 区域 / 显示器录制，支持 MP4、GIF、暂停 / 恢复、键鼠可视化和实时标注；MP4 可录制系统声音与麦克风。
- **识别与翻译**：本地 OCR、二维码、本地中英互译，以及 OpenAI-compatible 文本服务和 DeepL API；在线翻译仅发送识别文字。
- **剪贴板**：文本、图片和文件路径历史，支持分类、搜索、收藏、预览、贴图及 PDF / 长 PNG / 独立文件导出；文件历史只保存路径。
- **设置与更新**：自定义快捷键、中英文、浅色 / 深色主题、配置导入 / 导出、日志诊断及正式版 / 测试版在线更新。更新、确认和结果弹窗使用独立窗口。

## 下载

前往 [Releases](https://github.com/bitdan/snap-desk/releases) 下载 Windows x64 NSIS 安装包或独立 EXE。OCR 模型、运行库、本地翻译引擎及语言包在应用内按需安装。

## 本地开发

需要 Node.js、Rust 及 [Tauri 2 开发环境](https://v2.tauri.app/start/prerequisites/)。

```bash
git clone https://github.com/bitdan/snap-desk.git
cd snap-desk
npm ci
npm run desktop:dev
```

- `npm run dev:update`：启动浏览器模拟更新，演示下载进度与安装完成，不下载文件或退出应用。
- `npm run check`：目录边界、IPC 契约和 TypeScript 静态检查。
- `npm test`：前端与脚本测试。
- `npm run desktop:build:fast`：生成调试程序 `src-tauri/target/debug/snap-desk.exe`。
- `npm run desktop:build` / `npm run desktop:package`：生成发布程序 / 安装包；构建前从托盘退出旧程序。

发布以 `package.json` 为版本来源，修改后运行 `npm run version:sync`。使用同版本附注标签 `v<version>`，标签注释填写中文更新内容；流水线同步到 Releases 和更新清单，缺少内容时停止发布。

## 代码组织

```text
src/
  main.ts              # WebView 启动与窗口选择
  app/                 # 主界面、引导与全局样式
  features/<功能>/     # 业务组件、逻辑、API、事件与类型
  shared/              # 通用 UI、独立窗口、几何与语言
  services/            # IPC、命令白名单、日志与 API 汇总
src-tauri/src/
  app/                 # Tauri 组装、在线更新、托盘与快捷键
  capture/             # 截图；含 recorder/ 和 scroll/
  pins/                # 桌面贴图
  clipboard/           # 剪贴板历史与导出
  recognition/         # OCR、二维码、翻译与凭据
  settings/            # 配置、快捷键、自启与目录
  resources/           # 资源安装与空闲回收
  diagnostics/         # 日志与诊断
  shared/              # 渲染、导出、窗口与系统基础设施
tests/<功能>/          # 前端测试
```

组件和业务逻辑归属对应功能；命令与事件通过 `services/ipc.ts` 调用，`services/api.ts` 仅汇总。协作规则见 [AGENTS.md](AGENTS.md)，开发进度见 [TODO](Snap_Desk_TODO.md)。

## 许可证

[GPL-3.0-only](LICENSE)
