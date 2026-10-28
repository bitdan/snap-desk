# 本地翻译开发

截图、贴图及导入图片共用 OCR → CTranslate2 原生进程 → 原文 / 译文对照流程；截图文字翻译也支持编辑原文。默认使用本地服务，在线服务需显式选择与发送确认，本地失败不自动上传。

本地引擎目前支持英译中、中译英，每次至多 64 行、32,000 字节，每个待翻译片段至多 480 个源 token；超限报错，不静默截断。URL、Windows 路径、反引号代码、常见占位符和快捷键按片段保留，其余文本逐行翻译。尚未接入原图译文覆盖层、按文字框合并段落或长句自动切分。

## Windows x64 原生引擎

引擎、模型与主程序由三条流水线独立构建；发布顺序、版本锁及许可记录见 [CI 操作说明](../../../scripts/ci/README.md)。主程序构建不要求本机存在翻译 Worker，也不捆绑引擎。

开发引擎时，在 VS 2022 x64 Native Tools 环境中准备 CTranslate2 4.6.0 的 CPU 安装前缀，禁用 CUDA。仓库 CI 使用 RUY、静态 MSVC CRT，禁用 MKL、oneDNN、OpenBLAS 和 OpenMP；本地依赖须与运行库校验规则保持一致。最终用户无需构建工具或 Python。

从 snap-desk 根目录执行，将前缀替换为实际安装目录：

```powershell
cmake -S src-tauri/native/translator -B src-tauri/target/translator -A x64 -DCMAKE_PREFIX_PATH=C:/deps/ctranslate2
cmake --build src-tauri/target/translator --config Release
cmake --install src-tauri/target/translator --config Release --component translator-runtime --prefix "$PWD/src-tauri/translator-runtime"
```

CMake 固定 SentencePiece v0.2.0 和 nlohmann/json v3.11.3。运行目录需要 Worker、CT2、实际依赖 DLL 与第三方许可；不能依赖开发机 PATH。正式分发使用 CI 生成并校验的归档，应用只接受与内置引擎锁摘要一致的官方 ZIP，本地任意构建不能直接替换已锁定的安装包。

## 制作语言包

```powershell
python -m pip install -r scripts/translation-requirements.txt
python scripts/prepare-translation-model.py --direction en-zh --revision <40位HF提交> --worker src-tauri/translator-runtime/snap-translator-worker.exe --license-file <已审核许可及署名文件> --output src-tauri/target/models/en-zh
python scripts/prepare-translation-model.py --direction zh-en --revision <40位HF提交> --worker src-tauri/translator-runtime/snap-translator-worker.exe --license-file <已审核许可及署名文件> --output src-tauri/target/models/zh-en
```

脚本拒绝 main / tag，下载固定源 revision，使用 Transformers → CT2 INT8 转换；通过原生 Worker 分词、HF token ID 映射、目标解码及翻译样本检查后才产出目录。英译中注入 `>>cmn_Hans<<`，两方向均使用独立 source / target SPM，源 EOS 仅追加一次。

模型来源为 Helsinki-NLP/opus-mt-en-zh（Apache-2.0）与 opus-mt-zh-en（CC-BY-4.0）；固定版本及许可材料见 [许可说明](../../../legal/translation/README.md)。更换源 commit 时同步更新材料和审核记录。

在应用“识别与翻译”页选择方向并导入模型目录。导入仅复制固定模型文件白名单，不接受可执行程序；包内摘要校验不能代替来源审核。引擎和模型存于应用数据目录，下载 / 导入先暂存校验，再替换旧目录。

`src-tauri/assets/translation-models.json` 已包含两个方向的固定资源清单。更新清单使用 `scripts/ci/generate-model-catalog.py` 校验真实模型目录与线上附件；客户端使用随应用发布的内置清单，不接受前端传入下载地址或任意远端清单。
