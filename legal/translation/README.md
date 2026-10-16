# 模型许可审核材料

此目录故意不提供占位许可证。选择真实 HF commit 后，审核该版本模型卡、上游许可和署名，将完整材料保存为 `en-zh-LICENSE.txt` / `zh-en-LICENSE.txt`。它们会以 `LICENSE.txt` 随对应模型包分发。

在 `reviews.json` 中按方向加入记录，字段为：

- `sourceRevision`：经过审核的 40 位小写 HF commit。
- `sourceUrl`：`https://huggingface.co/Helsinki-NLP/opus-mt-<方向>/tree/<完整commit>`。
- `licenseSha256`：对应 LICENSE 文件的真实 SHA-256。
- `reviewedBy`：审核人或可追溯的审核 PR。

现有客户端契约要求 en-zh 为 Apache-2.0、zh-en 为 CC-BY-4.0；如果选定源版本不符合，先处理契约差异，不能仅修改 SPDX 字段使 CI 通过。许可正文须包含必要的来源和署名信息。

没有记录、revision 不符、许可缺失或摘要不符时，模型工作流会在安装 Torch 等依赖前失败。
