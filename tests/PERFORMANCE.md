# 测试与性能复现

普通测试不下载模型、不采集屏幕、不读取用户剪贴板历史。版本同步脚本通过隔离临时目录测试。

```powershell
npm test
npm run check
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
```

前端 Vitest 与 `scripts/sync-version.test.mjs` 的 Node 测试由 `npm test` 分开执行。
真实长截图用例依赖 `src-tauri/tests/fixtures/scroll/qrgxj8/` 中的
`0000.png` 至 `0006.png` 和 `expected.png`。当前仓库缺少这组原始图片，
对应的 5 项测试会失败；需要补齐原始素材后再验收，不将其计为通过。

## 性能基准

1. 按 `src-tauri/assets/downloads.json` 将四个资源下载到 `src-tauri/target/perf-assets/`，
   Windows 还需按 `src-tauri/assets/onnxruntime.json` 将对应架构的官方 ZIP 下载到
   `src-tauri/target/`；测试 EXE 编译完成后先运行
   `ocr_runtime::tests::install_official_runtime_for_ocr_test --ignored --exact --nocapture`，
   它只安装到性能资源目录并验证 DLL 加载。
   使用清单的本地文件名并核对大小、SHA-256。也可以设置 `SNAP_DESK_PERF_ASSETS` 指向独立目录。
2. `cargo test --manifest-path src-tauri/Cargo.toml --locked --release --lib --no-run`。
   复制输出的测试 EXE 路径，不能使用普通应用 EXE 或旧的测试 EXE。
3. 编译结束后串行运行以下基准，避免并发编译影响数据。以真实输出路径替换示例中的文件名。

```powershell
$binary = './src-tauri/target/release/deps/snap_desk_lib-实际哈希.exe'
foreach ($case in @(
  'resources::tests::perf_resource_validation',
  'clipboard::export::tests::perf_export_full_font',
  'ocr::tests::perf_ocr_cpu',
  'clipboard::export::tests::stress_4k_batch_exports'
)) {
  ./scripts/measure-native-test.ps1 -Binary $binary -Test $case -Label current
}
```

结果写到 `src-tauri/target/perf-results/`：标准输出、标准错误和 JSON 内存记录。
测量脚本每 100ms 读取进程内存高水位，不包括编译器、WebView 或主程序其他功能的占用。
OCR 每个分辨率测 4 次，第一个数字包含该尺寸的首次推理，后 3 次反映会话复用。
截图样本由固定字体绘制，不是真实屏幕；测试检查中英文和完整文本行数。
PDF 基准使用完整霞鹜文楷，限制短文本结果小于 1 MB，防止字体子集化静默回退。
生成的 `sample-text.pdf` 和 `sample-mixed.pdf` 用于独立提取文字、渲染检查。

4K 压力用例处理 7 张 3840×2160 图片，以及 2 万行文本；仅显式运行，
不应以其峰值解释常规单张截图的内存占用。

## 2026-10-16 实测记录

Windows，Intel Core i7-7700（4 核 8 线程），32 GB 内存，Rust 1.99，release 测试程序。
没有启动主窗口或 WebView。前后测量串行运行；系统负载、文件缓存会带来波动，
以下为该机器上的合成样本结果，不代表所有真实页面的识别速度或准确率。

| 项目 | 修改前 | 修改后 |
| --- | ---: | ---: |
| 30 行中文 PDF 大小（十进制 MB） | 26.51 MB | 0.497 MB |
| 30 行中文 PDF 耗时中位数（3 次） | 720 ms | 305 ms |
| 文本 + 1080p 图片 PDF 大小 | 26.70 MB | 0.687 MB |
| 文本 + 1080p 图片 PDF 耗时中位数 | 872 ms | 456 ms |
| 导出基准进程峰值工作集 | 132.2 MiB | 110.4 MiB |
| 1200×500 OCR 连续识别中位数（后 3 次） | 1133 ms | 940 ms |
| 1920×1080 OCR 连续识别中位数（后 3 次） | 2554 ms | 1935 ms |
| OCR 基准进程峰值工作集 | 771.7 MiB | 541.3 MiB |
| OCR 进程私有内存采样峰值 | 953.0 MiB | 688.2 MiB |

改动：启用 printpdf 0.7 的字体子集功能；OCR 限制每批 8 行、每会话最多 2 个推理线程。
字体仍使用用户选择/下载的完整文件，只有 PDF 内部嵌入内容被子集化。
OCR 仍复用缓存会话；对内存敏感的场景需注意首次识别后会保留模型和推理缓存。

7 张 4K 图片 + 2 万行文本压力用例通过，测试内部耗时 8.26 秒，峰值工作集 247.1 MiB。
资源检查没有修改：重复校验 3 个模型约 0.1–0.17 秒，完整字体读取和验证约 38–55 ms；
冷文件缓存可能更慢，暂未引入会导致外部修改不易被发现的校验缓存。
长图路径未修改，30 行文本 PNG 约 15–17 ms，文字 + 1080p 图片约 325–355 ms。

### 测试结果与边界

- 前端：20 个文件、116 项 Vitest 用例通过；5 项 Node 版本同步测试通过。
- Rust release 普通套件：134 通过、5 失败、6 默认忽略。5 个失败均来自前述缺失的真实长截图帧集。
- 显式运行：资源校验、完整字体导出、真实模型 OCR、4K 导出压力测试 4 项通过。
  其余两个忽略项为实机屏幕采集基准和外部真实长截图帧集，未运行。
- PDF 独立验证：pypdf 完整提取 30 行中英文；PyMuPDF 渲染文字页和图片页后已检查，无缺字或裁剪。
- Vue/TypeScript 类型检查、修改文件 Rust 格式检查、`git diff --check` 通过。
- 修复测试入口将 Node 测试误交给 Vitest 的问题，以及录屏默认尺寸断言仍使用旧值 28 的问题；
  生产默认值保持前后端现行的 20 像素、400ms、左上角。

完整 Rust 套件仍不能宣称全绿；补齐真实帧集后必须重跑对应 5 项测试。

此前记录的 OCR 性能数据使用静态链接引擎；切换动态 DLL 后需重新测量，不能作为新版性能保证。

## Windows 动态引擎验证（2026-10-16）

- `npm run check` 通过；Windows 依赖图启用 `load-dynamic` / `disable-linking`，未启用 `download-binaries`。
- 定向覆盖资源摘要、字体校验、引擎缺失可重试、归档/条目损坏不覆盖旧文件、仅提取白名单文件。
- 使用官方 x64 ZIP 在 `target/perf-assets` 安装 DLL，并通过真实中英文 OCR（1200×500、1920×1080 完整行数）及 4K 空白图断言。
- 本轮运行调试测试，数据不用于性能对比；未重新执行发布构建/打包，尚无新版主 EXE 大小。
- ARM64 仅校验官方包和清单，未实机加载；未启动浏览器验证设置页下载交互。
