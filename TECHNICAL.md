# Snap Desk 技术说明

[返回使用说明](README.md) · [开发进度](Snap_Desk_TODO.md) · [协作规则](AGENTS.md)

本文面向开发与集成人员，记录接口适配、模型边界、资源与凭据处理、开发构建和模块结构。功能清单、下载和日常操作见 [README](README.md)。

## 技术栈

- Rust + [GPUI](https://www.gpui.rs/)：桌面窗口与界面、截图 / 录屏、系统集成与后台服务调用，界面直接用 GPU 渲染，不依赖 WebView。
- 与界面无关的业务核心（设置、标注、剪贴板、识别、更新等）在 `crates/snap-core`，应用外壳在 `apps/desktop-gpui`。
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
| 服务管理界面 | [settings_services.rs](apps/desktop-gpui/src/settings_services.rs) |
| 服务目录、配置与凭据操作 | [services.rs](crates/snap-core/src/recognition/services.rs) |
| OCR 引擎选择 | [recognition/mod.rs](crates/snap-core/src/recognition/mod.rs) |
| 在线 OCR 适配 | [online_ocr.rs](crates/snap-core/src/recognition/online_ocr.rs) |
| Paddle 异步任务与结果解析 | [paddle_ocr.rs](crates/snap-core/src/recognition/paddle_ocr.rs) |
| 在线文本翻译 | [providers/](crates/snap-core/src/recognition/providers/) |

本地翻译资源的许可材料见 [翻译模型许可说明](legal/translation/README.md)。

## 本地开发

需要 Rust（stable）；发版脚本与测试另需 Node.js 22，没有 npm 依赖。

```bash
git clone https://github.com/bitdan/snap-desk.git
cd snap-desk
cargo run -p snap-desk-gpui
```

- `cargo test --workspace --locked`：snap-core、应用与签名工具的单元测试。
- `npm test`：snap-core 边界与内嵌资源检查（`check:architecture`），以及打包、签名、版本同步等脚本测试。
- `npm run desktop:dev`：以调试模式运行应用，程序在 `target/debug/snap-desk-gpui`（Windows 为 `.exe`）。
- `npm run desktop:package`：构建发布版并用 cargo-packager 生成安装包；构建前从托盘退出旧程序。
- `npm run desktop:standalone`：只构建发布版并生成 Windows 独立 EXE（`target/release/snap-desk-v<版本>-windows-x64-standalone.exe`），不生成安装包，也不需要 cargo-packager；其他平台只构建主程序。构建前同样从托盘退出旧程序。
- `npm run desktop:clean`：清理 `target/`。

Linux 另需截图库的系统依赖，Debian / Ubuntu 可执行 `bash scripts/ci/install-linux-deps.sh`。

### 发布与分发

| 平台 | 打包产物 | 在线更新目标 |
| --- | --- | --- |
| Windows x64 | NSIS 安装包、独立 EXE | `windows-x86_64-nsis`、`windows-x86_64-standalone` |
| macOS Apple 芯片 | DMG、`.app.tar.gz` 更新包 | `darwin-aarch64-app` |
| Linux x64 | AppImage、deb | `linux-x86_64-appimage`（deb 不在应用内更新） |

- 附件清单集中在 `scripts/release-assets.mjs`：打包格式、统一文件名、签名范围和更新清单都由它派生；增删平台或分发形式时同步 `apps/desktop-gpui/src/updater.rs` 的平台、分发形式与文件后缀。
- 安装包由 [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) 生成（`scripts/package-gpui.mjs`，需先 `cargo install cargo-packager --version 0.11.8 --locked`），产物在 `target/release/packages/`；macOS 的 `.app.tar.gz` 由该脚本压缩 `.app` 得到。Windows 安装包默认装到当前用户目录，不需要管理员权限，也不再检查 WebView2。
- 更新签名由 `tools/release-sign`（minisign）生成，可信注释写入 `version:<版本>`，客户端据此拒绝版本不符的包；私钥经 `SNAP_DESK_SIGNING_PRIVATE_KEY`（及可选的 `SNAP_DESK_SIGNING_PRIVATE_KEY_PASSWORD`）传入，CI 中取自 `TAURI_SIGNING_PRIVATE_KEY` Secret，公钥为 `SNAP_DESK_UPDATER_PUBLIC_KEY` Variable，编译时嵌入程序。
- 发布流水线分三段：校验标签 / 更新说明 / 签名配置 → 三个平台各自测试、打包并上传附件 → 汇总签名、生成 `updater.json` 并公开。任一平台失败都不会创建 Release。
- macOS 使用固定的自签名证书签名（`.github/actions/macos-signing`，证书由 `scripts/macos-signing-cert.sh` 生成一次：公钥 `resources/macos-signing.pem` 随仓库提交，`.p12` 与密码存入 `MACOS_SIGNING_CERTIFICATE` / `MACOS_SIGNING_CERTIFICATE_PASSWORD` secrets），签名身份跨版本不变，“屏幕录制”等系统授权在更新后仍然有效；未经公证，首次打开仍需在系统设置中放行。未配置 secrets 时回退到临时签名（ad-hoc），每次构建签名都会变化，更新后需重新授权。ONNX Runtime 不提供 Intel Mac 预编译库，因此不构建 macOS x64。
- Linux 在 Ubuntu 22.04 上构建，生成的程序与 AppImage 内置库最高要求 glibc 2.35（发布流水线用 `scripts/ci/check-glibc.sh` 检查），可在 Ubuntu 22.04 及同期以后的发行版运行；截图依赖 X11，Wayland 会话会给出提示。

- AppImage 由 linuxdeploy 把主程序依赖的系统库一并打包；deb 依赖系统安装的库，因此小得多。
- 正式版公开后，`distribute.yml` 把安装描述推送到包管理器（测试版不分发）；清单由 `scripts/distribution.mjs` 按附件实际哈希生成，也可 `gh workflow run distribute.yml -f tag=v<版本>` 补发。对应 secret 未配置的渠道会跳过：
  - 终端安装脚本先读取官网的 `install/downloads.tsv`，失败时改读 tap 的副本。官网由 `pages.yml` 构建部署：构建前 `scripts/site-downloads.mjs` 按最新正式版 Release 附件的 SHA-256 生成 `docs/_data/downloads.json`（不提交到仓库），发布流水线结束后自动重新部署。tap 的 `downloads.tsv` / `downloads.json` 由分发流水线从实际发布附件生成，与 Scoop、Homebrew 清单一起推送。
  - Scoop（独立 EXE）与 Homebrew Cask（DMG）共用 tap 仓库 `bitdan/homebrew-snap-desk`（`bucket/`、`Casks/`），需 `TAP_TOKEN`。安装：`scoop bucket add snap-desk https://github.com/bitdan/homebrew-snap-desk`、`brew install --cask bitdan/snap-desk/snap-desk`。
  - WinGet（NSIS 安装包，`bitdan.SnapDesk`）由 wingetcreate 向 `microsoft/winget-pkgs` 提 PR，需 `WINGET_TOKEN`（classic PAT，`public_repo`），合并要等社区审核。
  - AUR `snap-desk-bin`（解包 deb），需 `AUR_SSH_PRIVATE_KEY`，且包名需先在 AUR 网站登录账号后由首次推送创建。

### 持续集成

- 推送到 `master` 和 PR 触发脚本检查 / 测试，以及 Windows、macOS、Linux 三个平台的 `cargo test --workspace`；非 Windows 的平台分支只在这里被编译。
- 只改 `*.md`、`docs/` 或 `LICENSE` 的提交不触发 CI；测试与构建不读取这些文件，同时改了代码的提交照常运行。`legal/` 下的许可文本由校验脚本按字节审核，不在跳过范围内。
- 提交信息以 `chore(release):` 开头的发版准备提交跳过测试，随后推送的标签会在发布流水线里运行同一套测试。
- 测试包（`test-build.yml`）为任意分支构建安装包，不发布 Release、不签名、不生成更新清单：在 Actions 中手动运行并选择平台（`gh workflow run test-build.yml --ref <分支> -f platform=windows|macos|linux|all`），或给 PR 加 `test-build`（全部平台）/ `mac-build`（仅 macOS）标签。安装包作为 Actions 附件保留 7 天；测试包没有内置更新公钥，不能检查更新；macOS 与正式版使用同一张自签名证书。

发布以 `package.json` 为版本来源，修改后运行 `npm run version:sync`。使用同版本附注标签 `v<version>`，标签注释填写中文更新内容；流水线同步到 Releases 和更新清单，缺少内容时停止发布。

## 代码组织

```text
Cargo.toml             # workspace 根；Cargo.lock 与 target/ 在仓库根目录
apps/desktop-gpui/     # 桌面应用（主程序 snap-desk-gpui，打包时复制为 snap-desk）
  src/shell.rs         # 启动组装、托盘、全局快捷键与单实例转发
  src/capture.rs · selection.rs · magnifier.rs · scroll.rs · recording.rs  # 截图浮层、滚动截图与录屏
  src/annotate.rs · live_annotate.rs  # 标注
  src/pins.rs · native.rs              # 桌面贴图与平台窗口能力
  src/clipboard*.rs · shelf.rs         # 剪贴板历史与顶部暂存架
  src/recognition*.rs · translation_window.rs · local_translate.rs  # OCR、二维码 / 条形码与翻译
  src/settings_*.rs    # 设置窗口各分类页
  src/updater.rs       # 在线更新（portable.ps1 为独立 EXE 的替换脚本）
  assets/              # 界面图标与服务商品牌图
crates/snap-core/      # 与界面无关的业务核心
  src/capture/         # 桌面采集与窗口/控件候选（Windows、macOS、Linux X11/Wayland）、选区合成
  src/settings/        # 设置模型、校验、落盘与快捷键事务
  src/recognition/     # 服务目录、OCR、贴图文字层（选择文字）、翻译与凭据
  src/clipboard/       # 历史模型、分类、SQLite 仓库与 Windows 图片剪贴板
  src/recording/       # 录屏采集、编码与输入事件
  src/pins/            # 贴图布局、像素预算、缩放步进、变换、图片文件解码与文字贴图
  src/updates/         # 更新清单、下载与签名校验
  src/export.rs · annotations.rs · text_image.rs  # 导出、标注渲染与文字转图片
  src/platform.rs · desktop_support.rs · x11.rs · wayland_probe.rs · macos.rs  # 平台设施
  i18n/                # 界面与原生文案的英文词典
  tests/fixtures/      # 测试字体
resources/             # 图标、随附许可证、macOS Entitlements / Info.plist、下载与模型目录（assets/）
native/translator/     # 本地翻译 worker（CTranslate2，C++）
tools/release-sign/    # 发布工具：在线更新包的 minisign 签名
scripts/               # 打包、发版、签名与版本同步脚本（scripts/ci/ 为翻译引擎与模型流水线）
```

core 不依赖 GPUI（`check:architecture` 检查），需要外壳能力时由调用方以闭包或 trait 注入。设置文件沿用旧版的位置与格式；开发时可用环境变量 `SNAP_DESK_SETTINGS` 指向另一份设置，与已安装的版本同时运行。

协作规则见 [AGENTS.md](AGENTS.md)，开发进度见 [TODO](Snap_Desk_TODO.md)。
