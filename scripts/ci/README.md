# 翻译资源独立构建

三条工作流分别负责翻译引擎、模型和主程序，当前目标为 Windows x64 CPU / CTranslate2 4.6.0。最终用户在“识别与翻译”设置中按需下载或导入引擎与语言包，主程序不捆绑翻译运行库。

## 当前仓库配置

- [translation-engine.json](translation-engine.json) 已配置 `translation-engine-v4.6.0-r1` 及 ZIP SHA-256；应用与模型流水线共用此锁，不接受任意引擎 URL / tag。
- [模型目录](../../src-tauri/assets/translation-models.json) 已包含英译中、中译英两个固定源版本的文件 URL、大小与摘要。
- [许可记录](../../legal/translation/reviews.json) 与两份许可材料已存在；记录属于自动化来源核对，人工复核字段仍为 `false`。

这些文件表示仓库配置状态，不保证远程附件始终可下载，也不代表干净机器上的完整推理或人工许可审核已经完成。

## 更新引擎、模型与应用

1. 在 GitHub 仓库配置 `translation-engine-publish`、`translation-model-publish` 两个 Environment，按发布要求设置 required reviewers 和可发布分支；YAML 引用环境不会自动配置审批规则。
2. 运行 Build Translation Engine，先使用 `publish=false`。核对该次 artifact 中的 `runtime-build.json`、实际源码提交、编译选项、工具链、递归 DLL 依赖和许可材料，在干净 Windows x64 环境检查 health 和真实模型推理。
3. 引擎源码、选项或依赖变化时递增 `engine_revision`。运行 `publish=true` 并审核该次 artifact 后发布，不覆盖旧 Release；将对应 tag 与 ZIP 摘要写入引擎锁。
4. 更新模型源 commit 时，同步核对 [模型许可材料](../../legal/translation/README.md)。运行 Build Translation Models，指定方向、40 位 `hf_revision` 与新的 `package_revision`，先使用 `publish=false`；流水线使用固定 Python 依赖和仓库引擎锁检查分词、解码及真实翻译样本。
5. 审核模型质量、许可与该次 artifact 后以 `publish=true` 发布。每个方向使用独立、不可变的 prerelease，逐文件上传并复核大小和摘要；失败时调查对应版本，不覆盖旧附件。
6. 使用完整模型目录生成应用内置清单：

   ```powershell
   python scripts/ci/generate-model-catalog.py out/en-zh out/zh-en
   ```

   生成器同时校验本地模型与线上附件，写入 `src-tauri/assets/translation-models.json`，不会提交代码。通过代码审查更新清单，不手工编造 URL、大小或摘要。
7. 推送与应用版本一致的 `v*` 标签。主程序流水线校验引擎锁、模型清单与远程附件后构建；Windows 测试版使用 NSIS，正式版生成 NSIS + MSI，并提供 standalone EXE。引擎和模型仍需在应用内安装。

## 本地校验

```powershell
python -m unittest discover -s scripts/ci -p 'test_*.py'
python scripts/ci/generate-model-catalog.py --check
node --check scripts/build-desktop.mjs
```

`generate-model-catalog.py --check` 仅检查内置清单结构；增加 `--remote` 才会下载并复核附件。单独复核某个模型发布使用 `verify-model-release.py <模型目录> --expected-tag <模型tag> --remote`。

CI 辅助脚本使用 Python 3.11 标准库；模型转换另需 `scripts/translation-requirements.txt`。运行库校验器读取 x64 PE 普通导入和 delay-import 表，递归拒绝缺失的非系统 DLL，在清理 PATH 后运行 health。CI 使用静态 MSVC CRT；health 成功不能替代真实模型加载与翻译检查。

三个工作流固定 Python UTF-8 输出，摘要和 tag 文件固定 ASCII / LF，许可材料由 `.gitattributes` 保持字节内容。引擎先在目标同卷暂存、校验，通过后才安装。

模型附件仅允许固定仓库的 GitHub Release 下载地址，以及白名单内最多三次 HTTPS CDN 重定向。短暂 404 / 429 / 5xx 和连接错误最多重试三次，不忽略摘要、大小或重定向违规。更换下载来源或 CDN 时同步核对客户端、发布脚本和对应测试。
