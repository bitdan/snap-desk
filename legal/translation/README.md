# 模型许可审核材料

此目录故意不提供占位许可证。选择真实 HF commit 后，审核该版本模型卡、上游许可和署名，将完整材料保存为 `en-zh-LICENSE.txt` / `zh-en-LICENSE.txt`。它们会以 `LICENSE.txt` 随对应模型包分发。

已准备的两个固定版本、实际来源和核对范围见 [SOURCE-REVIEW.md](SOURCE-REVIEW.md)。当前记录由 Codex 完成自动化来源核对，明确标识尚无人工复核，不冒用维护者姓名。

在 `reviews.json` 中按方向加入记录，字段为：

- `sourceRevision`：经过审核的 40 位小写 HF commit。
- `sourceUrl`：`https://huggingface.co/Helsinki-NLP/opus-mt-<方向>/tree/<完整commit>`。
- `licenseSha256`：对应 LICENSE 文件的真实 SHA-256。
- `reviewedBy`：实际核对者及可追溯记录；自动化核对必须明确标识，并单独记录人工复核是否完成。

现有客户端契约要求 en-zh 为 Apache-2.0、zh-en 为 CC-BY-4.0；如果选定源版本不符合，先处理契约差异，不能仅修改 SPDX 字段使 CI 通过。许可正文须包含必要的来源和署名信息。

没有记录、revision 不符、许可缺失或摘要不符时，模型工作流会在安装 Torch 等依赖前失败。
