# 本地翻译开发与验收

已接入源码：截图原图 OCR → 可编辑原文 → CTranslate2 原生进程 → 双语面板。
默认本地；在线服务是显式可选项，本地失败不自动上传。当前只支持英译中、中译英，
每次至多 64 行、32 KB，每个待翻译片段至多 480 个源 token，超限报错而不静默截断。
URL、Windows 路径、反引号代码、常见占位符和快捷键按片段原样保留；其他文本逐行翻译。
尚未实现按文字框合并段落和长句自动切分，代码/路径等复杂内容应检查译文。

## Windows x64 原生引擎

GitHub Actions 已拆为引擎、模型和主程序三条流水线。首次启用、版本锁与许可审核步骤见
[CI 操作说明](../../../scripts/ci/README.md)。引擎锁尚未填写真实摘要时，发布 CI 会阻止构建。

仓库约定不自动编译。以下命令由开发者在 VS 2022 x64 Native Tools 环境中执行。
先按 [CTranslate2 官方构建说明](https://opennmt.net/CTranslate2/installation.html)
将 **v4.6.0** 的 CPU 版本安装到单独前缀，禁用 CUDA；启用可用的 CPU INT8 后端，
确保 MKL/oneDNN/OpenMP 等实际依赖 DLL 与安装结果匹配。最终用户无需构建工具或 Python。

从 snap-desk 根目录执行（将前缀替换为实际绝对路径）：

```powershell
cmake -S src-tauri/native/translator -B src-tauri/target/translator -A x64 -DCMAKE_PREFIX_PATH=C:/deps/ctranslate2
cmake --build src-tauri/target/translator --config Release
cmake --install src-tauri/target/translator --config Release --component translator-runtime --prefix "$PWD/src-tauri/translator-runtime"
```

CMake 固定 SentencePiece v0.2.0 和 nlohmann/json v3.11.3。
核对运行目录包含 worker、CT2 及实际依赖 DLL、第三方许可和必要 VC++ runtime；
不得把开发机 PATH 当作用户运行前提。`desktop:build` / `desktop:package` 在发现
worker 时自动合并 `tauri.translation.conf.json`，将整个 runtime 目录复制进应用资源。
缺少 worker 时会明确警告，普通截图仍可构建。直接调用 Tauri CLI 时需自行传该 config。

## 制作语言包（开发/CI）

```powershell
python -m pip install -r scripts/translation-requirements.txt
python scripts/prepare-translation-model.py --direction en-zh --revision <40位HF提交> --worker src-tauri/translator-runtime/snap-translator-worker.exe --license-file <已审核许可及署名文件> --output src-tauri/target/models/en-zh
python scripts/prepare-translation-model.py --direction zh-en --revision <40位HF提交> --worker src-tauri/translator-runtime/snap-translator-worker.exe --license-file <已审核许可及署名文件> --output src-tauri/target/models/zh-en
```

脚本拒绝 main/tag，下载固定源 revision，采用 Transformers → CT2 INT8 路线。
只有原生 Worker 的源分词、HF token ID 映射、目标解码及真实翻译样本检查成功后才产出目录。
英译中注入 `>>cmn_Hans<<`，两方向都只追加一次源 EOS，使用独立 source/target SPM。
这些规则依据 [官方 Transformers 特殊 token 契约](https://opennmt.net/CTranslate2/guides/transformers.html#special-tokens-in-translation)。
模型来源为 Helsinki-NLP/opus-mt-en-zh（Apache-2.0）和 opus-mt-zh-en（CC-BY-4.0）；
打包者须审核指定 revision 的模型卡及署名要求。

在应用“识别与翻译”页选择方向并导入产出的目录。导入只复制固定模型文件白名单，
不接受可执行程序；文件 SHA-256 仅证明包内一致性，不证明作者可信。
原生引擎始终来自应用安装目录。下载/导入先暂存并完整校验，再替换旧目录。

`assets/translation-models.json` 当前有意为空：没有已经转换验收并发布的模型资源，
不能编造 URL、大小或摘要。发布者使用 `--base-url` 填入每个固定版本的 HTTPS 文件目录，
上传文件后，将两份已验证 pack.json 对象组成数组写入该清单，再构建客户端。
不能让客户端接受远端随意替换清单或前端传入下载地址。

## 发布门槛（本次未执行）

- 在无 Python、无开发机 PATH、无网络、无 API Key 的 Windows x64 上验证双向截图翻译。
- 真实模型转换、上述原生 golden 校验、人工质量评审，以及 100 次切换/翻译的内存测试。
- 检查取消、超时、Worker 崩溃重试、损坏文件导入、下载中断、语言包更新/卸载。
- 验证中文用户目录、多屏/DPI、空白图；确认原 OCR、截图保存、标注不受影响。
- 核对打包 DLL、VC++ runtime、模型和所有原生依赖许可。

本次仅进行静态检查与无需编译的定向测试，不能据此宣称离线端到端验收通过。
