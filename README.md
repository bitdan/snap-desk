# Snap Desk

基于 **Tauri 2 + Vue 3 + TypeScript + Rust** 的桌面截图、标注、贴图与录屏工具，独立于仓库内的 Java / Python / tool-hub。

[项目源码](https://github.com/bitdan/snap-desk) · [功能与验收 TODO](跨平台截图工具_TODO.md) · [协作规则](AGENTS.md) · [GPL-3.0-only](LICENSE)

项目仍在开发中，已接入功能不代表全部通过原生或实机验收。当前以主显示器为主，录屏仅支持 Windows 10 2004+；macOS / Linux 兼容性见 TODO。

## 功能

- **截图**：主屏框选 / 全屏、延时、像素放大镜、选区微调与最近选区恢复。
- **标注**：形状、画笔、荧光笔、文字、序号、聚光灯、放大镜、马赛克、模糊、像素擦除及撤销 / 重做。
- **贴图**：截图、剪贴板、图片文件、文本和颜色；多窗口、置顶、缩放、透明度、分组及继续编辑。
- **导出**：PNG、JPEG、无损 WebP，支持质量、文件名及保存目录记忆。
- **长截图**：主屏区域纵向手动滚动、拼接预览、PNG 保存与复制，可保留原始帧。
- **录屏**：无声 MP4、30 / 60 FPS、三档画质、自动 / 软件编码、倒计时、暂停恢复及键鼠特效。

## 使用

图标按钮悬浮显示中文提示。托盘仅保留开始截图、贴图（剪贴板图片）、打开 Snap Desk、全局快捷键开关和退出。所有窗口共用 `index.html`，录屏控制条和选区边框按窗口标签加载 Vue 页面。

### 截图与贴图

按 **F1** 或通过主窗口 / 托盘开始截图，框选后使用工具栏标注、复制、保存或贴图。设置中可更改快捷键与延时；右上角或托盘可启用 / 禁用全部全局快捷键，状态会记住；关闭主窗口会收起到托盘，完全退出请使用托盘菜单。

| 操作 | 快捷键 |
| --- | --- |
| 复制 / 保存 / 贴图 | Ctrl（macOS 为 ⌘）+ C / S / P |
| 选择整个主屏 | Ctrl / ⌘ + A |
| 取消 | Esc / 右键 |
| 恢复最近选区 | R |
| 微调选区 | 方向键；Shift 加速，Alt 调整宽高 |
| 撤销 / 重做 | Ctrl / ⌘ + Z / Shift+Z；Windows 也可 Ctrl+Y |

最近选区只记录坐标，不保存截图内容；分辨率变化后不可直接恢复。JPEG 透明区域合成到白底，PNG / WebP 保留透明度。取消保存或写入失败时可继续编辑、重试。

### 长截图

1. 框选可滚动内容，点击“长截图”，尽量避开固定页头、滚动条及悬浮元素。
2. 在工具栏旁的二级面板选择是否保留原始帧，点击“开始长截图”。手动向下滚动，每次滚动后稍作停顿，等待预览更新。
3. 预览显示拼接尺寸，控制条显示帧数与高度。点击“保存”或使用已启用的截图快捷键后，自动保存 PNG 到应用数据目录 `scroll-results/` 并复制。

目前重试后仍匹配失败会结束采集，只输出已确认部分。暂停继续、动态区域过滤和人工接缝修正仍在计划中。

每轮最多 5 分钟、64 帧、60000 行，成图与源帧缓存各最多 3200 万像素。可选原始帧保存在 `scroll-frames/session-*`（每会话最多 128 帧 / 512 MiB），取消或退出后仍保留，需自行清理。

### 录屏

选择录制范围后点击工具栏“录屏”，在二级面板配置帧率、画质、光标和倒计时，再点击控制条“开始录制”。可暂停、继续、结束保存或取消；暂停时间不计入视频，当前保留 **3 分钟上限**。

- 录屏页可选择并记住 MP4 保存目录，默认使用应用数据目录 `recordings/`。
- 确认选区时可填写 1–80 Mbps 自定义目标码率（支持小数），优先于画质预设；清空后恢复按分辨率、帧率和画质自动计算。实际码率及文件大小取决于画面内容。
- 自动模式允许硬件编码，准备阶段失败时回退软件；界面展示实际识别类型，未知时不冒充硬件编码。
- 录屏页展示平均和近两秒有效 FPS；展开“录屏诊断”可查看采集、合成、入队、节流及分类丢弃计数，以及各阶段平均耗时。FPS 按成功编码提交帧的媒体时间计算，恢复时重置窗口；静态画面低 FPS 不等同于丢帧。
- 诊断页同时显示采集 / 合成的 CPU 复制量与 NV12 转换输出量。转换直接写入每帧独占的 Media Foundation 缓冲，省去一次整帧复制；这些数值不包含 GPU 或系统编码器内部传输，实际性能收益仍需原生对照测试。
- 选择二级面板中的“键鼠特效”可开启按键提示、点击光圈、移动 / 拖动轨迹、滚轮方向，调整颜色、尺寸、位置及持续时间，特效会写入视频。
- 键鼠效果默认关闭，仅显示白名单快捷键 / 功能键，不显示普通文字输入；暂停或结束时清理监听，静态桌面也会播放并清除动画。

目前仅主屏区域 / 全屏、SDR、无声 MP4，不支持窗口来源、多显示器、音频、HDR 或视频裁剪。正常取消 / 失败会清理临时视频；强制退出可能留下 `recording-pending-*` 目录。键鼠效果和高分辨率性能仍需实机验收。

## 开发与构建

环境：Node.js 18.20+、npm、Rust stable（至少 1.90）。Windows 需要 MSVC C++ 工具、Windows SDK 和 WebView2；其他平台依赖见 [Tauri 环境准备](https://v2.tauri.app/start/prerequisites/) 与 [xcap 平台依赖](https://github.com/nashaofu/xcap#system-dependencies)。

在 `snap-desk/` 目录执行：

```powershell
npm ci
npm run desktop:dev
```

| 命令 | 用途 |
| --- | --- |
| `npm run dev` | 仅前端预览，不提供原生截图能力 |
| `npm run check` | Vue / TypeScript 类型检查 |
| `npm test -- tests/recording.test.ts tests/recording-api.test.ts` | 录屏定向前端测试示例 |
| `npm run desktop:build` | 构建可运行程序 |
| `npm run desktop:package` | 构建程序及安装包 |
| `npm run desktop:clean` | 清理 Rust 构建缓存和产物 |

构建前从托盘退出程序。脚本固定复用 `src-tauri/target/`，已包含 `--ci` 和 `--locked`；Windows 程序位于 `src-tauri/target/release/snap-desk.exe`，安装包位于 `release/bundle/`。无需日常执行 clean，清理后会重新编译依赖。各平台安装包需在对应系统构建。

`npm run desktop:package` 会同时保留可运行的 `src-tauri/target/release/snap-desk.exe` 和安装包。GitHub Release 的 Windows x64 附件按以下规则提供，另附 `LICENSE`：

| 版本示例 | 可直接启动的程序 | EXE 安装包 | MSI 安装包 | Release 标记 |
| --- | --- | --- | --- | --- |
| `0.1.4-beta.1`、`0.1.4-rc.1` | `snap-desk-v版本-windows-x64-standalone.exe` | `*-setup.exe` | 不提供 | Pre-release |
| `0.1.4` | `snap-desk-v版本-windows-x64-standalone.exe` | `*-setup.exe` | `*.msi` | Latest |

`standalone.exe` 是主程序，下载后可直接启动，需要系统已安装 WebView2；`setup.exe` 和 `.msi` 是安装程序。主程序仍使用系统应用数据目录保存配置，不代表数据全部保存在 EXE 同目录的便携模式。

版本只维护 `package.json` 的 `version`，Git 标签在前面加 `v`。例如测试版 `0.1.4-beta.2` 对应 `v0.1.4-beta.2`，正式版 `0.1.4` 对应 `v0.1.4`。MSI 的版本字段有数字限制，无法直接使用 `beta.1` / `rc.1` 这样的文本后缀，所以测试版跳过 MSI。正式版采用三段数字，主版本和次版本最多 255，修订版本最多 65535；参见 [MSI ProductVersion](https://learn.microsoft.com/en-us/windows/win32/msi/productversion)。

开发协作默认不执行编译、打包或全量测试，按 [AGENTS.md](AGENTS.md) 做定向验证。需要原生检查时使用锁定依赖，例如 `cargo check --manifest-path src-tauri/Cargo.toml --locked`。

## 代码入口

| 位置 | 职责 |
| --- | --- |
| `src/components/`、`src/lib/api.ts` | 界面与 Tauri API 调用 |
| `src-tauri/src/capture.rs` | 截图会话与命令入口 |
| `src-tauri/src/annotations.rs`、`pins.rs` | 标注合成、贴图与窗口生命周期 |
| `src-tauri/src/scroll_capture.rs`、`stitch.rs` | 长截图采集与拼接 |
| `src-tauri/src/recorder/` | WGC 采集、Raw Input、QPC 时间轴、特效合成及 Media Foundation 编码 |
| `src-tauri/src/settings.rs`、`image_io.rs` | 配置、图片导出与剪贴板 |
| `tests/` | 前端定向测试；Rust 用例位于对应模块 |

## 常见问题

- **快捷键不可用**：先用按钮 / 托盘进入，检查快捷键冲突及截图权限。
- **主页初始化失败**：查看具体命令错误后点击“重新连接”；尚未读取的快捷键不代表已成功注册。
- **找不到 Cargo**：检查 PATH / `CARGO_HOME`；构建脚本会尝试默认 `.cargo/bin`。
- **无法覆盖 EXE**：从托盘退出旧程序后重试。
- **Windows 编码失败**：检查媒体组件、图形驱动和磁盘空间；Windows N 版本需 Media Feature Pack，可尝试软件编码。

源代码采用 [GPL-3.0-only](LICENSE)，第三方依赖遵循各自许可证。
