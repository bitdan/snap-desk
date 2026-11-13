# Snap Desk 技术说明

[返回使用说明](README.md) · [开发进度](Snap_Desk_TODO.md) · [协作规则](AGENTS.md)

本文面向开发与集成人员，记录接口适配、模型边界、资源与凭据处理、开发构建和模块结构。功能清单、下载和日常操作见 [README](README.md)。

## 技术栈

- Tauri 2 + Rust：桌面窗口、截图 / 录屏、系统集成与后台服务调用。
- Vue 3 + TypeScript：功能界面、配置表单与 IPC 调用。
- 分发平台：Windows x64（功能完整）、macOS Apple 芯片与 Linux x64（预览，平台专属能力以回退实现或“暂不支持”提示占位）；适配进度见 [TODO](Snap_Desk_TODO.md)。

## OCR 接入

| 状态 | 引擎 / 服务 | 类型 | 当前接入范围 |
| --- | --- | --- | --- |
| ✅ | PaddleOCR · PP-OCRv6 | 离线 | 本地模型按需安装，CPU 推理 |
| ✅ | Windows OCR | 离线 | Windows.Media.Ocr；使用已安装的系统 OCR 语言包 |
| ✅ | [OCR.space](https://ocr.space/ocrapi) | 在线 API | API Key；支持引擎 1 / 2 / 3，具体权限以账号为准；大截图自动压缩至 1 MB 内 |
| ✅ | [PaddleOCR 官方异步接口](https://ai.baidu.com/ai-doc/AISTUDIO/fml7mozw5) | 在线 API | AI Studio Token；PP-OCRv6、PaddleOCR-VL-1.6、PP-StructureV3，兼容 PP-OCRv5 |
| ✅ | [PaddleOCR 同步接口](https://ai.baidu.com/ai-doc/AISTUDIO/Kmfl2ycs0) | 在线 API / 本机 API | 自定义完整 `/ocr` 地址；通用 OCR 产线，模型由服务端决定 |
| ✅ | [Groq Vision](https://console.groq.com/docs/vision) | 在线 API | 图片输入模型；与 Groq 文本翻译模型分别配置 |
| ✅ | [DeepSeek 图像理解](https://api-docs.deepseek.com/zh-cn/guides/vision/) | 在线 API | `deepseek-flash`；Chat Completions 的 `image_url` 内联截图，返回文字，不提供定位框 |
| ✅ | 自定义兼容视觉模型 | 在线 API / 本机 API | 兼容 Chat Completions 图片输入，并明确启用图片能力 |

OCR 路由只调用已保存的引擎，不因本地失败自动回退到在线服务。启用在线 OCR 后，识别操作直接上传选区图片，不再逐次弹出上传确认。

Paddle API 地址下拉框提供 **官方异步接口（推荐）** 与 **同步接口（自定义地址）**。官方异步地址为 `https://paddleocr.aistudio-app.com/api/v2/ocr/jobs`；应用上传截图、轮询任务并读取 JSONL 文本结果，最多等待两分钟，不重复上传，也不下载结果中的可视化图片。在线 OCR 当前不提供定位框，图片翻译使用文本对照。

[OCR.space 免费规则](https://ocr.space/ocrapi)：1 MB/文件、25,000 次/月、500 次/日/IP；[官方 FAQ](https://ocr.space/faq)允许商业项目使用，免费版无可用性保证。[PaddleOCR 配额规则](https://ai.baidu.com/ai-doc/AISTUDIO/Xmjclapam)规定每用户、每模型每天 20,000 页；免费云端服务的商用授权尚未从公开条款确认，不能从本地开源许可推导。服务规则可能调整，以账号及官方页面为准。

## 翻译接入

| 状态 | 引擎 / 服务 | 类型 | 当前接入范围 |
| --- | --- | --- | --- |
| ✅ | CTranslate2 | 离线 | 本地中英互译；引擎与语言包按需安装 |
| ✅ | DeepL API Free / Pro | 在线 API | 官方 `/v2/translate`；使用 API Key |
| ✅ | Groq | 在线 API | 兼容 Chat Completions 的文本模型，内置地址预设 |
| ✅ | [DeepSeek](https://api-docs.deepseek.com/zh-cn/) | 在线 API | 内置 `https://api.deepseek.com/chat/completions` 与 `deepseek-flash`，复用兼容协议的批量文本翻译 |
| ✅ | OpenAI | 在线 API | 兼容 Chat Completions 的文本翻译，模型 ID 自行配置 |
| ✅ | Ollama 等本机兼容服务 | 本机 API | 通过 `/v1/chat/completions` 接入；不代表已适配各平台的原生专用协议 |
| ✅ | 自定义兼容文本模型 | 在线 API / 本机 API | 配置完整接口地址、模型 ID 与所需凭据 |

本地翻译目前仅支持中英双向；在线文本翻译只发送待翻译文字，不直接接收截图文件。截图与贴图先由所选 OCR 提取文字，再在识别工作区内就地翻译；从翻译窗口发起的截图仍把原文交回翻译窗口。

## 服务配置与凭据

DeepSeek 可在 OCR、翻译或服务管理中选择预设；服务管理保存的同一连接可分别用于 OCR 和翻译。密钥从 [DeepSeek 开放平台](https://platform.deepseek.com/api_keys) 获取。模型能力以[官方更新日志](https://api-docs.deepseek.com/zh-cn/updates/)为准；预设使用支持图片输入的 `deepseek-flash`，修改为其他模型时须同步核对图片能力。

服务目录维护连接、协议、地址、模型和图片能力，OCR 与翻译分别绑定已保存连接。菜单中“当前使用”根据已生效配置显示，保存失败不会把正在编辑的草稿标成当前配置。已有连接替代同供应商的添加预设，多账号 / 多模型连接仍各自保留。

- 文本模型与视觉模型分别配置；兼容文本协议不表示模型支持图片输入。
- 系统凭据按连接与服务地址隔离；更改地址不会把旧地址的密钥发送给新地址。
- 凭据存入 Windows 凭据库，配置导出只携带非敏感字段。
- 当前使用 API Key / Token；官方凭据页面入口只打开服务商页面，不等同于 OAuth 授权。
- 已保存的自定义同步地址保持原值；选择新的官方异步地址不会自动迁移旧凭据。
- 本地 OCR / 翻译资源按需下载或导入，进行校验并支持空闲回收。

实现入口：

| 职责 | 文件 |
| --- | --- |
| 服务目录与前端校验 | [services.ts](src/features/recognition/services.ts) |
| 原生配置与凭据操作 | [services.rs](src-tauri/src/recognition/services.rs) |
| OCR 引擎选择 | [recognition/mod.rs](src-tauri/src/recognition/mod.rs) |
| 在线 OCR 适配 | [online_ocr.rs](src-tauri/src/recognition/online_ocr.rs) |
| Paddle 异步任务与结果解析 | [paddle_ocr.rs](src-tauri/src/recognition/paddle_ocr.rs) |
| 在线文本翻译 | [providers/](src-tauri/src/recognition/providers/) |

本地翻译资源的许可材料见 [翻译模型许可说明](legal/translation/README.md)。

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
- `npm run desktop:build:fast`：生成调试程序 `src-tauri/target/debug/snap-desk`（Windows 为 `snap-desk.exe`）。
- `npm run desktop:build` / `npm run desktop:package`：生成发布程序 / 安装包；构建前从托盘退出旧程序。

Linux 另需截图库的系统依赖，Debian / Ubuntu 可执行 `bash scripts/ci/install-linux-deps.sh`。

### 发布与分发

| 平台 | 打包产物 | 在线更新目标 |
| --- | --- | --- |
| Windows x64 | NSIS 安装包、独立 EXE | `windows-x86_64-nsis`、`windows-x86_64-standalone` |
| macOS Apple 芯片 | DMG、`.app.tar.gz` 更新包 | `darwin-aarch64-app` |
| Linux x64 | AppImage、deb | `linux-x86_64-appimage`（deb 不在应用内更新） |

- 附件清单集中在 `scripts/release-assets.mjs`：打包目标、统一文件名、签名范围和更新清单都由它派生；增删平台或分发形式时同步 `src-tauri/src/app/updater.rs` 的平台、分发形式与文件后缀。
- 发布流水线分三段：校验标签 / 更新说明 / 签名配置 → 三个平台各自测试、打包并上传附件 → 汇总签名、生成 `updater.json` 并公开。任一平台失败都不会创建 Release。
- macOS 默认使用临时签名（ad-hoc），未经公证；在流水线中配置 `APPLE_SIGNING_IDENTITY` 及 Tauri 的 Apple 证书变量后改用开发者证书。ONNX Runtime 不提供 Intel Mac 预编译库，因此不构建 macOS x64。
- Linux 在 Ubuntu 24.04 上构建（截图库的 PipeWire 绑定要求 0.3.65 以上），生成的程序需要 glibc 2.39 及以上的发行版；截图依赖 X11，Wayland 会话会给出提示。

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
  clipboard/           # 剪贴板历史、独立列表窗口与导出
  shelf/               # 顶部暂存架：历史视图、偏好与窗口
  recognition/         # OCR、二维码、翻译与凭据
  settings/            # 配置、快捷键、自启与目录
  resources/           # 资源安装与空闲回收
  diagnostics/         # 日志与诊断
  shared/              # 渲染、导出、窗口与系统基础设施
tests/<功能>/          # 前端测试
```

组件和业务逻辑归属对应功能；命令与事件通过 `services/ipc.ts` 调用，`services/api.ts` 仅汇总。协作规则见 [AGENTS.md](AGENTS.md)，开发进度见 [TODO](Snap_Desk_TODO.md)。
