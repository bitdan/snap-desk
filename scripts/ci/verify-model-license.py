"""将人工审核材料绑定到 HF commit 和许可文件摘要，防止旧审核被用于新模型。"""
import argparse
import hashlib
import json
import re
from pathlib import Path

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--direction", choices=["en-zh", "zh-en"], required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--package-revision", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[a-f0-9]{40}", args.revision) or not re.fullmatch(r"r[1-9][0-9]*", args.package_revision):
        parser.error("revision 必须为 40 位小写 SHA，package revision 必须为 r1/r2 等")
    root = Path("legal/translation")
    review = json.loads((root / "reviews.json").read_text(encoding="utf-8")).get(args.direction)
    license_file = root / f"{args.direction}-LICENSE.txt"
    if not review or review.get("sourceRevision") != args.revision or not license_file.is_file() or license_file.stat().st_size == 0:
        raise ValueError("缺少此 HF commit 的许可/署名审核记录及真实许可文件")
    sha = hashlib.sha256(license_file.read_bytes()).hexdigest()
    if review.get("licenseSha256") != sha or not review.get("reviewedBy") or not review.get("sourceUrl") == f"https://huggingface.co/Helsinki-NLP/opus-mt-{args.direction}/tree/{args.revision}":
        raise ValueError("许可材料摘要、审核人或来源不匹配")
    print("固定模型 revision 的许可审核材料已核对")
