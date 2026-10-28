# 模型许可审核材料

此目录保存固定 HF commit 对应的模型许可和署名材料：`en-zh-LICENSE.txt` / `zh-en-LICENSE.txt`，以 `LICENSE.txt` 随对应模型包分发。更新模型源版本时须同步核对模型卡、上游许可及署名。

两个固定版本的来源、许可摘要和核对类型见 [reviews.json](reviews.json)。当前记录标识为自动化来源核对，`humanReviewCompleted` 为 `false`，不能视为人工许可审核已完成。

在 `reviews.json` 中按方向加入记录，字段为：

- `sourceRevision`：经过审核的 40 位小写 HF commit。
- `sourceUrl`：`https://huggingface.co/Helsinki-NLP/opus-mt-<方向>/tree/<完整commit>`。
- `licenseSha256`：对应 LICENSE 文件的真实 SHA-256。
- `reviewedBy`：实际核对者及可追溯记录；自动化核对必须明确标识，并单独记录人工复核是否完成。

现有客户端契约要求 en-zh 为 Apache-2.0、zh-en 为 CC-BY-4.0；如果选定源版本不符合，先处理契约差异，不能仅修改 SPDX 字段使 CI 通过。许可正文须包含必要的来源和署名信息。

没有记录、revision 不符、许可缺失或摘要不符时，模型工作流会在安装 Torch 等依赖前失败。
