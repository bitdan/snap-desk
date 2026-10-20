# Snap Desk

基于 **Tauri 2 + Vue 3 + TypeScript + Rust** 的桌面截图、标注、贴图与录屏工具。

[项目源码](https://github.com/bitdan/snap-desk) · [功能与验收 TODO](跨平台截图工具_TODO.md) · [协作规则](AGENTS.md) · [GPL-3.0-only](LICENSE)

测试命令、性能基准及已知缺失素材见 [测试与性能记录](tests/PERFORMANCE.md)。

项目仍在开发中，已接入功能不代表全部通过原生或实机验收。Windows 普通截图支持多显示器，录屏与长截图仍限主屏；录屏仅支持 Windows 10 2004+，macOS / Linux 兼容性见 TODO。

## 功能

- **截图**：Windows 多屏框选 / 整个桌面、窗口与原生控件吸附、原生窗口抓取、光标开关、延时、像素放大镜、选区微调与最近选区恢复；其他平台暂保留主屏截图。
- **标注**：形状、画笔、荧光笔、文字、序号、聚光灯、放大镜、马赛克、模糊、像素擦除及撤销 / 重做。
- **贴图**：截图、剪贴板、图片文件、文本和颜色；多窗口、置顶、缩放、透明度、分组及继续编辑。
- **导出**：PNG、JPEG、无损 WebP，支持质量、文件名及保存目录记忆。
- **长截图**：点击直接开始主屏区域纵向手动滚动采集，支持拼接预览、PNG 保存与复制。
- **录屏**：无声 MP4、30 / 60 FPS、三档画质、自动 / 软件编码、倒计时、暂停恢复及键鼠特效。

## 使用

图标按钮悬浮显示中文提示。托盘提供开始截图、贴图（剪贴板图片）、录屏、打开 Snap Desk、全局快捷键开关和退出。所有窗口共用 `index.html`，录屏控制条和选区边框按窗口标签加载 Vue 页面。

### OCR 与导出字体

在“识别与翻译”页的“文字识别”区域点击“下载识别资源”，下载 PP-OCRv6 small 中英文模型（约 30 MiB）和 Windows CPU 引擎，随后可在截图工具栏点击“识别文字”，编辑并复制结果。使用 OAR-OCR 0.10.0 + ort rc.13 在本机识别，不上传图片；当前针对正向文字，选区上限 1600 万像素。

模型使用上游固定链接按需下载，不打包进应用：

- [检测模型 pp-ocrv6_small_det.onnx](https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/pp-ocrv6_small_det.onnx)
- [识别模型 pp-ocrv6_small_rec.onnx](https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/pp-ocrv6_small_rec.onnx)
- [配套字典 ppocrv6_dict.txt](https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/ppocrv6_dict.txt)

OAR-OCR 库版本为 0.10.0，模型附件仍位于上游 v0.7.0 Release。应用按内置清单验证大小和 SHA-256，下载中断可重试；也可通过以上链接下载，将三个文件按原名放入设置页显示的资源目录，再点击“检查识别资源”。旧版 v5 模型不会被当作 v6 就绪资源，也不会自动删除；升级后需下载新模型。

PDF 和文本长图需要下载霞鹜文楷（约 24 MiB），或选择有嵌入授权的静态 TTF 字体（最大 64 MiB，不支持 TTC、OTF、可变字体）。选择的字体会复制到应用资源目录；点击“使用下载字体”会替换当前字体。缺字会报错，纯图片导出和 TXT 不依赖字体。

资源下载由用户主动触发，保存到 `app_data_dir()/resources`，设置页展示实际路径。[下载清单](src-tauri/assets/downloads.json) 固定版本、大小和 SHA-256；失败重试会跳过已经校验通过的模型和引擎。离线安装可按清单文件名放入三个模型文件；Windows 引擎还需按 [运行库清单](src-tauri/assets/onnxruntime.json) 的版本目录及文件相对路径放置 DLL、许可证和声明，并满足各文件的大小及摘要。字体许可证见 [LXGW-OFL.txt](src-tauri/licenses/LXGW-OFL.txt)。模型和生产字体不进入安装包。

Windows x64 / ARM64 的 ONNX Runtime 1.24.2 使用 `ort/load-dynamic`，首次识别时才从已校验的绝对路径加载 DLL，主程序和安装包不携带引擎。官方 ZIP 约 71 / 72 MiB（包含 PDB 调试符号），安装只保留 DLL、许可证和声明，x64 约 14 MiB；下载临时包会自动删除。未下载不影响启动、截图及非 OCR 功能；加载失败可重试，系统需具备对应架构的 Microsoft Visual C++ 2015–2022 运行库。已加载的 DLL 若需修复，应退出应用后重试。其他平台暂时保留构建时下载和链接 CPU 引擎的方式。

离线导出测试使用 `src-tauri/tests/fixtures/export-test.ttf` 的小型字体子集，保留原始字体度量及许可证。允许编译时可定向运行 `cargo test --locked resources::tests` 和 `cargo test --locked clipboard::export::tests`；发布前仍需实机检查下载失败重试、离线识别及中文 PDF/长图导出。

### 截图翻译（本地优先，Windows x64）

先安装上述 OCR 资源，再进入“识别与翻译”页。默认使用 CTranslate2 / OPUS-MT，本地支持英译中和中译英；选择目标语言并安装对应语言包后保存。引擎与语言包独立于 OCR，缺少翻译资源不影响截图或识别。原生 Worker、固定 revision 模型转换、离线包导入及发布步骤见 [本地翻译开发说明](src-tauri/native/translator/README.md)。当前仓库尚未包含编译后的 Worker 或已发布模型下载清单，需先完成这些准备，不能直接宣称离线版本已验收。

截图框选后点击“截图翻译”，本地识别无标注原图。在双语面板中检查或修正原文，然后翻译、取消或复制译文。本地模式不发送网络请求，超时或取消会终止原生推理进程；失败不自动切换在线服务。关闭面板会取消请求并丢弃迟到结果。

可显式选择在线文本服务，填写兼容 Chat Completions 的完整接口地址、模型和 API Key。密钥保存在 Windows 凭证库，不进入设置导出；每次翻译前需确认文本上传，固定样本连接测试也可能计费。图片不上传，已发送的请求即使取消也可能计费。尚未实现视觉 OCR、文字框段落重建、原位覆盖与翻译贴图；其他平台未完成原生引擎与凭证存储验收。

定向静态检查：`npm run check`；配置测试：`npx vitest run tests/ai.test.ts tests/settings.test.ts`。允许编译后，可在 `src-tauri` 运行 `cargo test --locked translation::tests` 和 `cargo test --locked settings::tests`，并在 Windows 实机验证凭证读写、本地 OCR、服务连接、取消及断网行为。

### 快捷键、首次引导与存储

“快捷键”页统一配置区域截图、剪贴板贴图和录屏，默认分别为 F1、F2、F3。点击快捷键框直接按组合键：Esc 取消，Backspace / Delete 清空并禁用该动作；也可单独恢复默认。录入时临时释放本应用全局键，全部松键后恢复原绑定，点击“保存快捷键”才提交草稿。重复绑定会指出冲突功能，系统占用或保存失败会回滚原配置。

主配置使用 `shortcutsEnabled` 和 `shortcuts: { capture, pin, record }`，空字符串表示禁用单个动作。旧 `shortcut` 自动迁移到 `shortcuts.capture`，贴图与录屏补默认值；下次保存写入新结构。关闭总开关会保留各动作绑定。

首次启动展示不遮挡截图操作的三步引导，完成或跳过后保存独立的 `onboardingVersion`；可从“帮助”重新打开。该版本不随应用升级递增。

“通用”页分别查看应用配置、上次截图保存、录屏输出和长截图结果目录，可打开目录或复制路径。录屏目录可选择或恢复默认；“导出主配置”仅导出已生效设置，不包含图片、视频及独立的目录偏好文件。“快捷键”页可复制默认示例配置。

“通用”页可开启或关闭“开机自启”：登录系统后自动启动到托盘，不弹出主窗口；手动打开程序仍显示主窗口。状态直接读取系统自启注册项，不包含在导出的主配置中；可点击“刷新状态”重新查询。独立 EXE 请放在固定位置后开启，移动程序后需在新位置关闭并重新开启自启。

### 截图与贴图

按 **F1** 或通过主窗口 / 托盘开始截图，框选后使用工具栏标注、复制、保存或贴图。框选后按贴图快捷键（默认 **F2**）或点击托盘贴图，会贴出当前选区并保留标注；没有截图会话时则贴出剪贴板图片。设置中可更改快捷键与延时；右上角或托盘可启用 / 禁用全部全局快捷键，状态会记住；关闭主窗口会收起到托盘，完全退出请使用托盘菜单。

| 操作 | 快捷键 |
| --- | --- |
| 复制 / 保存 / 贴图 | Ctrl（macOS 为 ⌘）+ C / S / P |
| 选择整个截图桌面（Windows 普通截图包含所有屏幕） | Ctrl / ⌘ + A |
| 取消 | Esc / 右键 |
| 恢复最近选区 | R |
| 微调选区 | 方向键；Shift 加速，Alt 调整宽高 |
| 撤销 / 重做 | Ctrl / ⌘ + Z / Shift+Z；Windows 也可 Ctrl+Y |

最近选区只记录坐标和显示器布局，不保存截图内容；分辨率、屏幕排列或缩放变化后不可直接恢复。JPEG 透明区域合成到白底，PNG / WebP 保留透明度。取消保存或写入失败时可继续编辑、重试。

Windows 截图中移动鼠标可预览窗口或原生控件边界，单击确认，拖动则自由框选并在靠近边缘时吸附；按住 **Alt** 只选窗口，按住 **Shift** 临时关闭吸附。选中窗口后，工具栏“原生窗口截图”会重新抓取该窗口，可获取被其他窗口遮挡的内容；目标不支持、无响应或位置变化时保留原图并提示。部分受保护或 GPU 绘制窗口可能返回黑屏，请使用区域截图。原生控件识别针对 Win32 子窗口，不包含浏览器 DOM 或 Canvas 内部元素。

“截图”页可保存“截图包含光标”和“窗口 / 元素吸附”偏好。光标默认隐藏；开启后将系统光标合成到导出图片，与录屏设置独立。多屏按原始物理像素拼接，不对高 DPI 屏幕二次缩放；屏幕之间未覆盖的桌面空洞填黑，虚拟桌面上限 6400 万像素。跨屏、副屏选区不能用于现有主屏录屏 / 长截图，界面会提示重新框选。

### 长截图

1. 框选可滚动内容，点击“长截图”，尽量避开固定页头、滚动条及悬浮元素。
2. 点击后直接开始采集，无需二次确认。手动向下滚动，每次滚动后稍作停顿，等待预览更新。
3. 预览显示拼接尺寸，控制条显示帧数与高度。点击“保存”或使用已启用的截图快捷键后，自动保存 PNG 到应用数据目录 `scroll-results/` 并复制。

目前重试后仍匹配失败会结束采集，只输出已确认部分。暂停继续、动态区域过滤和人工接缝修正仍在计划中。

每轮最多 5 分钟、64 帧、60000 行，成图与源帧缓存各最多 3200 万像素。不额外保存原始帧，排错请查看日志页。

### 录屏

选择录制范围后点击工具栏“录屏”，在二级面板配置帧率、画质、光标和倒计时，再点击控制条“开始录制”。可暂停、继续、结束保存或取消；暂停时间不计入视频，**不设录制时长上限**，按录屏快捷键（默认 F3）或点击停止结束并保存。

保存失败会保留已产生的源视频。录屏页的“待恢复视频”支持将已完成封装的视频重新导出到当前保存目录，也可先选择其他文件夹；未确认完成封装的文件保留供检查，不直接导出。明确取消会清理本次视频。退出时等待编码和封装，按真实处理进展判断超时，异常超时后恢复信息仍保留。

成功导出后，“录屏”页显示本次运行最近完成的文件，可使用系统播放器预览 MP4 / GIF、打开保存目录或复制路径。文件名包含本地日期时间、毫秒与随机标识，恢复导出使用 `Recovered` 标记；同名文件不会被覆盖。取消或失败不会替换最近成功结果。应用内预览与裁剪仍待实现。

- 录屏页可选择并记住 MP4 保存目录，默认使用应用数据目录 `recordings/`。
- 确认选区时可填写 1–80 Mbps 自定义目标码率（支持小数），优先于画质预设；清空后恢复按分辨率、帧率和画质自动计算。实际码率及文件大小取决于画面内容。
- 自动模式允许硬件编码，准备阶段失败时回退软件；界面展示实际识别类型，未知时不冒充硬件编码。
- 录屏页展示平均和近两秒有效 FPS；展开“录屏诊断”可查看采集、合成、入队、节流及分类丢弃计数，以及各阶段平均耗时。FPS 按成功编码提交帧的媒体时间计算，恢复时重置窗口；静态画面低 FPS 不等同于丢帧。
- 诊断页同时显示采集 / 合成的 CPU 复制量与 NV12 转换输出量。转换直接写入每帧独占的 Media Foundation 缓冲，省去一次整帧复制；这些数值不包含 GPU 或系统编码器内部传输，实际性能收益仍需原生对照测试。
- 选择二级面板中的“键鼠特效”可开启按键提示、点击光圈、移动 / 拖动轨迹、滚轮方向，调整颜色、尺寸、位置及持续时间，特效会写入视频。
- 键鼠效果默认关闭，仅显示白名单快捷键 / 功能键，不显示普通文字输入；暂停或结束时清理监听，静态桌面也会播放并清除动画。

目前仅主屏区域 / 全屏、SDR、无声 MP4，不支持窗口来源、多显示器、音频、HDR 或视频裁剪。正常取消 / 失败会清理临时视频；强制退出可能留下 `recording-pending-*` 目录。键鼠效果和高分辨率性能仍需实机验收。

## 开发与构建

环境：Node.js 18.20+、npm、Rust stable（至少 1.95）。Windows 需要 MSVC C++ 工具、Windows SDK 和 WebView2；其他平台依赖见 [Tauri 环境准备](https://v2.tauri.app/start/prerequisites/) 与 [xcap 平台依赖](https://github.com/nashaofu/xcap#system-dependencies)。

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
| `npm run desktop:dev` | 日常开发、调试，支持前端热更新 |
| `npm run desktop:build:fast` | Debug 构建可运行程序，不生成安装包，用于功能验证 |
| `npm run desktop:build` | Release 构建可运行程序，用于性能验收 |
| `npm run desktop:package` | Release 构建程序及安装包，用于正式发布 |
| `npm run desktop:clean` | 清理 Rust 构建缓存和产物 |

构建前从托盘退出程序。脚本固定复用 `src-tauri/target/`，已包含 `--ci` 和 `--locked`；Windows 程序位于 `src-tauri/target/release/snap-desk.exe`，安装包位于 `release/bundle/`。无需日常执行 clean，清理后会重新编译依赖。各平台安装包需在对应系统构建。

快速构建执行 `npm run desktop:build:fast`，Windows 程序位于 `src-tauri/target/debug/snap-desk.exe`。它复用现有构建脚本和版本同步逻辑，通过 Tauri 的 `--debug` 选择 Debug 构建；首次仍需编译依赖，后续复用 Debug 缓存。Debug 与 Release 产物分别存放，录屏、OCR、图像拼接等性能测试应使用 Release 版；现有 Release 优化配置保持不变。

排查编译耗时可运行 `npm run desktop:build -- --timings`，追加参数会传给 Cargo，报告位于 `src-tauri/target/cargo-timings/cargo-timing.html`。避免同时启动多个构建进程或频繁切换工具链、编译目标与优化参数。

发布流水线在选择 Rust 工具链后使用 `Swatinem/rust-cache@v2` 缓存依赖，目录映射为 `./src-tauri -> target`。缓存命中受工具链、依赖及 GitHub 缓存作用域影响；仅标签触发的发布不能假定复用其他标签的缓存，首次冷构建仍需完整编译。

`npm run desktop:package` 会同时保留可运行的 `src-tauri/target/release/snap-desk.exe` 和安装包。GitHub Release 的 Windows x64 附件按以下规则提供，另附 `LICENSE`：

| 版本示例 | 可直接启动的程序 | EXE 安装包 | MSI 安装包 | Release 标记 |
| --- | --- | --- | --- | --- |
| `0.1.4-beta.1`、`0.1.4-rc.1` | `snap-desk-v版本-windows-x64-standalone.exe` | `*-setup.exe` | 不提供 | Pre-release |
| `0.1.4` | `snap-desk-v版本-windows-x64-standalone.exe` | `*-setup.exe` | `*.msi` | Latest |

`standalone.exe` 是主程序，下载后可直接启动，需要系统已安装 WebView2；`setup.exe` 和 `.msi` 是安装程序。主程序仍使用系统应用数据目录保存配置，不代表数据全部保存在 EXE 同目录的便携模式。

发布构建启用 Thin LTO、单代码生成单元及符号裁剪，保留 `opt-level = 3` 与原有异常展开策略。
这些设置会增加构建时间，缩小幅度需以重新构建后的 EXE 为准。OCR 模型和生产字体按需下载，
Windows ONNX Runtime CPU 引擎也按需下载为独立 DLL；安装包经过压缩，因此下载体积通常明显小于裸 EXE。

版本只维护 `package.json` 的 `version`，Git 标签在前面加 `v`。例如测试版 `0.1.4-beta.2` 对应 `v0.1.4-beta.2`，正式版 `0.1.4` 对应 `v0.1.4`。MSI 的版本字段有数字限制，无法直接使用 `beta.1` / `rc.1` 这样的文本后缀，所以测试版跳过 MSI。正式版采用三段数字，主版本和次版本最多 255，修订版本最多 65535；参见 [MSI ProductVersion](https://learn.microsoft.com/en-us/windows/win32/msi/productversion)。

开发协作默认不执行编译、打包或全量测试，按 [AGENTS.md](AGENTS.md) 做定向验证。需要原生检查时使用锁定依赖，例如 `cargo check --manifest-path src-tauri/Cargo.toml --locked`。

## 代码入口

| 位置 | 职责 |
| --- | --- |
| `src/components/`、`src/lib/api.ts` | 界面与 Tauri API 调用 |
| `src-tauri/src/capture.rs` | 截图会话与命令入口 |
| `src-tauri/src/annotations.rs`、`pins.rs` | 标注合成、贴图与窗口生命周期 |
| `src-tauri/src/scroll/` | 长截图采集、稳定检测、匹配、拼接、失败恢复与会话管理 |
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

## 本地诊断日志

通用设置 → 应用日志目录可打开日志文件夹或复制路径。前后端共用 Tauri 日志文件，默认 INFO，使用带时区偏移的本地时间；每个文件 5 MB，最多保留 5 份归档，正常重启追加写入。

日志只记录固定事件、命令名、错误码与数量/耗时，不记录截图、剪贴板、OCR/翻译正文、凭据或完整文件路径。第三方库的日志不写入此诊断文件；前端插件自动附带的调用位置也不输出。旧版本已写出的日志不会自动清洗，分享文件前仍需检查。

通用设置 → 诊断与日志支持级别/模块筛选及本地搜索，最多扫描日志尾部 2 MB、展示 500 条记录。查看与导出共用白名单重建：旧版自由文本、未知事件和未经审计的字段会被省略。错误计数仅对应近期样本，不代表全历史。清理历史归档需经过原生确认，当前日志不会删除。

Windows 可主动导出 ZIP，固定包含 `diagnostic.json`、`logs/recent.log`、`logs/errors.log` 和 `README.txt`；输出先写同目录临时文件，成功后才发布。包内仅含经过过滤的近期事件及实际资源就绪状态，不包含主配置、媒体或剪贴板数据库。其他平台保留查看/清理功能，ZIP 按钮不可用。

临时 DEBUG 立即生效，固定 30 分钟，由后端单调时钟逐条判断到期；关闭或重启恢复 INFO，不保存永久开关。高频长截图细节每 500ms 最多采样一条，INFO/WARN/ERROR 不受采样限制。panic 仅新增受控行列号摘要并保留原有异常行为。开发控制台转发与可选 JSONL 格式未启用。

诊断读写仅允许主窗口，路径由后端固定解析，不开放通用 fs 权限。应用事件可通过 `op` 关联同一会话；新增事件、错误码或命令时需审计生产处字段，并同步 `src-tauri/src/diagnostics/catalog.rs` 导出白名单。

定向检查：`npm run check`，`npx vitest run tests/diagnostics.test.ts tests/logger.test.ts tests/api-errors.test.ts tests/recording-api.test.ts`。原生验收需自行构建后检查多窗口日志、重启追加、5 MB 轮转，以及截图/录屏成功与 OCR 失败事件；静态检查不替代这些运行验证。

原生补充验收：检查 DEBUG 开启/手动关闭/30 分钟到期/重启复位，辅助窗口无权调用诊断命令，归档清理取消与文件占用，ZIP 的固定条目及失败不留临时文件。已添加 Rust 定向用例（读取上限、无效查询、隐私过滤、ZIP 失败清理等）；按仓库约定未执行需要编译的 Rust 测试。可由维护者在 `src-tauri` 运行 `cargo test --locked diagnostics::` 后，再运行 Windows 打包验收。
