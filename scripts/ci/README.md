# 翻译资源独立构建

三条工作流分别负责引擎、模型和主程序。仅 Windows x64 CPU / CT2 4.6.0，模型仍按需下载；Worker 只随应用安装包更新。原始方案文档保留为设计依据。

## 首次启用顺序

1. 在 GitHub 仓库配置 `translation-engine-publish`、`translation-model-publish` 两个 Environment，启用 required reviewers 并限制可发布分支。**仅在 YAML 引用环境不会自动启用审批规则**。
2. 手动运行 Build Translation Engine，`publish=false`。下载 artifact，检查 `runtime-build.json` 的实际源码提交、编译选项、工具链、递归 DLL 依赖和 `licenses/`。脚本归档真实上游许可材料，但最终再分发许可完整性仍须人工审核。
3. 在无开发环境的 Windows x64 VM 运行 health、真实模型加载/翻译及离线验收。发布运行设置 `publish=true` 后在 Environment 审批处验收**该次构建 artifact**，再放行发布；每次源码、选项或依赖变化升级 `engine_revision`。不覆盖已发布 Release。
4. 将审核的引擎 tag 和 ZIP SHA-256 写入 `translation-engine.json`。当前 `sha256: null` 表示尚无真实受审产物，因此主程序和模型 CI 会明确失败。不要把同次网络下载的 `.sha256` 当作受审锁。
5. 按 `legal/translation/README.md` 补充真实模型 commit 对应的许可和审核记录。运行 Build Translation Models，先 `publish=false`；使用固定的 Python requirements 和 Windows Worker 完成既有 tokenizer/decode/翻译样本验证。模型与应用始终使用同一个仓库引擎锁，不接受自由输入的 engine URL/tag。
6. 审核模型质量和许可后运行 `publish=true`，审核该次 artifact。每个语言方向独立 Release，逐文件上传，使用 prerelease 且不设为 Latest。发布后自动匿名下载文件，复核大小和摘要；失败时调查此版本，不覆盖附件或启用 catalog。
7. 下载完整真实模型包后运行：

   ```powershell
   python scripts/ci/generate-model-catalog.py out/en-zh out/zh-en
   ```

   该命令验证本地文件与线上附件后写入 `src-tauri/assets/translation-models.json`，不会提交代码；在 PR 中审核生成的清单。没有真实模型时维持 `[]`。
8. 推送与应用版本一致的 `v*` 标签，主程序工作流下载锁定引擎、复核 catalog 和已发布模型文件，设置 `SNAP_DESK_REQUIRE_TRANSLATION=1` 后构建。保留测试版 NSIS、正式版 NSIS + MSI 规则。过渡期保留 standalone EXE，Release notes 明确它不包含本地翻译运行资源；完整便携 ZIP 未实现。

## 本地轻量验证

```powershell
python -m unittest discover -s scripts/ci -p 'test_*.py'
python scripts/ci/generate-model-catalog.py --check
node --check scripts/build-desktop.mjs
```

Python 辅助脚本使用 Python 3.11 标准库。runtime 校验器读取 x64 PE 普通导入和 delay-import 表，递归拒绝缺失的非系统 DLL，并在清理 PATH 的环境运行 health。CI 使用静态 MSVC CRT，避免依赖开发机 VC DLL；真实编译及干净机器模型加载依然是必要验收，health 不代表完整模型推理成功。

跨平台约定：三个工作流固定 Python UTF-8 输出；摘要和模型 tag 文件固定 ASCII/LF；许可文件由 `.gitattributes` 禁止自动换行转换，避免审核摘要在 Windows checkout 后变化。引擎下载先在目标同卷的临时目录验收，通过后才安装，失败可以直接重试。引擎编译前会运行脚本回归测试，覆盖编码、缓存解析、摘要换行、下载失败清理等已发现问题。

模型附件校验会对短暂 404/429/5xx 和连接错误最多尝试三次，但不会忽略摘要、大小或重定向违规。若发布后远程校验仍失败，Release 可能已经存在；先用 `verify-model-release.py <模型目录> --expected-tag <模型tag> --remote` 单独复核，不覆盖原附件。主程序和模型工作流仍需要真实引擎锁；模型工作流还需要真实许可审核记录，这些前置条件不能用测试数据填充。

模型下载仅允许固定的 `github.com/bitdan/snap-desk/releases/download/` 附件和最多三次 HTTPS `release-assets.githubusercontent.com` 跳转。该 CDN 于 2026-10-16 通过 `cli/cli` 的 `v2.81.0/gh_2.81.0_checksums.txt` 实际 302 确认；未来更换 CDN 须重新审核。在线 LLM 下载策略不受影响。

待执行的外部验收：GitHub hosted runner 原生编译、两方向真实模型转换、受保护环境设置、许可人工审核、NSIS/MSI 安装卸载及干净 Windows VM 离线翻译/内存/性能测试。本地静态通过不代表这些项目已完成。
