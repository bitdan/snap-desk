"""用真实且完整的模型目录生成内置 catalog；生成前必须校验线上附件。"""
import argparse
import json
from pathlib import Path
from model_release import read_pack, validate, verify_remote

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directories", type=Path, nargs="*")
    parser.add_argument("--output", type=Path, default=Path("src-tauri/assets/translation-models.json"))
    parser.add_argument("--check", action="store_true", help="仅检查现有内置目录，允许空目录")
    parser.add_argument("--remote", action="store_true", help="检查时同时下载附件复核")
    args = parser.parse_args()
    if args.check:
        packs = json.loads(args.output.read_text(encoding="utf-8"))
        if not isinstance(packs, list):
            raise ValueError("catalog 必须为数组")
        for pack in packs:
            validate(pack)
    else:
        if not args.directories:
            parser.error("请提供真实模型目录")
        packs = [read_pack(directory) for directory in args.directories]
    if len({p["modelId"] for p in packs}) != len(packs):
        raise ValueError("模型方向重复")
    if not args.check or args.remote:
        for pack in packs:
            verify_remote(pack)
    if not args.check:
        args.output.write_text(json.dumps(packs, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"已验证 {len(packs)} 个模型清单")
